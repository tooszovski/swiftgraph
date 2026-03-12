//! Dependency cycle detection.
//!
//! Finds circular dependencies at the file level (import/usage cycles).

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::Serialize;
use thiserror::Error;

use crate::storage::{self, queries};

#[derive(Debug, Error)]
pub enum CycleError {
    #[error("storage error: {0}")]
    Storage(#[from] crate::storage::StorageError),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

/// A detected dependency cycle.
#[derive(Debug, Serialize)]
pub struct DependencyCycle {
    /// Files involved in the cycle, in order.
    pub files: Vec<String>,
    /// Number of cross-file edges forming the cycle.
    pub edge_count: usize,
}

/// Cycle detection result.
#[derive(Debug, Serialize)]
pub struct CycleResult {
    pub cycles: Vec<DependencyCycle>,
    pub files_analyzed: usize,
}

/// Detect file-level dependency cycles.
pub fn detect_cycles(
    db_path: &Path,
    path_filter: Option<&str>,
    max_cycles: u32,
) -> Result<CycleResult, CycleError> {
    let conn = storage::open_db(db_path)?;

    // Build file-level dependency graph
    let edges = queries::get_cross_file_edges(&conn, path_filter, 50000)?;

    let mut file_deps: HashMap<String, HashSet<String>> = HashMap::new();
    let mut all_files: HashSet<String> = HashSet::new();

    for (source_file, target_file) in &edges {
        if source_file != target_file {
            file_deps
                .entry(source_file.clone())
                .or_default()
                .insert(target_file.clone());
            all_files.insert(source_file.clone());
            all_files.insert(target_file.clone());
        }
    }

    let files_analyzed = all_files.len();

    // Find cycles via DFS
    let mut cycles: Vec<DependencyCycle> = Vec::new();
    let mut visited: HashSet<String> = HashSet::new();
    let mut on_stack: HashSet<String> = HashSet::new();
    let mut path: Vec<String> = Vec::new();

    for file in &all_files {
        if !visited.contains(file) {
            dfs_cycles(
                file,
                &file_deps,
                &mut visited,
                &mut on_stack,
                &mut path,
                &mut cycles,
                max_cycles,
            );
        }
    }

    Ok(CycleResult {
        cycles,
        files_analyzed,
    })
}

fn dfs_cycles(
    node: &str,
    graph: &HashMap<String, HashSet<String>>,
    visited: &mut HashSet<String>,
    on_stack: &mut HashSet<String>,
    path: &mut Vec<String>,
    cycles: &mut Vec<DependencyCycle>,
    max_cycles: u32,
) {
    if cycles.len() >= max_cycles as usize {
        return;
    }

    visited.insert(node.to_string());
    on_stack.insert(node.to_string());
    path.push(node.to_string());

    if let Some(neighbors) = graph.get(node) {
        for neighbor in neighbors {
            if !visited.contains(neighbor.as_str()) {
                dfs_cycles(neighbor, graph, visited, on_stack, path, cycles, max_cycles);
            } else if on_stack.contains(neighbor.as_str()) {
                // Found a cycle — extract it
                if let Some(start) = path.iter().position(|p| p == neighbor) {
                    let cycle_files: Vec<String> = path[start..].to_vec();
                    if cycle_files.len() >= 2 {
                        cycles.push(DependencyCycle {
                            edge_count: cycle_files.len(),
                            files: cycle_files,
                        });
                    }
                }
            }
        }
    }

    path.pop();
    on_stack.remove(node);
}
