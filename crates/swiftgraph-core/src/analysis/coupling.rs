//! Module coupling analysis.
//!
//! Computes afferent coupling (Ca), efferent coupling (Ce), instability (I = Ce/(Ca+Ce)),
//! and abstractness (A = abstracts/total) for each module (directory grouping).

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::Serialize;
use thiserror::Error;

use crate::storage::{self, queries};

#[derive(Debug, Error)]
pub enum CouplingError {
    #[error("storage error: {0}")]
    Storage(#[from] crate::storage::StorageError),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

/// Coupling metrics for a module (directory grouping).
#[derive(Debug, Serialize)]
pub struct ModuleCoupling {
    pub module: String,
    pub file_count: u32,
    pub symbol_count: u32,
    /// Afferent coupling: number of external modules that depend on this module.
    pub ca: u32,
    /// Efferent coupling: number of external modules this module depends on.
    pub ce: u32,
    /// Instability: Ce / (Ca + Ce). 0 = maximally stable, 1 = maximally unstable.
    pub instability: f64,
    /// Abstractness: ratio of protocols+abstract classes to total type declarations.
    pub abstractness: f64,
    /// Distance from the main sequence: |A + I - 1|. 0 = ideal balance.
    pub distance: f64,
}

/// Coupling analysis result.
#[derive(Debug, Serialize)]
pub struct CouplingResult {
    pub modules: Vec<ModuleCoupling>,
}

/// Analyze coupling between modules.
///
/// A "module" is a directory path up to `depth` levels from the source root.
/// E.g., with depth=2 and root `/Sources/`, `Sources/Features/Login/LoginView.swift`
/// would be in module `Sources/Features`.
pub fn analyze_coupling(
    db_path: &Path,
    depth: u32,
    source_root: Option<&str>,
) -> Result<CouplingResult, CouplingError> {
    let conn = storage::open_db(db_path)?;

    let all_nodes = queries::get_all_nodes(&conn, 50000)?;

    // Map file → module
    let file_to_module = |file: &str| -> String {
        let path = if let Some(root) = source_root {
            file.strip_prefix(root).unwrap_or(file)
        } else {
            file
        };
        let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        let take = (depth as usize).min(parts.len().saturating_sub(1)); // exclude filename
        parts[..take].join("/")
    };

    // Group nodes by module
    let mut module_files: HashMap<String, HashSet<String>> = HashMap::new();
    let mut module_symbols: HashMap<String, u32> = HashMap::new();
    let mut module_abstracts: HashMap<String, u32> = HashMap::new();
    let mut module_types: HashMap<String, u32> = HashMap::new();

    for node in &all_nodes {
        let module = file_to_module(&node.location.file);
        module_files
            .entry(module.clone())
            .or_default()
            .insert(node.location.file.clone());
        *module_symbols.entry(module.clone()).or_default() += 1;

        // Count types and abstracts for abstractness
        let kind = node.kind.as_str();
        if matches!(kind, "class" | "struct" | "enum" | "protocol" | "typeAlias") {
            *module_types.entry(module.clone()).or_default() += 1;
            if kind == "protocol" {
                *module_abstracts.entry(module).or_default() += 1;
            }
        }
    }

    // Map node ID → module
    let node_to_module: HashMap<String, String> = all_nodes
        .iter()
        .map(|n| (n.id.clone(), file_to_module(&n.location.file)))
        .collect();

    // Query all edges and compute Ca/Ce by module
    let mut ca_sets: HashMap<String, HashSet<String>> = HashMap::new();
    let mut ce_sets: HashMap<String, HashSet<String>> = HashMap::new();

    {
        let mut stmt = conn.prepare("SELECT source, target FROM edges")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows.flatten() {
            let source_mod = node_to_module.get(&row.0);
            let target_mod = node_to_module.get(&row.1);

            if let (Some(sm), Some(tm)) = (source_mod, target_mod) {
                if sm != tm {
                    ce_sets.entry(sm.clone()).or_default().insert(tm.clone());
                    ca_sets.entry(tm.clone()).or_default().insert(sm.clone());
                }
            }
        }
    }

    let mut modules: Vec<ModuleCoupling> = module_files
        .iter()
        .map(|(module, files)| {
            let ca = ca_sets.get(module).map_or(0, |s| s.len() as u32);
            let ce = ce_sets.get(module).map_or(0, |s| s.len() as u32);
            let instability = if ca + ce > 0 {
                ce as f64 / (ca + ce) as f64
            } else {
                0.0
            };
            let total_types = *module_types.get(module).unwrap_or(&0);
            let abstracts = *module_abstracts.get(module).unwrap_or(&0);
            let abstractness = if total_types > 0 {
                abstracts as f64 / total_types as f64
            } else {
                0.0
            };
            let distance = (abstractness + instability - 1.0).abs();

            ModuleCoupling {
                module: module.clone(),
                file_count: files.len() as u32,
                symbol_count: *module_symbols.get(module).unwrap_or(&0),
                ca,
                ce,
                instability,
                abstractness,
                distance,
            }
        })
        .collect();

    // Sort by distance from main sequence (worst first)
    modules.sort_by(|a, b| {
        b.distance
            .partial_cmp(&a.distance)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(CouplingResult { modules })
}
