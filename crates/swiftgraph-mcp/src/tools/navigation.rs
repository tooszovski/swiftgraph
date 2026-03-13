//! Navigation and analysis tool functions.
//!
//! Each function opens a SQLite connection from `db_path`, executes queries,
//! and returns a typed response. These are the building blocks called by MCP
//! tool handlers in `server.rs` and by CLI subcommands in `main.rs`.

use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use swiftgraph_audit::engine::{AuditResult, Category, Severity};
use swiftgraph_audit::runner::{self, AuditOptions};
use swiftgraph_core::analysis;
use swiftgraph_core::graph::{GraphEdge, GraphNode};
use swiftgraph_core::storage::{self, queries};

/// Parameters for symbol search.
#[derive(Debug, Deserialize)]
pub struct SearchParams {
    /// Search query (prefix, wildcard, or empty for list-all).
    pub query: String,
    /// Filter by symbol kind (e.g. "class", "protocol").
    pub kind: Option<String>,
    /// Max results (default 20).
    pub limit: Option<u32>,
}

/// Search result with matching nodes and total count.
#[derive(Debug, Serialize)]
pub struct SearchResponse {
    pub results: Vec<GraphNode>,
    pub total: usize,
}

/// Search for symbols using FTS5 prefix → trigram → LIKE fallback chain.
pub fn search(db_path: &Path, params: SearchParams) -> Result<SearchResponse> {
    let conn = storage::open_db(db_path)?;
    let limit = params.limit.unwrap_or(20);
    let kind = params.kind.as_deref();
    let query = params.query.trim();

    // Handle wildcard / empty query as "list all"
    let is_list_all = query.is_empty() || query == "*";

    let results = if is_list_all {
        // List all (optionally filtered by kind)
        queries::find_nodes_by_name(&conn, "", kind, limit)?
    } else {
        // Try FTS5 with auto-prefix (append * for prefix matching)
        let fts_query = if query.contains('*') || query.contains('"') {
            query.to_string()
        } else {
            format!("{query}*")
        };

        let mut results = queries::search_nodes(&conn, &fts_query, limit).unwrap_or_default();

        // Apply kind filter (FTS5 doesn't support it natively)
        if let Some(k) = kind {
            results.retain(|n| n.kind.as_str() == k);
        }

        // Fallback chain: trigram substring → LIKE
        if results.is_empty() {
            // Try trigram FTS for substring matching (e.g., "Delegate" → "AppDelegate")
            if let Ok(mut tri) = queries::search_nodes_trigram(&conn, query, limit) {
                if let Some(k) = kind {
                    tri.retain(|n| n.kind.as_str() == k);
                }
                results = tri;
            }
        }
        if results.is_empty() {
            results = queries::find_nodes_by_name(&conn, query, kind, limit)?;
        }

        results
    };

    let total = results.len();
    Ok(SearchResponse { results, total })
}

/// Parameters for single-node lookup.
#[derive(Debug, Deserialize)]
pub struct NodeParams {
    /// Symbol ID (USR) or name.
    pub symbol: String,
}

/// Look up a single node by ID or name.
pub fn get_node(db_path: &Path, params: NodeParams) -> Result<Option<GraphNode>> {
    let conn = storage::open_db(db_path)?;
    let node = queries::get_node(&conn, &params.symbol)?;
    Ok(node)
}

/// Parameters for caller/callee/reference queries.
#[derive(Debug, Deserialize)]
pub struct CallersParams {
    /// Symbol ID (USR) or name.
    pub symbol: String,
    /// Max results (default 30).
    pub limit: Option<u32>,
}

/// Response containing edges and their count.
#[derive(Debug, Serialize)]
pub struct EdgesResponse {
    pub edges: Vec<GraphEdge>,
    pub count: usize,
}

/// Find all callers of a symbol (incoming `calls` edges).
pub fn get_callers(db_path: &Path, params: CallersParams) -> Result<EdgesResponse> {
    let conn = storage::open_db(db_path)?;
    let limit = params.limit.unwrap_or(30);
    let edges = queries::get_callers(&conn, &params.symbol, limit)?;
    let count = edges.len();
    Ok(EdgesResponse { edges, count })
}

/// Find all callees of a symbol (outgoing `calls` edges).
pub fn get_callees(db_path: &Path, params: CallersParams) -> Result<EdgesResponse> {
    let conn = storage::open_db(db_path)?;
    let limit = params.limit.unwrap_or(30);
    let edges = queries::get_callees(&conn, &params.symbol, limit)?;
    let count = edges.len();
    Ok(EdgesResponse { edges, count })
}

/// Find all references to a symbol (any incoming edge kind).
pub fn get_references(db_path: &Path, params: CallersParams) -> Result<EdgesResponse> {
    let conn = storage::open_db(db_path)?;
    let limit = params.limit.unwrap_or(50);
    let edges = queries::get_references(&conn, &params.symbol, limit)?;
    let count = edges.len();
    Ok(EdgesResponse { edges, count })
}

/// Parameters for type hierarchy traversal.
#[derive(Debug, Deserialize)]
pub struct HierarchyParams {
    /// Symbol ID (USR) or name.
    pub symbol: String,
    /// Direction: "subtypes" or "supertypes".
    pub direction: Option<String>,
    /// Max depth (default 3).
    pub depth: Option<u32>,
}

/// Hierarchy result with root symbol and related types.
#[derive(Debug, Serialize)]
pub struct HierarchyResponse {
    pub root: String,
    pub direction: String,
    pub related: Vec<GraphNode>,
}

