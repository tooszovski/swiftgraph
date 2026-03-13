//! Modernization audit rules (MOD-001 through MOD-005).

use crate::engine::{AuditIssue, Category, Severity};
use crate::rules::{find_descendants, has_attribute, node_text, AuditRule, FileContext};

/// MOD-001: ObservableObject that should migrate to @Observable.
pub struct ObservableObjectMigration;

impl AuditRule for ObservableObjectMigration {
    fn id(&self) -> &str {
        "MOD-001"
    }
    fn name(&self) -> &str {
        "observable-object-migration"
    }
    fn category(&self) -> Category {
        Category::Modernization
    }
    fn severity(&self) -> Severity {
        Severity::Low
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        let class_decls = find_descendants(root, ctx.source, &|node, src| {
            if node.kind() != "class_declaration" {
                return false;
            }
            let text = node_text(node, src);
            text.contains("ObservableObject")
        });

        for decl in class_decls {
            let name = crate::rules::decl_name(decl, ctx.source).unwrap_or_default();
            issues.push(AuditIssue {
                id: format!("{}:{}", self.id(), ctx.file_path),
                category: self.category(),
                severity: self.severity(),
                rule: self.id().to_string(),
                message: format!(
                    "`{name}` uses ObservableObject — consider migrating to @Observable (iOS 17+)"
                ),
                file: ctx.file_path.to_string(),
                line: decl.start_position().row as u32 + 1,
                symbol: Some(name),
                fix: Some(
                    "Replace ObservableObject with @Observable, remove @Published wrappers".into(),
                ),
            });
        }

        issues
    }
}

/// MOD-002: @StateObject → @State migration opportunity.
pub struct StateObjectMigration;

impl AuditRule for StateObjectMigration {
    fn id(&self) -> &str {
        "MOD-002"
    }
    fn name(&self) -> &str {
        "state-object-migration"
    }
    fn category(&self) -> Category {
        Category::Modernization
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
            has_attribute(node, src, "StateObject")
        });

        for prop in props {
            issues.push(AuditIssue {
                id: format!("{}:{}", self.id(), ctx.file_path),
                category: self.category(),
                severity: self.severity(),
                rule: self.id().to_string(),
                message:
                    "@StateObject can be replaced with @State when using @Observable (iOS 17+)"
                        .into(),
                file: ctx.file_path.to_string(),
                line: prop.start_position().row as u32 + 1,
                symbol: None,
                fix: Some("Use @State with @Observable class instead of @StateObject".into()),
            });
        }

        issues
    }
}

/// MOD-003: @ObservedObject → direct @Bindable usage.
pub struct ObservedObjectMigration;

impl AuditRule for ObservedObjectMigration {
    fn id(&self) -> &str {
        "MOD-003"
    }
    fn name(&self) -> &str {
        "observed-object-migration"
    }
    fn category(&self) -> Category {
        Category::Modernization
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
            has_attribute(node, src, "ObservedObject")
        });

        for prop in props {
            issues.push(AuditIssue {
                id: format!("{}:{}", self.id(), ctx.file_path),
                category: self.category(),
                severity: self.severity(),
                rule: self.id().to_string(),
                message:
                    "@ObservedObject can be removed with @Observable — pass the object directly"
                        .into(),
                file: ctx.file_path.to_string(),
                line: prop.start_position().row as u32 + 1,
                symbol: None,
                fix: Some(
                    "With @Observable, remove @ObservedObject and use @Bindable for bindings"
                        .into(),
                ),
            });
        }

        issues
    }
}

/// MOD-004: onChange(of:perform:) → onChange(of:) with two-param closure.
pub struct DeprecatedOnChange;

impl AuditRule for DeprecatedOnChange {
    fn id(&self) -> &str {
        "MOD-004"
    }
    fn name(&self) -> &str {
        "deprecated-on-change"
    }
    fn category(&self) -> Category {
        Category::Modernization
    }
    fn severity(&self) -> Severity {
        Severity::Low
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        let calls = find_descendants(root, ctx.source, &|node, src| {
            if node.kind() != "call_expression" {
                return false;
            }
            let text = node_text(node, src);
            text.contains(".onChange(of:") && text.contains("perform:")
        });

        for call in calls {
            issues.push(AuditIssue {
                id: format!("{}:{}", self.id(), ctx.file_path),
                category: self.category(),
                severity: self.severity(),
                rule: self.id().to_string(),
                message: "onChange(of:perform:) is deprecated in iOS 17 — use new onChange(of:) with oldValue/newValue".into(),
                file: ctx.file_path.to_string(),
                line: call.start_position().row as u32 + 1,
                symbol: None,
                fix: Some("Use .onChange(of: value) { oldValue, newValue in ... }".into()),
            });
        }

        issues
    }
}

/// MOD-005: NavigationView → NavigationStack migration.
pub struct NavigationViewMigration;

impl AuditRule for NavigationViewMigration {
    fn id(&self) -> &str {
        "MOD-005"
    }
    fn name(&self) -> &str {
        "navigation-view-migration"
    }
    fn category(&self) -> Category {
        Category::Modernization
    }
    fn severity(&self) -> Severity {
        Severity::Medium
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        let nav_views = find_descendants(root, ctx.source, &|node, src| {
            if node.kind() != "call_expression" {
                return false;
            }
            let text = node_text(node, src);
            text.starts_with("NavigationView")
        });

        for nav in nav_views {
            issues.push(AuditIssue {
                id: format!("{}:{}", self.id(), ctx.file_path),
                category: self.category(),
                severity: self.severity(),
                rule: self.id().to_string(),
                message: "NavigationView is deprecated — use NavigationStack (iOS 16+)".into(),
                file: ctx.file_path.to_string(),
                line: nav.start_position().row as u32 + 1,
                symbol: None,
                fix: Some(
                    "Replace NavigationView with NavigationStack for programmatic navigation"
                        .into(),
                ),
            });
        }

        issues
    }
}

/// All modernization rules.
pub fn all_rules() -> Vec<Box<dyn AuditRule>> {
    vec![
        Box::new(ObservableObjectMigration),
        Box::new(StateObjectMigration),
        Box::new(ObservedObjectMigration),
        Box::new(DeprecatedOnChange),
        Box::new(NavigationViewMigration),
    ]
}
