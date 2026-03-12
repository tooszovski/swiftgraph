//! Complexity analysis.
//!
//! Computes fan-in/fan-out and a complexity score for symbols.
//! True cyclomatic complexity requires CFG analysis (deferred to swift-syntax),
//! so we use structural metrics from the graph.

use std::collections::HashMap;
use std::path::Path;

use serde::Serialize;
use thiserror::Error;

use crate::storage::{self, queries};

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
    pub symbols: Vec<SymbolComplexity>,
    pub file_stats: Vec<FileComplexity>,
    pub total_symbols: usize,
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
pub fn analyze_complexity(
    db_path: &Path,
    path_filter: Option<&str>,
    limit: u32,
    sort_by: &str, // "score", "fan_in", "fan_out"
) -> Result<ComplexityResult, ComplexityError> {
    let conn = storage::open_db(db_path)?;

    // Get all nodes (filtered by path if specified)
    let nodes = if let Some(prefix) = path_filter {
        queries::get_nodes_by_path_prefix(&conn, prefix, 5000)?
    } else {
        queries::get_all_nodes(&conn, 5000)?
    };

    let mut symbols: Vec<SymbolComplexity> = Vec::new();
    let mut file_map: HashMap<String, Vec<&SymbolComplexity>> = HashMap::new();

    for node in &nodes {
        let fan_in = queries::count_incoming(&conn, &node.id).unwrap_or(0);
        let fan_out = queries::count_outgoing(&conn, &node.id).unwrap_or(0);
        let score = fan_in as f64 * 1.5 + fan_out as f64;

        symbols.push(SymbolComplexity {
            id: node.id.clone(),
            name: node.name.clone(),
            kind: node.kind.as_str().to_string(),
            file: node.location.file.clone(),
            fan_in,
            fan_out,
            score,
        });
    }

    // Sort
    match sort_by {
        "fan_in" => symbols.sort_by(|a, b| b.fan_in.cmp(&a.fan_in)),
        "fan_out" => symbols.sort_by(|a, b| b.fan_out.cmp(&a.fan_out)),
        _ => symbols.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
    }

    symbols.truncate(limit as usize);
    let total = symbols.len();

    // Compute file stats
    for s in &symbols {
        file_map.entry(s.file.clone()).or_default().push(s);
    }

    let file_stats: Vec<FileComplexity> = file_map
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

    Ok(ComplexityResult {
        symbols,
        file_stats,
        total_symbols: total,
    })
}
