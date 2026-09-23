//! Complexity analysis.
//!
//! Computes fan-in/fan-out and a complexity score for symbols.
//! True cyclomatic complexity requires CFG analysis (deferred to swift-syntax),
//! so we use structural metrics from the graph.

use std::collections::HashMap;
use std::path::Path;

use serde::Serialize;
use thiserror::Error;

use crate::storage;

#[derive(Debug, Error)]
pub enum ComplexityError {
    #[error("storage error: {0}")]
    Storage(#[from] crate::storage::StorageError),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

/// Complexity metrics for a single symbol.
#[derive(Debug, Serialize)]
pub struct SymbolComplexity {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub file: String,
    pub fan_in: u32,
    pub fan_out: u32,
    /// Structural complexity: fan_in + fan_out, weighted
    pub score: f64,
}

/// Complexity analysis result.
#[derive(Debug, Serialize)]
pub struct ComplexityResult {
    /// Top symbols, at most `limit`.
    pub symbols: Vec<SymbolComplexity>,
    /// Per-file statistics over the returned symbols.
    pub file_stats: Vec<FileComplexity>,
    /// Number of symbols analyzed.
    pub total_symbols: usize,
    /// `symbols` was cut to `limit`.
    pub truncated: bool,
}

/// Per-file complexity.
#[derive(Debug, Serialize)]
pub struct FileComplexity {
    pub file: String,
    pub symbol_count: u32,
    pub avg_fan_in: f64,
    pub avg_fan_out: f64,
    pub max_score: f64,
}

/// Analyze complexity for symbols, optionally filtered by file prefix.
///
/// Test targets and `Package.swift` manifests are skipped unless
/// `include_tests` is set (see [`super::is_test_or_manifest`]).
pub fn analyze_complexity(
    db_path: &Path,
    path_filter: Option<&str>,
    limit: u32,
    sort_by: &str, // "score", "fan_in", "fan_out"
    include_tests: bool,
) -> Result<ComplexityResult, ComplexityError> {
    let conn = storage::open_db(db_path)?;
    analyze_complexity_from_conn(&conn, path_filter, limit, sort_by, include_tests)
}

/// Analyze complexity from an existing connection.
///
/// Fan-in/fan-out count every edge kind except ambiguous call edges; all
/// symbols matching `path_filter` are analyzed with one grouped query.
pub fn analyze_complexity_from_conn(
    conn: &rusqlite::Connection,
    path_filter: Option<&str>,
    limit: u32,
    sort_by: &str,
    include_tests: bool,
) -> Result<ComplexityResult, ComplexityError> {
    let pattern = format!("{}%", path_filter.unwrap_or(""));
    let mut stmt = conn.prepare(
        "SELECT n.id, n.name, n.kind, n.file, COALESCE(i.c, 0), COALESCE(o.c, 0)
         FROM nodes n
         LEFT JOIN (SELECT target AS id, COUNT(*) AS c FROM edges
                    WHERE ambiguous = 0 GROUP BY target) i ON i.id = n.id
         LEFT JOIN (SELECT source AS id, COUNT(*) AS c FROM edges
                    WHERE ambiguous = 0 GROUP BY source) o ON o.id = n.id
         WHERE n.file LIKE ?1",
    )?;
    let rows = stmt.query_map([&pattern], |r| {
        let fan_in: u32 = r.get(4)?;
        let fan_out: u32 = r.get(5)?;
        Ok(SymbolComplexity {
            id: r.get(0)?,
            name: r.get(1)?,
            kind: r.get(2)?,
            file: r.get(3)?,
            fan_in,
            fan_out,
            score: fan_in as f64 * 1.5 + fan_out as f64,
        })
    })?;
    let mut symbols: Vec<SymbolComplexity> = Vec::new();
    for row in rows {
        let symbol = row?;
        if include_tests || !super::is_test_or_manifest(&symbol.file) {
            symbols.push(symbol);
        }
    }
    let total = symbols.len();
    let mut file_map: HashMap<String, Vec<&SymbolComplexity>> = HashMap::new();

    // Sort
    match sort_by {
        "fan_in" => symbols.sort_by(|a, b| b.fan_in.cmp(&a.fan_in).then_with(|| a.id.cmp(&b.id))),
        "fan_out" => {
            symbols.sort_by(|a, b| b.fan_out.cmp(&a.fan_out).then_with(|| a.id.cmp(&b.id)))
        }
        _ => symbols.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.id.cmp(&b.id))),
    }

    let truncated = symbols.len() > limit as usize;
    symbols.truncate(limit as usize);

    // Compute file stats
    for s in &symbols {
        file_map.entry(s.file.clone()).or_default().push(s);
    }

    let mut file_stats: Vec<FileComplexity> = file_map
        .iter()
        .map(|(file, syms)| {
            let count = syms.len() as u32;
            let avg_fi = syms.iter().map(|s| s.fan_in as f64).sum::<f64>() / count as f64;
            let avg_fo = syms.iter().map(|s| s.fan_out as f64).sum::<f64>() / count as f64;
            let max_score = syms.iter().map(|s| s.score).fold(0.0_f64, |a, b| a.max(b));
            FileComplexity {
                file: file.clone(),
                symbol_count: count,
                avg_fan_in: avg_fi,
                avg_fan_out: avg_fo,
                max_score,
            }
        })
        .collect();
    file_stats.sort_by(|a, b| {
        b.max_score
            .total_cmp(&a.max_score)
            .then_with(|| a.file.cmp(&b.file))
    });

    Ok(ComplexityResult {
        symbols,
        file_stats,
        total_symbols: total,
        truncated,
    })
}
