//! swift-syntax subprocess integration.
//!
//! Spawns `swiftgraph-parser` (Swift CLI, `crates/swiftgraph-parser`) to extract
//! declarations with the full swift-syntax AST. Gracefully degrades: if the
//! parser is missing or speaks another protocol version, indexing continues
//! with tree-sitter only.
//!
//! Protocol 2 (see `SwiftGraphParserCore/Models.swift`):
//! - `--version` → one [`ParserInfo`] JSON line (handshake);
//! - `--stdin` → reads one path per line, prints one JSON line per file in
//!   input order: a [`ParseResult`] or `{"file", "error"}`;
//! - `<file>` → a single [`ParseResult`].

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Deserialize;
use tracing::{debug, warn};

/// Protocol version this build of SwiftGraph understands.
pub const PROTOCOL_VERSION: u32 = 2;

/// Reply to `swiftgraph-parser --version`.
#[derive(Debug, Clone, Deserialize)]
pub struct ParserInfo {
    /// Always `swiftgraph-parser`.
    pub name: String,
    /// Informational parser version.
    pub version: String,
    /// JSON protocol version, must equal [`PROTOCOL_VERSION`].
    pub protocol: u32,
}

/// Declarations and imports of one file.
#[derive(Debug, Clone, Deserialize)]
pub struct ParseResult {
    /// Protocol version that produced this result.
    pub version: u32,
    /// Path of the parsed file.
    pub file: String,
    /// Top-level declarations (with nested members).
    pub declarations: Vec<Declaration>,
    /// `import` statements.
    pub imports: Vec<ImportDecl>,
}

/// An `import` statement.
#[derive(Debug, Clone, Deserialize)]
pub struct ImportDecl {
    /// Module path, e.g. `Foundation` or `UIKit.UIView`.
    pub name: String,
    /// 1-based line.
    pub line: u32,
    /// Attributes such as `@testable`.
    #[serde(default)]
    pub attributes: Vec<String>,
    /// Import kind for `import struct Foo.Bar`.
    #[serde(default)]
    pub kind: Option<String>,
}

/// A declaration extracted by swift-syntax.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Declaration {
    /// Simple name (`load`, `init`, `Store`).
    pub name: String,
    /// Declaration kind (`class`, `method`, `property`, `initializer`, ...).
    pub kind: String,
    /// 1-based start line.
    pub line: u32,
    /// 1-based end line.
    pub end_line: Option<u32>,
    /// Attributes, e.g. `@MainActor`.
    pub attributes: Vec<String>,
    /// Access level keyword, if written.
    pub access_level: Option<String>,
    /// Declaration signature without body.
    pub signature: Option<String>,
    /// `///` or `/** */` doc comment.
    pub doc_comment: Option<String>,
    /// Nested declarations of type-like declarations.
    pub members: Option<Vec<Declaration>>,
}

/// One line of `--stdin` output.
#[derive(Deserialize)]
#[serde(untagged)]
enum BatchLine {
    Parsed(ParseResult),
    Failed { file: String, error: String },
}

/// Errors from the swift-syntax subprocess.
#[derive(Debug, thiserror::Error)]
pub enum SwiftSyntaxError {
    /// The process could not be started.
    #[error("failed to spawn parser: {0}")]
    Spawn(String),
    /// The process exited unsuccessfully.
    #[error("parser exited with error: {0}")]
    ParserFailed(String),
    /// Output was not valid protocol JSON.
    #[error("failed to parse JSON output: {0}")]
    Json(String),
    /// The parser speaks another protocol version.
    #[error(
        "parser protocol {found} is not supported (expected {expected}); rebuild swiftgraph-parser"
    )]
    ProtocolMismatch {
        /// Version reported by the parser.
        found: u32,
        /// Version this build expects.
        expected: u32,
    },
}

/// A located parser binary that passed the `--version` handshake.
#[derive(Debug, Clone)]
pub struct SwiftSyntaxParser {
    /// Path to the binary.
    pub path: PathBuf,
    /// Handshake reply.
    pub info: ParserInfo,
}

impl SwiftSyntaxParser {
    /// Run the `--version` handshake against `path`.
    pub fn probe(path: &Path) -> Result<Self, SwiftSyntaxError> {
        let output = Command::new(path)
            .arg("--version")
            .stdin(Stdio::null())
            .output()
            .map_err(|e| SwiftSyntaxError::Spawn(e.to_string()))?;
        if !output.status.success() {
            return Err(SwiftSyntaxError::ParserFailed(
                String::from_utf8_lossy(&output.stderr).trim().to_string(),
            ));
        }
        let info: ParserInfo = serde_json::from_slice(&output.stdout)
            .map_err(|e| SwiftSyntaxError::Json(format!("--version: {e}")))?;
        if info.protocol != PROTOCOL_VERSION {
            return Err(SwiftSyntaxError::ProtocolMismatch {
                found: info.protocol,
                expected: PROTOCOL_VERSION,
            });
        }
        Ok(Self {
            path: path.to_path_buf(),
            info,
        })
    }

