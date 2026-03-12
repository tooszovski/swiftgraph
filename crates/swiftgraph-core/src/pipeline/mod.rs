use std::path::Path;

use rayon::prelude::*;
use sha2::{Digest, Sha256};
use thiserror::Error;
use walkdir::WalkDir;

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
}

/// Index all Swift files in the given directory.
pub fn index_directory(
    db_path: &Path,
    source_root: &Path,
    force: bool,
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

    // 2. Hash files and find changed ones
    let files_to_index: Vec<_> = if force {
        swift_files
    } else {
        swift_files
            .into_iter()
            .filter(|path| {
                let content = std::fs::read(path).unwrap_or_default();
                let hash = format!("{:x}", Sha256::digest(&content));
                let path_str = path.to_string_lossy();

                // Check if hash changed
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

    // 3. Parse files in parallel with tree-sitter
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

    // 4. Store results in a single transaction
    let files_indexed = parse_results.len();
    let mut nodes_added = 0;
    let mut edges_added = 0;

    conn.execute("BEGIN TRANSACTION", [])?;

    for (path, hash, parse_result) in &parse_results {
        let path_str = path.to_string_lossy();

        // Delete old data for this file
        conn.execute("DELETE FROM edges WHERE file = ?1", [path_str.as_ref()])?;
        conn.execute("DELETE FROM nodes WHERE file = ?1", [path_str.as_ref()])?;

        // Insert file record
        queries::upsert_file(&conn, &path_str, hash, parse_result.nodes.len() as u32)?;

        // Insert nodes
        for node in &parse_result.nodes {
            queries::upsert_node(&conn, node)?;
            nodes_added += 1;
        }

        // Insert edges
        for edge in &parse_result.edges {
            queries::insert_edge(&conn, edge)?;
            edges_added += 1;
        }
    }

    conn.execute("COMMIT", [])?;

    Ok(IndexResult {
        files_scanned,
        files_indexed,
        nodes_added,
        edges_added,
    })
}
