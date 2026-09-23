//! SwiftUI Performance audit rules (SUI-001 through SUI-006).

use crate::engine::{AuditIssue, Category, Severity};
use crate::rules::{find_descendants, has_attribute, node_text, AuditRule, FileContext};

/// SUI-001: Complex body — too many child nodes in a SwiftUI body property.
pub struct ComplexBody;

impl AuditRule for ComplexBody {
    fn id(&self) -> &str {
        "SUI-001"
    }
    fn name(&self) -> &str {
        "complex-body"
    }
    fn category(&self) -> Category {
        Category::SwiftuiPerformance
    }
    fn severity(&self) -> Severity {
        Severity::Medium
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        // Find `var body: some View { ... }` computed properties
        let body_props = find_descendants(root, ctx.source, &|node, src| {
            if node.kind() != "computed_property" && node.kind() != "property_declaration" {
                return false;
            }
            let text = node_text(node, src);
            text.contains("var body") && text.contains("some View")
        });

        for prop in body_props {
            let text = node_text(prop, ctx.source);
            // Count lines as proxy for complexity
            let line_count = text.lines().count();
            if line_count > 60 {
                issues.push(AuditIssue {
                    id: format!("{}:{}", self.id(), ctx.file_path),
                    category: self.category(),
                    severity: self.severity(),
                    rule: self.id().to_string(),
                    message: format!(
                        "SwiftUI body has {line_count} lines — extract subviews for better performance and readability"
                    ),
                    file: ctx.file_path.to_string(),
                    line: prop.start_position().row as u32 + 1,
                    column: None,
                    symbol: None,
                    fix: Some("Break body into smaller extracted subviews".into()),
                });
            }
        }

        issues
    }
}

/// SUI-002: Heavy work in .onAppear without Task.
pub struct HeavyOnAppear;

impl AuditRule for HeavyOnAppear {
    fn id(&self) -> &str {
        "SUI-002"
    }
    fn name(&self) -> &str {
        "heavy-on-appear"
    }
    fn category(&self) -> Category {
        Category::SwiftuiPerformance
    }
    fn severity(&self) -> Severity {
        Severity::Medium
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        // Find .onAppear { ... } blocks with synchronous heavy operations
        let on_appear_calls = find_descendants(root, ctx.source, &|node, src| {
            if node.kind() != "call_expression" {
                return false;
            }
            let text = node_text(node, src);
            text.starts_with(".onAppear") || text.contains(".onAppear")
        });

        for call in on_appear_calls {
            let text = node_text(call, ctx.source);
            // Check for synchronous heavy work patterns
            let has_sync_heavy = text.contains("JSONDecoder()")
                || text.contains("JSONEncoder()")
                || text.contains("FileManager")
                || text.contains("Data(contentsOf:")
                || text.contains("String(contentsOf:");

            if has_sync_heavy && !text.contains("Task {") && !text.contains("Task.detached") {
                issues.push(AuditIssue {
                    id: format!("{}:{}", self.id(), ctx.file_path),
                    category: self.category(),
                    severity: self.severity(),
                    rule: self.id().to_string(),
                    message: "Synchronous heavy work in .onAppear — may cause frame drops".into(),
                    file: ctx.file_path.to_string(),
                    line: call.start_position().row as u32 + 1,
                    column: None,
                    symbol: None,
                    fix: Some(
                        "Wrap heavy work in a Task { } block to avoid blocking the main thread"
                            .into(),
                    ),
                });
            }
        }

        issues
    }
}

/// SUI-003: Missing Equatable conformance on @Observable or ObservableObject.
pub struct MissingEquatable;

