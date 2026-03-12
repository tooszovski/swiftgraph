//! Audit runner — scans Swift files, applies rules, collects findings.

use std::path::Path;

use rayon::prelude::*;
use thiserror::Error;
use walkdir::WalkDir;

use crate::engine::{AuditIssue, AuditResult, Category, Severity};
use crate::rules::{self, AuditRule, FileContext};

#[derive(Debug, Error)]
pub enum RunnerError {
    #[error("tree-sitter error")]
    TreeSitter,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Options for running an audit.
#[derive(Debug, Clone)]
pub struct AuditOptions {
    /// Filter by categories (empty = all).
    pub categories: Vec<Category>,
    /// Minimum severity to report.
    pub min_severity: Severity,
    /// File path filter (prefix match).
    pub path_filter: Option<String>,
    /// Max issues to return.
    pub max_issues: usize,
}

impl Default for AuditOptions {
    fn default() -> Self {
        Self {
            categories: Vec::new(),
            min_severity: Severity::Low,
            path_filter: None,
            max_issues: 500,
        }
    }
}

/// Run audit on a project directory.
pub fn run_audit(project_root: &Path, options: &AuditOptions) -> Result<AuditResult, RunnerError> {
    // Collect all rules
    let all_rules = collect_rules(options);

    // Find Swift files
    let swift_files: Vec<_> = WalkDir::new(project_root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            let path = e.path();
            path.extension().is_some_and(|ext| ext == "swift")
                && !path.to_string_lossy().contains("/.build/")
                && !path.to_string_lossy().contains("/Pods/")
                && !path.to_string_lossy().contains("/Generated/")
                && !path.to_string_lossy().contains("/DerivedData/")
        })
        .filter(|e| {
            if let Some(ref prefix) = options.path_filter {
                e.path().to_string_lossy().contains(prefix.as_str())
            } else {
                true
            }
        })
        .map(|e| e.into_path())
        .collect();

    // Process files in parallel
    let all_issues: Vec<AuditIssue> = swift_files
        .par_iter()
        .flat_map(|path| check_file(path, &all_rules).unwrap_or_default())
        .filter(|issue| issue.severity >= options.min_severity)
        .collect();

    // Limit and sort
    let mut issues = all_issues;
    issues.sort_by(|a, b| b.severity.cmp(&a.severity));
    issues.truncate(options.max_issues);

    Ok(AuditResult::from_issues(issues))
}

/// Check a single file against all rules.
fn check_file(path: &Path, rules: &[Box<dyn AuditRule>]) -> Result<Vec<AuditIssue>, RunnerError> {
    let source = std::fs::read_to_string(path).map_err(RunnerError::Io)?;
    let file_path = path.to_string_lossy().to_string();

    let mut parser = rules::swift_parser().map_err(|_| RunnerError::TreeSitter)?;
    let tree = parser.parse(&source, None).ok_or(RunnerError::TreeSitter)?;

    let ctx = FileContext {
        file_path: &file_path,
        source: &source,
        tree: &tree,
    };

    let mut issues = Vec::new();
    for rule in rules {
        issues.extend(rule.check(&ctx));
    }

    Ok(issues)
}

/// Collect rules based on options.
fn collect_rules(options: &AuditOptions) -> Vec<Box<dyn AuditRule>> {
    let mut rules: Vec<Box<dyn AuditRule>> = Vec::new();

    let include_all = options.categories.is_empty();

    if include_all || options.categories.contains(&Category::Concurrency) {
        rules.extend(rules::concurrency::all_rules());
    }
    if include_all || options.categories.contains(&Category::Memory) {
        rules.extend(rules::memory::all_rules());
    }
    if include_all || options.categories.contains(&Category::Security) {
        rules.extend(rules::security::all_rules());
    }

    rules
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn detect_strong_delegate() {
        let source = r#"
class MyView: UIView {
    var delegate: MyDelegate?
    weak var weakDelegate: MyDelegate?
}
"#;
        let mut parser = rules::swift_parser().unwrap();
        let tree = parser.parse(source, None).unwrap();
        let ctx = FileContext {
            file_path: "test.swift",
            source,
            tree: &tree,
        };

        let rule = rules::memory::StrongDelegate;
        let issues = rule.check(&ctx);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].rule, "MEM-002");
    }

    #[test]
    fn detect_insecure_storage() {
        let source = r#"
func saveToken(_ token: String) {
    UserDefaults.standard.set(token, forKey: "accessToken")
}
"#;
        let mut parser = rules::swift_parser().unwrap();
        let tree = parser.parse(source, None).unwrap();
        let ctx = FileContext {
            file_path: "test.swift",
            source,
            tree: &tree,
        };

        let rule = rules::security::InsecureStorage;
        let issues = rule.check(&ctx);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].rule, "SEC-002");
    }

    #[test]
    fn detect_http_url() {
        let source = r#"
let url = URL(string: "http://api.example.com/data")!
"#;
        let mut parser = rules::swift_parser().unwrap();
        let tree = parser.parse(source, None).unwrap();
        let ctx = FileContext {
            file_path: "test.swift",
            source,
            tree: &tree,
        };

        let rule = rules::security::AtsBypass;
        let issues = rule.check(&ctx);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].rule, "SEC-004");
    }

    #[test]
    fn no_false_positive_localhost() {
        let source = r#"
let url = URL(string: "http://localhost:8080/api")!
"#;
        let mut parser = rules::swift_parser().unwrap();
        let tree = parser.parse(source, None).unwrap();
        let ctx = FileContext {
            file_path: "test.swift",
            source,
            tree: &tree,
        };

        let rule = rules::security::AtsBypass;
        let issues = rule.check(&ctx);
        assert!(issues.is_empty(), "localhost should not trigger SEC-004");
    }
}
