//! Concurrency audit rules (CONC-001 through CONC-004).

use crate::engine::{AuditIssue, Category, Severity};
use crate::rules::{
    class_keyword, decl_name, find_descendants, has_attribute, node_text, AuditRule, FileContext,
};
use tree_sitter::Node;

/// CONC-001: Missing @MainActor on UIViewController subclass or ObservableObject.
pub struct MissingMainActor;

impl AuditRule for MissingMainActor {
    fn id(&self) -> &str {
        "CONC-001"
    }
    fn name(&self) -> &str {
        "missing-main-actor"
    }
    fn category(&self) -> Category {
        Category::Concurrency
    }
    fn severity(&self) -> Severity {
        Severity::High
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        let class_decls = find_descendants(root, ctx.source, &|node, _| {
            node.kind() == "class_declaration"
        });

        for decl in class_decls {
            let keyword = class_keyword(decl, ctx.source);
            if keyword != "class" {
                continue;
            }

            // Check if it inherits from UIViewController or conforms to ObservableObject
            let inherits_ui =
                inherits_from(decl, ctx.source, &["UIViewController", "ObservableObject"]);
            if !inherits_ui {
                continue;
            }

            // Check if @MainActor is present
            if has_attribute(decl, ctx.source, "MainActor") {
                continue;
            }

            let name = decl_name(decl, ctx.source).unwrap_or_default();
            issues.push(AuditIssue {
                id: format!("{}:{}", self.id(), ctx.file_path),
                category: self.category(),
                severity: self.severity(),
                rule: self.id().to_string(),
                message: format!(
                    "`{name}` inherits UIViewController or ObservableObject but is missing @MainActor"
                ),
                file: ctx.file_path.to_string(),
                line: decl.start_position().row as u32 + 1,
                symbol: Some(name),
                fix: Some("Add @MainActor to the class declaration".into()),
            });
        }

        issues
    }
}

/// CONC-002: Task capturing self without [weak self].
pub struct UnsafeTaskCapture;

impl AuditRule for UnsafeTaskCapture {
    fn id(&self) -> &str {
        "CONC-002"
    }
    fn name(&self) -> &str {
        "unsafe-task-capture"
    }
    fn category(&self) -> Category {
        Category::Concurrency
    }
    fn severity(&self) -> Severity {
        Severity::High
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        // Find Task { ... } or Task.detached { ... } call expressions
        let call_exprs = find_descendants(root, ctx.source, &|node, src| {
            if node.kind() != "call_expression" {
                return false;
            }
            let text = node_text(node, src);
            text.starts_with("Task") && (text.contains("Task {") || text.contains("Task.detached"))
        });

        for call in call_exprs {
            let text = node_text(call, ctx.source);

            // Check if body references `self` without `[weak self]`
            let has_weak_self = text.contains("[weak self]");
            let uses_self = text.contains("self.");

            if uses_self && !has_weak_self {
                issues.push(AuditIssue {
                    id: format!("{}:{}", self.id(), ctx.file_path),
                    category: self.category(),
                    severity: self.severity(),
                    rule: self.id().to_string(),
                    message: "Task captures `self` strongly — may cause retain cycle".into(),
                    file: ctx.file_path.to_string(),
                    line: call.start_position().row as u32 + 1,
                    symbol: None,
                    fix: Some(
                        "Use `[weak self]` capture list or restructure to avoid retaining self"
                            .into(),
                    ),
                });
            }
        }

        issues
    }
}

/// CONC-003: @MainActor property accessed from Task.detached.
pub struct MainActorFromDetached;

impl AuditRule for MainActorFromDetached {
    fn id(&self) -> &str {
        "CONC-003"
    }
    fn name(&self) -> &str {
        "main-actor-detached-access"
    }
    fn category(&self) -> Category {
        Category::Concurrency
    }
    fn severity(&self) -> Severity {
        Severity::Critical
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        let call_exprs = find_descendants(root, ctx.source, &|node, src| {
            if node.kind() != "call_expression" {
                return false;
            }
            let text = node_text(node, src);
            text.contains("Task.detached")
        });

        for call in call_exprs {
            let text = node_text(call, ctx.source);
            // Heuristic: if Task.detached body accesses self.property without await MainActor
            if text.contains("self.")
                && !text.contains("MainActor.run")
                && !text.contains("@MainActor")
            {
                issues.push(AuditIssue {
                    id: format!("{}:{}", self.id(), ctx.file_path),
                    category: self.category(),
                    severity: self.severity(),
                    rule: self.id().to_string(),
                    message: "Task.detached accesses `self` properties — may violate actor isolation".into(),
                    file: ctx.file_path.to_string(),
                    line: call.start_position().row as u32 + 1,
                    symbol: None,
                    fix: Some("Use `await MainActor.run { }` for MainActor-isolated property access, or use Task { } instead".into()),
                });
            }
        }

        issues
    }
}

/// CONC-004: Actor hop in loop — awaiting actor-isolated code inside a loop.
pub struct ActorHopInLoop;

impl AuditRule for ActorHopInLoop {
    fn id(&self) -> &str {
        "CONC-004"
    }
    fn name(&self) -> &str {
        "actor-hop-in-loop"
    }
    fn category(&self) -> Category {
        Category::Concurrency
    }
    fn severity(&self) -> Severity {
        Severity::Medium
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        // Find for/while loops
        let loops = find_descendants(root, ctx.source, &|node, _| {
            matches!(
                node.kind(),
                "for_statement" | "while_statement" | "repeat_while_statement"
            )
        });

        for loop_node in loops {
            let text = node_text(loop_node, ctx.source);
            // Heuristic: await inside loop body suggests repeated actor hops
            let await_count = text.matches("await ").count();
            if await_count > 0 {
                // Check if it's awaiting on an actor (e.g., `await actor.method()`)
                if text.contains("await ") && (text.contains("MainActor") || text.contains(".run"))
                {
                    issues.push(AuditIssue {
                        id: format!("{}:{}", self.id(), ctx.file_path),
                        category: self.category(),
                        severity: self.severity(),
                        rule: self.id().to_string(),
                        message: format!(
                            "Loop contains {await_count} await(s) — potential repeated actor hops causing performance issues"
                        ),
                        file: ctx.file_path.to_string(),
                        line: loop_node.start_position().row as u32 + 1,
                        symbol: None,
                        fix: Some("Batch work on the target actor to reduce hop overhead".into()),
                    });
                }
            }
        }

        issues
    }
}

/// Helper: check if a class_declaration inherits from any of the given types.
fn inherits_from(node: Node, source: &str, types: &[&str]) -> bool {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "inheritance_specifier" {
                let text = node_text(child, source);
                for t in types {
                    if text.contains(t) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// All concurrency rules.
pub fn all_rules() -> Vec<Box<dyn AuditRule>> {
    vec![
        Box::new(MissingMainActor),
        Box::new(UnsafeTaskCapture),
        Box::new(MainActorFromDetached),
        Box::new(ActorHopInLoop),
    ]
}
