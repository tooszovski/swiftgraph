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
    /// Swift files found under the root after include/exclude filtering.
    pub files_scanned: usize,
    /// Files whose data was (re)written in this run.
    pub files_indexed: usize,
    /// Nodes written.
    pub nodes_added: usize,
    /// Edges written.
    pub edges_added: usize,
    /// Which indexing strategy was used.
    pub strategy: IndexStrategy,
    /// Nodes enriched by swift-syntax (0 when the parser is unavailable).
    pub nodes_enriched: usize,
}

/// Whether the pipeline enriches tree-sitter declarations with swift-syntax.
#[derive(Debug, Clone, Default)]
pub enum SwiftSyntaxMode {
    /// Discover `swiftgraph-parser` and use it if the handshake passes.
    #[default]
    Auto,
    /// Never run swift-syntax.
    Disabled,
    /// Use this (already verified) parser.
    Parser(crate::swift_syntax::SwiftSyntaxParser),
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

impl IndexStrategy {
    /// Stable string form, stored in the `meta` table and shown by `status`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::IndexStore => "index-store",
            Self::TreeSitter => "tree-sitter",
            Self::Hybrid => "hybrid",
        }
    }

    /// Whether Index Store data contributed to the graph.
    pub fn uses_index_store(&self) -> bool {
        matches!(self, Self::IndexStore | Self::Hybrid)
    }
}

/// `meta` key holding the [`IndexStrategy`] of the last indexing run.
pub const META_INDEX_STRATEGY: &str = "index_strategy";
/// `meta` key identifying the backend data source (`tree-sitter` or
/// `index-store:<path>`). A change forces a full rebuild so USR-based and
/// `ts::`-based node IDs never mix in one database.
pub const META_INDEX_SOURCE: &str = "index_source";

/// Index all Swift files in the given directory.
///
/// The Index Store is resolved with [`crate::project::resolve_index_store`]
/// (config `index_store_path`, then auto-detection); files it does not cover
/// fall back to tree-sitter.
pub fn index_directory(
    db_path: &Path,
    source_root: &Path,
    force: bool,
) -> Result<IndexResult, PipelineError> {
    let store = crate::project::resolve_index_store(source_root);
    index_directory_with_store(db_path, source_root, force, store.as_deref())
}

/// Index with an explicit Index Store path (`None` = tree-sitter only).
pub fn index_directory_with_store(
    db_path: &Path,
    source_root: &Path,
    force: bool,
    index_store_path: Option<&Path>,
) -> Result<IndexResult, PipelineError> {
    index_directory_with_options(
        db_path,
        source_root,
        force,
        index_store_path,
        &SwiftSyntaxMode::Auto,
    )
}

