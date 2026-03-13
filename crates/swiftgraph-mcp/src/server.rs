use std::path::PathBuf;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::schemars;
use rmcp::{tool, tool_handler, tool_router, ServerHandler};
use serde::Deserialize;
use serde_json::json;

use crate::tools::{navigation, status};

// --- Parameter types for MCP tools ---

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct EmptyParams {}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReindexParams {
    /// Force full reindex
    pub force: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchToolParams {
    /// Search query
    pub query: String,
    /// Filter by symbol kind (class, struct, enum, protocol, method, etc.)
    pub kind: Option<String>,
    /// Max results (default 20)
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SymbolParams {
    /// Symbol ID (USR) or name
    pub symbol: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SymbolLimitParams {
    /// Symbol ID (USR) or name
    pub symbol: String,
    /// Max results (default 30)
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct HierarchyToolParams {
    /// Symbol ID (USR) or name
    pub symbol: String,
    /// Direction: "subtypes" or "supertypes"
    pub direction: Option<String>,
    /// Max depth (default 3)
    pub depth: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct FilesToolParams {
    /// Filter by path prefix (e.g. "Sources/Features/")
    pub path: Option<String>,
    /// Max results (default 100)
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ExtensionsToolParams {
    /// Symbol ID (USR) or name
    pub symbol: String,
    /// Max results (default 50)
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ConformancesToolParams {
    /// Symbol ID (USR) or name — typically a protocol name
    pub symbol: String,
    /// Direction: "conforms" (what does symbol conform to) or "conformedBy" (who conforms to symbol)
    pub direction: Option<String>,
    /// Max results (default 50)
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ContextToolParams {
    /// Task description in natural language
    pub task: String,
    /// Max nodes to return (default 25)
    pub max_nodes: Option<u32>,
    /// Include test files in results (default false)
    pub include_tests: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ImpactToolParams {
    /// Symbol ID (USR) or name
    pub symbol: String,
    /// Depth of transitive analysis (default 3)
    pub depth: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DiffImpactToolParams {
    /// Git ref: "staged", "unstaged", or a range like "HEAD~3..HEAD"
    #[serde(rename = "ref")]
    pub git_ref: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ComplexityToolParams {
    /// Filter by file path prefix
    pub path: Option<String>,
    /// Max symbols to return (default 30)
    pub limit: Option<u32>,
    /// Sort by: "score", "fan_in", or "fan_out" (default "score")
    pub sort_by: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DeadCodeToolParams {
    /// Filter by file path prefix
    pub path: Option<String>,
    /// Include test files (default false)
    pub include_tests: Option<bool>,
    /// Max results (default 50)
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CyclesToolParams {
    /// Filter by file path prefix
    pub path: Option<String>,
    /// Max cycles to return (default 20)
    pub max_cycles: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CouplingToolParams {
    /// Directory depth for module grouping (default 2)
    pub depth: Option<u32>,
    /// Source root prefix to strip from paths
    pub source_root: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ArchitectureToolParams {
    /// Expected pattern to validate: "mvvm", "viper", "tca", "mvc". Empty = auto-detect
    pub expected: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ImportsToolParams {
    /// Filter by file path prefix
    pub path: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AuditToolParams {
    /// Comma-separated categories to check (e.g. "concurrency,memory,security"). Empty = all
    pub categories: Option<String>,
    /// Minimum severity: "low", "medium", "high", "critical" (default "low")
    pub min_severity: Option<String>,
    /// Filter by file path prefix (e.g. "Sources/Features/")
    pub path_filter: Option<String>,
    /// Max issues to return (default 100)
    pub max_issues: Option<usize>,
}

/// SwiftGraph MCP Server state.
#[derive(Clone)]
pub struct SwiftGraphServer {
    pub project_root: PathBuf,
    pub db_path: PathBuf,
    tool_router: ToolRouter<Self>,
}

impl SwiftGraphServer {
    pub fn new(project_root: PathBuf) -> Self {
        let db_path = project_root.join(".swiftgraph/db.sqlite");
        Self {
            project_root,
            db_path,
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_router]
impl SwiftGraphServer {
    /// Get SwiftGraph index status, project info, and statistics
    #[tool(name = "swiftgraph_status")]
    pub async fn swiftgraph_status(
        &self,
        rmcp::handler::server::wrapper::Parameters(_params): rmcp::handler::server::wrapper::Parameters<EmptyParams>,
    ) -> String {
        match status::get_status(&self.project_root) {
            Ok(resp) => serde_json::to_string_pretty(&resp).unwrap_or_default(),
            Err(e) => json!({"error": e.to_string()}).to_string(),
        }
    }

    /// Reindex Swift files in the project
    #[tool(name = "swiftgraph_reindex")]
    pub async fn swiftgraph_reindex(
        &self,
        rmcp::handler::server::wrapper::Parameters(params): rmcp::handler::server::wrapper::Parameters<ReindexParams>,
    ) -> String {
        match swiftgraph_core::pipeline::index_directory(
            &self.db_path,
            &self.project_root,
            params.force.unwrap_or(false),
        ) {
            Ok(result) => json!({
                "files_scanned": result.files_scanned,
                "files_indexed": result.files_indexed,
                "nodes_added": result.nodes_added,
                "edges_added": result.edges_added
            })
            .to_string(),
            Err(e) => json!({"error": e.to_string()}).to_string(),
        }
    }

    /// Search for symbols by name or pattern. Supports fuzzy matching via FTS5
    #[tool(name = "swiftgraph_search")]
    pub async fn swiftgraph_search(
        &self,
        rmcp::handler::server::wrapper::Parameters(params): rmcp::handler::server::wrapper::Parameters<SearchToolParams>,
    ) -> String {
        let nav_params = navigation::SearchParams {
            query: params.query,
            kind: params.kind,
            limit: params.limit,
        };
        match navigation::search(&self.db_path, nav_params) {
            Ok(resp) => serde_json::to_string_pretty(&resp).unwrap_or_default(),
            Err(e) => json!({"error": e.to_string()}).to_string(),
        }
    }

    /// Get detailed info about a symbol by its ID or name
    #[tool(name = "swiftgraph_node")]
    pub async fn swiftgraph_node(
        &self,
        rmcp::handler::server::wrapper::Parameters(params): rmcp::handler::server::wrapper::Parameters<SymbolParams>,
    ) -> String {
        let nav_params = navigation::NodeParams {
            symbol: params.symbol,
        };
        match navigation::get_node(&self.db_path, nav_params) {
            Ok(Some(node)) => serde_json::to_string_pretty(&node).unwrap_or_default(),
            Ok(None) => json!({"error": "symbol not found"}).to_string(),
            Err(e) => json!({"error": e.to_string()}).to_string(),
        }
    }

    /// Find all callers of a symbol (compiler-accurate via USR)
    #[tool(name = "swiftgraph_callers")]
    pub async fn swiftgraph_callers(
        &self,
        rmcp::handler::server::wrapper::Parameters(params): rmcp::handler::server::wrapper::Parameters<SymbolLimitParams>,
    ) -> String {
        let nav_params = navigation::CallersParams {
            symbol: params.symbol,
            limit: params.limit,
        };
        match navigation::get_callers(&self.db_path, nav_params) {
            Ok(resp) => serde_json::to_string_pretty(&resp).unwrap_or_default(),
            Err(e) => json!({"error": e.to_string()}).to_string(),
        }
    }

    /// Find all callees of a symbol
    #[tool(name = "swiftgraph_callees")]
    pub async fn swiftgraph_callees(
        &self,
        rmcp::handler::server::wrapper::Parameters(params): rmcp::handler::server::wrapper::Parameters<SymbolLimitParams>,
    ) -> String {
        let nav_params = navigation::CallersParams {
            symbol: params.symbol,
            limit: params.limit,
        };
        match navigation::get_callees(&self.db_path, nav_params) {
            Ok(resp) => serde_json::to_string_pretty(&resp).unwrap_or_default(),
            Err(e) => json!({"error": e.to_string()}).to_string(),
        }
    }

    /// Find all references to a symbol (broader than callers — includes reads, type annotations)
    #[tool(name = "swiftgraph_references")]
    pub async fn swiftgraph_references(
        &self,
        rmcp::handler::server::wrapper::Parameters(params): rmcp::handler::server::wrapper::Parameters<SymbolLimitParams>,
    ) -> String {
        let nav_params = navigation::CallersParams {
            symbol: params.symbol,
            limit: params.limit,
        };
        match navigation::get_references(&self.db_path, nav_params) {
            Ok(resp) => serde_json::to_string_pretty(&resp).unwrap_or_default(),
            Err(e) => json!({"error": e.to_string()}).to_string(),
        }
    }

    /// Get type hierarchy (subtypes/supertypes) for a symbol
    #[tool(name = "swiftgraph_hierarchy")]
    pub async fn swiftgraph_hierarchy(
        &self,
        rmcp::handler::server::wrapper::Parameters(params): rmcp::handler::server::wrapper::Parameters<HierarchyToolParams>,
    ) -> String {
        let nav_params = navigation::HierarchyParams {
            symbol: params.symbol,
            direction: params.direction,
            depth: params.depth,
        };
        match navigation::get_hierarchy(&self.db_path, nav_params) {
            Ok(resp) => serde_json::to_string_pretty(&resp).unwrap_or_default(),
            Err(e) => json!({"error": e.to_string()}).to_string(),
        }
    }

    /// List indexed files with stats (node count, last indexed). Filter by path prefix
    #[tool(name = "swiftgraph_files")]
    pub async fn swiftgraph_files(
        &self,
        rmcp::handler::server::wrapper::Parameters(params): rmcp::handler::server::wrapper::Parameters<FilesToolParams>,
    ) -> String {
        let nav_params = navigation::FilesParams {
            path: params.path,
            limit: params.limit,
        };
        match navigation::get_files(&self.db_path, nav_params) {
            Ok(resp) => serde_json::to_string_pretty(&resp).unwrap_or_default(),
            Err(e) => json!({"error": e.to_string()}).to_string(),
        }
    }

    /// Find all extensions of a type
    #[tool(name = "swiftgraph_extensions")]
    pub async fn swiftgraph_extensions(
        &self,
        rmcp::handler::server::wrapper::Parameters(params): rmcp::handler::server::wrapper::Parameters<ExtensionsToolParams>,
    ) -> String {
        let nav_params = navigation::ExtensionsParams {
            symbol: params.symbol,
            limit: params.limit,
        };
        match navigation::get_extensions(&self.db_path, nav_params) {
            Ok(resp) => serde_json::to_string_pretty(&resp).unwrap_or_default(),
            Err(e) => json!({"error": e.to_string()}).to_string(),
        }
    }

    /// Query protocol conformances — who conforms to a protocol, or what protocols a type conforms to
    #[tool(name = "swiftgraph_conformances")]
    pub async fn swiftgraph_conformances(
        &self,
        rmcp::handler::server::wrapper::Parameters(params): rmcp::handler::server::wrapper::Parameters<ConformancesToolParams>,
    ) -> String {
        let nav_params = navigation::ConformancesParams {
            symbol: params.symbol,
            direction: params.direction,
            limit: params.limit,
        };
        match navigation::get_conformances(&self.db_path, nav_params) {
            Ok(resp) => serde_json::to_string_pretty(&resp).unwrap_or_default(),
            Err(e) => json!({"error": e.to_string()}).to_string(),
        }
    }

    /// Build task-relevant context: extracts keywords, searches graph, expands 2 levels, ranks by importance
    #[tool(name = "swiftgraph_context")]
    pub async fn swiftgraph_context(
        &self,
        rmcp::handler::server::wrapper::Parameters(params): rmcp::handler::server::wrapper::Parameters<ContextToolParams>,
    ) -> String {
        let nav_params = navigation::ContextParams {
            task: params.task,
            max_nodes: params.max_nodes,
            include_tests: params.include_tests,
        };
        match navigation::get_context(&self.db_path, nav_params) {
            Ok(resp) => serde_json::to_string_pretty(&resp).unwrap_or_default(),
            Err(e) => json!({"error": e.to_string()}).to_string(),
        }
    }

    /// Analyze blast radius of changing a symbol — direct/transitive impact, affected files/tests
    #[tool(name = "swiftgraph_impact")]
    pub async fn swiftgraph_impact(
        &self,
        rmcp::handler::server::wrapper::Parameters(params): rmcp::handler::server::wrapper::Parameters<ImpactToolParams>,
    ) -> String {
        let nav_params = navigation::ImpactParams {
            symbol: params.symbol,
            depth: params.depth,
        };
        match navigation::get_impact(&self.db_path, nav_params) {
            Ok(resp) => serde_json::to_string_pretty(&resp).unwrap_or_default(),
            Err(e) => json!({"error": e.to_string()}).to_string(),
        }
    }

    /// Analyze impact of git diff — changed symbols, blast radius, affected tests
    #[tool(name = "swiftgraph_diff_impact")]
    pub async fn swiftgraph_diff_impact(
        &self,
        rmcp::handler::server::wrapper::Parameters(params): rmcp::handler::server::wrapper::Parameters<DiffImpactToolParams>,
    ) -> String {
        let nav_params = navigation::DiffImpactParams {
            git_ref: params.git_ref,
        };
        match navigation::get_diff_impact(&self.db_path, &self.project_root, nav_params) {
            Ok(resp) => serde_json::to_string_pretty(&resp).unwrap_or_default(),
            Err(e) => json!({"error": e.to_string()}).to_string(),
        }
    }

    /// Analyze structural complexity — fan-in/fan-out metrics for symbols
    #[tool(name = "swiftgraph_complexity")]
    pub async fn swiftgraph_complexity(
        &self,
        rmcp::handler::server::wrapper::Parameters(params): rmcp::handler::server::wrapper::Parameters<ComplexityToolParams>,
    ) -> String {
        match navigation::get_complexity(
            &self.db_path,
            params.path.as_deref(),
            params.limit,
            params.sort_by.as_deref(),
        ) {
            Ok(resp) => serde_json::to_string_pretty(&resp).unwrap_or_default(),
            Err(e) => json!({"error": e.to_string()}).to_string(),
        }
    }

    /// Find potentially dead code — symbols with no incoming references
    #[tool(name = "swiftgraph_dead_code")]
    pub async fn swiftgraph_dead_code(
        &self,
        rmcp::handler::server::wrapper::Parameters(params): rmcp::handler::server::wrapper::Parameters<DeadCodeToolParams>,
    ) -> String {
        match navigation::get_dead_code(
            &self.db_path,
            params.path.as_deref(),
            params.include_tests.unwrap_or(false),
            params.limit,
        ) {
            Ok(resp) => serde_json::to_string_pretty(&resp).unwrap_or_default(),
            Err(e) => json!({"error": e.to_string()}).to_string(),
        }
    }

    /// Detect file-level dependency cycles
    #[tool(name = "swiftgraph_cycles")]
    pub async fn swiftgraph_cycles(
        &self,
        rmcp::handler::server::wrapper::Parameters(params): rmcp::handler::server::wrapper::Parameters<CyclesToolParams>,
    ) -> String {
        match navigation::get_cycles(&self.db_path, params.path.as_deref(), params.max_cycles) {
            Ok(resp) => serde_json::to_string_pretty(&resp).unwrap_or_default(),
            Err(e) => json!({"error": e.to_string()}).to_string(),
        }
    }

    /// Analyze module coupling — afferent/efferent coupling, instability, abstractness, distance from main sequence
    #[tool(name = "swiftgraph_coupling")]
    pub async fn swiftgraph_coupling(
        &self,
        rmcp::handler::server::wrapper::Parameters(params): rmcp::handler::server::wrapper::Parameters<CouplingToolParams>,
    ) -> String {
        match navigation::get_coupling(&self.db_path, params.depth, params.source_root.as_deref()) {
            Ok(resp) => serde_json::to_string_pretty(&resp).unwrap_or_default(),
            Err(e) => json!({"error": e.to_string()}).to_string(),
        }
    }

    /// Auto-detect or validate architectural pattern (MVVM, VIPER, TCA, MVC) with evidence and violations
    #[tool(name = "swiftgraph_architecture")]
    pub async fn swiftgraph_architecture(
        &self,
        rmcp::handler::server::wrapper::Parameters(params): rmcp::handler::server::wrapper::Parameters<ArchitectureToolParams>,
    ) -> String {
        match navigation::get_architecture(&self.db_path, params.expected.as_deref()) {
            Ok(resp) => serde_json::to_string_pretty(&resp).unwrap_or_default(),
            Err(e) => json!({"error": e.to_string()}).to_string(),
        }
    }

    /// Analyze module import dependencies — which modules are imported, by how many files
    #[tool(name = "swiftgraph_imports")]
    pub async fn swiftgraph_imports(
        &self,
        rmcp::handler::server::wrapper::Parameters(params): rmcp::handler::server::wrapper::Parameters<ImportsToolParams>,
    ) -> String {
        match navigation::get_imports(&self.db_path, params.path.as_deref()) {
            Ok(resp) => serde_json::to_string_pretty(&resp).unwrap_or_default(),
            Err(e) => json!({"error": e.to_string()}).to_string(),
        }
    }

    /// Run static analysis audit — checks for concurrency, memory, and security issues
    #[tool(name = "swiftgraph_audit")]
    pub async fn swiftgraph_audit(
        &self,
        rmcp::handler::server::wrapper::Parameters(params): rmcp::handler::server::wrapper::Parameters<AuditToolParams>,
    ) -> String {
        let options = navigation::parse_audit_options(
            params.categories.as_deref(),
            params.min_severity.as_deref(),
            params.path_filter,
            params.max_issues,
        );
        match navigation::run_audit(&self.project_root, options) {
            Ok(resp) => serde_json::to_string_pretty(&resp).unwrap_or_default(),
            Err(e) => json!({"error": e.to_string()}).to_string(),
        }
    }
}

#[tool_handler]
impl ServerHandler for SwiftGraphServer {
    fn get_info(&self) -> rmcp::model::ServerInfo {
        let mut info = rmcp::model::ServerInfo::default();
        info.instructions = Some("SwiftGraph: compiler-accurate Swift code graph MCP server. Tools: status, reindex, search, node, callers, callees, references, hierarchy, files, extensions, conformances, context, impact, diff_impact, complexity, dead_code, cycles, coupling, architecture, imports, audit.".into());
        info
    }
}
