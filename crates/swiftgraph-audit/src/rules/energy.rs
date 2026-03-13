//! Energy audit rules (NRG-001 through NRG-006).

use crate::engine::{AuditIssue, Category, Severity};
use crate::rules::{find_descendants, node_text, AuditRule, FileContext};

/// NRG-001: Timer with interval < 1s (battery drain).
pub struct FrequentTimer;

impl AuditRule for FrequentTimer {
    fn id(&self) -> &str {
        "NRG-001"
    }
    fn name(&self) -> &str {
        "frequent-timer"
    }
    fn category(&self) -> Category {
        Category::Energy
    }
    fn severity(&self) -> Severity {
        Severity::High
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        let timers = find_descendants(root, ctx.source, &|node, src| {
            if node.kind() != "call_expression" {
                return false;
            }
            let text = node_text(node, src);
            text.contains("Timer.scheduledTimer") || text.contains("Timer.publish")
        });

        for timer in timers {
            let text = node_text(timer, ctx.source);
            // Check for very small intervals
            let has_small_interval = text.contains("0.0")
                || text.contains("0.1")
                || text.contains("0.01")
                || text.contains("0.5")
                || text.contains("every: 0.");

            if has_small_interval {
                issues.push(AuditIssue {
                    id: format!("{}:{}", self.id(), ctx.file_path),
                    category: self.category(),
                    severity: self.severity(),
                    rule: self.id().to_string(),
                    message: "Timer with interval < 1s — significant battery drain".into(),
                    file: ctx.file_path.to_string(),
                    line: timer.start_position().row as u32 + 1,
                    symbol: None,
                    fix: Some("Increase interval or use CADisplayLink for frame-rate work".into()),
                });
            }
        }

        issues
    }
}

/// NRG-002: Polling instead of push notifications or observers.
pub struct PollingPattern;

impl AuditRule for PollingPattern {
    fn id(&self) -> &str {
        "NRG-002"
    }
    fn name(&self) -> &str {
        "polling-pattern"
    }
    fn category(&self) -> Category {
        Category::Energy
    }
    fn severity(&self) -> Severity {
        Severity::Medium
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        // Find repeating timers that fetch/refresh data
        let timers = find_descendants(root, ctx.source, &|node, src| {
            if node.kind() != "call_expression" {
                return false;
            }
            let text = node_text(node, src);
            (text.contains("Timer.scheduledTimer") || text.contains("Timer.publish"))
                && (text.contains("fetch")
                    || text.contains("refresh")
                    || text.contains("reload")
                    || text.contains("poll"))
        });

        for timer in timers {
            issues.push(AuditIssue {
                id: format!("{}:{}", self.id(), ctx.file_path),
                category: self.category(),
                severity: self.severity(),
                rule: self.id().to_string(),
                message:
                    "Timer-based polling detected — prefer push notifications or KVO observers"
                        .into(),
                file: ctx.file_path.to_string(),
                line: timer.start_position().row as u32 + 1,
                symbol: None,
                fix: Some(
                    "Use NotificationCenter, Combine publishers, or server push instead of polling"
                        .into(),
                ),
            });
        }

        issues
    }
}

/// NRG-003: Continuous location updates without activity type.
pub struct ContinuousLocation;

impl AuditRule for ContinuousLocation {
    fn id(&self) -> &str {
        "NRG-003"
    }
    fn name(&self) -> &str {
        "continuous-location"
    }
    fn category(&self) -> Category {
        Category::Energy
    }
    fn severity(&self) -> Severity {
        Severity::High
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        let calls = find_descendants(root, ctx.source, &|node, src| {
            if node.kind() != "call_expression" && node.kind() != "navigation_expression" {
                return false;
            }
            let text = node_text(node, src);
            text.contains("startUpdatingLocation")
        });

        for call in calls {
            // Check if activityType is set nearby
            let file_text = ctx.source;
            if !file_text.contains("activityType")
                && !file_text.contains("allowsBackgroundLocationUpdates = false")
            {
                issues.push(AuditIssue {
                    id: format!("{}:{}", self.id(), ctx.file_path),
                    category: self.category(),
                    severity: self.severity(),
                    rule: self.id().to_string(),
                    message: "startUpdatingLocation without activityType — GPS stays active unnecessarily".into(),
                    file: ctx.file_path.to_string(),
                    line: call.start_position().row as u32 + 1,
                    symbol: None,
                    fix: Some("Set activityType and use significant location changes when possible".into()),
                });
            }
        }

        issues
    }
}

