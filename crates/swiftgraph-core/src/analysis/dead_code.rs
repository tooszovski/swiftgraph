//! Dead code detection.
//!
//! Finds symbols with no incoming edges (no callers, no references).
//! Excludes: public API, tests, entry points, @main, protocols, extensions.
//!
//! "Possibly used" wins over "dead": ambiguous call edges count as uses, and
//! so does a call site whose receiver could not be resolved but whose name
//! matches the symbol (`name_refs`). Tree-sitter mode cannot prove a
//! symbol unused, so it only reports symbols nothing could be calling.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::Serialize;
use thiserror::Error;

use crate::storage::{self, queries};

#[derive(Debug, Error)]
pub enum DeadCodeError {
    #[error("storage error: {0}")]
    Storage(#[from] crate::storage::StorageError),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

/// A potentially dead symbol.
#[derive(Debug, Serialize)]
pub struct DeadSymbol {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub file: String,
    pub line: u32,
    pub access_level: String,
    /// Why it's considered dead.
    pub reason: String,
}

/// Dead code analysis result.
#[derive(Debug, Serialize)]
pub struct DeadCodeResult {
    /// Dead symbols, at most `limit`.
    pub dead_symbols: Vec<DeadSymbol>,
    /// Symbols matching the path filter.
    pub total_symbols_checked: usize,
    /// All dead symbols found (may exceed `dead_symbols.len()`).
    pub dead_count: usize,
    pub dead_percentage: f64,
    /// `dead_symbols` was cut to `limit`.
    pub truncated: bool,
}

/// Find dead code: symbols with no incoming edges.
pub fn find_dead_code(
    db_path: &Path,
    path_filter: Option<&str>,
    include_tests: bool,
    limit: u32,
) -> Result<DeadCodeResult, DeadCodeError> {
    let conn = storage::open_db(db_path)?;
    find_dead_code_from_conn(&conn, path_filter, include_tests, limit)
}

/// Find dead code from an existing connection.
pub fn find_dead_code_from_conn(
    conn: &rusqlite::Connection,
    path_filter: Option<&str>,
    include_tests: bool,
    limit: u32,
) -> Result<DeadCodeResult, DeadCodeError> {
    let nodes = queries::get_nodes_by_path_prefix(conn, path_filter.unwrap_or(""), u32::MAX)?;
    let total_checked = nodes.len();

    // One grouped query instead of two COUNTs per node. Ambiguous edges are
    // included on purpose: a possible caller keeps a symbol alive.
    let mut incoming: HashMap<String, u32> = HashMap::new();
    let mut outgoing: HashMap<String, u32> = HashMap::new();
    for (sql, map) in [
        (
            "SELECT target, COUNT(*) FROM edges GROUP BY target",
            &mut incoming,
        ),
        (
            "SELECT source, COUNT(*) FROM edges GROUP BY source",
            &mut outgoing,
        ),
    ] {
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?)))?;
        for row in rows {
            let (id, n) = row?;
            map.insert(id, n);
        }
    }
    let unresolved: HashSet<String> = {
        let mut stmt = conn.prepare("SELECT DISTINCT name FROM name_refs")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.collect::<Result<_, _>>()?
    };
    let mut dead: Vec<DeadSymbol> = Vec::new();

    for node in &nodes {
        // Skip excluded kinds
        let kind = node.kind.as_str();
        if matches!(
            kind,
            "protocol" | "extension" | "import" | "associatedType" | "module"
        ) {
            continue;
        }

        // Skip test files unless requested
        if !include_tests && super::is_test_or_manifest(&node.location.file) {
            continue;
        }

        // Skip public/open API (may be used externally)
        let access = format!("{:?}", node.access_level);
        if matches!(
            node.access_level,
            crate::graph::AccessLevel::Public | crate::graph::AccessLevel::Open
        ) {
            continue;
        }

        // Skip entry points
        if node.name == "body"
            || node.name == "main"
            || node.name.starts_with("application(")
            || node.name.starts_with("scene(")
        {
            continue;
        }

        // Called somewhere through a receiver we could not resolve
        let base_name = node.name.split('(').next().unwrap_or(&node.name);
        if unresolved.contains(base_name) {
            continue;
        }

        // Check incoming edges
        if incoming.get(&node.id).copied().unwrap_or(0) == 0 {
            // Check if it's a container (has children) — containers are structural, not dead
            let outgoing = outgoing.get(&node.id).copied().unwrap_or(0);
            let is_container = kind == "class" || kind == "struct" || kind == "enum";
            if is_container && outgoing > 0 {
                continue;
            }

            dead.push(DeadSymbol {
                id: node.id.clone(),
                name: node.name.clone(),
                kind: kind.to_string(),
                file: node.location.file.clone(),
                line: node.location.line,
                access_level: access.to_string(),
                reason: "No incoming edges (no callers or references)".into(),
            });
        }
    }

    dead.sort_by(|a, b| (&a.file, a.line, &a.id).cmp(&(&b.file, b.line, &b.id)));
    let dead_count = dead.len();
    let truncated = dead_count > limit as usize;
    dead.truncate(limit as usize);
    let dead_percentage = if total_checked > 0 {
        (dead_count as f64 / total_checked as f64) * 100.0
    } else {
        0.0
    };

    Ok(DeadCodeResult {
        dead_symbols: dead,
        total_symbols_checked: total_checked,
        dead_count,
        dead_percentage,
        truncated,
    })
}
