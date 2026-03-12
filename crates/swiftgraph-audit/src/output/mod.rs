//! Output formatters for audit results.

use crate::engine::AuditResult;

/// Format audit result as a human-readable text summary.
pub fn format_text(result: &AuditResult) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Audit: {} issues ({} critical, {} high, {} medium, {} low)\n",
        result.total_issues,
        result.by_severity.critical,
        result.by_severity.high,
        result.by_severity.medium,
        result.by_severity.low,
    ));
    out.push('\n');

    for issue in &result.issues {
        out.push_str(&format!(
            "[{}] {} ({}:{}): {}\n",
            issue.severity.as_str().to_uppercase(),
            issue.rule,
            issue.file.rsplit('/').next().unwrap_or(&issue.file),
            issue.line,
            issue.message,
        ));
        if let Some(ref fix) = issue.fix {
            out.push_str(&format!("  Fix: {fix}\n"));
        }
    }

    out
}
