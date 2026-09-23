//! Memory audit rules (MEM-001 through MEM-004).

use crate::engine::{AuditIssue, Category, Severity};
use crate::rules::{find_descendants, node_text, AuditRule, FileContext};

/// MEM-001: Closure capturing self without [weak self] in escaping context.
///
/// A closure is considered escaping when it is stored (assigned to a
/// property or variable), passed as a completion-like argument
/// (`completion:`, `handler:`, `onX:` ...), or is a trailing closure of a
/// completion-like call or of a Combine pipeline. Closures of
/// non-escaping standard library calls (`map`, `filter`, `forEach`,
/// `first(where:)` ...) and closures inside structs and enums, where `self`
/// is a value, are ignored.
pub struct ClosureRetainCycle;

/// Standard library calls whose closure parameters are non-escaping.
const NON_ESCAPING_CALLS: &[&str] = &[
    "map",
    "flatMap",
    "compactMap",
    "filter",
    "reduce",
    "forEach",
    "sorted",
    "sort",
    "first",
    "firstIndex",
    "last",
    "lastIndex",
    "contains",
    "allSatisfy",
    "min",
    "max",
    "removeAll",
    "partition",
    "split",
    "drop",
    "prefix",
    "mapValues",
    "compactMapValues",
    "withAnimation",
    "withTransaction",
    "autoreleasepool",
    "sync",
    "performAndWait",
    "withUnsafeBytes",
    "withUnsafeMutableBytes",
    "withUnsafePointer",
    "withLock",
    "elementsEqual",
    "lexicographicallyPrecedes",
    "starts",
    "enumerateKeysAndObjects",
];

/// Combine operators: a closure anywhere in such a chain lives as long as
/// the subscription.
const COMBINE_MARKERS: &[&str] = &[
    ".sink",
    ".store(in:",
    ".assign(to:",
    ".eraseToAnyPublisher",
    ".receive(on:",
    ".handleEvents",
];

/// Parameter labels and call names that usually take stored callbacks.
const ESCAPING_WORDS: &[&str] = &[
    "completion",
    "handler",
    "callback",
    "closure",
    "block",
    "observ",
    "subscribe",
    "sink",
];

fn escaping_word(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    ESCAPING_WORDS.iter().any(|w| lower.contains(w))
        || (name.starts_with("on") && name[2..].starts_with(char::is_uppercase))
}

/// The outermost expression of a member chain containing `call`.
fn chain_root(call: tree_sitter::Node) -> tree_sitter::Node {
    let mut root = call;
    while let Some(parent) = root.parent() {
        if matches!(
            parent.kind(),
            "navigation_expression" | "call_expression" | "call_suffix" | "postfix_expression"
        ) {
            root = parent;
        } else {
            break;
        }
    }
    root
}

/// Whether the closure outlives the call it is passed to.
fn is_escaping(closure: tree_sitter::Node, source: &str) -> bool {
    let Some(parent) = closure.parent() else {
        return false;
    };
    let call = match parent.kind() {
        // `handler = { ... }`, `self.onTap = { ... }`
        "assignment" => return true,
        // `var onTap = { ... }` in a type body
        "property_declaration" => {
            return parent
                .parent()
                .is_some_and(|p| matches!(p.kind(), "class_body" | "enum_class_body"));
        }
        // `f(completion: { ... })`
        "value_argument" => {
            if let Some(label) = (0..parent.named_child_count())
                .filter_map(|i| parent.named_child(i))
                .find(|c| c.kind() == "value_argument_label")
            {
                if escaping_word(node_text(label, source)) {
                    return true;
                }
            }
            parent.parent().and_then(|args| args.parent())
        }
        // `f { ... }`
        "call_suffix" => Some(parent),
        _ => None,
    };
    let Some(call) = call.and_then(|suffix| suffix.parent()) else {
        return false;
    };
    if call.kind() != "call_expression" {
        return false;
    }
    let name = crate::rules::callee_name(call, source).unwrap_or_default();
    let chain = node_text(chain_root(call), source);
    if COMBINE_MARKERS.iter().any(|m| chain.contains(m)) {
        return true;
    }
    if NON_ESCAPING_CALLS.contains(&name) {
        return false;
    }
    // Trailing closure of a completion-like call
    parent.kind() == "call_suffix" && escaping_word(name)
}

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

        let closures =
            find_descendants(root, ctx.source, &|node, _| node.kind() == "lambda_literal");

        for closure in closures {
            let text = node_text(closure, ctx.source);
            let has_self = text.contains("self.");
            let has_weak_self = text.contains("[weak self]") || text.contains("[unowned self]");

            if !has_self || has_weak_self {
                continue;
            }
            if crate::rules::in_value_type(closure, ctx.source, ctx.project) {
                continue;
            }
            // Only the innermost closure mentioning `self` is reported.
            let nested_self =
                find_descendants(closure, ctx.source, &|n, _| n.kind() == "lambda_literal")
                    .into_iter()
                    .skip(1)
                    .any(|inner| node_text(inner, ctx.source).contains("self."));
            if nested_self && !direct_self_use(closure) {
                continue;
            }

            if is_escaping(closure, ctx.source) {
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

        issues
    }
}

