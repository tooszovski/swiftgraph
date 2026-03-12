use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

mod server;
mod tools;

#[derive(Parser)]
#[command(name = "swiftgraph", version, about = "Swift code graph MCP server")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Initialize SwiftGraph in the current project
    Init {
        /// Project root path (defaults to current directory)
        #[arg(long)]
        project: Option<PathBuf>,
    },
    /// Index Swift files
    Index {
        /// Project root path
        #[arg(long)]
        project: Option<PathBuf>,
        /// Force full reindex
        #[arg(long)]
        force: bool,
        /// Custom Index Store path
        #[arg(long)]
        index_store_path: Option<PathBuf>,
    },
    /// Start MCP server
    Serve {
        /// Enable MCP mode (JSON-RPC over stdin/stdout)
        #[arg(long)]
        mcp: bool,
        /// Project root path
        #[arg(long)]
        project: Option<PathBuf>,
    },
    /// Search for symbols
    Search {
        /// Search query
        query: String,
        /// Filter by kind
        #[arg(long)]
        kind: Option<String>,
        /// Max results
        #[arg(long, default_value = "20")]
        limit: u32,
    },
    /// Find callers of a symbol
    Callers {
        /// Symbol name or USR
        symbol: String,
        /// Max results
        #[arg(long, default_value = "30")]
        limit: u32,
    },
    /// Get type hierarchy
    Hierarchy {
        /// Symbol name or USR
        symbol: String,
        /// Direction: subtypes or supertypes
        #[arg(long, default_value = "subtypes")]
        direction: String,
    },
    /// Build task-relevant context
    Context {
        /// Task description
        task: String,
        /// Max nodes (default 25)
        #[arg(long, default_value = "25")]
        max_nodes: u32,
        /// Include test files
        #[arg(long)]
        include_tests: bool,
    },
    /// Analyze blast radius of changing a symbol
    Impact {
        /// Symbol name or USR
        symbol: String,
        /// Depth of transitive analysis (default 3)
        #[arg(long, default_value = "3")]
        depth: u32,
    },
    /// Analyze impact of git diff
    DiffImpact {
        /// Git ref: "staged", "unstaged", or range like "HEAD~3..HEAD"
        #[arg(long, default_value = "unstaged")]
        git_ref: String,
    },
    /// Run static analysis audit
    Audit {
        /// Comma-separated categories: concurrency, memory, security (empty = all)
        #[arg(long)]
        categories: Option<String>,
        /// Minimum severity: low, medium, high, critical
        #[arg(long, default_value = "low")]
        min_severity: String,
        /// Filter by file path prefix
        #[arg(long)]
        path: Option<String>,
        /// Output format: json, text
        #[arg(long, default_value = "text")]
        format: String,
        /// Max issues
        #[arg(long, default_value = "100")]
        max_issues: usize,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();

    match cli.command {
        Command::Init { project } => {
            let root = get_project_root(project);
            cmd_init(&root)?;
        }
        Command::Index {
            project,
            force,
            index_store_path,
        } => {
            let root = get_project_root(project);
            cmd_index(&root, force, index_store_path.as_deref())?;
        }
        Command::Serve { mcp, project } => {
            let root = get_project_root(project);
            if mcp {
                cmd_serve_mcp(root).await?;
            } else {
                eprintln!("Use --mcp flag to start MCP server");
            }
        }
        Command::Search { query, kind, limit } => {
            let root = get_project_root(None);
            let db_path = root.join(".swiftgraph/db.sqlite");
            let params = tools::navigation::SearchParams {
                query,
                kind,
                limit: Some(limit),
            };
            let result = tools::navigation::search(&db_path, params)?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Callers { symbol, limit } => {
            let root = get_project_root(None);
            let db_path = root.join(".swiftgraph/db.sqlite");
            let params = tools::navigation::CallersParams {
                symbol,
                limit: Some(limit),
            };
            let result = tools::navigation::get_callers(&db_path, params)?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Hierarchy {
            symbol, direction, ..
        } => {
            let root = get_project_root(None);
            let db_path = root.join(".swiftgraph/db.sqlite");
            let params = tools::navigation::HierarchyParams {
                symbol,
                direction: Some(direction),
                depth: Some(3),
            };
            let result = tools::navigation::get_hierarchy(&db_path, params)?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Context {
            task,
            max_nodes,
            include_tests,
        } => {
            let root = get_project_root(None);
            let db_path = root.join(".swiftgraph/db.sqlite");
            let params = tools::navigation::ContextParams {
                task,
                max_nodes: Some(max_nodes),
                include_tests: Some(include_tests),
            };
            let result = tools::navigation::get_context(&db_path, params)?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Impact { symbol, depth } => {
            let root = get_project_root(None);
            let db_path = root.join(".swiftgraph/db.sqlite");
            let params = tools::navigation::ImpactParams {
                symbol,
                depth: Some(depth),
            };
            let result = tools::navigation::get_impact(&db_path, params)?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::DiffImpact { git_ref } => {
            let root = get_project_root(None);
            let db_path = root.join(".swiftgraph/db.sqlite");
            let params = tools::navigation::DiffImpactParams {
                git_ref: Some(git_ref),
            };
            let result = tools::navigation::get_diff_impact(&db_path, &root, params)?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Audit {
            categories,
            min_severity,
            path,
            format,
            max_issues,
        } => {
            let root = get_project_root(None);
            let options = tools::navigation::parse_audit_options(
                categories.as_deref(),
                Some(min_severity.as_str()),
                path,
                Some(max_issues),
            );
            let result = tools::navigation::run_audit(&root, options)?;
            if format == "json" {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                print!("{}", swiftgraph_audit::output::format_text(&result));
            }
        }
    }

    Ok(())
}

fn get_project_root(path: Option<PathBuf>) -> PathBuf {
    path.unwrap_or_else(|| std::env::current_dir().expect("could not get current directory"))
}

fn cmd_init(root: &Path) -> Result<()> {
    let config_dir = root.join(".swiftgraph");
    std::fs::create_dir_all(&config_dir)?;

    let config_path = config_dir.join("config.json");
    if !config_path.exists() {
        let config = serde_json::json!({
            "version": 1,
            "include": ["Sources/**/*.swift", "Tests/**/*.swift"],
            "exclude": ["**/Generated/**", "**/Pods/**", "**/.build/**"],
            "index_store_path": "auto",
            "swift_syntax_path": "auto",
            "audit": {
                "enabled_categories": ["all"],
                "severity_min": "medium",
                "exclude_rules": []
            }
        });
        std::fs::write(&config_path, serde_json::to_string_pretty(&config)?)?;
    }

    // Detect project
    match swiftgraph_core::project::detect_project(root) {
        Ok(info) => {
            eprintln!(
                "Initialized SwiftGraph for {} ({}) project",
                info.name,
                info.project_type.as_str()
            );
            if let Some(ref idx) = info.index_store_path {
                eprintln!("Index Store found: {}", idx.display());
            } else {
                eprintln!("Index Store not found — will use tree-sitter fallback");
            }
        }
        Err(e) => {
            eprintln!("Warning: {e}");
            eprintln!("SwiftGraph initialized but no Swift project detected");
        }
    }

    eprintln!("Config: {}", config_path.display());
    Ok(())
}

fn cmd_index(root: &Path, force: bool, index_store_path: Option<&Path>) -> Result<()> {
    let db_path = root.join(".swiftgraph/db.sqlite");
    eprintln!("Indexing {}...", root.display());

    // Auto-detect Index Store from project info if not provided
    let store_path = index_store_path.map(|p| p.to_path_buf()).or_else(|| {
        swiftgraph_core::project::detect_project(root)
            .ok()
            .and_then(|info| info.index_store_path)
    });

    let result = swiftgraph_core::pipeline::index_directory_with_store(
        &db_path,
        root,
        force,
        store_path.as_deref(),
    )?;

    eprintln!(
        "Done ({:?}): {} files scanned, {} indexed, {} nodes, {} edges",
        result.strategy,
        result.files_scanned,
        result.files_indexed,
        result.nodes_added,
        result.edges_added
    );
    Ok(())
}

async fn cmd_serve_mcp(root: PathBuf) -> Result<()> {
    eprintln!("Starting SwiftGraph MCP server for {}", root.display());

    let server = server::SwiftGraphServer::new(root);

    let transport = rmcp::transport::io::stdio();
    let handle = rmcp::serve_server(server, transport)
        .await
        .map_err(|e| anyhow::anyhow!("MCP server init error: {e}"))?;
    handle
        .waiting()
        .await
        .map_err(|e| anyhow::anyhow!("MCP server error: {e}"))?;

    Ok(())
}
