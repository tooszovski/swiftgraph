//! Networking audit rules (NET-001 through NET-006).

use crate::engine::{AuditIssue, Category, Severity};
use crate::rules::{find_descendants, node_text, AuditRule, FileContext};

/// NET-001: Deprecated SCNetworkReachability usage.
pub struct DeprecatedReachability;

impl AuditRule for DeprecatedReachability {
    fn id(&self) -> &str {
        "NET-001"
    }
    fn name(&self) -> &str {
        "deprecated-reachability"
    }
    fn category(&self) -> Category {
        Category::Networking
    }
    fn severity(&self) -> Severity {
        Severity::Medium
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        let deprecated_apis = ["SCNetworkReachability", "CFSocket", "NSStream", "CFStream"];

        let calls = find_descendants(root, ctx.source, &|node, src| {
            let text = node_text(node, src);
            deprecated_apis.iter().any(|api| text.contains(api))
                && (node.kind() == "call_expression"
                    || node.kind() == "navigation_expression"
                    || node.kind() == "simple_identifier")
        });

        for call in calls {
            let text = node_text(call, ctx.source);
            let api = deprecated_apis
                .iter()
                .find(|a| text.contains(*a))
                .unwrap_or(&"unknown");
            issues.push(AuditIssue {
                id: format!("{}:{}", self.id(), ctx.file_path),
                category: self.category(),
                severity: self.severity(),
                rule: self.id().to_string(),
                message: format!(
                    "Deprecated networking API `{api}` — use NWPathMonitor or Network.framework"
                ),
                file: ctx.file_path.to_string(),
                line: call.start_position().row as u32 + 1,
                symbol: None,
                fix: Some("Migrate to Network.framework (NWConnection, NWPathMonitor)".into()),
            });
        }

        issues
    }
}

/// NET-002: Missing error handling on URLSession calls.
pub struct MissingNetworkErrorHandling;

impl AuditRule for MissingNetworkErrorHandling {
    fn id(&self) -> &str {
        "NET-002"
    }
    fn name(&self) -> &str {
        "missing-network-error-handling"
    }
    fn category(&self) -> Category {
        Category::Networking
    }
    fn severity(&self) -> Severity {
        Severity::High
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        // Find try? with URLSession (swallows errors)
        let try_optional = find_descendants(root, ctx.source, &|node, src| {
            if node.kind() != "try_expression" {
                return false;
            }
            let text = node_text(node, src);
            text.starts_with("try?") && text.contains("URLSession")
        });

        for expr in try_optional {
            issues.push(AuditIssue {
                id: format!("{}:{}", self.id(), ctx.file_path),
                category: self.category(),
                severity: self.severity(),
                rule: self.id().to_string(),
                message: "`try?` on URLSession call silently swallows network errors".into(),
                file: ctx.file_path.to_string(),
                line: expr.start_position().row as u32 + 1,
                symbol: None,
                fix: Some(
                    "Use try/catch and handle network errors explicitly (timeout, no connection, etc.)"
                        .into(),
                ),
            });
        }

        issues
    }
}

/// NET-003: Hardcoded IP address or URL in source.
pub struct HardcodedUrl;

impl AuditRule for HardcodedUrl {
    fn id(&self) -> &str {
        "NET-003"
    }
    fn name(&self) -> &str {
        "hardcoded-url"
    }
    fn category(&self) -> Category {
        Category::Networking
    }
    fn severity(&self) -> Severity {
        Severity::Low
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        let strings = find_descendants(root, ctx.source, &|node, src| {
            if node.kind() != "line_string_literal" && node.kind() != "multi_line_string_literal" {
                return false;
            }
            let text = node_text(node, src);
            // Match hardcoded IPs (not localhost)
            let has_ip = text.contains("://") && {
                let after_scheme = text.split("://").nth(1).unwrap_or("");
                after_scheme.starts_with(|c: char| c.is_ascii_digit())
                    && !after_scheme.starts_with("127.0.0.1")
                    && !after_scheme.starts_with("0.0.0.0")
            };
            has_ip
        });

        for s in strings {
            issues.push(AuditIssue {
                id: format!("{}:{}", self.id(), ctx.file_path),
                category: self.category(),
                severity: self.severity(),
                rule: self.id().to_string(),
                message: "Hardcoded IP address in URL — use configuration or DNS".into(),
                file: ctx.file_path.to_string(),
                line: s.start_position().row as u32 + 1,
                symbol: None,
                fix: Some("Move to a configuration file or environment variable".into()),
            });
        }

        issues
    }
}