/// Whether `self.` occurs in the closure outside of nested closures.
fn direct_self_use(closure: tree_sitter::Node) -> bool {
    fn walk(node: tree_sitter::Node, top: bool) -> bool {
        if !top && node.kind() == "lambda_literal" {
            return false;
        }
        if node.kind() == "self_expression"
            && node
                .parent()
                .is_some_and(|p| p.kind() == "navigation_expression")
        {
            return true;
        }
        (0..node.child_count())
            .filter_map(|i| node.child(i))
            .any(|c| walk(c, false))
    }
    walk(closure, true)
}

/// MEM-002: Strong delegate reference.
///
/// Only stored properties of classes and actors named `...delegate` or
/// `...dataSource` with a declared reference-like type are reported; locals,
/// computed properties, closures and value types are not delegates, and a
/// property initialized in place (`lazy var dataSource = ...`) holds an
/// object its owner created, not a back-reference.
pub struct StrongDelegate;

/// Declared types that are values, never delegates.
const VALUE_TYPES: &[&str] = &[
    "String",
    "Substring",
    "Int",
    "Int8",
    "Int16",
    "Int32",
    "Int64",
    "UInt",
    "UInt8",
    "UInt16",
    "UInt32",
    "UInt64",
    "Double",
    "Float",
    "CGFloat",
    "Decimal",
    "Bool",
    "Data",
    "Date",
    "URL",
    "UUID",
    "Character",
    "NSRange",
    "CGRect",
    "CGSize",
    "CGPoint",
];

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
                && node.parent().is_some_and(|p| p.kind() == "class_body")
        });

        for prop in properties {
            // Stored properties of classes and actors only
            let Some(owner) = prop.parent().and_then(|body| body.parent()) else {
                continue;
            };
            let keyword = crate::rules::class_keyword(owner, ctx.source);
            let owner_is_class = match keyword {
                "class" | "actor" => true,
                "extension" => crate::rules::decl_type_name(owner, ctx.source)
                    .and_then(|n| ctx.project.kind_of(&n).map(str::to_string))
                    .is_some_and(|k| matches!(k.as_str(), "class" | "actor")),
                _ => false,
            };
            if !owner_is_class {
                continue;
            }

            let mut name = None;
            let mut declared_type = None;
            let mut computed = false;
            for i in 0..prop.child_count() {
                let Some(child) = prop.child(i) else { continue };
                match child.kind() {
                    "pattern" if name.is_none() => name = Some(node_text(child, ctx.source)),
                    "type_annotation" => declared_type = child.named_child(0),
                    "computed_property" => computed = true,
                    _ => {}
                }
            }
            // Initialized in place (other than `= nil`): an owned object,
            // not a back-reference
            if prop
                .child_by_field_name("value")
                .is_some_and(|v| node_text(v, ctx.source).trim() != "nil")
            {
                continue;
            }
            let Some(name) = name else { continue };
            let lower = name.to_ascii_lowercase();
            if !(lower.ends_with("delegate") || lower.ends_with("datasource")) || computed {
                continue;
            }
            // A reference-like declared type: not a closure, collection or value
            let Some(ty) = declared_type else { continue };
            let type_text = node_text(ty, ctx.source)
                .trim_end_matches(['?', '!'])
                .trim();
            if matches!(
                ty.kind(),
                "function_type" | "array_type" | "dictionary_type" | "tuple_type"
            ) || type_text.starts_with('(')
                || type_text.starts_with('[')
                || type_text.contains("->")
                || VALUE_TYPES.contains(&type_text)
            {
                continue;
            }

            let modifiers = (0..prop.child_count())
                .filter_map(|i| prop.child(i))
                .filter(|c| c.kind() == "modifiers")
                .map(|m| node_text(m, ctx.source))
                .collect::<Vec<_>>()
                .join(" ");
            if modifiers.contains("weak") || modifiers.contains("unowned") {
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
                symbol: Some(name.to_string()),
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

/// MEM-005: KVO observation not removed.
pub struct KvoLeak;

impl AuditRule for KvoLeak {
    fn id(&self) -> &str {
        "MEM-005"
    }
    fn name(&self) -> &str {
        "kvo-observer-leak"
    }
    fn category(&self) -> Category {
        Category::Memory
    }
    fn severity(&self) -> Severity {
        Severity::Medium
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        // Find calls to observe(_:options:changeHandler:) or addObserver(_:forKeyPath:...)
        let calls = find_descendants(root, ctx.source, &|node, src| {
            if node.kind() != "call_expression" {
                return false;
            }
            let text = node_text(node, src);
            text.contains(".observe(") || text.contains("addObserver(")
        });

        for call in calls {
            let text = node_text(call, ctx.source);
            // Modern KVO (observe) returns a token — check if it's stored
            if text.contains(".observe(") {
                // Check if the result is assigned to a property
                if let Some(parent) = call.parent() {
                    let parent_text = node_text(parent, ctx.source);
                    if !parent_text.contains("= ") && !parent_text.contains("let ") {
                        issues.push(AuditIssue {
                            id: format!("{}:{}", self.id(), ctx.file_path),
                            category: self.category(),
                            severity: self.severity(),
                            rule: self.id().to_string(),
                            message: "KVO observation result not stored — will be immediately invalidated".into(),
                            file: ctx.file_path.to_string(),
                            line: call.start_position().row as u32 + 1,
                            symbol: None,
                            fix: Some("Store the NSKeyValueObservation token in a property".into()),
                        });
                    }
                }
            }

            // Legacy KVO: addObserver without matching removeObserver
            if text.contains("addObserver(") && !ctx.source.contains("removeObserver(") {
                issues.push(AuditIssue {
                    id: format!("{}:{}", self.id(), ctx.file_path),
                    category: self.category(),
                    severity: self.severity(),
                    rule: self.id().to_string(),
                    message: "addObserver() without matching removeObserver() — KVO leak".into(),
                    file: ctx.file_path.to_string(),
                    line: call.start_position().row as u32 + 1,
                    symbol: None,
                    fix: Some(
                        "Call removeObserver() in deinit or use modern KVO with observation tokens"
                            .into(),
                    ),
                });
            }
        }

        issues
    }
}

/// MEM-006: PHAsset/PHImageManager request accumulation without cancellation.
pub struct PhotoKitAccumulation;

impl AuditRule for PhotoKitAccumulation {
    fn id(&self) -> &str {
        "MEM-006"
    }
    fn name(&self) -> &str {
        "photokit-accumulation"
    }
    fn category(&self) -> Category {
        Category::Memory
    }
    fn severity(&self) -> Severity {
        Severity::Medium
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        // Find PHImageManager.requestImage calls
        let calls = find_descendants(root, ctx.source, &|node, src| {
            if node.kind() != "call_expression" {
                return false;
            }
            let text = node_text(node, src);
            text.contains("requestImage(") || text.contains("requestAVAsset(")
        });

        for call in calls {
            // Check if cancelImageRequest is called anywhere in the file
            if !ctx.source.contains("cancelImageRequest(") {
                issues.push(AuditIssue {
                    id: format!("{}:{}", self.id(), ctx.file_path),
                    category: self.category(),
                    severity: self.severity(),
                    rule: self.id().to_string(),
                    message: "PHImageManager request without cancelImageRequest — may accumulate memory in scroll contexts".into(),
                    file: ctx.file_path.to_string(),
                    line: call.start_position().row as u32 + 1,
                    symbol: None,
                    fix: Some("Store the PHImageRequestID and cancel previous requests before starting new ones".into()),
                });
                break; // Only one issue per file
            }
        }

        issues
    }
}

/// All memory rules.
pub fn all_rules() -> Vec<Box<dyn AuditRule>> {
    vec![
        Box::new(ClosureRetainCycle),
        Box::new(StrongDelegate),
        Box::new(TimerLeak),
        Box::new(ObserverLeak),
        Box::new(KvoLeak),
        Box::new(PhotoKitAccumulation),
    ]
}
