//! Accessibility audit rules (A11Y-001 through A11Y-004).

use crate::engine::{AuditIssue, Category, Severity};
use crate::rules::{
    find_descendants, in_preview, label_owner, modifier_calls, modifier_chain, node_text,
    AuditRule, FileContext,
};

/// A11Y-001: Image without accessibility label.
pub struct MissingAccessibilityLabel;

impl AuditRule for MissingAccessibilityLabel {
    fn id(&self) -> &str {
        "A11Y-001"
    }
    fn name(&self) -> &str {
        "missing-accessibility-label"
    }
    fn category(&self) -> Category {
        Category::Accessibility
    }
    fn severity(&self) -> Severity {
        Severity::High
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        let images = find_descendants(root, ctx.source, &|node, src| {
            node.kind() == "call_expression"
                && node.named_child(0).is_some_and(|c| {
                    c.kind() == "simple_identifier" && node_text(c, src) == "Image"
                })
        });

        for img in images {
            let text = node_text(img, ctx.source);
            if text.contains("decorative") || text.contains(".init()") {
                continue;
            }
            if in_preview(img, ctx.source) {
                continue;
            }
            let (modifiers, outer) = modifier_chain(img, ctx.source);
            if modifiers.iter().any(|m| m.starts_with("accessibility")) {
                continue;
            }
            // Only views placed in a view builder: not values stored in
            // properties, returned from model code or passed as arguments.
            let Some(statements) = outer.parent().filter(|p| p.kind() == "statements") else {
                continue;
            };
            let alone = statements.named_child_count() == 1;
            let interactive = modifiers
                .iter()
                .any(|m| matches!(*m, "onTapGesture" | "onLongPressGesture"));
            let icon_only_control = alone
                && label_owner(statements, ctx.source).is_some_and(|control| {
                    !modifier_chain(control, ctx.source)
                        .0
                        .iter()
                        .any(|m| m.starts_with("accessibility"))
                });
            // The only content of an `if` branch: state shown by the icon alone
            // (with an `else` it is a fallback, like a placeholder image)
            let state_icon = alone
                && statements.parent().is_some_and(|p| {
                    p.kind() == "if_statement"
                        && !(0..p.child_count())
                            .filter_map(|i| p.child(i))
                            .any(|c| c.kind() == "else")
                });
            if !(interactive || icon_only_control || state_icon) {
                continue;
            }
            let pos = img.start_position();
            issues.push(AuditIssue {
                id: format!("{}:{}", self.id(), ctx.file_path),
                category: self.category(),
                severity: self.severity(),
                rule: self.id().to_string(),
                message: if state_icon {
                    "Image is the only indicator of state and has no accessibilityLabel".into()
                } else {
                    "Image without accessibilityLabel — invisible to VoiceOver users".into()
                },
                file: ctx.file_path.to_string(),
                line: pos.row as u32 + 1,
                column: Some(pos.column as u32 + 1),
                symbol: None,
                fix: Some(
                    "Add .accessibilityLabel() to the image or its control, or mark it decorative"
                        .into(),
                ),
            });
        }

        issues
    }
}

/// A11Y-002: Missing Dynamic Type support (fixed font sizes).
pub struct FixedFontSize;

