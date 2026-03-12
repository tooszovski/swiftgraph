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
        Command::Index { project, force, .. } => {
            let root = get_project_root(project);
            cmd_index(&root, force)?;
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

fn cmd_index(root: &Path, force: bool) -> Result<()> {
    let db_path = root.join(".swiftgraph/db.sqlite");
    eprintln!("Indexing {}...", root.display());

    let result = swiftgraph_core::pipeline::index_directory(&db_path, root, force)?;

    eprintln!(
        "Done: {} files scanned, {} indexed, {} nodes, {} edges",
        result.files_scanned, result.files_indexed, result.nodes_added, result.edges_added
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
