//! Dead code detection.
//!
//! Finds symbols with no incoming edges (no callers, no references).
//! Excludes: public API, tests, entry points, @main, protocols, extensions.

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
    pub dead_symbols: Vec<DeadSymbol>,
    pub total_symbols_checked: usize,
    pub dead_count: usize,
    pub dead_percentage: f64,
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
    let nodes = if let Some(prefix) = path_filter {
        queries::get_nodes_by_path_prefix(conn, prefix, 10000)?
    } else {
        queries::get_all_nodes(conn, 10000)?
    };

    let total_checked = nodes.len();
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
        if !include_tests
            && (node.location.file.contains("/Tests/")
                || node.location.file.contains("Tests.swift"))
        {
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

        // Check incoming edges
        let incoming = queries::count_incoming(conn, &node.id).unwrap_or(0);
        if incoming == 0 {
            // Check if it's a container (has children) — containers are structural, not dead
            let outgoing = queries::count_outgoing(conn, &node.id).unwrap_or(0);
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

    dead.truncate(limit as usize);
    let dead_count = dead.len();
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
    })
}