impl AuditRule for FixedFontSize {
    fn id(&self) -> &str {
        "A11Y-002"
    }
    fn name(&self) -> &str {
        "fixed-font-size"
    }
    fn category(&self) -> Category {
        Category::Accessibility
    }
    fn severity(&self) -> Severity {
        Severity::Medium
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        // `@ScaledMetric var size` already scales with Dynamic Type
        let scaled: Vec<&str> = ctx
            .source
            .match_indices("@ScaledMetric")
            .filter_map(|(i, _)| {
                let rest = &ctx.source[i..];
                let var = rest.find("var ")?;
                let name = rest[var + 4..].trim_start();
                let end = name
                    .find(|c: char| !(c.is_alphanumeric() || c == '_'))
                    .unwrap_or(name.len());
                Some(&name[..end])
            })
            .collect();

        let mut found = Vec::new();
        for m in modifier_calls(root, ctx.source, "font") {
            let args = m
                .call
                .named_child(1)
                .map(|a| node_text(a, ctx.source))
                .unwrap_or("");
            let args = args.trim_start_matches('(').trim_start();
            if !args.starts_with(".system(size:") || args.contains("relativeTo") {
                continue;
            }
            let size = args[".system(size:".len()..]
                .split([',', ')'])
                .next()
                .unwrap_or("")
                .trim();
            if scaled.contains(&size) {
                continue;
            }
            // Sizing an SF Symbol image, not text
            if modifier_target_root(m.call, ctx.source) == Some("Image") {
                continue;
            }
            found.push(m);
        }
        for name in ["systemFont", "boldSystemFont", "monospacedSystemFont"] {
            for m in modifier_calls(root, ctx.source, name) {
                // Scaled right away: `UIFontMetrics(...).scaledFont(for: font)`
                let mut scope = m.call;
                while let Some(p) = scope.parent() {
                    scope = p;
                    if matches!(
                        p.kind(),
                        "function_body" | "computed_property" | "statements"
                    ) && p.parent().is_some_and(|g| g.kind() != "lambda_literal")
                    {
                        break;
                    }
                }
                let statement = node_text(scope, ctx.source);
                if !statement.contains("scaledFont") && !statement.contains("UIFontMetrics") {
                    found.push(m);
                }
            }
        }

        found.sort_by_key(|m| m.position());
        for m in found {
            if in_preview(m.call, ctx.source) {
                continue;
            }
            let (line, column) = m.position();
            issues.push(AuditIssue {
                id: format!("{}:{}", self.id(), ctx.file_path),
                category: self.category(),
                severity: self.severity(),
                rule: self.id().to_string(),
                message: "Fixed font size — won't scale with Dynamic Type accessibility setting"
                    .into(),
                file: ctx.file_path.to_string(),
                line,
                column: Some(column),
                symbol: None,
                fix: Some("Use .font(.body) or .font(.system(size:, relativeTo:)) for Dynamic Type support".into()),
            });
        }

        issues
    }
}

/// A11Y-003: Color-only information (no shape/text alternative).
pub struct ColorOnlyInfo;

impl AuditRule for ColorOnlyInfo {
    fn id(&self) -> &str {
        "A11Y-003"
    }
    fn name(&self) -> &str {
        "color-only-info"
    }
    fn category(&self) -> Category {
        Category::Accessibility
    }
    fn severity(&self) -> Severity {
        Severity::Medium
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        const SHAPES: &[&str] = &[
            "Circle",
            "Rectangle",
            "RoundedRectangle",
            "Capsule",
            "Ellipse",
        ];
        const COLORING: &[&str] = &[
            "fill",
            "foregroundColor",
            "foregroundStyle",
            "background",
            "tint",
        ];
        let is_color = |text: &str| {
            let t = text.trim();
            t.starts_with("Color")
                || t.starts_with(".red")
                || t.starts_with(".green")
                || t.starts_with(".orange")
                || t.starts_with(".yellow")
        };

        for modifier in COLORING {
            for m in modifier_calls(root, ctx.source, modifier) {
                let Some(ternary) =
                    find_descendants(m.call, ctx.source, &|n, _| n.kind() == "ternary_expression")
                        .into_iter()
                        .next()
                else {
                    continue;
                };
                // `cond ? green : red`: both branches are colors
                let branches: Vec<&str> = (0..ternary.named_child_count())
                    .filter_map(|i| ternary.named_child(i))
                    .skip(1)
                    .map(|b| node_text(b, ctx.source))
                    .collect();
                if branches.len() != 2 || !branches.iter().all(|b| is_color(b)) {
                    continue;
                }
                // Applied to a shape: nothing but the color tells the state
                let mut target = m
                    .call
                    .named_child(0)
                    .and_then(|nav| nav.child_by_field_name("target"));
                while let Some(t) = target.filter(|t| {
                    t.kind() == "call_expression"
                        && t.named_child(0)
                            .is_some_and(|c| c.kind() == "navigation_expression")
                }) {
                    target = t
                        .named_child(0)
                        .and_then(|nav| nav.child_by_field_name("target"));
                }
                let Some(shape) = target.and_then(|t| crate::rules::callee_name(t, ctx.source))
                else {
                    continue;
                };
                if !SHAPES.contains(&shape) {
                    continue;
                }
                let (modifiers, _) = modifier_chain(m.call, ctx.source);
                if modifiers.iter().any(|m| m.starts_with("accessibility"))
                    || in_preview(m.call, ctx.source)
                {
                    continue;
                }
                let pos = ternary.start_position();
                issues.push(AuditIssue {
                    id: format!("{}:{}", self.id(), ctx.file_path),
                    category: self.category(),
                    severity: self.severity(),
                    rule: self.id().to_string(),
                    message: "Status conveyed by color alone — inaccessible to color-blind users"
                        .into(),
                    file: ctx.file_path.to_string(),
                    line: pos.row as u32 + 1,
                    column: Some(pos.column as u32 + 1),
                    symbol: None,
                    fix: Some("Add text label, icon, or accessibilityLabel alongside color".into()),
                });
            }
        }

        issues
    }
}

