use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use swiftgraph_core::pipeline::{self, IndexStrategy};
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
    /// Strategy of the last indexing run (`index-store`, `hybrid`, `tree-sitter`),
    /// `None` if the project has not been indexed yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_strategy: Option<String>,
    /// swift-syntax parser used for enrichment (`None` if missing or
    /// incompatible, in which case indexing runs without enrichment).
    pub swift_syntax_parser: Option<String>,
}

/// Build the status report. Never fails just because no project markers were
/// found: in that case `project_type` is `"unknown"` and DB statistics are still reported.
pub fn get_status(project_root: &Path) -> Result<StatusResponse> {
    let (project_name, project_type) = match project::detect_project(project_root) {
        Ok(info) => (info.name, info.project_type.as_str().to_string()),
        Err(project::ProjectError::NotFound(_)) => (
            project_root
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "Unknown".into()),
            "unknown".to_string(),
        ),
        Err(e) => return Err(e.into()),
    };
    let index_store_path = project::resolve_index_store(project_root);

    let db_path = project_root.join(".swiftgraph/db.sqlite");
    let (files, nodes, edges, index_strategy) = if db_path.exists() {
        let conn = storage::open_db(&db_path)?;
        let stats = queries::get_stats(&conn)?;
        let strategy = queries::get_meta(&conn, pipeline::META_INDEX_STRATEGY)?;
        (
            stats.file_count,
            stats.node_count,
            stats.edge_count,
            strategy,
        )
    } else {
        (0, 0, 0, None)
    };

    // "full" means the graph actually contains Index Store data; before the
    // first index it reports what the next run would use.
    let uses_store = match index_strategy.as_deref() {
        Some(s) => s != IndexStrategy::TreeSitter.as_str(),
        None => index_store_path.is_some(),
    };
    let mode = if uses_store { "full" } else { "tree-sitter" };

    Ok(StatusResponse {
        project_name,
        project_type,
        mode: mode.to_string(),
        files,
        nodes,
        edges,
        index_store_available: index_store_path.is_some(),
        db_path: db_path.to_string_lossy().to_string(),
        index_strategy,
        swift_syntax_parser: swiftgraph_core::swift_syntax::SwiftSyntaxParser::discover()
            .map(|p| p.describe()),
    })
}
