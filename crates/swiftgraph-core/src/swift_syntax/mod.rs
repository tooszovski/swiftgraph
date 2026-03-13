//! swift-syntax subprocess integration.
//!
//! Spawns `swiftgraph-parser` (Swift CLI) to extract declarations from Swift files
//! using the full swift-syntax AST. Gracefully degrades: if the parser binary is
//! not found, returns `None` and the pipeline continues with tree-sitter only.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;
use tracing::{debug, warn};

/// JSON protocol version 1 output from swiftgraph-parser.
#[derive(Debug, Clone, Deserialize)]
pub struct ParseResult {
    pub version: u32,
    pub file: String,
    pub declarations: Vec<Declaration>,
    pub imports: Vec<String>,
}

/// A declaration extracted by swift-syntax.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Declaration {
    pub name: String,
    pub kind: String,
    pub line: u32,
    pub end_line: Option<u32>,
    pub attributes: Vec<String>,
    pub access_level: Option<String>,
    pub signature: Option<String>,
    pub doc_comment: Option<String>,
    pub members: Option<Vec<Declaration>>,
}

/// Find the swiftgraph-parser binary.
///
/// Search order:
/// 1. `$SWIFTGRAPH_PARSER_PATH` env var
/// 2. Adjacent to current executable (for release builds)
/// 3. In the workspace at `crates/swiftgraph-parser/.build/release/swiftgraph-parser`
/// 4. In `$PATH` via `which`
pub fn find_parser() -> Option<PathBuf> {
    // 1. Env var
    if let Ok(path) = std::env::var("SWIFTGRAPH_PARSER_PATH") {
        let p = PathBuf::from(&path);
        if p.exists() {
            return Some(p);
        }
    }

    // 2. Adjacent to current exe
    if let Ok(exe) = std::env::current_exe() {
        let adjacent = exe.parent().map(|d| d.join("swiftgraph-parser"));
        if let Some(ref p) = adjacent {
            if p.exists() {
                return Some(p.clone());
            }
        }
    }

    // 3. Workspace build directory
    let workspace_paths = [
        "crates/swiftgraph-parser/.build/release/swiftgraph-parser",
        "crates/swiftgraph-parser/.build/debug/swiftgraph-parser",
    ];
    for rel in &workspace_paths {
        let p = PathBuf::from(rel);
        if p.exists() {
            return Some(p);
        }
    }

    // 4. In PATH
    if let Ok(output) = Command::new("which").arg("swiftgraph-parser").output() {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return Some(PathBuf::from(path));
            }
        }
    }

    None
}

/// Parse a Swift file using the swift-syntax subprocess.
///
/// Returns `None` if the parser binary is not found (graceful degradation).
/// Returns `Err` only on actual parse/IO failures.
pub fn parse_file(parser_path: &Path, swift_file: &Path) -> Result<ParseResult, SwiftSyntaxError> {
    let output = Command::new(parser_path)
        .arg(swift_file.to_string_lossy().as_ref())
        .output()
        .map_err(|e| SwiftSyntaxError::Spawn(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(SwiftSyntaxError::ParserFailed(stderr.into_owned()));
    }

    let result: ParseResult = serde_json::from_slice(&output.stdout)
        .map_err(|e| SwiftSyntaxError::Json(e.to_string()))?;

    Ok(result)
}

/// Try to parse a file, returning None on any failure (for pipeline use).
pub fn try_parse_file(swift_file: &Path) -> Option<ParseResult> {
    let parser = find_parser()?;
    match parse_file(&parser, swift_file) {
        Ok(result) => {
            debug!(file = %swift_file.display(), declarations = result.declarations.len(), "swift-syntax parsed");
            Some(result)
        }
        Err(e) => {
            warn!(file = %swift_file.display(), error = %e, "swift-syntax parse failed");
            None
        }
    }
}

/// Errors from swift-syntax subprocess.
#[derive(Debug, thiserror::Error)]
pub enum SwiftSyntaxError {
    #[error("failed to spawn parser: {0}")]
    Spawn(String),
    #[error("parser exited with error: {0}")]
    ParserFailed(String),
    #[error("failed to parse JSON output: {0}")]
    Json(String),
}
