use std::path::Path;

use rayon::prelude::*;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tracing::{debug, info, info_span, warn};
use walkdir::WalkDir;

use crate::config::Config;
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
    let _span = info_span!("index_directory", root = %source_root.display()).entered();
    let conn = storage::open_db(db_path)?;

    // On force reindex, rebuild FTS content sync to avoid corruption
    if force {
        let _ = conn.execute_batch(
            "DELETE FROM nodes; DELETE FROM edges; DELETE FROM files;
             INSERT INTO node_fts(node_fts) VALUES('rebuild');",
        );
        // Rebuild trigram table if it exists
        let _ = conn.execute_batch("INSERT INTO node_trigram(node_trigram) VALUES('rebuild');");
    }

    // Load config for include/exclude globs
    let config = Config::load(source_root);
    let include_set = config.include_globset();
    let exclude_set = config.exclude_globset();

    // 1. Scan for .swift files
    let swift_files: Vec<_> = WalkDir::new(source_root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "swift"))
        .filter(|e| {
            let path = e.path();
            // Use config globs for filtering
            let relative = path.strip_prefix(source_root).unwrap_or(path);
            config.should_include(relative, &include_set, &exclude_set)
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

    // 4. Optional swift-syntax enrichment (if parser binary available)
    if let Some(parser_path) = crate::swift_syntax::find_parser() {
        let enriched = enrich_with_swift_syntax(&conn, &parser_path, &parse_results)?;
        if enriched > 0 {
            info!("swift-syntax enriched {enriched} nodes with attributes/doc-comments");
        }
    }

    // 5. Resolve name:: edge targets to real node IDs (creates cross-file edges)
    let resolved = resolve_name_edges(&conn)?;
    if resolved > 0 {
        info!("Resolved {resolved} call edges to real targets");
        edges_added += resolved;
    }

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

/// Resolve `name::` prefixed edge targets to real node IDs.
///
/// After tree-sitter parsing, call edges use `name::functionName` as target.
/// This pass finds all such edges, looks up matching nodes by name, and creates
/// real edges to the resolved targets. The unresolved `name::` edges are then deleted.
fn resolve_name_edges(conn: &rusqlite::Connection) -> Result<usize, PipelineError> {
    struct UnresolvedEdge {
        source: String,
        target: String,
        kind: String,
        file: Option<String>,
        line: Option<u32>,
        col: Option<u32>,
        is_implicit: bool,
    }

    // Collect all unresolved edges
    let mut stmt = conn.prepare(
        "SELECT source, target, kind, file, line, col, is_implicit FROM edges WHERE target LIKE 'name::%'",
    )?;
    let unresolved: Vec<UnresolvedEdge> = stmt
        .query_map([], |row| {
            Ok(UnresolvedEdge {
                source: row.get(0)?,
                target: row.get(1)?,
                kind: row.get(2)?,
                file: row.get(3)?,
                line: row.get(4)?,
                col: row.get(5)?,
                is_implicit: row.get(6)?,
            })
        })?
        .filter_map(|r| r.ok())
        .collect();

    if unresolved.is_empty() {
        return Ok(0);
    }

    // Build a lookup of name → [node IDs] from all indexed nodes
    let mut name_to_ids: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    let mut node_stmt = conn.prepare("SELECT id, name FROM nodes WHERE kind IN ('function', 'method', 'property', 'class', 'struct', 'enum', 'protocol', 'typeAlias')")?;
    let rows = node_stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    for row in rows.flatten() {
        name_to_ids.entry(row.1).or_default().push(row.0);
    }

    conn.execute("BEGIN TRANSACTION", [])?;

    // Delete all unresolved name:: edges
    conn.execute("DELETE FROM edges WHERE target LIKE 'name::%'", [])?;

    let mut resolved = 0;
    let mut insert_stmt = conn.prepare(
        "INSERT OR IGNORE INTO edges (source, target, kind, file, line, col, is_implicit) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?;

    for edge in &unresolved {
        let name = edge.target.strip_prefix("name::").unwrap_or(&edge.target);
        if let Some(target_ids) = name_to_ids.get(name) {
            for target_id in target_ids {
                // Skip self-edges (calling yourself)
                if *target_id == edge.source {
                    continue;
                }
                insert_stmt.execute(rusqlite::params![
                    edge.source,
                    target_id,
                    edge.kind,
                    edge.file,
                    edge.line,
                    edge.col,
                    edge.is_implicit
                ])?;
                resolved += 1;
            }
        }
        // If no match found, the edge is silently dropped (SDK functions, etc.)
    }

    conn.execute("COMMIT", [])?;

    Ok(resolved)
}

/// Enrich tree-sitter-parsed nodes with swift-syntax data (attributes, doc comments, signatures).
fn enrich_with_swift_syntax(
    conn: &rusqlite::Connection,
    parser_path: &std::path::Path,
    parse_results: &[(
        std::path::PathBuf,
        String,
        crate::tree_sitter::parser::ParseResult,
    )],
) -> Result<usize, PipelineError> {
    let mut enriched_count = 0;

    for (path, _hash, _ts_result) in parse_results {
        let syntax_result = match crate::swift_syntax::parse_file(parser_path, path) {
            Ok(r) => r,
            Err(e) => {
                debug!("swift-syntax skipped {}: {e}", path.display());
                continue;
            }
        };

        // Match declarations by name+line to existing nodes and update attributes/doc_comment
        for decl in &syntax_result.declarations {
            let path_str = path.to_string_lossy();
            // Find matching node by name and approximate line
            let matching_node = conn
                .query_row(
                    "SELECT id FROM nodes WHERE name = ?1 AND file = ?2 AND ABS(line - ?3) <= 2 LIMIT 1",
                    rusqlite::params![decl.name, path_str.as_ref(), decl.line],
                    |row| row.get::<_, String>(0),
                )
                .ok();

            if let Some(node_id) = matching_node {
                let mut updated = false;

                // Enrich attributes if swift-syntax found more
                if !decl.attributes.is_empty() {
                    let attrs_json = serde_json::to_string(&decl.attributes).unwrap_or_default();
                    conn.execute(
                        "UPDATE nodes SET attributes = ?1 WHERE id = ?2",
                        rusqlite::params![attrs_json, node_id],
                    )?;
                    updated = true;
                }

                // Enrich doc comment
                if let Some(ref doc) = decl.doc_comment {
                    conn.execute(
                        "UPDATE nodes SET doc_comment = ?1 WHERE id = ?2",
                        rusqlite::params![doc, node_id],
                    )?;
                    updated = true;
                }

                // Enrich access level
                if let Some(ref access) = decl.access_level {
                    let level = match access.as_str() {
                        "open" => "Open",
                        "public" => "Public",
                        "package" => "Package",
                        "internal" => "Internal",
                        "fileprivate" => "FilePrivate",
                        "private" => "Private",
                        _ => "Internal",
                    };
                    conn.execute(
                        "UPDATE nodes SET access_level = ?1 WHERE id = ?2",
                        rusqlite::params![level, node_id],
                    )?;
                    updated = true;
                }

                // Enrich signature
                if let Some(ref sig) = decl.signature {
                    conn.execute(
                        "UPDATE nodes SET signature = ?1 WHERE id = ?2",
                        rusqlite::params![sig, node_id],
                    )?;
                    updated = true;
                }

                if updated {
                    enriched_count += 1;
                }
            }
        }
    }

    Ok(enriched_count)
}

/// Try to auto-detect the Index Store path from DerivedData.
fn auto_detect_index_store(source_root: &Path) -> Option<&Path> {
    // For now, rely on project detection to provide this.
    // The `project::detect_project` already finds the Index Store path.
    // This will be wired up when called from the CLI/MCP.
    let _ = source_root;
    None
}
