//! Concurrency audit rules (CONC-001 through CONC-004).

use crate::engine::{AuditIssue, Category, Severity};
use crate::rules::{
    class_keyword, decl_name, find_descendants, has_attribute, node_text, AuditRule, FileContext,
};
use tree_sitter::Node;

/// CONC-001: Missing @MainActor on an ObservableObject.
///
/// UIKit classes (`UIViewController`, `UIView`) are main-actor isolated by
/// the SDK, so subclasses are not reported. High when the class or its
/// extensions in the file have asynchronous code (async/await, `Task`,
/// `DispatchQueue`, `receive(on:)`) and no explicit hop to the main actor;
/// advisory when there is no asynchronous code or the class hops explicitly
/// (`@MainActor` members, `MainActor.run`, `receive(on: DispatchQueue.main)`,
/// helpers named `...OnMain`) — a convention question rather than a race. Projects
/// that hop to the main actor explicitly can disable the rule or change its
/// severity in `.swiftgraph/config.json` (`audit.disabled_rules`,
/// `audit.severity`).
pub struct MissingMainActor;

/// Markers of an explicit hop to the main actor or main queue.
const MAIN_HOP_MARKERS: &[&str] = &[
    "@MainActor",
    "MainActor.run",
    "DispatchQueue.main",
    "RunLoop.main",
    "OnMain(",
    "onMain(",
    "OnMain {",
    "onMain {",
];

