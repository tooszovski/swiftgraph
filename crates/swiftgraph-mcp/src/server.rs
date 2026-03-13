use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use lru::LruCache;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::model::ServerCapabilities;
use rmcp::schemars;
use rmcp::{tool, tool_handler, tool_router, ServerHandler};
use serde::Deserialize;
use serde_json::json;

use crate::tools::{concurrency, navigation, status};

/// LRU cache for MCP tool responses, keyed by (tool_name, params_hash).
type ResponseCache = Arc<Mutex<LruCache<String, String>>>;

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
    /// Include source code snippet (default false)
    pub include_code: Option<bool>,
    /// Include relations: conformances, extensions, container info (default false)
    pub include_relations: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SymbolLimitParams {
    /// Symbol ID (USR) or name
    pub symbol: String,
    /// Max results (default 30)
    pub limit: Option<u32>,
    /// Include transitive callers/callees via BFS (default false)
    pub transitive: Option<bool>,
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
pub struct BoundariesToolParams {
    /// JSON string of boundary config with layers and rules.
    /// Example: {"layers": [{"name": "Views", "pattern": "**/Views/**"}, {"name": "Services", "pattern": "**/Services/**"}], "rules": [{"from": "Views", "to": "Services", "allowed": false}]}
    pub config: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AuditToolParams {
    /// Comma-separated categories to check (e.g. "concurrency,memory,security,performance"). Empty = all
    pub categories: Option<String>,
    /// Minimum severity: "low", "medium", "high", "critical" (default "low")
    pub min_severity: Option<String>,
    /// Filter by file path prefix (e.g. "Sources/Features/")
    pub path_filter: Option<String>,
    /// Max issues to return (default 100)
    pub max_issues: Option<usize>,
    /// Include fix suggestions in output (default true). When false, fix field is stripped from results.
    #[serde(default)]
    pub fix_suggestions: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ConcurrencyToolParams {
    /// Symbol ID (USR) or name to analyze for concurrency patterns
    pub symbol: String,
}

/// SwiftGraph MCP server holding project state, DB path, and response cache.
///
/// Created once per project and serves all 22 MCP tools. Uses an LRU cache
/// for hot-path queries (search) that is invalidated on reindex.
#[derive(Clone)]
pub struct SwiftGraphServer {
    /// Root directory of the Swift project.
    pub project_root: PathBuf,
    /// Path to `.swiftgraph/db.sqlite`.
    pub db_path: PathBuf,
    tool_router: ToolRouter<Self>,
    cache: ResponseCache,
}

const CACHE_CAPACITY: usize = 256;

impl SwiftGraphServer {
    /// Create a new server for the given project root.
    pub fn new(project_root: PathBuf) -> Self {
        let db_path = project_root.join(".swiftgraph/db.sqlite");
        Self {
            project_root,
            db_path,
            tool_router: Self::tool_router(),
            cache: Arc::new(Mutex::new(LruCache::new(
                std::num::NonZeroUsize::new(CACHE_CAPACITY).unwrap(),
            ))),
        }
    }

    /// Get a cached response or compute and cache it.
    fn cached(&self, key: &str, f: impl FnOnce() -> String) -> String {
        let key = key.to_string();
        if let Ok(mut cache) = self.cache.lock() {
            if let Some(cached) = cache.get(&key) {
                return cached.clone();
            }
        }
        let result = f();
        if let Ok(mut cache) = self.cache.lock() {
            cache.put(key, result.clone());
        }
        result
    }

    /// Execute a tool handler body within a tracing span that includes a unique request ID.
    fn with_request_span(&self, tool: &str, f: impl FnOnce() -> String) -> String {
        let request_id = uuid::Uuid::new_v4();
        let span = tracing::info_span!("mcp_tool", %request_id, tool);
        let _guard = span.enter();
        tracing::info!("started");
        let start = std::time::Instant::now();
        let result = f();
        tracing::info!(elapsed_ms = start.elapsed().as_millis() as u64, "completed");
        result
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
        let project_root = self.project_root.clone();
        self.with_request_span("swiftgraph_status", || {
            match status::get_status(&project_root) {
                Ok(resp) => serde_json::to_string_pretty(&resp).unwrap_or_default(),
                Err(e) => json!({"error": e.to_string()}).to_string(),
            }
        })
    }

    /// Reindex Swift files in the project
    #[tool(name = "swiftgraph_reindex")]
    pub async fn swiftgraph_reindex(
        &self,
        rmcp::handler::server::wrapper::Parameters(params): rmcp::handler::server::wrapper::Parameters<ReindexParams>,
    ) -> String {
        let db_path = self.db_path.clone();
        let project_root = self.project_root.clone();
        let cache = self.cache.clone();
        self.with_request_span("swiftgraph_reindex", || {
            let result = swiftgraph_core::pipeline::index_directory(
                &db_path,
                &project_root,
                params.force.unwrap_or(false),
            );
            match result {
                Ok(result) => {
                    if let Ok(mut c) = cache.lock() {
                        c.clear();
                    }
                    json!({
                        "files_scanned": result.files_scanned,
                        "files_indexed": result.files_indexed,
                        "nodes_added": result.nodes_added,
                        "edges_added": result.edges_added
                    })
                    .to_string()
                }
                Err(e) => json!({"error": e.to_string()}).to_string(),
            }
        })
    }

    /// Search for symbols by name or pattern. Supports fuzzy matching via FTS5
    #[tool(name = "swiftgraph_search")]
    pub async fn swiftgraph_search(
        &self,
        rmcp::handler::server::wrapper::Parameters(params): rmcp::handler::server::wrapper::Parameters<SearchToolParams>,
    ) -> String {
        let cache_key = format!(
            "search:{}:{}:{}",
            params.query,
            params.kind.as_deref().unwrap_or(""),
            params.limit.unwrap_or(20)
        );
        let db_path = self.db_path.clone();
        self.with_request_span("swiftgraph_search", || {
            self.cached(&cache_key, || {
                let nav_params = navigation::SearchParams {
                    query: params.query,
                    kind: params.kind,
                    limit: params.limit,
                };
                match navigation::search(&db_path, nav_params) {
                    Ok(resp) => serde_json::to_string_pretty(&resp).unwrap_or_default(),
                    Err(e) => json!({"error": e.to_string()}).to_string(),
                }
            })
        })
    }

    /// Get detailed info about a symbol by its ID or name. Optionally include source code and relations.
    #[tool(name = "swiftgraph_node")]
    pub async fn swiftgraph_node(
        &self,
        rmcp::handler::server::wrapper::Parameters(params): rmcp::handler::server::wrapper::Parameters<SymbolParams>,
    ) -> String {
        let db_path = self.db_path.clone();
        self.with_request_span("swiftgraph_node", || {
            let nav_params = navigation::NodeParams {
                symbol: params.symbol,
                include_code: params.include_code.unwrap_or(false),
                include_relations: params.include_relations.unwrap_or(false),
            };
            match navigation::get_node_detailed(&db_path, nav_params) {
                Ok(Some(resp)) => serde_json::to_string_pretty(&resp).unwrap_or_default(),
                Ok(None) => json!({"error": "symbol not found"}).to_string(),
                Err(e) => json!({"error": e.to_string()}).to_string(),
            }
        })
    }

    /// Find all callers of a symbol (compiler-accurate via USR). Set transitive=true for BFS expansion.
    #[tool(name = "swiftgraph_callers")]
    pub async fn swiftgraph_callers(
        &self,
        rmcp::handler::server::wrapper::Parameters(params): rmcp::handler::server::wrapper::Parameters<SymbolLimitParams>,
    ) -> String {
        let db_path = self.db_path.clone();
        self.with_request_span("swiftgraph_callers", || {
            if params.transitive.unwrap_or(false) {
                match navigation::get_transitive_callers(
                    &db_path,
                    &params.symbol,
                    params.limit.unwrap_or(30),
                ) {
                    Ok(resp) => serde_json::to_string_pretty(&resp).unwrap_or_default(),
                    Err(e) => json!({"error": e.to_string()}).to_string(),
                }
            } else {
                let nav_params = navigation::CallersParams {
                    symbol: params.symbol,
                    limit: params.limit,
                };
                match navigation::get_callers(&db_path, nav_params) {
                    Ok(resp) => serde_json::to_string_pretty(&resp).unwrap_or_default(),
                    Err(e) => json!({"error": e.to_string()}).to_string(),
                }
            }
        })
    }

    /// Find all callees of a symbol
    #[tool(name = "swiftgraph_callees")]
    pub async fn swiftgraph_callees(
        &self,
        rmcp::handler::server::wrapper::Parameters(params): rmcp::handler::server::wrapper::Parameters<SymbolLimitParams>,
    ) -> String {
        let db_path = self.db_path.clone();
        self.with_request_span("swiftgraph_callees", || {
            let nav_params = navigation::CallersParams {
                symbol: params.symbol,
                limit: params.limit,
            };
            match navigation::get_callees(&db_path, nav_params) {
                Ok(resp) => serde_json::to_string_pretty(&resp).unwrap_or_default(),
                Err(e) => json!({"error": e.to_string()}).to_string(),
            }
        })
    }

    /// Find all references to a symbol (broader than callers — includes reads, type annotations)
    #[tool(name = "swiftgraph_references")]
    pub async fn swiftgraph_references(
        &self,
        rmcp::handler::server::wrapper::Parameters(params): rmcp::handler::server::wrapper::Parameters<SymbolLimitParams>,
    ) -> String {
        let db_path = self.db_path.clone();
        self.with_request_span("swiftgraph_references", || {
            let nav_params = navigation::CallersParams {
                symbol: params.symbol,
                limit: params.limit,
            };
            match navigation::get_references(&db_path, nav_params) {
                Ok(resp) => serde_json::to_string_pretty(&resp).unwrap_or_default(),
                Err(e) => json!({"error": e.to_string()}).to_string(),
            }
        })
    }

    /// Get type hierarchy (subtypes/supertypes) for a symbol
    #[tool(name = "swiftgraph_hierarchy")]
    pub async fn swiftgraph_hierarchy(
        &self,
        rmcp::handler::server::wrapper::Parameters(params): rmcp::handler::server::wrapper::Parameters<HierarchyToolParams>,
    ) -> String {
        let db_path = self.db_path.clone();
        self.with_request_span("swiftgraph_hierarchy", || {
            let nav_params = navigation::HierarchyParams {
                symbol: params.symbol,
                direction: params.direction,
                depth: params.depth,
            };
            match navigation::get_hierarchy(&db_path, nav_params) {
                Ok(resp) => serde_json::to_string_pretty(&resp).unwrap_or_default(),
                Err(e) => json!({"error": e.to_string()}).to_string(),
            }
        })
    }

    /// List indexed files with stats (node count, last indexed). Filter by path prefix
    #[tool(name = "swiftgraph_files")]
    pub async fn swiftgraph_files(
        &self,
        rmcp::handler::server::wrapper::Parameters(params): rmcp::handler::server::wrapper::Parameters<FilesToolParams>,
    ) -> String {
        let db_path = self.db_path.clone();
        self.with_request_span("swiftgraph_files", || {
            let nav_params = navigation::FilesParams {
                path: params.path,
                limit: params.limit,
            };
            match navigation::get_files(&db_path, nav_params) {
                Ok(resp) => serde_json::to_string_pretty(&resp).unwrap_or_default(),
                Err(e) => json!({"error": e.to_string()}).to_string(),
            }
        })
    }

    /// Find all extensions of a type
    #[tool(name = "swiftgraph_extensions")]
    pub async fn swiftgraph_extensions(
        &self,
        rmcp::handler::server::wrapper::Parameters(params): rmcp::handler::server::wrapper::Parameters<ExtensionsToolParams>,
    ) -> String {
        let db_path = self.db_path.clone();
        self.with_request_span("swiftgraph_extensions", || {
            let nav_params = navigation::ExtensionsParams {
                symbol: params.symbol,
                limit: params.limit,
            };
            match navigation::get_extensions(&db_path, nav_params) {
                Ok(resp) => serde_json::to_string_pretty(&resp).unwrap_or_default(),
                Err(e) => json!({"error": e.to_string()}).to_string(),
            }
        })
    }

    /// Query protocol conformances — who conforms to a protocol, or what protocols a type conforms to
    #[tool(name = "swiftgraph_conformances")]
    pub async fn swiftgraph_conformances(
        &self,
        rmcp::handler::server::wrapper::Parameters(params): rmcp::handler::server::wrapper::Parameters<ConformancesToolParams>,
    ) -> String {
        let db_path = self.db_path.clone();
        self.with_request_span("swiftgraph_conformances", || {
            let nav_params = navigation::ConformancesParams {
                symbol: params.symbol,
                direction: params.direction,
                limit: params.limit,
            };
            match navigation::get_conformances(&db_path, nav_params) {
                Ok(resp) => serde_json::to_string_pretty(&resp).unwrap_or_default(),
                Err(e) => json!({"error": e.to_string()}).to_string(),
            }
        })
    }

    /// Build task-relevant context: extracts keywords, searches graph, expands 2 levels, ranks by importance
    #[tool(name = "swiftgraph_context")]
    pub async fn swiftgraph_context(
        &self,
        rmcp::handler::server::wrapper::Parameters(params): rmcp::handler::server::wrapper::Parameters<ContextToolParams>,
    ) -> String {
        let db_path = self.db_path.clone();
        self.with_request_span("swiftgraph_context", || {
            let nav_params = navigation::ContextParams {
                task: params.task,
                max_nodes: params.max_nodes,
                include_tests: params.include_tests,
            };
            match navigation::get_context(&db_path, nav_params) {
                Ok(resp) => serde_json::to_string_pretty(&resp).unwrap_or_default(),
                Err(e) => json!({"error": e.to_string()}).to_string(),
            }
        })
    }

    /// Analyze blast radius of changing a symbol — direct/transitive impact, affected files/tests
    #[tool(name = "swiftgraph_impact")]
    pub async fn swiftgraph_impact(
        &self,
        rmcp::handler::server::wrapper::Parameters(params): rmcp::handler::server::wrapper::Parameters<ImpactToolParams>,
    ) -> String {
        let db_path = self.db_path.clone();
        self.with_request_span("swiftgraph_impact", || {
            let nav_params = navigation::ImpactParams {
                symbol: params.symbol,
                depth: params.depth,
            };
            match navigation::get_impact(&db_path, nav_params) {
                Ok(resp) => serde_json::to_string_pretty(&resp).unwrap_or_default(),
                Err(e) => json!({"error": e.to_string()}).to_string(),
            }
        })
    }

    /// Analyze impact of git diff — changed symbols, blast radius, affected tests
    #[tool(name = "swiftgraph_diff_impact")]
    pub async fn swiftgraph_diff_impact(
        &self,
        rmcp::handler::server::wrapper::Parameters(params): rmcp::handler::server::wrapper::Parameters<DiffImpactToolParams>,
    ) -> String {
        let db_path = self.db_path.clone();
        let project_root = self.project_root.clone();
        self.with_request_span("swiftgraph_diff_impact", || {
            let nav_params = navigation::DiffImpactParams {
                git_ref: params.git_ref,
            };
            match navigation::get_diff_impact(&db_path, &project_root, nav_params) {
                Ok(resp) => serde_json::to_string_pretty(&resp).unwrap_or_default(),
                Err(e) => json!({"error": e.to_string()}).to_string(),
            }
        })
    }

    /// Analyze structural complexity — fan-in/fan-out metrics for symbols
    #[tool(name = "swiftgraph_complexity")]
    pub async fn swiftgraph_complexity(
        &self,
        rmcp::handler::server::wrapper::Parameters(params): rmcp::handler::server::wrapper::Parameters<ComplexityToolParams>,
    ) -> String {
        let db_path = self.db_path.clone();
        self.with_request_span(
            "swiftgraph_complexity",
            || match navigation::get_complexity(
                &db_path,
                params.path.as_deref(),
                params.limit,
                params.sort_by.as_deref(),
            ) {
                Ok(resp) => serde_json::to_string_pretty(&resp).unwrap_or_default(),
                Err(e) => json!({"error": e.to_string()}).to_string(),
            },
        )
    }

    /// Find potentially dead code — symbols with no incoming references
    #[tool(name = "swiftgraph_dead_code")]
    pub async fn swiftgraph_dead_code(
        &self,
        rmcp::handler::server::wrapper::Parameters(params): rmcp::handler::server::wrapper::Parameters<DeadCodeToolParams>,
    ) -> String {
        let db_path = self.db_path.clone();
        self.with_request_span("swiftgraph_dead_code", || {
            match navigation::get_dead_code(
                &db_path,
                params.path.as_deref(),
                params.include_tests.unwrap_or(false),
                params.limit,
            ) {
                Ok(resp) => serde_json::to_string_pretty(&resp).unwrap_or_default(),
                Err(e) => json!({"error": e.to_string()}).to_string(),
            }
        })
    }

    /// Detect file-level dependency cycles
    #[tool(name = "swiftgraph_cycles")]
    pub async fn swiftgraph_cycles(
        &self,
        rmcp::handler::server::wrapper::Parameters(params): rmcp::handler::server::wrapper::Parameters<CyclesToolParams>,
    ) -> String {
        let db_path = self.db_path.clone();
        self.with_request_span("swiftgraph_cycles", || {
            match navigation::get_cycles(&db_path, params.path.as_deref(), params.max_cycles) {
                Ok(resp) => serde_json::to_string_pretty(&resp).unwrap_or_default(),
                Err(e) => json!({"error": e.to_string()}).to_string(),
            }
        })
    }

    /// Analyze module coupling — afferent/efferent coupling, instability, abstractness, distance from main sequence
    #[tool(name = "swiftgraph_coupling")]
    pub async fn swiftgraph_coupling(
        &self,
        rmcp::handler::server::wrapper::Parameters(params): rmcp::handler::server::wrapper::Parameters<CouplingToolParams>,
    ) -> String {
        let db_path = self.db_path.clone();
        self.with_request_span("swiftgraph_coupling", || {
            match navigation::get_coupling(&db_path, params.depth, params.source_root.as_deref()) {
                Ok(resp) => serde_json::to_string_pretty(&resp).unwrap_or_default(),
                Err(e) => json!({"error": e.to_string()}).to_string(),
            }
        })
    }

    /// Auto-detect or validate architectural pattern (MVVM, VIPER, TCA, MVC) with evidence and violations
    #[tool(name = "swiftgraph_architecture")]
    pub async fn swiftgraph_architecture(
        &self,
        rmcp::handler::server::wrapper::Parameters(params): rmcp::handler::server::wrapper::Parameters<ArchitectureToolParams>,
    ) -> String {
        let db_path = self.db_path.clone();
        self.with_request_span(
            "swiftgraph_architecture",
            || match navigation::get_architecture(&db_path, params.expected.as_deref()) {
                Ok(resp) => serde_json::to_string_pretty(&resp).unwrap_or_default(),
                Err(e) => json!({"error": e.to_string()}).to_string(),
            },
        )
    }

    /// Analyze module import dependencies — which modules are imported, by how many files
    #[tool(name = "swiftgraph_imports")]
    pub async fn swiftgraph_imports(
        &self,
        rmcp::handler::server::wrapper::Parameters(params): rmcp::handler::server::wrapper::Parameters<ImportsToolParams>,
    ) -> String {
        let db_path = self.db_path.clone();
        self.with_request_span("swiftgraph_imports", || {
            match navigation::get_imports(&db_path, params.path.as_deref()) {
                Ok(resp) => serde_json::to_string_pretty(&resp).unwrap_or_default(),
                Err(e) => json!({"error": e.to_string()}).to_string(),
            }
        })
    }

    /// Check architecture boundary violations — define layers and allowed/disallowed dependencies
    #[tool(name = "swiftgraph_boundaries")]
    pub async fn swiftgraph_boundaries(
        &self,
        rmcp::handler::server::wrapper::Parameters(params): rmcp::handler::server::wrapper::Parameters<BoundariesToolParams>,
    ) -> String {
        let db_path = self.db_path.clone();
        self.with_request_span(
            "swiftgraph_boundaries",
            || match navigation::get_boundaries(&db_path, &params.config) {
                Ok(resp) => serde_json::to_string_pretty(&resp).unwrap_or_default(),
                Err(e) => json!({"error": e.to_string()}).to_string(),
            },
        )
    }

    /// Run static analysis audit — checks for concurrency, memory, and security issues
    #[tool(name = "swiftgraph_audit")]
    pub async fn swiftgraph_audit(
        &self,
        rmcp::handler::server::wrapper::Parameters(params): rmcp::handler::server::wrapper::Parameters<AuditToolParams>,
    ) -> String {
        let project_root = self.project_root.clone();
        self.with_request_span("swiftgraph_audit", || {
            let options = navigation::parse_audit_options(
                params.categories.as_deref(),
                params.min_severity.as_deref(),
                params.path_filter,
                params.max_issues,
            );
            match navigation::run_audit(&project_root, options) {
                Ok(mut resp) => {
                    // Strip fix suggestions if not requested
                    if !params.fix_suggestions.unwrap_or(true) {
                        for issue in &mut resp.issues {
                            issue.fix = None;
                        }
                    }
                    serde_json::to_string_pretty(&resp).unwrap_or_default()
                }
                Err(e) => json!({"error": e.to_string()}).to_string(),
            }
        })
    }

    /// Analyze concurrency annotations for a symbol — isolation, Sendable, cross-actor calls, mutable state
    #[tool(name = "swiftgraph_concurrency")]
    pub async fn swiftgraph_concurrency(
        &self,
        rmcp::handler::server::wrapper::Parameters(params): rmcp::handler::server::wrapper::Parameters<ConcurrencyToolParams>,
    ) -> String {
        let db_path = self.db_path.clone();
        self.with_request_span("swiftgraph_concurrency", || {
            let c_params = concurrency::ConcurrencyParams {
                symbol: params.symbol,
            };
            match concurrency::analyze_concurrency(&db_path, c_params) {
                Ok(resp) => serde_json::to_string_pretty(&resp).unwrap_or_default(),
                Err(e) => json!({"error": e.to_string()}).to_string(),
            }
        })
    }
}

#[tool_handler]
impl ServerHandler for SwiftGraphServer {
    fn get_info(&self) -> rmcp::model::ServerInfo {
        let mut info = rmcp::model::ServerInfo::default();
        info.instructions = Some("SwiftGraph: compiler-accurate Swift code graph MCP server. Tools: status, reindex, search, node, callers, callees, references, hierarchy, files, extensions, conformances, context, impact, diff_impact, complexity, dead_code, cycles, coupling, architecture, imports, boundaries, audit, concurrency.".into());
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info
    }
}
