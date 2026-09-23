use serde::{Deserialize, Serialize};

/// Severity level for audit findings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Style or optimization suggestion without a concrete defect. Hidden
    /// unless requested (`--include-advisory`, `min_severity: advisory`).
    Advisory,
    Low,
    Medium,
    High,
    Critical,
}

impl Severity {
    /// Parse `low`, `medium`, `high` or `critical` (case-insensitive).
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "advisory" | "info" => Some(Self::Advisory),
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            "critical" => Some(Self::Critical),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Advisory => "advisory",
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
    /// 1-based column of the element the finding is about, when the rule
    /// knows it; distinguishes several findings on one line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
    pub symbol: Option<String>,
    pub fix: Option<String>,
}

/// Result of running an audit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditResult {
    /// Issues returned (after the `max_issues` cap).
    pub total_issues: usize,
    /// Issues found before the `max_issues` cap.
    #[serde(default)]
    pub found_issues: usize,
    /// Some findings were dropped by the `max_issues` cap.
    #[serde(default)]
    pub truncated: bool,
    pub by_severity: BySeverity,
    pub issues: Vec<AuditIssue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BySeverity {
    pub critical: usize,
    pub high: usize,
    pub medium: usize,
    pub low: usize,
    #[serde(default)]
    pub advisory: usize,
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
            advisory: issues
                .iter()
                .filter(|i| i.severity == Severity::Advisory)
                .count(),
        };
        Self {
            total_issues: issues.len(),
            found_issues: issues.len(),
            truncated: false,
            by_severity,
            issues,
        }
    }
}