/// Markers of code that may run off the main thread.
const ASYNC_MARKERS: &[&str] = &[
    "async",
    "await",
    "Task {",
    "Task(",
    "Task.detached",
    "DispatchQueue",
    "OperationQueue",
    ".receive(on:",
    ".subscribe(on:",
    "Thread.",
];

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

        for decl in class_decls.iter().copied() {
            let keyword = class_keyword(decl, ctx.source);
            if keyword != "class" {
                continue;
            }

            // UIKit classes are already @MainActor; ObservableObject is not
            if !inherits_from(decl, ctx.source, &["ObservableObject"]) {
                continue;
            }

            // Check if @MainActor is present
            if has_attribute(decl, ctx.source, "MainActor") {
                continue;
            }

            let name = decl_name(decl, ctx.source).unwrap_or_default();
            // The class and its extensions in this file
            let mut text = node_text(decl, ctx.source).to_string();
            for ext in &class_decls {
                if class_keyword(*ext, ctx.source) == "extension"
                    && crate::rules::decl_type_name(*ext, ctx.source).as_deref() == Some(&name)
                {
                    text.push_str(node_text(*ext, ctx.source));
                }
            }
            let has_async = ASYNC_MARKERS.iter().any(|m| text.contains(m));
            let hops = MAIN_HOP_MARKERS.iter().any(|m| text.contains(m));
            // Does code that may run off the main actor touch stored state?
            let state = stored_vars(decl, ctx.source, true);
            let mut scopes = vec![decl];
            scopes.extend(class_decls.iter().copied().filter(|ext| {
                class_keyword(*ext, ctx.source) == "extension"
                    && crate::rules::decl_type_name(*ext, ctx.source).as_deref() == Some(&name)
            }));
            let touches_state = scopes.iter().any(|scope| {
                unstructured_tasks(*scope, ctx.source)
                    .into_iter()
                    .chain(nonisolated_async_bodies(*scope, ctx.source))
                    .any(|body| state.iter().any(|var| contains_word(body, var)))
            });
            let (severity, note) = if !has_async {
                (Severity::Advisory, " (no asynchronous code in the class)")
            } else if hops {
                (
                    Severity::Advisory,
                    " (the class hops to the main actor explicitly)",
                )
            } else if !touches_state {
                (
                    Severity::Advisory,
                    " (asynchronous code does not touch its stored state)",
                )
            } else {
                (self.severity(), "")
            };
            issues.push(AuditIssue {
                id: format!("{}:{}", self.id(), ctx.file_path),
                category: self.category(),
                severity,
                rule: self.id().to_string(),
                message: format!("`{name}` is an ObservableObject without @MainActor{note}"),
                file: ctx.file_path.to_string(),
                line: crate::rules::declaration_line(decl, ctx.source),
                column: None,
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
        // Advisory: precision 0/15 on a sampled 7300-file app.
        Severity::Advisory
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
                    column: None,
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
                    column: None,
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
                        column: None,
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
///
/// A non-isolated class whose stored `var` is referenced inside an
/// unstructured `Task { }` / `Task.detached { }` in the class or its
/// extensions. Tasks marked `@MainActor in`, task-group children and tasks
/// of other types in the file are not counted; classes nested in actors are
/// confined by the actor.
pub struct SendableViolation;

/// Names of stored `var` properties declared directly in a class body
/// (not computed, lazy or weak). Property-wrapped ones (`@Injected`,
/// `@Published`) are included only with `include_wrapped`.
fn stored_vars<'a>(decl: Node<'a>, source: &'a str, include_wrapped: bool) -> Vec<&'a str> {
    let Some(body) = decl.child_by_field_name("body") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for i in 0..body.named_child_count() {
        let Some(prop) = body.named_child(i) else {
            continue;
        };
        if prop.kind() != "property_declaration" {
            continue;
        }
        let text = node_text(prop, source);
        let mut cursor = prop.walk();
        let children: Vec<Node> = prop.children(&mut cursor).collect();
        let is_var = children.iter().any(|c| {
            c.kind() == "value_binding_pattern" && node_text(*c, source).starts_with("var")
        });
        let computed = children.iter().any(|c| c.kind() == "computed_property");
        let wrapped = text.trim_start().starts_with('@');
        if !is_var
            || computed
            || text.contains("lazy ")
            || text.contains("weak ")
            || (wrapped && !include_wrapped)
        {
            continue;
        }
        if let Some(name) = children
            .iter()
            .find(|c| c.kind() == "pattern")
            .map(|p| node_text(*p, source))
        {
            out.push(name);
        }
    }
    out
}

/// Bodies of `Task { }` / `Task.detached { }` calls under `scope`, excluding
/// closures that start with `@MainActor`, tasks created in `@MainActor`
/// methods (they inherit the isolation) and bodies that hop to the main
/// actor themselves (`MainActor.run`, `...OnMain`).
fn unstructured_tasks<'a>(scope: Node<'a>, source: &'a str) -> Vec<&'a str> {
    find_descendants(scope, source, &|n, src| {
        if n.kind() != "call_expression" {
            return false;
        }
        let Some(callee) = n.named_child(0) else {
            return false;
        };
        matches!(node_text(callee, src), "Task" | "Task.detached")
    })
    .into_iter()
    .filter_map(|call| {
        let body = find_descendants(call, source, &|n, _| n.kind() == "lambda_literal")
            .into_iter()
            .next()?;
        let text = node_text(body, source);
        let main_closure = text
            .trim_start_matches('{')
            .trim_start()
            .starts_with("@MainActor");
        let hops = text.contains("MainActor.run") || text.contains("OnMain");
        let mut in_main_method = false;
        let mut up = call.parent();
        while let Some(n) = up {
            if n.kind() == "function_declaration" {
                in_main_method = has_attribute(n, source, "MainActor");
                break;
            }
            up = n.parent();
        }
        (!main_closure && !hops && !in_main_method).then_some(text)
    })
    .collect()
}

/// Bodies of `async` methods that are not `@MainActor`.
fn nonisolated_async_bodies<'a>(scope: Node<'a>, source: &'a str) -> Vec<&'a str> {
    find_descendants(scope, source, &|n, _| n.kind() == "function_declaration")
        .into_iter()
        .filter(|f| !has_attribute(*f, source, "MainActor"))
        .filter_map(|f| {
            let body = f.child_by_field_name("body")?;
            let signature = &source[f.start_byte()..body.start_byte()];
            signature
                .contains(" async")
                .then(|| node_text(body, source))
        })
        .collect()
}

/// Whether `word` occurs in `text` as a whole identifier.
fn contains_word(text: &str, word: &str) -> bool {
    text.match_indices(word).any(|(i, _)| {
        let before = text[..i].chars().next_back();
        let after = text[i + word.len()..].chars().next();
        let ident = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
        !ident(before) && !ident(after)
    })
}

/// Whether the declaration is nested inside an `actor`.
fn nested_in_actor(decl: Node, source: &str) -> bool {
    let mut current = decl.parent();
    while let Some(n) = current {
        if n.kind() == "class_declaration" && class_keyword(n, source) == "actor" {
            return true;
        }
        current = n.parent();
    }
    false
}

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
        // Medium: sampled precision 0.55 after pattern fixes.
        Severity::Medium
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        let class_decls = find_descendants(root, ctx.source, &|node, _| {
            node.kind() == "class_declaration"
        });

        for decl in class_decls.iter().copied() {
            if class_keyword(decl, ctx.source) != "class"
                || has_attribute(decl, ctx.source, "MainActor")
                || crate::rules::inheritance_names(decl, ctx.source)
                    .iter()
                    .any(|n| n == "Sendable")
                || node_text(decl, ctx.source).contains("@unchecked")
                || nested_in_actor(decl, ctx.source)
            {
                continue;
            }
            let name = decl_name(decl, ctx.source).unwrap_or_default();

            // Stored mutable properties declared in the class body
            let stored = stored_vars(decl, ctx.source, false);
            if stored.is_empty() {
                continue;
            }

            // Unstructured tasks in the class and its extensions in this file
            let mut scopes = vec![decl];
            scopes.extend(class_decls.iter().copied().filter(|ext| {
                class_keyword(*ext, ctx.source) == "extension"
                    && crate::rules::decl_type_name(*ext, ctx.source).as_deref() == Some(&name)
            }));
            let touches_state = scopes.iter().any(|scope| {
                unstructured_tasks(*scope, ctx.source)
                    .into_iter()
                    .any(|body| stored.iter().any(|var| contains_word(body, var)))
            });
            if !touches_state {
                continue;
            }

            issues.push(AuditIssue {
                id: format!("{}:{}", self.id(), ctx.file_path),
                category: self.category(),
                severity: self.severity(),
                rule: self.id().to_string(),
                message: format!(
                    "`{name}` mutable state is used from an unstructured Task off the main actor — potential data race"
                ),
                file: ctx.file_path.to_string(),
                line: crate::rules::declaration_line(decl, ctx.source),
                column: None,
                symbol: Some(name),
                fix: Some(
                    "Make the class final + Sendable, use @MainActor, or convert to an actor"
                        .into(),
                ),
            });
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
        // Advisory: precision 0/15 on a sampled 7300-file app.
        Severity::Advisory
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
                    column: None,
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
        // Advisory: precision 0/3 on a sampled 7300-file app.
        Severity::Advisory
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
                    column: None,
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