impl AuditRule for MissingEquatable {
    fn id(&self) -> &str {
        "SUI-003"
    }
    fn name(&self) -> &str {
        "missing-equatable"
    }
    fn category(&self) -> Category {
        Category::SwiftuiPerformance
    }
    fn severity(&self) -> Severity {
        // Advisory: precision 1/15 on a sampled 7300-file app.
        Severity::Advisory
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        // Find struct types used as @State or passed to child views
        let structs = find_descendants(root, ctx.source, &|node, src| {
            if node.kind() != "class_declaration" {
                return false;
            }
            let text = node_text(node, src);
            text.starts_with("struct ")
        });

        for decl in structs {
            let text = node_text(decl, ctx.source);
            // Check if used as view model data (has multiple properties)
            let prop_count = find_descendants(decl, ctx.source, &|node, src| {
                node.kind() == "property_declaration" && node_text(node, src).starts_with("var ")
            })
            .len();

            if prop_count >= 3 && !text.contains("Equatable") && !text.contains("Identifiable") {
                let name = crate::rules::decl_name(decl, ctx.source).unwrap_or_default();
                // Only flag if it looks like a data model (not a View)
                if !text.contains("some View") && !text.contains(": View") {
                    issues.push(AuditIssue {
                        id: format!("{}:{}", self.id(), ctx.file_path),
                        category: self.category(),
                        severity: self.severity(),
                        rule: self.id().to_string(),
                        message: format!(
                            "Struct `{name}` has {prop_count} properties but no Equatable conformance — SwiftUI may diff unnecessarily"
                        ),
                        file: ctx.file_path.to_string(),
                        line: decl.start_position().row as u32 + 1,
                        column: None,
                        symbol: Some(name),
                        fix: Some("Add Equatable conformance for efficient SwiftUI diffing".into()),
                    });
                }
            }
        }

        issues
    }
}

/// SUI-004: @StateObject used where @State + @Observable would be better (iOS 17+).
pub struct StateObjectDeprecated;

impl AuditRule for StateObjectDeprecated {
    fn id(&self) -> &str {
        "SUI-004"
    }
    fn name(&self) -> &str {
        "state-object-deprecated"
    }
    fn category(&self) -> Category {
        Category::SwiftuiPerformance
    }
    fn severity(&self) -> Severity {
        // Advisory: precision 0/15 on a sampled 7300-file app.
        Severity::Advisory
    }
    fn min_ios_major(&self) -> Option<u32> {
        Some(17)
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        let props = find_descendants(root, ctx.source, &|node, src| {
            if node.kind() != "property_declaration" {
                return false;
            }
            has_attribute(node, src, "StateObject")
        });

        for prop in props {
            issues.push(AuditIssue {
                id: format!("{}:{}", self.id(), ctx.file_path),
                category: self.category(),
                severity: self.severity(),
                rule: self.id().to_string(),
                message: "@StateObject can be replaced with @State + @Observable (iOS 17+)".into(),
                file: ctx.file_path.to_string(),
                line: prop.start_position().row as u32 + 1,
                column: None,
                symbol: None,
                fix: Some(
                    "Migrate to @Observable class with @State instead of @StateObject".into(),
                ),
            });
        }

        issues
    }
}

/// SUI-005: Large list without lazy loading.
///
/// `ForEach` over a literal collection of at most [`SMALL_COLLECTION`]
/// elements (`["a", "b"]`, `0..<3`) is not reported. Outside a
/// `ScrollView` the content cannot be long, so the finding is advisory; inside a
/// `ScrollView` it keeps the rule's severity.
pub struct NonLazyList;

/// Literal collections up to this size are rendered eagerly without cost.
pub const SMALL_COLLECTION: usize = 10;

/// Whether the first argument of `ForEach(...)` is a small literal.
fn small_literal_collection(call: tree_sitter::Node, source: &str) -> bool {
    let Some(arg) =
        crate::rules::find_descendants(call, source, &|n, _| n.kind() == "value_argument")
            .into_iter()
            .next()
    else {
        return false;
    };
    let text = node_text(arg, source).trim();
    if let Some(inner) = text.strip_prefix('[').and_then(|t| t.strip_suffix(']')) {
        return inner.split(',').filter(|e| !e.trim().is_empty()).count() <= SMALL_COLLECTION;
    }
    for op in ["..<", "..."] {
        if let Some((lo, hi)) = text.split_once(op) {
            if let (Ok(lo), Ok(hi)) = (lo.trim().parse::<usize>(), hi.trim().parse::<usize>()) {
                return hi.saturating_sub(lo) <= SMALL_COLLECTION;
            }
        }
    }
    false
}

/// Whether `node` is inside a `ScrollView { ... }`.
fn in_scroll_view(node: tree_sitter::Node, source: &str) -> bool {
    let mut current = node.parent();
    while let Some(n) = current {
        if n.kind() == "call_expression"
            && crate::rules::callee_name(n, source) == Some("ScrollView")
        {
            return true;
        }
        current = n.parent();
    }
    false
}

