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

pub fn get_status(project_root: &Path) -> Result<StatusResponse> {
    let project_info = project::detect_project(project_root)?;

    let db_path = project_root.join(".swiftgraph/db.sqlite");
    let mode = if project_info.index_store_path.is_some() {
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
        project_name: project_info.name,
        project_type: project_info.project_type.as_str().to_string(),
        mode: mode.to_string(),
        files,
        nodes,
        edges,
        index_store_available: project_info.index_store_path.is_some(),
        db_path: db_path.to_string_lossy().to_string(),
    })
}
