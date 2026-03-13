//! Accessibility audit rules (A11Y-001 through A11Y-004).

use crate::engine::{AuditIssue, Category, Severity};
use crate::rules::{find_descendants, node_text, AuditRule, FileContext};

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

        // Find Image() or UIImageView without accessibility
        let images = find_descendants(root, ctx.source, &|node, src| {
            if node.kind() != "call_expression" {
                return false;
            }
            let text = node_text(node, src);
            (text.starts_with("Image(") || text.starts_with("Image(systemName:"))
                && !text.contains("decorative")
        });

        for img in images {
            // Check if .accessibilityLabel is chained
            let parent_text = img
                .parent()
                .and_then(|p| p.parent())
                .map(|p| node_text(p, ctx.source))
                .unwrap_or("");

            if !parent_text.contains("accessibilityLabel")
                && !parent_text.contains("accessibilityHidden")
                && !node_text(img, ctx.source).contains("decorative")
            {
                issues.push(AuditIssue {
                    id: format!("{}:{}", self.id(), ctx.file_path),
                    category: self.category(),
                    severity: self.severity(),
                    rule: self.id().to_string(),
                    message: "Image without accessibilityLabel — invisible to VoiceOver users"
                        .into(),
                    file: ctx.file_path.to_string(),
                    line: img.start_position().row as u32 + 1,
                    symbol: None,
                    fix: Some(
                        "Add .accessibilityLabel() or mark as decorative with Image(decorative:)"
                            .into(),
                    ),
                });
            }
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

        // Find .font(.system(size:)) without .relativeTo
        let font_calls = find_descendants(root, ctx.source, &|node, src| {
            if node.kind() != "call_expression" {
                return false;
            }
            let text = node_text(node, src);
            text.contains(".font(.system(size:")
                || text.contains("UIFont.systemFont(ofSize:")
                || text.contains("UIFont(name:")
        });

        for call in font_calls {
            let text = node_text(call, ctx.source);
            if !text.contains("relativeTo") && !text.contains("UIFontMetrics") {
                issues.push(AuditIssue {
                    id: format!("{}:{}", self.id(), ctx.file_path),
                    category: self.category(),
                    severity: self.severity(),
                    rule: self.id().to_string(),
                    message: "Fixed font size — won't scale with Dynamic Type accessibility setting"
                        .into(),
                    file: ctx.file_path.to_string(),
                    line: call.start_position().row as u32 + 1,
                    symbol: None,
                    fix: Some("Use .font(.body) or .font(.system(size:, relativeTo:)) for Dynamic Type support".into()),
                });
            }
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

        // Find conditional color changes (status indicators)
        let conditionals = find_descendants(root, ctx.source, &|node, src| {
            if node.kind() != "ternary_expression" && node.kind() != "if_statement" {
                return false;
            }
            let text = node_text(node, src);
            (text.contains(".red") || text.contains(".green") || text.contains("Color("))
                && text.contains("foregroundColor")
                && !text.contains("accessibilityLabel")
                && !text.contains("Text(")
        });

        for cond in conditionals {
            issues.push(AuditIssue {
                id: format!("{}:{}", self.id(), ctx.file_path),
                category: self.category(),
                severity: self.severity(),
                rule: self.id().to_string(),
                message: "Status conveyed by color alone — inaccessible to color-blind users"
                    .into(),
                file: ctx.file_path.to_string(),
                line: cond.start_position().row as u32 + 1,
                symbol: None,
                fix: Some("Add text label, icon, or accessibilityLabel alongside color".into()),
            });
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

        // Find .frame(width:, height:) with small values on buttons/tappable views
        let frames = find_descendants(root, ctx.source, &|node, src| {
            if node.kind() != "call_expression" {
                return false;
            }
            let text = node_text(node, src);
            text.contains(".frame(") && text.contains("height:")
        });

        for frame in frames {
            let text = node_text(frame, ctx.source);
            // Try to extract height value
            if let Some(height_idx) = text.find("height:") {
                let after = &text[height_idx + 7..];
                let num_str: String = after
                    .trim()
                    .chars()
                    .take_while(|c| c.is_ascii_digit() || *c == '.')
                    .collect();
                if let Ok(height) = num_str.parse::<f64>() {
                    if height < 44.0 && height > 0.0 {
                        // Check if it's tappable
                        let parent_text = frame
                            .parent()
                            .map(|p| node_text(p, ctx.source))
                            .unwrap_or("");
                        if parent_text.contains("onTapGesture")
                            || parent_text.contains("Button")
                            || parent_text.contains("NavigationLink")
                        {
                            issues.push(AuditIssue {
                                id: format!("{}:{}", self.id(), ctx.file_path),
                                category: self.category(),
                                severity: self.severity(),
                                rule: self.id().to_string(),
                                message: format!(
                                    "Touch target height {height}pt is below 44pt minimum — hard to tap"
                                ),
                                file: ctx.file_path.to_string(),
                                line: frame.start_position().row as u32 + 1,
                                symbol: None,
                                fix: Some("Ensure minimum 44x44pt touch target or use .contentShape(Rectangle())".into()),
                            });
                        }
                    }
                }
            }
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