impl AuditRule for NonLazyList {
    fn id(&self) -> &str {
        "SUI-005"
    }
    fn name(&self) -> &str {
        "non-lazy-list"
    }
    fn category(&self) -> Category {
        Category::SwiftuiPerformance
    }
    fn severity(&self) -> Severity {
        Severity::Medium
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        // Find ForEach inside VStack/HStack (not in List/LazyVStack/LazyHStack)
        let for_each_calls = find_descendants(root, ctx.source, &|node, src| {
            if node.kind() != "call_expression" {
                return false;
            }
            let text = node_text(node, src);
            text.starts_with("ForEach")
        });

        for call in for_each_calls {
            // Walk up to find parent container
            let mut parent = call.parent();
            let mut in_lazy = false;
            while let Some(p) = parent {
                let text = node_text(p, ctx.source);
                if text.starts_with("LazyVStack")
                    || text.starts_with("LazyHStack")
                    || text.starts_with("List")
                    || text.starts_with("LazyVGrid")
                    || text.starts_with("LazyHGrid")
                {
                    in_lazy = true;
                    break;
                }
                if text.starts_with("VStack") || text.starts_with("HStack") {
                    break;
                }
                parent = p.parent();
            }

            if !in_lazy {
                let text = node_text(call, ctx.source);
                // Only flag if ForEach iterates over something that could be large
                if text.contains("ForEach(") && !small_literal_collection(call, ctx.source) {
                    let scrolling = in_scroll_view(call, ctx.source);
                    issues.push(AuditIssue {
                        id: format!("{}:{}", self.id(), ctx.file_path),
                        category: self.category(),
                        severity: if scrolling {
                            self.severity()
                        } else {
                            // 0/15 true positives outside a ScrollView on a sampled app
                            Severity::Advisory
                        },
                        rule: self.id().to_string(),
                        message: if scrolling {
                            "ForEach in non-lazy container inside ScrollView — all items rendered at once".into()
                        } else {
                            "ForEach in non-lazy container — all items rendered at once (not scrollable, likely short)".into()
                        },
                        file: ctx.file_path.to_string(),
                        line: call.start_position().row as u32 + 1,
                        column: None,
                        symbol: None,
                        fix: Some("Use List, LazyVStack, or LazyHStack for better performance with many items".into()),
                    });
                }
            }
        }

        issues
    }
}

/// SUI-006: Expensive operation in view body (not in onAppear/task).
pub struct ExpensiveInBody;

impl AuditRule for ExpensiveInBody {
    fn id(&self) -> &str {
        "SUI-006"
    }
    fn name(&self) -> &str {
        "expensive-in-body"
    }
    fn category(&self) -> Category {
        Category::SwiftuiPerformance
    }
    fn severity(&self) -> Severity {
        // Advisory: precision 0/3 on a sampled 7300-file app.
        Severity::Advisory
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        let body_props = find_descendants(root, ctx.source, &|node, src| {
            if node.kind() != "computed_property" && node.kind() != "property_declaration" {
                return false;
            }
            let text = node_text(node, src);
            text.contains("var body") && text.contains("some View")
        });

        for prop in body_props {
            let text = node_text(prop, ctx.source);
            let expensive_patterns = [
                "DateFormatter()",
                "NumberFormatter()",
                "NSRegularExpression(",
                "JSONDecoder()",
                "JSONEncoder()",
                ".sorted(",
                ".filter(",
                ".map(",
            ];

            for pattern in &expensive_patterns {
                if text.contains(pattern) {
                    issues.push(AuditIssue {
                        id: format!("{}:{}", self.id(), ctx.file_path),
                        category: self.category(),
                        severity: self.severity(),
                        rule: self.id().to_string(),
                        message: format!(
                            "Expensive operation `{pattern}` in view body — recalculated on every render"
                        ),
                        file: ctx.file_path.to_string(),
                        line: prop.start_position().row as u32 + 1,
                        column: None,
                        symbol: None,
                        fix: Some("Move to a computed property, onAppear, or cache as a stored property".into()),
                    });
                    break; // One issue per body
                }
            }
        }

        issues
    }
}

/// All SwiftUI performance rules.
pub fn all_rules() -> Vec<Box<dyn AuditRule>> {
    vec![
        Box::new(ComplexBody),
        Box::new(HeavyOnAppear),
        Box::new(MissingEquatable),
        Box::new(StateObjectDeprecated),
        Box::new(NonLazyList),
        Box::new(ExpensiveInBody),
    ]
}