/// Index with an explicit Index Store path and swift-syntax mode.
pub fn index_directory_with_options(
    db_path: &Path,
    source_root: &Path,
    force: bool,
    index_store_path: Option<&Path>,
    swift_syntax: &SwiftSyntaxMode,
) -> Result<IndexResult, PipelineError> {
    let _span = info_span!("index_directory", root = %source_root.display()).entered();
    // Index Store paths are absolute; canonicalize so tree-sitter paths match them.
    let canonical_root = source_root
        .canonicalize()
        .unwrap_or_else(|_| source_root.to_path_buf());
    let source_root = canonical_root.as_path();
    let conn = storage::open_db(db_path)?;

    // Read the Index Store up front so we know which backend this run uses.
    let store_data = index_store_path.and_then(|store_path| {
        match IndexStoreLib::load()
            .map_err(|e| e.to_string())
            .and_then(|lib| reader::read_index_store(&lib, store_path).map_err(|e| e.to_string()))
        {
            Ok(data) => Some((store_path, data)),
            Err(e) => {
                warn!("Index Store unavailable, falling back to tree-sitter: {e}");
                None
            }
        }
    });

    let source = match &store_data {
        Some((p, _)) => format!("index-store:{}", p.display()),
        None => "tree-sitter".to_string(),
    };
    let previous_source = queries::get_meta(&conn, META_INDEX_SOURCE)?;
    let force = force
        || match previous_source.as_deref() {
            Some(prev) if prev != source => {
                info!("Index backend changed ({prev} -> {source}), rebuilding database");
                true
            }
            _ => false,
        };

    // On force reindex, start from an empty graph (FTS is kept in sync by triggers)
    if force {
        conn.execute_batch("DELETE FROM edges; DELETE FROM nodes; DELETE FROM files;")?;
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
    let scanned_paths: std::collections::HashSet<String> = swift_files
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();

    // 2. Try Index Store first
    let mut index_store_files: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut nodes_added = 0;
    let mut edges_added = 0;
    let mut used_index_store = false;

    if let Some((_, data)) = &store_data {
        match write_index_store(&conn, data) {
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
                warn!("Failed to store Index Store data, falling back to tree-sitter: {e}");
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
    let files_for_treesitter_empty = files_for_treesitter.is_empty();

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
        queries::delete_file_data(&conn, &path_str)?;

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

    // Purge files that no longer exist (or are no longer included)
    let purged = purge_missing_files(&conn, &scanned_paths, &index_store_files)?;
    if purged > 0 {
        info!("Purged {purged} deleted files from the index");
    }

    conn.execute("COMMIT", [])?;

    // 4. Optional swift-syntax enrichment (one batch parser process)
    let parser = match swift_syntax {
        SwiftSyntaxMode::Auto if !parse_results.is_empty() => {
            crate::swift_syntax::SwiftSyntaxParser::discover()
        }
        SwiftSyntaxMode::Parser(p) => Some(p.clone()),
        _ => None,
    };
    let mut nodes_enriched = 0;
    if let Some(parser) = parser {
        let files: Vec<std::path::PathBuf> =
            parse_results.iter().map(|(p, _, _)| p.clone()).collect();
        nodes_enriched = enrich_with_swift_syntax(&conn, &parser, &files)?;
        if nodes_enriched > 0 {
            info!("swift-syntax enriched {nodes_enriched} nodes");
        }
    }

    // 5. Resolve name:: edge targets to real node IDs (creates cross-file edges)
    let resolved = resolve_name_edges(&conn)?;
    if resolved > 0 {
        info!("Resolved {resolved} call edges to real targets");
        edges_added += resolved;
    }

    let files_indexed = index_store_files.len() + ts_files_indexed;
    let ts_files_present = !files_for_treesitter_empty;
    let strategy = match (used_index_store, ts_files_present) {
        (true, true) => IndexStrategy::Hybrid,
        (true, false) => IndexStrategy::IndexStore,
        _ => IndexStrategy::TreeSitter,
    };
    let recorded_source = if used_index_store {
        source.as_str()
    } else {
        "tree-sitter"
    };
    queries::set_meta(&conn, META_INDEX_SOURCE, recorded_source)?;
    queries::set_meta(&conn, META_INDEX_STRATEGY, strategy.as_str())?;

    debug!(
        "Indexing complete ({strategy:?}): {files_indexed} files, {nodes_added} nodes, {edges_added} edges"
    );

    Ok(IndexResult {
        files_scanned,
        files_indexed,
        nodes_added,
        edges_added,
        strategy,
        nodes_enriched,
    })
}

/// Write Index Store data to the database.
/// Returns (nodes_added, edges_added, set of file paths covered).
fn write_index_store(
    conn: &rusqlite::Connection,
    data: &reader::IndexStoreData,
) -> Result<(usize, usize, std::collections::HashSet<String>), PipelineError> {
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

/// Delete index data for files recorded in the DB that were neither scanned
/// from disk nor provided by the Index Store in this run.
fn purge_missing_files(
    conn: &rusqlite::Connection,
    scanned: &std::collections::HashSet<String>,
    from_index_store: &std::collections::HashSet<String>,
) -> Result<usize, PipelineError> {
    let known: Vec<String> = {
        let mut stmt = conn.prepare("SELECT path FROM files")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.collect::<Result<_, _>>()?
    };
    let mut purged = 0;
    for path in known {
        if scanned.contains(&path) || from_index_store.contains(&path) {
            continue;
        }
        // The file is gone, so edges from other files into it are dangling too.
        conn.execute(
            "DELETE FROM edges WHERE target IN (SELECT id FROM nodes WHERE file = ?1)",
            [&path],
        )?;
        queries::delete_file_data(conn, &path)?;
        conn.execute("DELETE FROM files WHERE path = ?1", [&path])?;
        purged += 1;
    }
    Ok(purged)
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
                    edge.line.unwrap_or(0),
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

/// Enrich tree-sitter nodes of `files` with swift-syntax data: attributes,
/// doc comments, access level and signature for declarations and their
/// nested members, and attributes of imports (missing imports are added).
///
/// Runs one parser process for all files and writes in one transaction.
/// Declarations are matched by file, name and the nearest line (ties broken
/// by line and id, so the result is deterministic). Parser failures only
/// skip enrichment; they never fail indexing.
fn enrich_with_swift_syntax(
    conn: &rusqlite::Connection,
    parser: &crate::swift_syntax::SwiftSyntaxParser,
    files: &[std::path::PathBuf],
) -> Result<usize, PipelineError> {
    use crate::swift_syntax::Declaration;

    let results = match parser.parse_batch(files) {
        Ok(r) => r,
        Err(e) => {
            warn!("swift-syntax enrichment skipped: {e}");
            return Ok(0);
        }
    };

    let tx = conn.unchecked_transaction()?;
    let mut enriched = 0;
    let mut failed = 0;
    {
        let mut find = tx.prepare(
            "SELECT id FROM nodes WHERE file = ?1 AND name = ?2 AND ABS(line - ?3) <= 2
             ORDER BY ABS(line - ?3), line, id LIMIT 1",
        )?;
        let mut update = tx.prepare(
            "UPDATE nodes SET attributes = COALESCE(?1, attributes),
                              doc_comment = COALESCE(?2, doc_comment),
                              access_level = COALESCE(?3, access_level),
                              signature = COALESCE(?4, signature)
             WHERE id = ?5",
        )?;
        let mut find_import =
            tx.prepare("SELECT id FROM nodes WHERE file = ?1 AND kind = 'import' AND name = ?2 ORDER BY line, id LIMIT 1")?;

        fn access(level: &str) -> Option<&'static str> {
            Some(match level {
                "open" => "Open",
                "public" => "Public",
                "package" => "Package",
                "internal" => "Internal",
                "fileprivate" => "FilePrivate",
                "private" => "Private",
                _ => return None,
            })
        }

        for (path, result) in files.iter().zip(results) {
            let result = match result {
                Ok(r) => r,
                Err(e) => {
                    failed += 1;
                    debug!("swift-syntax skipped {e}");
                    continue;
                }
            };
            let file = path.to_string_lossy();

            let mut stack: Vec<&Declaration> = result.declarations.iter().collect();
            while let Some(decl) = stack.pop() {
                if let Some(members) = &decl.members {
                    stack.extend(members.iter());
                }
                let node_id: Option<String> = find
                    .query_row(rusqlite::params![file, decl.name, decl.line], |r| r.get(0))
                    .ok();
                let Some(node_id) = node_id else { continue };
                let attrs = (!decl.attributes.is_empty())
                    .then(|| serde_json::to_string(&decl.attributes).unwrap_or_default());
                let level = decl.access_level.as_deref().and_then(access);
                if attrs.is_none()
                    && decl.doc_comment.is_none()
                    && level.is_none()
                    && decl.signature.is_none()
                {
                    continue;
                }
                update.execute(rusqlite::params![
                    attrs,
                    decl.doc_comment,
                    level,
                    decl.signature,
                    node_id
                ])?;
                enriched += 1;
            }

            for import in &result.imports {
                let existing: Option<String> = find_import
                    .query_row(rusqlite::params![file, import.name], |r| r.get(0))
                    .ok();
                match existing {
                    Some(id) if !import.attributes.is_empty() => {
                        let attrs = serde_json::to_string(&import.attributes).unwrap_or_default();
                        update.execute(rusqlite::params![
                            Some(attrs),
                            None::<String>,
                            None::<String>,
                            None::<String>,
                            id
                        ])?;
                        enriched += 1;
                    }
                    Some(_) => {}
                    None => {
                        let line = import.line.max(1);
                        let node = crate::graph::GraphNode {
                            id: format!("ts::{file}::{}::{}", import.name, line - 1),
                            name: import.name.clone(),
                            qualified_name: import.name.clone(),
                            kind: crate::graph::SymbolKind::Import,
                            sub_kind: None,
                            location: crate::graph::Location {
                                file: file.to_string(),
                                line,
                                column: 1,
                                end_line: Some(line),
                                end_column: None,
                            },
                            signature: Some(format!("import {}", import.name)),
                            attributes: import.attributes.clone(),
                            access_level: crate::graph::AccessLevel::Internal,
                            container_usr: None,
                            doc_comment: None,
                            metrics: None,
                        };
                        queries::upsert_node(&tx, &node)?;
                        enriched += 1;
                    }
                }
            }
        }
    }
    tx.commit()?;

    if failed > 0 {
        warn!(
            "swift-syntax could not parse {failed} of {} files",
            files.len()
        );
    }
    Ok(enriched)
}