/// NRG-004: Animation running when view is not visible.
pub struct AnimationLeak;

impl AuditRule for AnimationLeak {
    fn id(&self) -> &str {
        "NRG-004"
    }
    fn name(&self) -> &str {
        "animation-leak"
    }
    fn category(&self) -> Category {
        Category::Energy
    }
    fn severity(&self) -> Severity {
        Severity::Medium
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        // Find CADisplayLink or withAnimation in repeating contexts
        let display_links = find_descendants(root, ctx.source, &|node, src| {
            if node.kind() != "call_expression" {
                return false;
            }
            let text = node_text(node, src);
            text.contains("CADisplayLink(")
        });

        for link in display_links {
            let file_text = ctx.source;
            if !file_text.contains("invalidate()") {
                issues.push(AuditIssue {
                    id: format!("{}:{}", self.id(), ctx.file_path),
                    category: self.category(),
                    severity: self.severity(),
                    rule: self.id().to_string(),
                    message:
                        "CADisplayLink created without invalidate() — animation runs indefinitely"
                            .into(),
                    file: ctx.file_path.to_string(),
                    line: link.start_position().row as u32 + 1,
                    symbol: None,
                    fix: Some("Call invalidate() in deinit or when the view disappears".into()),
                });
            }
        }

        issues
    }
}

/// NRG-005: Background mode registered without justification.
pub struct UnnecessaryBackgroundMode;

impl AuditRule for UnnecessaryBackgroundMode {
    fn id(&self) -> &str {
        "NRG-005"
    }
    fn name(&self) -> &str {
        "unnecessary-background-mode"
    }
    fn category(&self) -> Category {
        Category::Energy
    }
    fn severity(&self) -> Severity {
        Severity::Medium
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        // Find beginBackgroundTask without expiration handler
        let bg_tasks = find_descendants(root, ctx.source, &|node, src| {
            if node.kind() != "call_expression" {
                return false;
            }
            let text = node_text(node, src);
            text.contains("beginBackgroundTask")
        });

        for task in bg_tasks {
            let text = node_text(task, ctx.source);
            if !text.contains("expirationHandler") {
                issues.push(AuditIssue {
                    id: format!("{}:{}", self.id(), ctx.file_path),
                    category: self.category(),
                    severity: self.severity(),
                    rule: self.id().to_string(),
                    message:
                        "beginBackgroundTask without expirationHandler — app may be terminated"
                            .into(),
                    file: ctx.file_path.to_string(),
                    line: task.start_position().row as u32 + 1,
                    symbol: None,
                    fix: Some(
                        "Always provide an expirationHandler and call endBackgroundTask".into(),
                    ),
                });
            }
        }

        issues
    }
}

/// NRG-006: Network request without waitsForConnectivity.
pub struct EagerNetworking;

impl AuditRule for EagerNetworking {
    fn id(&self) -> &str {
        "NRG-006"
    }
    fn name(&self) -> &str {
        "eager-networking"
    }
    fn category(&self) -> Category {
        Category::Energy
    }
    fn severity(&self) -> Severity {
        Severity::Low
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        // Find URLSessionConfiguration without waitsForConnectivity
        let configs = find_descendants(root, ctx.source, &|node, src| {
            if node.kind() != "call_expression" {
                return false;
            }
            let text = node_text(node, src);
            text.contains("URLSessionConfiguration")
                && (text.contains(".default") || text.contains(".ephemeral"))
        });

        for config in configs {
            if !ctx.source.contains("waitsForConnectivity") {
                issues.push(AuditIssue {
                    id: format!("{}:{}", self.id(), ctx.file_path),
                    category: self.category(),
                    severity: self.severity(),
                    rule: self.id().to_string(),
                    message: "URLSession config without waitsForConnectivity — may wake radio unnecessarily".into(),
                    file: ctx.file_path.to_string(),
                    line: config.start_position().row as u32 + 1,
                    symbol: None,
                    fix: Some("Set waitsForConnectivity = true to defer requests until connected".into()),
                });
            }
        }

        issues
    }
}

/// All energy rules.
pub fn all_rules() -> Vec<Box<dyn AuditRule>> {
    vec![
        Box::new(FrequentTimer),
        Box::new(PollingPattern),
        Box::new(ContinuousLocation),
        Box::new(AnimationLeak),
        Box::new(UnnecessaryBackgroundMode),
        Box::new(EagerNetworking),
    ]
}
