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

/// CONC-005: Non-Sendable type used across concurrency boundary.
pub struct SendableViolation;

impl AuditRule for SendableViolation {
    fn id(&self) -> &str {
        "CONC-005"
    }
    fn name(&self) -> &str {
        "sendable-violation"
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

        // Find class declarations (non-final, non-actor) with mutable stored properties
        // that don't conform to Sendable or @unchecked Sendable
        let class_decls = find_descendants(root, ctx.source, &|node, _| {
            node.kind() == "class_declaration"
        });

        for decl in class_decls {
            let keyword = class_keyword(decl, ctx.source);
            if keyword != "class" {
                continue;
            }

            let name = decl_name(decl, ctx.source).unwrap_or_default();
            let decl_text = node_text(decl, ctx.source);

            // Skip if already Sendable
            if decl_text.contains("Sendable") || decl_text.contains("@unchecked") {
                continue;
            }

            // Check if this class has mutable state (var properties)
            let has_var = find_descendants(decl, ctx.source, &|node, src| {
                node.kind() == "property_declaration" && node_text(node, src).starts_with("var ")
            });

            if has_var.is_empty() {
                continue;
            }

            // Check if the class is used in Task/async context within this file
            let file_text = ctx.source;
            let name_in_task = file_text.contains("Task {") || file_text.contains("Task.detached");

            if !name_in_task {
                continue;
            }

            // Heuristic: non-Sendable class with mutable state used alongside Task
            if !has_attribute(decl, ctx.source, "MainActor") {
                issues.push(AuditIssue {
                    id: format!("{}:{}", self.id(), ctx.file_path),
                    category: self.category(),
                    severity: self.severity(),
                    rule: self.id().to_string(),
                    message: format!(
                        "`{name}` has mutable state but doesn't conform to Sendable — potential data race"
                    ),
                    file: ctx.file_path.to_string(),
                    line: decl.start_position().row as u32 + 1,
                    symbol: Some(name),
                    fix: Some(
                        "Make the class final + Sendable, use @MainActor, or convert to an actor"
                            .into(),
                    ),
                });
            }
        }

        issues
    }
}

/// CONC-006: Stored Task without cancellation handling.
pub struct StoredTaskWithoutCancel;

impl AuditRule for StoredTaskWithoutCancel {
    fn id(&self) -> &str {
        "CONC-006"
    }
    fn name(&self) -> &str {
        "stored-task-no-cancel"
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

        // Find property declarations that store a Task
        let props = find_descendants(root, ctx.source, &|node, src| {
            if node.kind() != "property_declaration" {
                return false;
            }
            let text = node_text(node, src);
            text.contains("Task<") || text.contains(": Task?") || text.contains(": Task<")
        });

        for prop in props {
            let name = decl_name(prop, ctx.source).unwrap_or_default();
            let file_text = ctx.source;

            // Check if .cancel() is called on this property anywhere in the file
            let cancel_pattern = format!("{name}.cancel()");
            if !file_text.contains(&cancel_pattern) {
                issues.push(AuditIssue {
                    id: format!("{}:{}", self.id(), ctx.file_path),
                    category: self.category(),
                    severity: self.severity(),
                    rule: self.id().to_string(),
                    message: format!(
                        "Stored Task `{name}` has no .cancel() call — may leak work on dealloc"
                    ),
                    file: ctx.file_path.to_string(),
                    line: prop.start_position().row as u32 + 1,
                    symbol: Some(name),
                    fix: Some("Cancel the task in deinit or when no longer needed".into()),
                });
            }
        }

        issues
    }
}

/// CONC-007: Nonisolated access to mutable state.
pub struct NonisolatedMutableAccess;

impl AuditRule for NonisolatedMutableAccess {
    fn id(&self) -> &str {
        "CONC-007"
    }
    fn name(&self) -> &str {
        "nonisolated-mutable-access"
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

        // Find functions marked nonisolated that access self.property
        let funcs = find_descendants(root, ctx.source, &|node, src| {
            if node.kind() != "function_declaration" {
                return false;
            }
            let text = node_text(node, src);
            text.starts_with("nonisolated ") || text.starts_with("nonisolated(unsafe)")
        });

        for func in funcs {
            let text = node_text(func, ctx.source);
            // Check if the function accesses mutable self properties
            if text.contains("self.") {
                let name = decl_name(func, ctx.source).unwrap_or_default();
                issues.push(AuditIssue {
                    id: format!("{}:{}", self.id(), ctx.file_path),
                    category: self.category(),
                    severity: self.severity(),
                    rule: self.id().to_string(),
                    message: format!(
                        "nonisolated function `{name}` accesses `self` — potential data race"
                    ),
                    file: ctx.file_path.to_string(),
                    line: func.start_position().row as u32 + 1,
                    symbol: Some(name),
                    fix: Some("Remove nonisolated or avoid accessing actor-isolated state".into()),
                });
            }
        }

        issues
    }
}

/// All concurrency rules.
pub fn all_rules() -> Vec<Box<dyn AuditRule>> {
    vec![
        Box::new(MissingMainActor),
        Box::new(UnsafeTaskCapture),
        Box::new(MainActorFromDetached),
        Box::new(ActorHopInLoop),
        Box::new(SendableViolation),
        Box::new(StoredTaskWithoutCancel),
        Box::new(NonisolatedMutableAccess),
    ]
}