/// A11Y-004: Interactive element too small for touch (< 44pt).
pub struct SmallTouchTarget;

impl AuditRule for SmallTouchTarget {
    fn id(&self) -> &str {
        "A11Y-004"
    }
    fn name(&self) -> &str {
        "small-touch-target"
    }
    fn category(&self) -> Category {
        Category::Accessibility
    }
    fn severity(&self) -> Severity {
        Severity::Medium
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        let enlarges = |m: &&str| matches!(*m, "padding" | "contentShape");
        for m in modifier_calls(root, ctx.source, "frame") {
            let args = m
                .call
                .named_child(1)
                .map(|a| node_text(a, ctx.source))
                .unwrap_or("");
            if args.contains("minHeight") {
                continue;
            }
            let Some(height_idx) = args.find("height:") else {
                continue;
            };
            let num: String = args[height_idx + 7..]
                .trim_start()
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            let Ok(height) = num.parse::<f64>() else {
                continue;
            };
            if !(height > 0.0 && height < 44.0) || in_preview(m.call, ctx.source) {
                continue;
            }
            // Modifiers applied after the frame, and the view's placement
            let (after, outer) = modifier_chain(m.call, ctx.source);
            // Only the outermost frame of a chain counts; hit testing off
            // means it is not a target at all
            if after.iter().any(enlarges)
                || after.contains(&"frame")
                || after.contains(&"allowsHitTesting")
                || in_toolbar(m.call, ctx.source)
            {
                continue;
            }
            let tappable = after
                .iter()
                .any(|m| matches!(*m, "onTapGesture" | "onLongPressGesture"));
            let control_label = outer
                .parent()
                .filter(|p| p.kind() == "statements" && p.named_child_count() == 1)
                .and_then(|statements| label_owner(statements, ctx.source))
                .is_some_and(|control| {
                    let (control_mods, _) = modifier_chain(control, ctx.source);
                    !control_mods
                        .iter()
                        .any(|m| enlarges(m) || *m == "frame" || *m == "allowsHitTesting")
                });
            if !(tappable || control_label) {
                continue;
            }
            let (line, column) = m.position();
            issues.push(AuditIssue {
                id: format!("{}:{}", self.id(), ctx.file_path),
                category: self.category(),
                severity: self.severity(),
                rule: self.id().to_string(),
                message: format!(
                    "Touch target height {height}pt is below 44pt minimum — hard to tap"
                ),
                file: ctx.file_path.to_string(),
                line,
                column: Some(column),
                symbol: None,
                fix: Some(
                    "Ensure minimum 44x44pt touch target or use .contentShape(Rectangle())".into(),
                ),
            });
        }

        issues
    }
}

/// All accessibility rules.
pub fn all_rules() -> Vec<Box<dyn AuditRule>> {
    vec![
        Box::new(MissingAccessibilityLabel),
        Box::new(FixedFontSize),
        Box::new(ColorOnlyInfo),
        Box::new(SmallTouchTarget),
    ]
}

/// Callee name of the view a modifier chain starts from
/// (`Image(...).font(...)` → `Image`).
fn modifier_target_root<'a>(call: tree_sitter::Node<'a>, source: &'a str) -> Option<&'a str> {
    let mut target = call
        .named_child(0)
        .and_then(|nav| nav.child_by_field_name("target"));
    while let Some(t) = target.filter(|t| {
        t.kind() == "call_expression"
            && t.named_child(0)
                .is_some_and(|c| c.kind() == "navigation_expression")
    }) {
        target = t
            .named_child(0)
            .and_then(|nav| nav.child_by_field_name("target"));
    }
    target.and_then(|t| crate::rules::callee_name(t, source))
}

/// Whether the node is inside a `ToolbarItem`, where the system sizes the
/// hit area.
fn in_toolbar(node: tree_sitter::Node, source: &str) -> bool {
    let mut current = node.parent();
    while let Some(n) = current {
        if n.kind() == "call_expression"
            && matches!(
                crate::rules::callee_name(n, source),
                Some("ToolbarItem" | "ToolbarItemGroup")
            )
        {
            return true;
        }
        current = n.parent();
    }
    false
}
