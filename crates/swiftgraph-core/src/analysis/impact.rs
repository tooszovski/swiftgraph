//! Blast radius analysis.
//!
//! Given a symbol, compute the direct and transitive impact of changing it.

use std::collections::HashSet;
use std::path::Path;

use serde::Serialize;
use thiserror::Error;

use crate::storage::{self, queries};

#[derive(Debug, Error)]
pub enum ImpactError {
    #[error("storage error: {0}")]
    Storage(#[from] crate::storage::StorageError),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("symbol not found: {0}")]
    SymbolNotFound(String),
}

/// Impact analysis result for a single symbol.
#[derive(Debug, Serialize)]
pub struct ImpactResult {
    /// The symbol being analyzed.
    pub symbol: String,
    /// Number of direct dependents (1 hop).
    pub direct_impact: usize,
    /// Number of transitive dependents (all hops up to depth).
    pub transitive_impact: usize,
    /// Files affected.
    pub affected_files: Vec<String>,
    /// Test files affected.
    pub affected_tests: Vec<String>,
    /// Risk level based on impact.
    pub risk_level: String,
    /// Breakdown by edge kind.
    pub breakdown: ImpactBreakdown,
}

/// Breakdown of impact by relationship type.
#[derive(Debug, Serialize)]
pub struct ImpactBreakdown {
    pub callers: Vec<String>,
    pub conforming_types: Vec<String>,
    pub subtypes: Vec<String>,
    pub extensions: Vec<String>,
    pub overrides: Vec<String>,
}

/// Analyze the blast radius of changing a symbol.
pub fn analyze_impact(
    db_path: &Path,
    symbol_id: &str,
    depth: u32,
) -> Result<ImpactResult, ImpactError> {
    let conn = storage::open_db(db_path)?;

    // Verify symbol exists
    let _node = queries::get_node(&conn, symbol_id)?
        .ok_or_else(|| ImpactError::SymbolNotFound(symbol_id.into()))?;

    // Collect direct dependents
    let mut breakdown = ImpactBreakdown {
        callers: Vec::new(),
        conforming_types: Vec::new(),
        subtypes: Vec::new(),
        extensions: Vec::new(),
        overrides: Vec::new(),
    };

    let direct_edges = queries::get_all_incoming(&conn, symbol_id, 500)?;
    let mut direct_ids: HashSet<String> = HashSet::new();

    for edge in &direct_edges {
        direct_ids.insert(edge.source.clone());
        match edge.kind.as_str() {
            "calls" => breakdown.callers.push(edge.source.clone()),
            "conformsTo" => breakdown.conforming_types.push(edge.source.clone()),
            "inheritsFrom" => breakdown.subtypes.push(edge.source.clone()),
            "extendsType" => breakdown.extensions.push(edge.source.clone()),
            "overrides" => breakdown.overrides.push(edge.source.clone()),
            _ => {}
        }
    }

    let direct_impact = direct_ids.len();

    // BFS for transitive dependents
    let mut all_affected: HashSet<String> = direct_ids.clone();
    all_affected.insert(symbol_id.to_owned());
    let mut frontier: Vec<String> = direct_ids.into_iter().collect();

    for _level in 1..depth {
        let mut next_frontier = Vec::new();
        for id in &frontier {
            let incoming = queries::get_all_incoming(&conn, id, 100).unwrap_or_default();
            for edge in incoming {
                if all_affected.insert(edge.source.clone()) {
                    next_frontier.push(edge.source);
                }
            }
        }
        if next_frontier.is_empty() {
            break;
        }
        frontier = next_frontier;
    }

    // Remove the symbol itself from count
    all_affected.remove(symbol_id);
    let transitive_impact = all_affected.len();

    // Collect affected files
    let mut all_files: HashSet<String> = HashSet::new();
    let mut test_files: HashSet<String> = HashSet::new();

    for id in &all_affected {
        if let Ok(Some(node)) = queries::get_node(&conn, id) {
            let file = &node.location.file;
            if file.contains("/Tests/") || file.contains("Tests.swift") {
                test_files.insert(file.clone());
            } else {
                all_files.insert(file.clone());
            }
        }
    }

    let risk_level = match transitive_impact {
        0..=5 => "low",
        6..=20 => "medium",
        21..=50 => "high",
        _ => "critical",
    }
    .to_owned();

    Ok(ImpactResult {
        symbol: symbol_id.to_owned(),
        direct_impact,
        transitive_impact,
        affected_files: all_files.into_iter().collect(),
        affected_tests: test_files.into_iter().collect(),
        risk_level,
        breakdown,
    })
}
