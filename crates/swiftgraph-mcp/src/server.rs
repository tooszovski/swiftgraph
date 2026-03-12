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
}

#[tool_handler]
impl ServerHandler for SwiftGraphServer {
    fn get_info(&self) -> rmcp::model::ServerInfo {
        let mut info = rmcp::model::ServerInfo::default();
        info.instructions = Some("SwiftGraph: compiler-accurate Swift code graph MCP server. Use swiftgraph_status to check index, swiftgraph_reindex to index files, then query with search/node/callers/callees/references/hierarchy/files.".into());
        info
    }
}
