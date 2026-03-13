//! Architecture boundary enforcement.
//!
//! Users define layers (module groups) and rules about which layers can depend on which.
//! The engine checks the actual dependency graph for violations.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::storage::{self, StorageError};

#[derive(Debug, Error)]
pub enum BoundaryError {
    #[error("storage error: {0}")]
    Storage(#[from] StorageError),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

/// A layer definition: a named group of files matching a path pattern.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Layer {
    /// Layer name (e.g., "Views", "Services", "Domain").
    pub name: String,
    /// Path pattern to match files (e.g., "*/Views/*", "Sources/Services/**").
    pub pattern: String,
}

/// A boundary rule: which layers are allowed to depend on which.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoundaryRule {
    /// Source layer name.
    pub from: String,
    /// Target layer name.
    pub to: String,
    /// Whether this dependency is allowed.
    pub allowed: bool,
}

/// Boundary configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoundaryConfig {
    pub layers: Vec<Layer>,
    pub rules: Vec<BoundaryRule>,
}

/// A boundary violation.
#[derive(Debug, Clone, Serialize)]
pub struct Violation {
    /// Source file path.
    pub source_file: String,
    /// Target file path.
    pub target_file: String,
    /// Source layer name.
    pub source_layer: String,
    /// Target layer name.
    pub target_layer: String,
    /// Symbol in source.
    pub source_symbol: Option<String>,
    /// Symbol in target.
    pub target_symbol: Option<String>,
}

/// Result of boundary check.
#[derive(Debug, Clone, Serialize)]
pub struct BoundaryResult {
    pub violations: Vec<Violation>,
    pub total_violations: usize,
    pub layers_found: HashMap<String, usize>,
}

/// Check architecture boundaries given a config.
pub fn check_boundaries(
    db_path: &Path,
    config: &BoundaryConfig,
) -> Result<BoundaryResult, BoundaryError> {
    let conn = storage::open_db(db_path)?;

    // Build layer → files mapping
    let mut layer_files: HashMap<String, Vec<String>> = HashMap::new();
    let mut file_layer: HashMap<String, String> = HashMap::new();

    // Get all files
    let mut stmt = conn.prepare("SELECT DISTINCT file FROM nodes WHERE file IS NOT NULL")?;
    let all_files: Vec<String> = stmt
        .query_map([], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();

    for file in &all_files {
        for layer in &config.layers {
            if matches_pattern(file, &layer.pattern) {
                layer_files
                    .entry(layer.name.clone())
                    .or_default()
                    .push(file.clone());
                file_layer.insert(file.clone(), layer.name.clone());
                break; // First matching layer wins
            }
        }
    }

    // Build disallowed edges set
    let mut disallowed: HashMap<(String, String), bool> = HashMap::new();
    for rule in &config.rules {
        if !rule.allowed {
            disallowed.insert((rule.from.clone(), rule.to.clone()), true);
        }
    }

    // Query all cross-file edges
    let mut edge_stmt = conn.prepare(
        "SELECT e.source, e.target, n1.file, n2.file, n1.name, n2.name \
         FROM edges e \
         JOIN nodes n1 ON e.source = n1.id \
         JOIN nodes n2 ON e.target = n2.id \
         WHERE n1.file IS NOT NULL AND n2.file IS NOT NULL AND n1.file != n2.file",
    )?;

    let mut violations = Vec::new();

    let rows = edge_stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, Option<String>>(5)?,
        ))
    })?;

    for row in rows {
        let (_source_id, _target_id, source_file, target_file, source_name, target_name) = row?;

        let source_layer = file_layer.get(&source_file);
        let target_layer = file_layer.get(&target_file);

        if let (Some(sl), Some(tl)) = (source_layer, target_layer) {
            if sl == tl {
                continue; // Same layer, always ok
            }
            if disallowed.contains_key(&(sl.clone(), tl.clone())) {
                violations.push(Violation {
                    source_file,
                    target_file,
                    source_layer: sl.clone(),
                    target_layer: tl.clone(),
                    source_symbol: source_name,
                    target_symbol: target_name,
                });
            }
        }
    }

    let total = violations.len();
    let layers_found: HashMap<String, usize> = layer_files
        .iter()
        .map(|(k, v)| (k.clone(), v.len()))
        .collect();

    Ok(BoundaryResult {
        violations,
        total_violations: total,
        layers_found,
    })
}

/// Simple glob-like pattern matching for file paths.
fn matches_pattern(file_path: &str, pattern: &str) -> bool {
    // Support simple patterns: "*/Views/*", "Sources/Domain/**"
    if let Ok(glob) = globset::Glob::new(pattern) {
        let matcher = glob.compile_matcher();
        matcher.is_match(file_path)
    } else {
        // Fallback to substring match
        file_path.contains(pattern)
    }
}
