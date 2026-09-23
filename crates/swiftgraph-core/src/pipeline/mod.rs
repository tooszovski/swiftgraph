mod resolve;

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
    /// Why an Index Store that exists was not used, if any.
    pub index_store_note: Option<String>,
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
/// `meta` key explaining why an Index Store that exists was not used.
pub const META_INDEX_STORE_NOTE: &str = "index_store_note";
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
    let (store, note) = crate::project::resolve_index_store_with_note(source_root);
    let result = index_directory_with_store(db_path, source_root, force, store.as_deref())?;
    // Record why an existing store was not used, for `status`
    let conn = storage::open_db(db_path)?;
    match &note {
        Some(n) => queries::set_meta(&conn, META_INDEX_STORE_NOTE, n)?,
        None => {
            conn.execute("DELETE FROM meta WHERE key = ?1", [META_INDEX_STORE_NOTE])?;
        }
    }
    Ok(IndexResult {
        index_store_note: note,
        ..result
    })
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
        conn.execute_batch(
            "DELETE FROM edges; DELETE FROM name_refs; DELETE FROM member_types; DELETE FROM nodes; DELETE FROM files;",
        )?;
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

    // Only project sources: the store also describes SPM checkouts,
    // DerivedSources and files excluded by the config.
    let store_data = store_data.map(|(path, data)| {
        (
            path,
            restrict_to_project(data, source_root, &config, &include_set, &exclude_set),
        )
    });

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
    let (covered_files, files_for_treesitter): (Vec<_>, Vec<_>) = if used_index_store {
        swift_files
            .into_iter()
            .partition(|p| index_store_files.contains(&p.to_string_lossy().to_string()))
    } else {
        (Vec::new(), swift_files)
    };

    // Files the store covers keep its USR nodes and edges; tree-sitter adds
    // what the store lacks (attributes, access, signature, extent, imports).
    let stitched = stitch_index_store_nodes(&conn, &covered_files)?;
    if stitched > 0 {
        info!("tree-sitter details added to {stitched} Index Store nodes");
    }
    let files_for_treesitter_empty = files_for_treesitter.is_empty();

    // Filter by hash for incremental reindex
    let candidates: std::collections::HashSet<std::path::PathBuf> =
        files_for_treesitter.iter().cloned().collect();
    let mut files_to_index: Vec<_> = if force {
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

    // Unchanged files whose call edges point into changed files must be
    // resolved again: node IDs contain line numbers.
    let old_names = if force {
        std::collections::HashSet::new()
    } else {
        let (dependents, names) = incoming_dependents(&conn, &files_to_index)?;
        files_to_index.extend(dependents.into_iter().filter(|p| candidates.contains(p)));
        names
    };

    let parse = |paths: &[std::path::PathBuf]| -> Vec<_> {
        paths
            .par_iter()
            .filter_map(|path| {
                let mut parser = TreeSitterParser::new().ok()?;
                let result = parser.parse_file(path).ok()?;
                let content = std::fs::read(path).ok()?;
                let hash = format!("{:x}", Sha256::digest(&content));
                Some((path.clone(), hash, result))
            })
            .collect()
    };

    // Parse files in parallel with tree-sitter
    let mut parse_results: Vec<_> = parse(&files_to_index);

    // Declarations that did not exist before may be the targets of calls
    // that found nothing; files that mention those names resolve again.
    if !force && !parse_results.is_empty() {
        let new_names: std::collections::BTreeSet<&str> = parse_results
            .iter()
            .flat_map(|(_, _, r)| r.nodes.iter().map(|n| n.name.as_str()))
            .filter(|n| !old_names.contains(*n))
            .collect();
        let parsed: std::collections::HashSet<&std::path::PathBuf> =
            parse_results.iter().map(|(p, _, _)| p).collect();
        let mut more = Vec::new();
        let mut stmt = conn.prepare("SELECT DISTINCT file FROM name_refs WHERE name = ?1")?;
        for name in new_names {
            for file in stmt.query_map([name], |r| r.get::<_, String>(0))? {
                let path = std::path::PathBuf::from(file?);
                if candidates.contains(&path) && !parsed.contains(&path) && !more.contains(&path) {
                    more.push(path);
                }
            }
        }
        drop(stmt);
        if !more.is_empty() {
            debug!(
                "re-resolving {} files that mention new declarations",
                more.len()
            );
            let extra = parse(&more);
            parse_results.extend(extra);
        }
    }

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
        SwiftSyntaxMode::Auto if !parse_results.is_empty() || !covered_files.is_empty() => {
            crate::swift_syntax::SwiftSyntaxParser::discover()
        }
        SwiftSyntaxMode::Parser(p) => Some(p.clone()),
        _ => None,
    };
    let mut nodes_enriched = 0;
    if let Some(parser) = parser {
        let files: Vec<std::path::PathBuf> = parse_results
            .iter()
            .map(|(p, _, _)| p.clone())
            .chain(covered_files.iter().cloned())
            .collect();
        nodes_enriched = enrich_with_swift_syntax(&conn, &parser, &files)?;
        if nodes_enriched > 0 {
            info!("swift-syntax enriched {nodes_enriched} nodes");
        }
    }

    // 5. Resolve tree-sitter call sites against all declarations in the DB
    let calls = resolve_calls(&conn, &parse_results, &config.resolution)?;
    if calls.edges > 0 || calls.unresolved > 0 {
        info!(
            "Resolved calls: {} edges ({} ambiguous), {} call sites without a confident target; {} of {} call sites have an unknown receiver",
            calls.edges, calls.ambiguous, calls.unresolved, calls.unknown_receiver, calls.sites
        );
    }
    edges_added += calls.edges;

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
        index_store_note: None,
    })
}

