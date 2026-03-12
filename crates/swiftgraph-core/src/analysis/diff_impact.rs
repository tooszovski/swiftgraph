//! Git diff-based impact analysis.
//!
//! Uses `gix` to parse diffs, identify changed symbols, and compute
//! the aggregate blast radius.

use std::collections::HashSet;
use std::path::Path;

use serde::Serialize;
use thiserror::Error;

use crate::storage::{self, queries};

use super::impact;

#[derive(Debug, Error)]
pub enum DiffImpactError {
    #[error("storage error: {0}")]
    Storage(#[from] crate::storage::StorageError),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("git error: {0}")]
    Git(String),
    #[error("impact error: {0}")]
    Impact(#[from] impact::ImpactError),
}

/// Result of diff-based impact analysis.
#[derive(Debug, Serialize)]
pub struct DiffImpactResult {
    /// The git ref used (e.g. "HEAD~3..HEAD", "staged", "unstaged").
    pub git_ref: String,
    /// Changed files in the diff.
    pub changed_files: Vec<String>,
    /// Symbols found in the changed files.
    pub changed_symbols: Vec<String>,
    /// Total direct impact (sum across all changed symbols).
    pub total_direct_impact: usize,
    /// Total transitive impact.
    pub total_transitive_impact: usize,
    /// All affected files (deduplicated).
    pub affected_files: Vec<String>,
    /// All affected test files.
    pub affected_tests: Vec<String>,
    /// Risk level.
    pub risk_level: String,
}

/// Analyze impact of a git diff.
///
/// `git_ref` can be:
/// - `"staged"` — changes in the staging area
/// - `"unstaged"` — uncommitted changes
/// - `"HEAD~N..HEAD"` — range of commits
pub fn analyze_diff_impact(
    db_path: &Path,
    repo_root: &Path,
    git_ref: &str,
) -> Result<DiffImpactResult, DiffImpactError> {
    // Get changed files from git
    let changed_files = get_changed_files(repo_root, git_ref)?;

    if changed_files.is_empty() {
        return Ok(DiffImpactResult {
            git_ref: git_ref.to_owned(),
            changed_files: Vec::new(),
            changed_symbols: Vec::new(),
            total_direct_impact: 0,
            total_transitive_impact: 0,
            affected_files: Vec::new(),
            affected_tests: Vec::new(),
            risk_level: "none".into(),
        });
    }

    let conn = storage::open_db(db_path)?;

    // Find symbols in changed files
    let mut changed_symbols = Vec::new();
    for file in &changed_files {
        // Convert relative path to absolute for DB lookup
        let abs_path = if file.starts_with('/') {
            file.clone()
        } else {
            repo_root.join(file).to_string_lossy().into_owned()
        };

        let nodes = queries::get_nodes_in_file(&conn, &abs_path).unwrap_or_default();
        for node in nodes {
            changed_symbols.push(node.id);
        }
    }

    // Compute aggregate impact
    let mut all_affected_files: HashSet<String> = HashSet::new();
    let mut all_affected_tests: HashSet<String> = HashSet::new();
    let mut total_direct = 0;
    let mut all_transitive: HashSet<String> = HashSet::new();

    for symbol_id in &changed_symbols {
        if let Ok(result) = impact::analyze_impact(db_path, symbol_id, 3) {
            total_direct += result.direct_impact;
            for f in &result.affected_files {
                all_affected_files.insert(f.clone());
            }
            for t in &result.affected_tests {
                all_affected_tests.insert(t.clone());
            }
            // Track unique transitive symbols via breakdown
            for c in &result.breakdown.callers {
                all_transitive.insert(c.clone());
            }
            for c in &result.breakdown.conforming_types {
                all_transitive.insert(c.clone());
            }
            for c in &result.breakdown.subtypes {
                all_transitive.insert(c.clone());
            }
        }
    }

    let total_transitive_impact = all_transitive.len();
    let risk_level = match total_transitive_impact {
        0..=10 => "low",
        11..=50 => "medium",
        51..=150 => "high",
        _ => "critical",
    }
    .to_owned();

    Ok(DiffImpactResult {
        git_ref: git_ref.to_owned(),
        changed_files,
        changed_symbols,
        total_direct_impact: total_direct,
        total_transitive_impact,
        affected_files: all_affected_files.into_iter().collect(),
        affected_tests: all_affected_tests.into_iter().collect(),
        risk_level,
    })
}

/// Get changed files from a git ref using `gix`.
fn get_changed_files(repo_root: &Path, git_ref: &str) -> Result<Vec<String>, DiffImpactError> {
    // Use git CLI for now — gix diff API is complex and not all features are stable
    let args = match git_ref {
        "staged" => vec!["diff", "--cached", "--name-only", "--diff-filter=ACMR"],
        "unstaged" => vec!["diff", "--name-only", "--diff-filter=ACMR"],
        range => {
            // e.g. "HEAD~3..HEAD"
            vec!["diff", "--name-only", "--diff-filter=ACMR", range]
        }
    };

    let output = std::process::Command::new("git")
        .args(&args)
        .current_dir(repo_root)
        .output()
        .map_err(|e| DiffImpactError::Git(format!("failed to run git: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DiffImpactError::Git(format!("git diff failed: {stderr}")));
    }

    let files: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|l| l.ends_with(".swift"))
        .map(|l| l.to_owned())
        .collect();

    Ok(files)
}
