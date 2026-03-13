//! SwiftUI Architecture audit rules (ARCH-001 through ARCH-005).

use crate::engine::{AuditIssue, Category, Severity};
use crate::rules::{find_descendants, node_text, AuditRule, FileContext};

/// ARCH-001: Business logic in view body.
pub struct LogicInView;

impl AuditRule for LogicInView {
    fn id(&self) -> &str {
        "ARCH-001"
    }
    fn name(&self) -> &str {
        "logic-in-view"
    }
    fn category(&self) -> Category {
        Category::SwiftuiArchitecture
    }
    fn severity(&self) -> Severity {
        Severity::Medium
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
            let logic_patterns = [
                "URLSession",
                "UserDefaults",
                "FileManager",
                "try await",
                "JSONDecoder()",
                "CoreData",
                "NSFetchRequest",
            ];

            for pattern in &logic_patterns {
                if text.contains(pattern) {
                    issues.push(AuditIssue {
                        id: format!("{}:{}", self.id(), ctx.file_path),
                        category: self.category(),
                        severity: self.severity(),
                        rule: self.id().to_string(),
                        message: format!(
                            "Business logic (`{pattern}`) found in view body — violates separation of concerns"
                        ),
                        file: ctx.file_path.to_string(),
                        line: prop.start_position().row as u32 + 1,
                        symbol: None,
                        fix: Some("Move business logic to a ViewModel or service layer".into()),
                    });
                    break;
                }
            }
        }

        issues
    }
}

/// ARCH-002: Massive view body (>100 lines).
pub struct MassiveViewBody;

impl AuditRule for MassiveViewBody {
    fn id(&self) -> &str {
        "ARCH-002"
    }
    fn name(&self) -> &str {
        "massive-view-body"
    }
    fn category(&self) -> Category {
        Category::SwiftuiArchitecture
    }
    fn severity(&self) -> Severity {
        Severity::Medium
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
            let line_count = text.lines().count();
            if line_count > 100 {
                issues.push(AuditIssue {
                    id: format!("{}:{}", self.id(), ctx.file_path),
                    category: self.category(),
                    severity: self.severity(),
                    rule: self.id().to_string(),
                    message: format!(
                        "View body is {line_count} lines — extract subviews for maintainability"
                    ),
                    file: ctx.file_path.to_string(),
                    line: prop.start_position().row as u32 + 1,
                    symbol: None,
                    fix: Some("Extract logical sections into separate View structs".into()),
                });
            }
        }

        issues
    }
}

/// ARCH-003: @EnvironmentObject used (prefer @Environment with Observable).
pub struct EnvironmentObjectUsage;

impl AuditRule for EnvironmentObjectUsage {
    fn id(&self) -> &str {
        "ARCH-003"
    }
    fn name(&self) -> &str {
        "environment-object-usage"
    }
    fn category(&self) -> Category {
        Category::SwiftuiArchitecture
    }
    fn severity(&self) -> Severity {
        Severity::Low
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        let props = find_descendants(root, ctx.source, &|node, src| {
            if node.kind() != "property_declaration" {
                return false;
            }
            crate::rules::has_attribute(node, src, "EnvironmentObject")
        });

        for prop in props {
            issues.push(AuditIssue {
                id: format!("{}:{}", self.id(), ctx.file_path),
                category: self.category(),
                severity: self.severity(),
                rule: self.id().to_string(),
                message:
                    "@EnvironmentObject is legacy — use @Environment with @Observable (iOS 17+)"
                        .into(),
                file: ctx.file_path.to_string(),
                line: prop.start_position().row as u32 + 1,
                symbol: None,
                fix: Some("Migrate to @Observable class and inject via @Environment".into()),
            });
        }

        issues
    }
}

/// ARCH-004: View struct with too many @State properties.
pub struct TooManyStateProperties;

impl AuditRule for TooManyStateProperties {
    fn id(&self) -> &str {
        "ARCH-004"
    }
    fn name(&self) -> &str {
        "too-many-state"
    }
    fn category(&self) -> Category {
        Category::SwiftuiArchitecture
    }
    fn severity(&self) -> Severity {
        Severity::Medium
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        // Find View structs
        let structs = find_descendants(root, ctx.source, &|node, src| {
            if node.kind() != "class_declaration" {
                return false;
            }
            let text = node_text(node, src);
            text.starts_with("struct ") && text.contains(": View")
        });

        for decl in structs {
            let state_count = find_descendants(decl, ctx.source, &|node, src| {
                if node.kind() != "property_declaration" {
                    return false;
                }
                crate::rules::has_attribute(node, src, "State")
                    && !crate::rules::has_attribute(node, src, "StateObject")
            })
            .len();

            if state_count > 5 {
                let name = crate::rules::decl_name(decl, ctx.source).unwrap_or_default();
                issues.push(AuditIssue {
                    id: format!("{}:{}", self.id(), ctx.file_path),
                    category: self.category(),
                    severity: self.severity(),
                    rule: self.id().to_string(),
                    message: format!(
                        "View `{name}` has {state_count} @State properties — consider extracting state into a model"
                    ),
                    file: ctx.file_path.to_string(),
                    line: decl.start_position().row as u32 + 1,
                    symbol: Some(name),
                    fix: Some("Group related state into an @Observable model".into()),
                });
            }
        }

        issues
    }
}

/// ARCH-005: @Published property in non-ObservableObject class.
pub struct PublishedWithoutObservable;

impl AuditRule for PublishedWithoutObservable {
    fn id(&self) -> &str {
        "ARCH-005"
    }
    fn name(&self) -> &str {
        "published-without-observable"
    }
    fn category(&self) -> Category {
        Category::SwiftuiArchitecture
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
            let keyword = crate::rules::class_keyword(decl, ctx.source);
            if keyword != "class" {
                continue;
            }

            let text = node_text(decl, ctx.source);
            if text.contains("ObservableObject") {
                continue;
            }

            // Check for @Published properties
            let published_props = find_descendants(decl, ctx.source, &|node, src| {
                if node.kind() != "property_declaration" {
                    return false;
                }
                crate::rules::has_attribute(node, src, "Published")
            });

            if !published_props.is_empty() {
                let name = crate::rules::decl_name(decl, ctx.source).unwrap_or_default();
                issues.push(AuditIssue {
                    id: format!("{}:{}", self.id(), ctx.file_path),
                    category: self.category(),
                    severity: self.severity(),
                    rule: self.id().to_string(),
                    message: format!(
                        "`{name}` has @Published properties but doesn't conform to ObservableObject"
                    ),
                    file: ctx.file_path.to_string(),
                    line: decl.start_position().row as u32 + 1,
                    symbol: Some(name),
                    fix: Some("Add ObservableObject conformance or migrate to @Observable".into()),
                });
            }
        }

        issues
    }
}

/// All SwiftUI architecture rules.
pub fn all_rules() -> Vec<Box<dyn AuditRule>> {
    vec![
        Box::new(LogicInView),
        Box::new(MassiveViewBody),
        Box::new(EnvironmentObjectUsage),
        Box::new(TooManyStateProperties),
        Box::new(PublishedWithoutObservable),
    ]
}