/// Add tree-sitter details to the Index Store nodes of `files`: attributes,
/// access level (the store often records none), signature, extent and
/// sub-kind, matched by file, base name and the declaration's line range.
/// Import declarations, which the store does not record, are added as
/// tree-sitter nodes. Returns the number of nodes updated.
fn stitch_index_store_nodes(
    conn: &rusqlite::Connection,
    files: &[std::path::PathBuf],
) -> Result<usize, PipelineError> {
    if files.is_empty() {
        return Ok(0);
    }
    let parsed: Vec<_> = files
        .par_iter()
        .filter_map(|path| {
            let mut parser = TreeSitterParser::new().ok()?;
            Some((path.clone(), parser.parse_file(path).ok()?))
        })
        .collect();

    let tx = conn.unchecked_transaction()?;
    let mut stitched = 0;
    {
        let mut store_nodes = tx.prepare(
            "SELECT id, name, line, access_level FROM nodes WHERE file = ?1 AND id NOT LIKE 'ts::%'",
        )?;
        let mut update = tx.prepare(
            "UPDATE nodes SET attributes = ?1, access_level = ?2,
                              signature = COALESCE(signature, ?3),
                              end_line = ?4, end_col = ?5,
                              sub_kind = COALESCE(sub_kind, ?6)
             WHERE id = ?7",
        )?;
        for (path, result) in &parsed {
            let file = path.to_string_lossy();
            let candidates: Vec<(String, String, u32, String)> = store_nodes
                .query_map([file.as_ref()], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                })?
                .collect::<Result<_, _>>()?;
            let mut used = std::collections::HashSet::new();
            for ts in &result.nodes {
                if ts.kind == crate::graph::SymbolKind::Import {
                    queries::upsert_node(&tx, ts)?;
                    continue;
                }
                let start = ts.location.line;
                let end = ts.location.end_line.unwrap_or(start);
                // The store's line is the name's line, after attributes
                let Some((id, _, _, store_access)) = candidates
                    .iter()
                    .filter(|(id, name, line, _)| {
                        !used.contains(id)
                            && name.split('(').next() == Some(ts.name.as_str())
                            && (start..=end).contains(line)
                    })
                    .min_by_key(|(_, _, line, _)| line - start)
                else {
                    continue;
                };
                used.insert(id.clone());
                let access = if store_access == "Internal" {
                    format!("{:?}", ts.access_level)
                } else {
                    store_access.clone()
                };
                let attributes = serde_json::to_string(&ts.attributes).unwrap_or_default();
                update.execute(rusqlite::params![
                    attributes,
                    access,
                    ts.signature,
                    ts.location.end_line,
                    ts.location.end_column,
                    ts.sub_kind.map(|k| format!("{k:?}")),
                    id
                ])?;
                stitched += 1;
            }
        }
    }
    tx.commit()?;
    Ok(stitched)
}

