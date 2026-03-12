use std::path::Path;

use rayon::prelude::*;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tracing::{debug, info, warn};
use walkdir::WalkDir;

use crate::index_store::ffi::IndexStoreLib;
use crate::index_store::reader;
use crate::storage::{self, queries, StorageError};
use crate::tree_sitter::TreeSitterParser;

#[derive(Debug, Error)]
pub enum PipelineError {
    #[error("storage error: {0}")]
    Storage(#[from] StorageError),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("parse error: {0}")]
    Parse(#[from] crate::tree_sitter::parser::ParseError),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Result of indexing a project.
#[derive(Debug)]
pub struct IndexResult {
    pub files_scanned: usize,
    pub files_indexed: usize,
    pub nodes_added: usize,
    pub edges_added: usize,
    /// Which indexing strategy was used.
    pub strategy: IndexStrategy,
}

/// Which indexing backend was used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexStrategy {
    /// Compiler-accurate data from Xcode Index Store.
    IndexStore,
    /// Fallback: tree-sitter-swift AST parsing.
    TreeSitter,
    /// Index Store for structure, tree-sitter for files not in the store.
    Hybrid,
}

/// Index all Swift files in the given directory.
///
/// Tries Index Store first (if `index_store_path` is provided or auto-detected),
/// then falls back to tree-sitter for any remaining files.
pub fn index_directory(
    db_path: &Path,
    source_root: &Path,
    force: bool,
) -> Result<IndexResult, PipelineError> {
    index_directory_with_store(db_path, source_root, force, None)
}

/// Index with an explicit Index Store path.
pub fn index_directory_with_store(
    db_path: &Path,
    source_root: &Path,
    force: bool,
    index_store_path: Option<&Path>,
) -> Result<IndexResult, PipelineError> {
    let conn = storage::open_db(db_path)?;

    // 1. Scan for .swift files
    let swift_files: Vec<_> = WalkDir::new(source_root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "swift"))
        .filter(|e| {
            let path = e.path().to_string_lossy();
            !path.contains("/.build/")
                && !path.contains("/Pods/")
                && !path.contains("/Generated/")
                && !path.contains("/DerivedData/")
        })
        .map(|e| e.into_path())
        .collect();

    let files_scanned = swift_files.len();

    // 2. Try Index Store first
    let mut index_store_files: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut nodes_added = 0;
    let mut edges_added = 0;
    let mut used_index_store = false;

    if let Some(store_path) = index_store_path.or_else(|| auto_detect_index_store(source_root)) {
        match try_index_store(&conn, store_path, force) {
            Ok((n, e, files)) => {
                nodes_added = n;
                edges_added = e;
                index_store_files = files;
                used_index_store = true;
                info!(
                    "Index Store: {} nodes, {} edges from {} files",
                    nodes_added,
                    edges_added,
                    index_store_files.len()
                );
            }
            Err(e) => {
                warn!("Index Store unavailable, falling back to tree-sitter: {e}");
            }
        }
    }

    // 3. Tree-sitter for remaining files (or all files if no Index Store)
    let files_for_treesitter: Vec<_> = if used_index_store {
        swift_files
            .into_iter()
            .filter(|p| !index_store_files.contains(&p.to_string_lossy().to_string()))
            .collect()
    } else {
        swift_files
    };

    // Filter by hash for incremental reindex
    let files_to_index: Vec<_> = if force {
        files_for_treesitter
    } else {
        files_for_treesitter
            .into_iter()
            .filter(|path| {
                let content = std::fs::read(path).unwrap_or_default();
                let hash = format!("{:x}", Sha256::digest(&content));
                let path_str = path.to_string_lossy();

                let stored_hash: Option<String> = conn
                    .query_row(
                        "SELECT hash FROM files WHERE path = ?1",
                        [path_str.as_ref()],
                        |row| row.get(0),
                    )
                    .ok();

                stored_hash.as_deref() != Some(&hash)
            })
            .collect()
    };

    // Parse files in parallel with tree-sitter
    let parse_results: Vec<_> = files_to_index
        .par_iter()
        .filter_map(|path| {
            let mut parser = TreeSitterParser::new().ok()?;
            let result = parser.parse_file(path).ok()?;
            let content = std::fs::read(path).ok()?;
            let hash = format!("{:x}", Sha256::digest(&content));
            Some((path.clone(), hash, result))
        })
        .collect();

    // Store tree-sitter results in a single transaction
    let ts_files_indexed = parse_results.len();

    conn.execute("BEGIN TRANSACTION", [])?;

    for (path, hash, parse_result) in &parse_results {
        let path_str = path.to_string_lossy();

        // Delete old data for this file
        conn.execute("DELETE FROM edges WHERE file = ?1", [path_str.as_ref()])?;
        conn.execute("DELETE FROM nodes WHERE file = ?1", [path_str.as_ref()])?;

        // Insert file record
        queries::upsert_file(&conn, &path_str, hash, parse_result.nodes.len() as u32)?;

        for node in &parse_result.nodes {
            queries::upsert_node(&conn, node)?;
            nodes_added += 1;
        }

        for edge in &parse_result.edges {
            queries::insert_edge(&conn, edge)?;
            edges_added += 1;
        }
    }

    conn.execute("COMMIT", [])?;

    let files_indexed = index_store_files.len() + ts_files_indexed;
    let strategy = match (used_index_store, ts_files_indexed > 0) {
        (true, true) => IndexStrategy::Hybrid,
        (true, false) => IndexStrategy::IndexStore,
        _ => IndexStrategy::TreeSitter,
    };

    debug!(
        "Indexing complete ({strategy:?}): {files_indexed} files, {nodes_added} nodes, {edges_added} edges"
    );

    Ok(IndexResult {
        files_scanned,
        files_indexed,
        nodes_added,
        edges_added,
        strategy,
    })
}

/// Try to read data from Index Store and write to the database.
/// Returns (nodes_added, edges_added, set of file paths covered).
fn try_index_store(
    conn: &rusqlite::Connection,
    store_path: &Path,
    _force: bool,
) -> Result<(usize, usize, std::collections::HashSet<String>), Box<dyn std::error::Error>> {
    let lib = IndexStoreLib::load()?;
    let data = reader::read_index_store(&lib, store_path)?;

    let mut nodes_added = 0;
    let mut edges_added = 0;

    conn.execute("BEGIN TRANSACTION", [])?;

    for node in &data.nodes {
        // Upsert file record for files we see
        let hash = "indexstore"; // Placeholder — Index Store doesn't provide file hashes
        queries::upsert_file(conn, &node.location.file, hash, 0)?;

        queries::upsert_node(conn, node)?;
        nodes_added += 1;
    }

    for edge in &data.edges {
        queries::insert_edge(conn, edge)?;
        edges_added += 1;
    }

    conn.execute("COMMIT", [])?;

    let files: std::collections::HashSet<String> = data.file_nodes.keys().cloned().collect();

    Ok((nodes_added, edges_added, files))
}

/// Try to auto-detect the Index Store path from DerivedData.
fn auto_detect_index_store(source_root: &Path) -> Option<&Path> {
    // For now, rely on project detection to provide this.
    // The `project::detect_project` already finds the Index Store path.
    // This will be wired up when called from the CLI/MCP.
    let _ = source_root;
    None
}
