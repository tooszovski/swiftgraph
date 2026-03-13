//! Module dependency graph based on import declarations.
//!
//! Builds a graph of module imports (import Foundation, import UIKit, etc.)
//! and computes per-module statistics.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::Serialize;
use thiserror::Error;

use crate::graph::SymbolKind;
use crate::storage::{self, queries};

#[derive(Debug, Error)]
pub enum ImportsError {
    #[error("storage error: {0}")]
    Storage(#[from] crate::storage::StorageError),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

/// A module dependency edge.
#[derive(Debug, Serialize)]
pub struct ModuleEdge {
    pub from_file: String,
    pub to_module: String,
}

/// Stats for an imported module.
#[derive(Debug, Serialize)]
pub struct ModuleStats {
    pub module: String,
    pub import_count: u32,
    pub importing_files: Vec<String>,
}

/// Module dependency analysis result.
#[derive(Debug, Serialize)]
pub struct ImportsResult {
    pub total_imports: u32,
    pub unique_modules: u32,
    pub modules: Vec<ModuleStats>,
    pub file_import_counts: Vec<FileImportCount>,
}

/// Import count per file.
#[derive(Debug, Serialize)]
pub struct FileImportCount {
    pub file: String,
    pub import_count: u32,
    pub modules: Vec<String>,
}

/// Analyze module imports across the project.
pub fn analyze_imports(
    db_path: &Path,
    path_filter: Option<&str>,
) -> Result<ImportsResult, ImportsError> {
    let conn = storage::open_db(db_path)?;

    let all_nodes = if let Some(prefix) = path_filter {
        queries::get_nodes_by_path_prefix(&conn, prefix, 50000)?
    } else {
        queries::get_all_nodes(&conn, 50000)?
    };

    // Collect import nodes
    let imports: Vec<_> = all_nodes
        .iter()
        .filter(|n| n.kind == SymbolKind::Import)
        .collect();

    let mut module_to_files: HashMap<String, HashSet<String>> = HashMap::new();
    let mut file_to_modules: HashMap<String, Vec<String>> = HashMap::new();

    for import in &imports {
        let module = &import.name;
        let file = &import.location.file;

        module_to_files
            .entry(module.clone())
            .or_default()
            .insert(file.clone());
        file_to_modules
            .entry(file.clone())
            .or_default()
            .push(module.clone());
    }

    let mut modules: Vec<ModuleStats> = module_to_files
        .iter()
        .map(|(module, files)| {
            let mut importing_files: Vec<String> = files.iter().cloned().collect();
            importing_files.sort();
            importing_files.truncate(10);
            ModuleStats {
                module: module.clone(),
                import_count: files.len() as u32,
                importing_files,
            }
        })
        .collect();
    modules.sort_by(|a, b| b.import_count.cmp(&a.import_count));

    let mut file_import_counts: Vec<FileImportCount> = file_to_modules
        .iter()
        .map(|(file, mods)| {
            let mut modules = mods.clone();
            modules.sort();
            modules.dedup();
            FileImportCount {
                file: file.clone(),
                import_count: modules.len() as u32,
                modules,
            }
        })
        .collect();
    file_import_counts.sort_by(|a, b| b.import_count.cmp(&a.import_count));
    file_import_counts.truncate(50);

    Ok(ImportsResult {
        total_imports: imports.len() as u32,
        unique_modules: modules.len() as u32,
        modules,
        file_import_counts,
    })
}