/// Drop Index Store symbols and relations of files outside `root` or
/// excluded by the config (including generated sources).
fn restrict_to_project(
    mut data: reader::IndexStoreData,
    root: &Path,
    config: &Config,
    include_set: &Option<globset::GlobSet>,
    exclude_set: &globset::GlobSet,
) -> reader::IndexStoreData {
    let keep = |file: &str| {
        Path::new(file)
            .strip_prefix(root)
            .is_ok_and(|relative| config.should_include(relative, include_set, exclude_set))
    };
    data.nodes.retain(|n| keep(&n.location.file));
    let kept: std::collections::HashSet<&str> = data.nodes.iter().map(|n| n.id.as_str()).collect();
    data.edges.retain(|e| match &e.location {
        Some(loc) => keep(&loc.file),
        None => kept.contains(e.source.as_str()),
    });
    data.file_nodes.retain(|file, _| keep(file));
    data
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

/// Files (other than `changed`) with call edges into declarations of
/// `changed`, and the names currently declared in `changed`.
fn incoming_dependents(
    conn: &rusqlite::Connection,
    changed: &[std::path::PathBuf],
) -> Result<(Vec<std::path::PathBuf>, std::collections::HashSet<String>), PipelineError> {
    let mut dependents = std::collections::BTreeSet::new();
    let mut names = std::collections::HashSet::new();
    let changed_set: std::collections::HashSet<String> = changed
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    let mut edges = conn.prepare(
        "SELECT DISTINCT e.file FROM edges e JOIN nodes n ON n.id = e.target
         WHERE n.file = ?1 AND e.kind = 'calls' AND e.file IS NOT NULL AND e.file != ?1",
    )?;
    let mut declared = conn.prepare("SELECT DISTINCT name FROM nodes WHERE file = ?1")?;
    for file in &changed_set {
        for row in edges.query_map([file], |r| r.get::<_, String>(0))? {
            let row = row?;
            if !changed_set.contains(&row) {
                dependents.insert(row);
            }
        }
        for row in declared.query_map([file], |r| r.get::<_, String>(0))? {
            names.insert(row?);
        }
    }
    Ok((
        dependents
            .into_iter()
            .map(std::path::PathBuf::from)
            .collect(),
        names,
    ))
}

/// Counters of [`resolve_calls`].
#[derive(Debug, Default)]
struct CallStats {
    edges: usize,
    ambiguous: usize,
    unresolved: usize,
    /// Call sites seen.
    sites: usize,
    /// Call sites whose receiver type is unknown.
    unknown_receiver: usize,
}

/// Turn the call sites of freshly parsed files into `calls` edges.
///
/// Resolution runs against every declaration in the database, so calls into
/// unchanged files resolve too. See [`resolve`] for the rules.
fn resolve_calls(
    conn: &rusqlite::Connection,
    parsed: &[(
        std::path::PathBuf,
        String,
        crate::tree_sitter::parser::ParseResult,
    )],
    config: &crate::config::ResolutionConfig,
) -> Result<CallStats, PipelineError> {
    let mut stats = CallStats::default();
    if parsed
        .iter()
        .all(|(_, _, r)| r.calls.is_empty() && r.references.is_empty())
    {
        return Ok(stats);
    }
    // Property types of the parsed files, read back with all others
    {
        let tx = conn.unchecked_transaction()?;
        {
            let mut insert = tx.prepare(
                "INSERT OR REPLACE INTO member_types (file, owner, member, type_name) VALUES (?1, ?2, ?3, ?4)",
            )?;
            for (path, _, result) in parsed {
                let file = path.to_string_lossy();
                for m in &result.member_types {
                    insert.execute(rusqlite::params![file, m.owner, m.member, m.type_name])?;
                }
            }
        }
        tx.commit()?;
    }
    let resolver = resolve::Resolver::load(conn, config)?;

    let tx = conn.unchecked_transaction()?;
    {
        let mut insert = tx.prepare(
            "INSERT OR IGNORE INTO edges (source, target, kind, file, line, col, is_implicit, ambiguous)
             VALUES (?1, ?2, 'calls', ?3, ?4, ?5, 0, ?6)",
        )?;
        let mut refs =
            tx.prepare("INSERT OR IGNORE INTO name_refs (file, name) VALUES (?1, ?2)")?;
        // Names read or used as types; only those declared in the project matter.
        for (path, _, result) in parsed {
            let file = path.to_string_lossy();
            for name in result.references.iter().filter(|n| resolver.is_declared(n)) {
                refs.execute(rusqlite::params![file, name])?;
            }
        }
        for call in parsed.iter().flat_map(|(_, _, r)| &r.calls) {
            stats.sites += 1;
            if resolver.receiver_unknown(call) {
                stats.unknown_receiver += 1;
            }
            let (targets, ambiguous) = match resolver.resolve(call) {
                resolve::Resolution::Confident(t) => (t, false),
                resolve::Resolution::Ambiguous(t) => (t, true),
                resolve::Resolution::Unresolved(known) => {
                    if known {
                        refs.execute(rusqlite::params![call.location.file, call.name])?;
                        stats.unresolved += 1;
                    }
                    continue;
                }
            };
            for target in targets {
                let added = insert.execute(rusqlite::params![
                    call.caller,
                    target,
                    call.location.file,
                    call.location.line,
                    call.location.column,
                    ambiguous
                ])?;
                stats.edges += added;
                if ambiguous {
                    stats.ambiguous += added;
                }
            }
        }
    }
    tx.commit()?;
    Ok(stats)
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
            "SELECT id FROM nodes WHERE file = ?1
               AND (name = ?2 OR substr(name, 1, length(?2) + 1) = ?2 || '(')
               AND ABS(line - ?3) <= 2
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{AccessLevel, EdgeKind, GraphEdge, GraphNode, Location, SymbolKind};

    fn node(id: &str, file: &str) -> GraphNode {
        GraphNode {
            id: id.into(),
            name: id.into(),
            qualified_name: id.into(),
            kind: SymbolKind::Function,
            sub_kind: None,
            location: Location {
                file: file.into(),
                line: 1,
                column: 1,
                end_line: None,
                end_column: None,
            },
            signature: None,
            attributes: vec![],
            access_level: AccessLevel::Internal,
            container_usr: None,
            doc_comment: None,
            metrics: None,
        }
    }

    fn edge(source: &str, target: &str, file: &str) -> GraphEdge {
        GraphEdge {
            source: source.into(),
            target: target.into(),
            kind: EdgeKind::Calls,
            location: Some(Location {
                file: file.into(),
                line: 2,
                column: 1,
                end_line: None,
                end_column: None,
            }),
            is_implicit: false,
            ambiguous: false,
        }
    }

    #[test]
    fn index_store_data_is_restricted_to_project_sources() {
        let root = Path::new("/p/ios");
        let files = [
            ("app", "/p/ios/App/Feature.swift"),
            (
                "dep",
                "/Users/me/DerivedData/SourcePackages/checkouts/Snap/Diff.swift",
            ),
            (
                "gen",
                "/p/ios/Build/DerivedSources/GeneratedStringSymbols_Localizable.swift",
            ),
            ("pods", "/p/ios/Pods/X/Y.swift"),
        ];
        let mut data = reader::IndexStoreData::default();
        for (id, file) in files {
            data.nodes.push(node(id, file));
            data.file_nodes
                .insert(file.to_string(), vec![id.to_string()]);
        }
        data.edges
            .push(edge("app", "dep", "/p/ios/App/Feature.swift"));
        data.edges.push(edge(
            "dep",
            "app",
            "/Users/me/DerivedData/SourcePackages/checkouts/Snap/Diff.swift",
        ));
        let config = Config::default();
        let data = restrict_to_project(
            data,
            root,
            &config,
            &config.include_globset(),
            &config.exclude_globset(),
        );
        let ids: Vec<&str> = data.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, vec!["app"]);
        // Calls from project code to dependencies stay (external targets)
        assert_eq!(data.edges.len(), 1);
        assert_eq!(data.edges[0].source, "app");
        assert_eq!(
            data.file_nodes.keys().collect::<Vec<_>>(),
            vec!["/p/ios/App/Feature.swift"]
        );
    }
}