    /// Find the parser (see [`find_parser`]) and verify it. A parser that is
    /// found but fails the handshake is reported with a warning and ignored.
    pub fn discover() -> Option<Self> {
        let path = find_parser()?;
        match Self::probe(&path) {
            Ok(parser) => Some(parser),
            Err(e) => {
                warn!(
                    "ignoring swiftgraph-parser at {}: {e}; continuing without swift-syntax",
                    path.display()
                );
                None
            }
        }
    }

    /// Human-readable description for status output.
    pub fn describe(&self) -> String {
        format!(
            "{} {} (protocol {}) at {}",
            self.info.name,
            self.info.version,
            self.info.protocol,
            self.path.display()
        )
    }

    /// Parse many files in one parser process (`--stdin` mode). Results are
    /// returned in input order; a per-file failure is an `Err(message)`.
    pub fn parse_batch(
        &self,
        files: &[PathBuf],
    ) -> Result<Vec<Result<ParseResult, String>>, SwiftSyntaxError> {
        if files.is_empty() {
            return Ok(Vec::new());
        }
        let mut child = Command::new(&self.path)
            .arg("--stdin")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| SwiftSyntaxError::Spawn(e.to_string()))?;

        let input: String = files
            .iter()
            .map(|p| format!("{}\n", p.to_string_lossy()))
            .collect();
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| SwiftSyntaxError::Spawn("no stdin".into()))?;
        // Write from a thread so a parser that streams output cannot deadlock us.
        let writer = std::thread::spawn(move || stdin.write_all(input.as_bytes()));
        let output = child
            .wait_with_output()
            .map_err(|e| SwiftSyntaxError::Spawn(e.to_string()))?;
        let _ = writer.join();

        if !output.status.success() {
            return Err(SwiftSyntaxError::ParserFailed(
                String::from_utf8_lossy(&output.stderr).trim().to_string(),
            ));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<&str> = stdout.lines().filter(|l| !l.trim().is_empty()).collect();
        if lines.len() != files.len() {
            return Err(SwiftSyntaxError::Json(format!(
                "expected {} result lines, got {}",
                files.len(),
                lines.len()
            )));
        }
        lines
            .into_iter()
            .map(|line| match serde_json::from_str::<BatchLine>(line) {
                Ok(BatchLine::Parsed(r)) => Ok(Ok(r)),
                Ok(BatchLine::Failed { file, error }) => Ok(Err(format!("{file}: {error}"))),
                Err(e) => Err(SwiftSyntaxError::Json(e.to_string())),
            })
            .collect()
    }
}

/// Find the swiftgraph-parser binary (without verifying it).
///
/// Search order:
/// 1. `$SWIFTGRAPH_PARSER_PATH`
/// 2. next to the current executable (release layouts, Homebrew `bin/`)
/// 3. `crates/swiftgraph-parser/.build/{release,debug}/` relative to the
///    current directory (running from a source checkout)
/// 4. `$PATH`
pub fn find_parser() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("SWIFTGRAPH_PARSER_PATH") {
        let p = PathBuf::from(&path);
        if p.is_file() {
            return Some(p);
        }
        warn!("SWIFTGRAPH_PARSER_PATH={path} does not exist");
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(p) = exe.parent().map(|d| d.join("swiftgraph-parser")) {
            if p.is_file() {
                return Some(p);
            }
        }
    }

    for rel in [
        "crates/swiftgraph-parser/.build/release/swiftgraph-parser",
        "crates/swiftgraph-parser/.build/debug/swiftgraph-parser",
    ] {
        let p = PathBuf::from(rel);
        if p.is_file() {
            return Some(p);
        }
    }

    let path_var = std::env::var_os("PATH")?;
    std::env::split_paths(&path_var)
        .map(|dir| dir.join("swiftgraph-parser"))
        .find(|p| p.is_file())
}

/// Parse a single Swift file with the given parser binary (`<file>` mode).
pub fn parse_file(parser_path: &Path, swift_file: &Path) -> Result<ParseResult, SwiftSyntaxError> {
    let output = Command::new(parser_path)
        .arg(swift_file.to_string_lossy().as_ref())
        .output()
        .map_err(|e| SwiftSyntaxError::Spawn(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(SwiftSyntaxError::ParserFailed(stderr.into_owned()));
    }

    serde_json::from_slice(&output.stdout).map_err(|e| SwiftSyntaxError::Json(e.to_string()))
}

/// Discover the parser and parse one file, returning `None` on any failure.
pub fn try_parse_file(swift_file: &Path) -> Option<ParseResult> {
    let parser = SwiftSyntaxParser::discover()?;
    match parse_file(&parser.path, swift_file) {
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
