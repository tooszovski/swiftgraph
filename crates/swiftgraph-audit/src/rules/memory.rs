//! Memory audit rules (MEM-001 through MEM-004).

use crate::engine::{AuditIssue, Category, Severity};
use crate::rules::{find_descendants, node_text, AuditRule, FileContext};

/// MEM-001: Closure capturing self without [weak self] in escaping context.
pub struct ClosureRetainCycle;

impl AuditRule for ClosureRetainCycle {
    fn id(&self) -> &str {
        "MEM-001"
    }
    fn name(&self) -> &str {
        "closure-retain-cycle"
    }
    fn category(&self) -> Category {
        Category::Memory
    }
    fn severity(&self) -> Severity {
        Severity::High
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        // Find closures in escaping positions (stored properties, completion handlers)
        let closures =
            find_descendants(root, ctx.source, &|node, _| node.kind() == "lambda_literal");

        for closure in closures {
            let text = node_text(closure, ctx.source);
            let has_self = text.contains("self.");
            let has_weak_self = text.contains("[weak self]") || text.contains("[unowned self]");

            if !has_self || has_weak_self {
                continue;
            }

            // Check if in escaping context (heuristic: assigned to property, or in completion handler)
            if let Some(parent) = closure.parent() {
                let parent_text = node_text(parent, ctx.source);
                let is_escaping = parent_text.contains("completion")
                    || parent_text.contains("handler")
                    || parent_text.contains("callback")
                    || parent_text.contains("closure")
                    || is_property_assignment(parent, ctx.source);

                if is_escaping {
                    issues.push(AuditIssue {
                        id: format!("{}:{}", self.id(), ctx.file_path),
                        category: self.category(),
                        severity: self.severity(),
                        rule: self.id().to_string(),
                        message: "Closure captures `self` strongly in potentially escaping context"
                            .into(),
                        file: ctx.file_path.to_string(),
                        line: closure.start_position().row as u32 + 1,
                        symbol: None,
                        fix: Some("Use `[weak self]` capture list".into()),
                    });
                }
            }
        }

        issues
    }
}

/// MEM-002: Strong delegate reference.
pub struct StrongDelegate;

impl AuditRule for StrongDelegate {
    fn id(&self) -> &str {
        "MEM-002"
    }
    fn name(&self) -> &str {
        "strong-delegate"
    }
    fn category(&self) -> Category {
        Category::Memory
    }
    fn severity(&self) -> Severity {
        Severity::High
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        let properties = find_descendants(root, ctx.source, &|node, _| {
            node.kind() == "property_declaration"
        });

        for prop in properties {
            let text = node_text(prop, ctx.source);
            let text_lower = text.to_lowercase();

            // Check if property name suggests delegate/datasource
            let is_delegate = text_lower.contains("delegate") || text_lower.contains("datasource");
            if !is_delegate {
                continue;
            }

            // Check if it's weak
            let is_weak = text.contains("weak ");
            if is_weak {
                continue;
            }

            // Skip protocol declarations (they don't have "var"/"let")
            if !text.contains("var ") && !text.contains("let ") {
                continue;
            }

            issues.push(AuditIssue {
                id: format!("{}:{}", self.id(), ctx.file_path),
                category: self.category(),
                severity: self.severity(),
                rule: self.id().to_string(),
                message: "Delegate/datasource property is not declared as `weak` — potential retain cycle".into(),
                file: ctx.file_path.to_string(),
                line: prop.start_position().row as u32 + 1,
                symbol: None,
                fix: Some("Add `weak` modifier: `weak var delegate: ...`".into()),
            });
        }

        issues
    }
}

/// MEM-003: Timer not invalidated — Timer.scheduledTimer without invalidate.
pub struct TimerLeak;

impl AuditRule for TimerLeak {
    fn id(&self) -> &str {
        "MEM-003"
    }
    fn name(&self) -> &str {
        "timer-leak"
    }
    fn category(&self) -> Category {
        Category::Memory
    }
    fn severity(&self) -> Severity {
        Severity::Medium
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let mut issues = Vec::new();

        // Simple text scan: if file has Timer.scheduledTimer, check for invalidate
        let has_timer =
            ctx.source.contains("Timer.scheduledTimer") || ctx.source.contains("Timer.publish");
        let has_invalidate = ctx.source.contains(".invalidate()");

        if has_timer && !has_invalidate {
            // Find the timer creation line
            for (i, line) in ctx.source.lines().enumerate() {
                if line.contains("Timer.scheduledTimer") || line.contains("Timer.publish") {
                    issues.push(AuditIssue {
                        id: format!("{}:{}", self.id(), ctx.file_path),
                        category: self.category(),
                        severity: self.severity(),
                        rule: self.id().to_string(),
                        message: "Timer created but no `.invalidate()` found in this file — potential memory leak".into(),
                        file: ctx.file_path.to_string(),
                        line: i as u32 + 1,
                        symbol: None,
                        fix: Some("Invalidate the timer in deinit or when no longer needed".into()),
                    });
                }
            }
        }

        issues
    }
}

/// MEM-004: NotificationCenter observer without removal.
pub struct ObserverLeak;

impl AuditRule for ObserverLeak {
    fn id(&self) -> &str {
        "MEM-004"
    }
    fn name(&self) -> &str {
        "observer-leak"
    }
    fn category(&self) -> Category {
        Category::Memory
    }
    fn severity(&self) -> Severity {
        Severity::Medium
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let mut issues = Vec::new();

        let has_add_observer = ctx
            .source
            .contains("NotificationCenter.default.addObserver");
        let has_remove = ctx.source.contains("removeObserver")
            || ctx
                .source
                .contains("NotificationCenter.default.removeObserver");

        if has_add_observer && !has_remove {
            for (i, line) in ctx.source.lines().enumerate() {
                if line.contains("addObserver") {
                    issues.push(AuditIssue {
                        id: format!("{}:{}", self.id(), ctx.file_path),
                        category: self.category(),
                        severity: self.severity(),
                        rule: self.id().to_string(),
                        message: "NotificationCenter observer added but no `removeObserver` found in this file".into(),
                        file: ctx.file_path.to_string(),
                        line: i as u32 + 1,
                        symbol: None,
                        fix: Some("Remove observer in deinit: `NotificationCenter.default.removeObserver(self)`".into()),
                    });
                }
            }
        }

        issues
    }
}

/// Helper: check if a node is a property assignment context.
fn is_property_assignment(node: tree_sitter::Node, source: &str) -> bool {
    let text = node_text(node, source);
    text.contains("= ") || node.kind() == "assignment"
}

/// All memory rules.
pub fn all_rules() -> Vec<Box<dyn AuditRule>> {
    vec![
        Box::new(ClosureRetainCycle),
        Box::new(StrongDelegate),
        Box::new(TimerLeak),
        Box::new(ObserverLeak),
    ]
}