/// Get type hierarchy (subtypes or supertypes) for a symbol.
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

/// Parameters for listing indexed files.
#[derive(Debug, Deserialize)]
pub struct FilesParams {
    /// Filter by path prefix (e.g. "Sources/").
    pub path: Option<String>,
    /// Max results (default 100).
    pub limit: Option<u32>,
}

/// File listing result.
#[derive(Debug, Serialize)]
pub struct FilesResponse {
    pub files: Vec<swiftgraph_core::storage::queries::FileInfo>,
    pub count: usize,
}

/// List indexed files, optionally filtered by path prefix.
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

/// Parameters for extension lookup.
#[derive(Debug, Deserialize)]
pub struct ExtensionsParams {
    pub symbol: String,
    pub limit: Option<u32>,
}

/// Find all extensions of a type.
pub fn get_extensions(db_path: &Path, params: ExtensionsParams) -> Result<EdgesResponse> {
    let conn = storage::open_db(db_path)?;
    let limit = params.limit.unwrap_or(50);
    let edges = queries::get_extensions(&conn, &params.symbol, limit)?;
    let count = edges.len();
    Ok(EdgesResponse { edges, count })
}

// --- v0.2: Conformances ---

/// Parameters for conformance queries.
#[derive(Debug, Deserialize)]
pub struct ConformancesParams {
    /// Symbol ID (USR) or name.
    pub symbol: String,
    /// Direction: "conforms" or "conformedBy".
    pub direction: Option<String>,
    pub limit: Option<u32>,
}

/// Query protocol conformances in either direction.
pub fn get_conformances(db_path: &Path, params: ConformancesParams) -> Result<EdgesResponse> {
    let conn = storage::open_db(db_path)?;
    let direction = params.direction.as_deref().unwrap_or("conforms");
    let limit = params.limit.unwrap_or(50);
    let edges = queries::get_conformances(&conn, &params.symbol, direction, limit)?;
    let count = edges.len();
    Ok(EdgesResponse { edges, count })
}

// --- v0.2: Context ---

/// Parameters for task-relevant context building.
#[derive(Debug, Deserialize)]
pub struct ContextParams {
    /// Task description in natural language.
    pub task: String,
    /// Max nodes to return (default 25).
    pub max_nodes: Option<u32>,
    /// Include test files in results (default false).
    pub include_tests: Option<bool>,
}

/// Build task-relevant context by keyword extraction and graph expansion.
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

/// Parameters for blast-radius impact analysis.
#[derive(Debug, Deserialize)]
pub struct ImpactParams {
    /// Symbol ID (USR) or name.
    pub symbol: String,
    /// Depth of transitive analysis (default 3).
    pub depth: Option<u32>,
}

/// Analyze the blast radius of changing a symbol.
pub fn get_impact(db_path: &Path, params: ImpactParams) -> Result<analysis::impact::ImpactResult> {
    let depth = params.depth.unwrap_or(3);
    // Resolve name to ID if needed
    let symbol_id = resolve_symbol_id(db_path, &params.symbol)?;
    let result = analysis::impact::analyze_impact(db_path, &symbol_id, depth)?;
    Ok(result)
}

// --- v0.2: Diff Impact ---

/// Parameters for git-diff-based impact analysis.
#[derive(Debug, Deserialize)]
pub struct DiffImpactParams {
    /// Git ref: "staged", "unstaged", or a range like "HEAD~3..HEAD".
    pub git_ref: Option<String>,
}

/// Analyze the impact of git-changed symbols.
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

/// Analyze structural complexity (fan-in/fan-out) for symbols.
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

/// Find potentially dead code (symbols with no incoming references).
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

/// Detect file-level dependency cycles via DFS.
pub fn get_cycles(
    db_path: &Path,
    path_filter: Option<&str>,
    max_cycles: Option<u32>,
) -> Result<analysis::cycles::CycleResult> {
    let result = analysis::cycles::detect_cycles(db_path, path_filter, max_cycles.unwrap_or(20))?;
    Ok(result)
}

// --- v0.4: Coupling, Architecture, Imports ---

/// Analyze module coupling metrics (afferent/efferent, instability).
pub fn get_coupling(
    db_path: &Path,
    depth: Option<u32>,
    source_root: Option<&str>,
) -> Result<analysis::coupling::CouplingResult> {
    let result = analysis::coupling::analyze_coupling(db_path, depth.unwrap_or(2), source_root)?;
    Ok(result)
}

/// Auto-detect or validate architectural pattern (MVVM/VIPER/TCA/MVC).
pub fn get_architecture(
    db_path: &Path,
    expected: Option<&str>,
) -> Result<analysis::architecture::ArchitectureResult> {
    let result = analysis::architecture::analyze_architecture(db_path, expected)?;
    Ok(result)
}

/// Analyze module import dependencies.
pub fn get_imports(
    db_path: &Path,
    path_filter: Option<&str>,
) -> Result<analysis::imports::ImportsResult> {
    let result = analysis::imports::analyze_imports(db_path, path_filter)?;
    Ok(result)
}

/// Check architecture boundaries from JSON config.
pub fn get_boundaries(
    db_path: &Path,
    config_json: &str,
) -> Result<analysis::boundaries::BoundaryResult> {
    let config: analysis::boundaries::BoundaryConfig = serde_json::from_str(config_json)
        .map_err(|e| anyhow::anyhow!("Invalid boundary config JSON: {e}"))?;
    let result = analysis::boundaries::check_boundaries(db_path, &config)?;
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
