use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use swiftgraph_audit::engine::{AuditResult, Category, Severity};
use swiftgraph_audit::runner::{self, AuditOptions};
use swiftgraph_core::analysis;
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

/// Resolve a symbol name to its ID. If the input looks like an ID (contains "::"), return as-is.
/// Otherwise, search by name and return the first match.
fn resolve_symbol_id(db_path: &Path, symbol: &str) -> Result<String> {
    if symbol.contains("::") {
        return Ok(symbol.to_string());
    }
    let conn = storage::open_db(db_path)?;
    let results = queries::search_nodes(&conn, symbol, 1)
        .or_else(|_| queries::find_nodes_by_name(&conn, symbol, None, 1))?;
    results
        .into_iter()
        .next()
        .map(|n| n.id)
        .ok_or_else(|| anyhow::anyhow!("symbol not found: {symbol}"))
}

// --- v0.2: Extensions ---

#[derive(Debug, Deserialize)]
pub struct ExtensionsParams {
    pub symbol: String,
    pub limit: Option<u32>,
}

pub fn get_extensions(db_path: &Path, params: ExtensionsParams) -> Result<EdgesResponse> {
    let conn = storage::open_db(db_path)?;
    let limit = params.limit.unwrap_or(50);
    let edges = queries::get_extensions(&conn, &params.symbol, limit)?;
    let count = edges.len();
    Ok(EdgesResponse { edges, count })
}

// --- v0.2: Conformances ---

#[derive(Debug, Deserialize)]
pub struct ConformancesParams {
    pub symbol: String,
    pub direction: Option<String>, // "conforms" | "conformedBy"
    pub limit: Option<u32>,
}

pub fn get_conformances(db_path: &Path, params: ConformancesParams) -> Result<EdgesResponse> {
    let conn = storage::open_db(db_path)?;
    let direction = params.direction.as_deref().unwrap_or("conforms");
    let limit = params.limit.unwrap_or(50);
    let edges = queries::get_conformances(&conn, &params.symbol, direction, limit)?;
    let count = edges.len();
    Ok(EdgesResponse { edges, count })
}

// --- v0.2: Context ---

#[derive(Debug, Deserialize)]
pub struct ContextParams {
    pub task: String,
    pub max_nodes: Option<u32>,
    pub include_tests: Option<bool>,
}

pub fn get_context(
    db_path: &Path,
    params: ContextParams,
) -> Result<analysis::context::ContextResult> {
    let max_nodes = params.max_nodes.unwrap_or(25);
    let include_tests = params.include_tests.unwrap_or(false);
    let result = analysis::context::build_context(db_path, &params.task, max_nodes, include_tests)?;
    Ok(result)
}

// --- v0.2: Impact ---

#[derive(Debug, Deserialize)]
pub struct ImpactParams {
    pub symbol: String,
    pub depth: Option<u32>,
}

pub fn get_impact(db_path: &Path, params: ImpactParams) -> Result<analysis::impact::ImpactResult> {
    let depth = params.depth.unwrap_or(3);
    // Resolve name to ID if needed
    let symbol_id = resolve_symbol_id(db_path, &params.symbol)?;
    let result = analysis::impact::analyze_impact(db_path, &symbol_id, depth)?;
    Ok(result)
}

// --- v0.2: Diff Impact ---

#[derive(Debug, Deserialize)]
pub struct DiffImpactParams {
    pub git_ref: Option<String>, // "staged", "unstaged", "HEAD~3..HEAD"
}

pub fn get_diff_impact(
    db_path: &Path,
    project_root: &Path,
    params: DiffImpactParams,
) -> Result<analysis::diff_impact::DiffImpactResult> {
    let git_ref = params.git_ref.as_deref().unwrap_or("unstaged");
    let result = analysis::diff_impact::analyze_diff_impact(db_path, project_root, git_ref)?;
    Ok(result)
}

// --- v0.4: Analysis ---

pub fn get_complexity(
    db_path: &Path,
    path_filter: Option<&str>,
    limit: Option<u32>,
    sort_by: Option<&str>,
) -> Result<analysis::complexity::ComplexityResult> {
    let result = analysis::complexity::analyze_complexity(
        db_path,
        path_filter,
        limit.unwrap_or(30),
        sort_by.unwrap_or("score"),
    )?;
    Ok(result)
}

pub fn get_dead_code(
    db_path: &Path,
    path_filter: Option<&str>,
    include_tests: bool,
    limit: Option<u32>,
) -> Result<analysis::dead_code::DeadCodeResult> {
    let result = analysis::dead_code::find_dead_code(
        db_path,
        path_filter,
        include_tests,
        limit.unwrap_or(50),
    )?;
    Ok(result)
}

pub fn get_cycles(
    db_path: &Path,
    path_filter: Option<&str>,
    max_cycles: Option<u32>,
) -> Result<analysis::cycles::CycleResult> {
    let result = analysis::cycles::detect_cycles(db_path, path_filter, max_cycles.unwrap_or(20))?;
    Ok(result)
}

// --- v0.4: Coupling, Architecture, Imports ---

pub fn get_coupling(
    db_path: &Path,
    depth: Option<u32>,
    source_root: Option<&str>,
) -> Result<analysis::coupling::CouplingResult> {
    let result = analysis::coupling::analyze_coupling(db_path, depth.unwrap_or(2), source_root)?;
    Ok(result)
}

pub fn get_architecture(
    db_path: &Path,
    expected: Option<&str>,
) -> Result<analysis::architecture::ArchitectureResult> {
    let result = analysis::architecture::analyze_architecture(db_path, expected)?;
    Ok(result)
}

pub fn get_imports(
    db_path: &Path,
    path_filter: Option<&str>,
) -> Result<analysis::imports::ImportsResult> {
    let result = analysis::imports::analyze_imports(db_path, path_filter)?;
    Ok(result)
}

// --- v0.3: Audit ---

/// Parse audit options from string parameters.
pub fn parse_audit_options(
    categories: Option<&str>,
    min_severity: Option<&str>,
    path_filter: Option<String>,
    max_issues: Option<usize>,
) -> AuditOptions {
    let cats = categories
        .map(|s| {
            s.split(',')
                .filter_map(|c| match c.trim() {
                    "concurrency" => Some(Category::Concurrency),
                    "memory" => Some(Category::Memory),
                    "security" => Some(Category::Security),
                    "swiftui-performance" | "swiftui_performance" => {
                        Some(Category::SwiftuiPerformance)
                    }
                    "swiftui-architecture" | "swiftui_architecture" => {
                        Some(Category::SwiftuiArchitecture)
                    }
                    "networking" => Some(Category::Networking),
                    "codable" => Some(Category::Codable),
                    "energy" => Some(Category::Energy),
                    "storage" => Some(Category::Storage),
                    "accessibility" => Some(Category::Accessibility),
                    "testing" => Some(Category::Testing),
                    "modernization" => Some(Category::Modernization),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();

    let severity = match min_severity {
        Some("critical") => Severity::Critical,
        Some("high") => Severity::High,
        Some("medium") => Severity::Medium,
        _ => Severity::Low,
    };

    AuditOptions {
        categories: cats,
        min_severity: severity,
        path_filter,
        max_issues: max_issues.unwrap_or(100),
    }
}

/// Run audit on a project.
pub fn run_audit(project_root: &Path, options: AuditOptions) -> Result<AuditResult> {
    let result = runner::run_audit(project_root, &options)?;
    Ok(result)
}
