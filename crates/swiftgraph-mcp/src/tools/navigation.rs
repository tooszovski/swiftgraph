use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use swiftgraph_core::graph::{GraphEdge, GraphNode};
use swiftgraph_core::storage::{self, queries};

#[derive(Debug, Deserialize)]
pub struct SearchParams {
    pub query: String,
    pub kind: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct SearchResponse {
    pub results: Vec<GraphNode>,
    pub total: usize,
}

pub fn search(db_path: &Path, params: SearchParams) -> Result<SearchResponse> {
    let conn = storage::open_db(db_path)?;
    let limit = params.limit.unwrap_or(20);

    // Try FTS5 first, fallback to LIKE
    let results = queries::search_nodes(&conn, &params.query, limit).or_else(|_| {
        queries::find_nodes_by_name(&conn, &params.query, params.kind.as_deref(), limit)
    })?;

    let total = results.len();
    Ok(SearchResponse { results, total })
}

#[derive(Debug, Deserialize)]
pub struct NodeParams {
    pub symbol: String,
}

pub fn get_node(db_path: &Path, params: NodeParams) -> Result<Option<GraphNode>> {
    let conn = storage::open_db(db_path)?;
    let node = queries::get_node(&conn, &params.symbol)?;
    Ok(node)
}

#[derive(Debug, Deserialize)]
pub struct CallersParams {
    pub symbol: String,
    pub limit: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct EdgesResponse {
    pub edges: Vec<GraphEdge>,
    pub count: usize,
}

pub fn get_callers(db_path: &Path, params: CallersParams) -> Result<EdgesResponse> {
    let conn = storage::open_db(db_path)?;
    let limit = params.limit.unwrap_or(30);
    let edges = queries::get_callers(&conn, &params.symbol, limit)?;
    let count = edges.len();
    Ok(EdgesResponse { edges, count })
}

pub fn get_callees(db_path: &Path, params: CallersParams) -> Result<EdgesResponse> {
    let conn = storage::open_db(db_path)?;
    let limit = params.limit.unwrap_or(30);
    let edges = queries::get_callees(&conn, &params.symbol, limit)?;
    let count = edges.len();
    Ok(EdgesResponse { edges, count })
}

pub fn get_references(db_path: &Path, params: CallersParams) -> Result<EdgesResponse> {
    let conn = storage::open_db(db_path)?;
    let limit = params.limit.unwrap_or(50);
    let edges = queries::get_references(&conn, &params.symbol, limit)?;
    let count = edges.len();
    Ok(EdgesResponse { edges, count })
}

#[derive(Debug, Deserialize)]
pub struct HierarchyParams {
    pub symbol: String,
    pub direction: Option<String>, // "subtypes" | "supertypes"
    pub depth: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct HierarchyResponse {
    pub root: String,
    pub direction: String,
    pub related: Vec<GraphNode>,
}

pub fn get_hierarchy(db_path: &Path, params: HierarchyParams) -> Result<HierarchyResponse> {
    let conn = storage::open_db(db_path)?;
    let direction = params.direction.as_deref().unwrap_or("subtypes");
    let limit = params.depth.unwrap_or(3) * 50; // approximate

    let edges = match direction {
        "supertypes" => queries::get_supertypes(&conn, &params.symbol, limit)?,
        _ => queries::get_subtypes(&conn, &params.symbol, limit)?,
    };

    // Resolve target nodes
    let related: Vec<GraphNode> = edges
        .iter()
        .filter_map(|e| {
            let id = if direction == "supertypes" {
                &e.target
            } else {
                &e.source
            };
            queries::get_node(&conn, id).ok().flatten()
        })
        .collect();

    Ok(HierarchyResponse {
        root: params.symbol,
        direction: direction.to_string(),
        related,
    })
}

#[derive(Debug, Deserialize)]
pub struct FilesParams {
    pub path: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct FilesResponse {
    pub files: Vec<swiftgraph_core::storage::queries::FileInfo>,
    pub count: usize,
}

pub fn get_files(db_path: &Path, params: FilesParams) -> Result<FilesResponse> {
    let conn = storage::open_db(db_path)?;
    let limit = params.limit.unwrap_or(100);
    let files = queries::get_files(&conn, params.path.as_deref(), limit)?;
    let count = files.len();
    Ok(FilesResponse { files, count })
}
