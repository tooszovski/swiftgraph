//! Output formatters for audit results.

use crate::engine::{AuditResult, Severity};

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

/// Format audit result as SARIF v2.1.0 JSON for CI/CD integration.
///
/// Compatible with GitHub Code Scanning, SonarQube, and other SARIF consumers.
pub fn format_sarif(result: &AuditResult) -> String {
    let severity_to_level = |s: &Severity| -> &str {
        match s {
            Severity::Critical | Severity::High => "error",
            Severity::Medium => "warning",
            Severity::Low => "note",
        }
    };

    // Collect unique rules
    let mut rule_ids: Vec<String> = result
        .issues
        .iter()
        .map(|i| i.rule.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    rule_ids.sort();

    let rules: Vec<serde_json::Value> = rule_ids
        .iter()
        .map(|id| {
            let sample = result.issues.iter().find(|i| &i.rule == id);
            serde_json::json!({
                "id": id,
                "shortDescription": {
                    "text": sample.map(|i| i.message.as_str()).unwrap_or(id)
                },
                "defaultConfiguration": {
                    "level": sample.map(|i| severity_to_level(&i.severity)).unwrap_or("warning")
                },
                "properties": {
                    "category": sample.map(|i| format!("{:?}", i.category)).unwrap_or_default()
                }
            })
        })
        .collect();

    let results: Vec<serde_json::Value> = result
        .issues
        .iter()
        .map(|issue| {
            let mut result_obj = serde_json::json!({
                "ruleId": issue.rule,
                "level": severity_to_level(&issue.severity),
                "message": {
                    "text": issue.message
                },
                "locations": [{
                    "physicalLocation": {
                        "artifactLocation": {
                            "uri": issue.file,
                            "uriBaseId": "%SRCROOT%"
                        },
                        "region": {
                            "startLine": issue.line,
                            "startColumn": 1
                        }
                    }
                }]
            });
            if let Some(ref fix) = issue.fix {
                result_obj["fixes"] = serde_json::json!([{
                    "description": {
                        "text": fix
                    }
                }]);
            }
            result_obj
        })
        .collect();

    let sarif = serde_json::json!({
        "$schema": "https://raw.githubusercontent.com/oasis-tcs/sarif-spec/main/sarif-2.1/schema/sarif-schema-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "tool": {
                "driver": {
                    "name": "SwiftGraph",
                    "version": env!("CARGO_PKG_VERSION"),
                    "informationUri": "https://github.com/nicklama/swiftgraph",
                    "rules": rules
                }
            },
            "results": results,
            "invocations": [{
                "executionSuccessful": true,
                "toolConfigurationNotifications": []
            }]
        }]
    });

    serde_json::to_string_pretty(&sarif).unwrap_or_default()
}