/// NET-004: Reachability check before network call (anti-pattern).
pub struct ReachabilityPrecheck;

impl AuditRule for ReachabilityPrecheck {
    fn id(&self) -> &str {
        "NET-004"
    }
    fn name(&self) -> &str {
        "reachability-precheck"
    }
    fn category(&self) -> Category {
        Category::Networking
    }
    fn severity(&self) -> Severity {
        Severity::Medium
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        let checks = find_descendants(root, ctx.source, &|node, src| {
            if node.kind() != "if_statement" {
                return false;
            }
            let text = node_text(node, src);
            (text.contains("isReachable")
                || text.contains("isConnected")
                || text.contains("networkReachability"))
                && (text.contains("URLSession")
                    || text.contains("fetch")
                    || text.contains("request"))
        });

        for check in checks {
            issues.push(AuditIssue {
                id: format!("{}:{}", self.id(), ctx.file_path),
                category: self.category(),
                severity: self.severity(),
                rule: self.id().to_string(),
                message:
                    "Reachability check before network request — just make the request and handle errors"
                        .into(),
                file: ctx.file_path.to_string(),
                line: check.start_position().row as u32 + 1,
                symbol: None,
                fix: Some(
                    "Remove pre-check; network state can change between check and request".into(),
                ),
            });
        }

        issues
    }
}

/// NET-005: URLSession without timeout configuration.
pub struct MissingTimeout;

impl AuditRule for MissingTimeout {
    fn id(&self) -> &str {
        "NET-005"
    }
    fn name(&self) -> &str {
        "missing-timeout"
    }
    fn category(&self) -> Category {
        Category::Networking
    }
    fn severity(&self) -> Severity {
        Severity::Low
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        // Find URLSessionConfiguration or URLRequest creation without timeout
        let configs = find_descendants(root, ctx.source, &|node, src| {
            if node.kind() != "call_expression" {
                return false;
            }
            let text = node_text(node, src);
            text.contains("URLSessionConfiguration") && text.contains(".default")
        });

        for config in configs {
            // Check if timeoutIntervalForRequest is set nearby
            let parent_text = config
                .parent()
                .map(|p| node_text(p, ctx.source))
                .unwrap_or("");
            if !parent_text.contains("timeoutInterval") {
                issues.push(AuditIssue {
                    id: format!("{}:{}", self.id(), ctx.file_path),
                    category: self.category(),
                    severity: self.severity(),
                    rule: self.id().to_string(),
                    message: "URLSessionConfiguration without explicit timeout — defaults to 60s"
                        .into(),
                    file: ctx.file_path.to_string(),
                    line: config.start_position().row as u32 + 1,
                    symbol: None,
                    fix: Some(
                        "Set timeoutIntervalForRequest and timeoutIntervalForResource".into(),
                    ),
                });
            }
        }

        issues
    }
}

/// NET-006: Using URLSession.shared for uploads/downloads.
pub struct SharedSessionForTransfer;

impl AuditRule for SharedSessionForTransfer {
    fn id(&self) -> &str {
        "NET-006"
    }
    fn name(&self) -> &str {
        "shared-session-transfer"
    }
    fn category(&self) -> Category {
        Category::Networking
    }
    fn severity(&self) -> Severity {
        Severity::Low
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        let calls = find_descendants(root, ctx.source, &|node, src| {
            if node.kind() != "call_expression" && node.kind() != "navigation_expression" {
                return false;
            }
            let text = node_text(node, src);
            text.contains("URLSession.shared")
                && (text.contains("upload") || text.contains("download"))
        });

        for call in calls {
            issues.push(AuditIssue {
                id: format!("{}:{}", self.id(), ctx.file_path),
                category: self.category(),
                severity: self.severity(),
                rule: self.id().to_string(),
                message:
                    "URLSession.shared used for upload/download — no background transfer support"
                        .into(),
                file: ctx.file_path.to_string(),
                line: call.start_position().row as u32 + 1,
                symbol: None,
                fix: Some(
                    "Use a custom URLSession with background configuration for large transfers"
                        .into(),
                ),
            });
        }

        issues
    }
}

/// All networking rules.
pub fn all_rules() -> Vec<Box<dyn AuditRule>> {
    vec![
        Box::new(DeprecatedReachability),
        Box::new(MissingNetworkErrorHandling),
        Box::new(HardcodedUrl),
        Box::new(ReachabilityPrecheck),
        Box::new(MissingTimeout),
        Box::new(SharedSessionForTransfer),
    ]
}
