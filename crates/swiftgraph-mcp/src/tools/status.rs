use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use swiftgraph_core::project;
use swiftgraph_core::storage::{self, queries};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusResponse {
    pub project_name: String,
    pub project_type: String,
    pub mode: String,
    pub files: u32,
    pub nodes: u32,
    pub edges: u32,
    pub index_store_available: bool,
    pub db_path: String,
}

/// Build the status report. Never fails just because no project markers were
/// found: in that case `project_type` is `"unknown"` and DB statistics are still reported.
pub fn get_status(project_root: &Path) -> Result<StatusResponse> {
    let (project_name, project_type, index_store_path) = match project::detect_project(project_root)
    {
        Ok(info) => (
            info.name,
            info.project_type.as_str().to_string(),
            info.index_store_path,
        ),
        Err(project::ProjectError::NotFound(_)) => (
            project_root
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "Unknown".into()),
            "unknown".to_string(),
            None,
        ),
        Err(e) => return Err(e.into()),
    };

    let db_path = project_root.join(".swiftgraph/db.sqlite");
    let mode = if index_store_path.is_some() {
        "full"
    } else {
        "tree-sitter"
    };

    let (files, nodes, edges) = if db_path.exists() {
        let conn = storage::open_db(&db_path)?;
        let stats = queries::get_stats(&conn)?;
        (stats.file_count, stats.node_count, stats.edge_count)
    } else {
        (0, 0, 0)
    };

    Ok(StatusResponse {
        project_name,
        project_type,
        mode: mode.to_string(),
        files,
        nodes,
        edges,
        index_store_available: index_store_path.is_some(),
        db_path: db_path.to_string_lossy().to_string(),
    })
}
