use serde::{Deserialize, Serialize};

/// Severity level for audit findings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
}

impl Severity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }
}

/// Category of an audit rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    Concurrency,
    Memory,
    SwiftPerformance,
    SwiftuiPerformance,
    SwiftuiArchitecture,
    Security,
    Energy,
    Networking,
    Codable,
    Storage,
    Accessibility,
    Testing,
    Modernization,
}

/// A single audit finding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditIssue {
    pub id: String,
    pub category: Category,
    pub severity: Severity,
    pub rule: String,
    pub message: String,
    pub file: String,
    pub line: u32,
    pub symbol: Option<String>,
    pub fix: Option<String>,
}

/// Result of running an audit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditResult {
    pub total_issues: usize,
    pub by_severity: BySeverity,
    pub issues: Vec<AuditIssue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BySeverity {
    pub critical: usize,
    pub high: usize,
    pub medium: usize,
    pub low: usize,
}

impl AuditResult {
    pub fn from_issues(issues: Vec<AuditIssue>) -> Self {
        let by_severity = BySeverity {
            critical: issues
                .iter()
                .filter(|i| i.severity == Severity::Critical)
                .count(),
            high: issues
                .iter()
                .filter(|i| i.severity == Severity::High)
                .count(),
            medium: issues
                .iter()
                .filter(|i| i.severity == Severity::Medium)
                .count(),
            low: issues
                .iter()
                .filter(|i| i.severity == Severity::Low)
                .count(),
        };
        Self {
            total_issues: issues.len(),
            by_severity,
            issues,
        }
    }
}
