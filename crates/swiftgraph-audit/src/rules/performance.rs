//! Swift performance audit rules (PERF-001 through PERF-006).

use crate::engine::{AuditIssue, Category, Severity};
use crate::rules::{find_descendants, node_text, AuditRule, FileContext};

/// PERF-001: Unnecessary copy — large struct (>3 stored properties) without borrowing/consuming.
pub struct UnnecessaryCopy;

impl AuditRule for UnnecessaryCopy {
    fn id(&self) -> &str {
        "PERF-001"
    }
    fn name(&self) -> &str {
        "unnecessary-copy"
    }
    fn category(&self) -> Category {
        Category::SwiftPerformance
    }
    fn severity(&self) -> Severity {
        Severity::Medium
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        // Find struct declarations
        let structs = find_descendants(root, ctx.source, &|node, _src| {
            node.kind() == "class_declaration"
                && node
                    .child(0)
                    .and_then(|c| c.utf8_text(ctx.source.as_bytes()).ok())
                    == Some("struct")
        });

        for struct_node in structs {
            // Count stored properties (var/let declarations inside the struct body)
            let props = find_descendants(struct_node, ctx.source, &|node, _src| {
                node.kind() == "property_declaration"
                    && node.parent().map(|p| p.kind()) == Some("class_body")
            });

            if props.len() > 3 {
                // Check if any function parameter takes this struct by borrowing/consuming
                let struct_name = struct_node
                    .child_by_field_name("name")
                    .or_else(|| {
                        (0..struct_node.child_count())
                            .filter_map(|i| struct_node.child(i))
                            .find(|c| {
                                c.kind() == "simple_identifier" || c.kind() == "type_identifier"
                            })
                    })
                    .and_then(|n| n.utf8_text(ctx.source.as_bytes()).ok())
                    .unwrap_or("unknown");

                let text = node_text(struct_node, ctx.source);
                let has_borrowing = text.contains("borrowing") || text.contains("consuming");

                if !has_borrowing {
                    issues.push(AuditIssue {
                        id: format!("{}:{}", self.id(), ctx.file_path),
                        category: self.category(),
                        severity: self.severity(),
                        rule: self.id().to_string(),
                        message: format!(
                            "Large struct `{struct_name}` ({} properties) — consider `borrowing`/`consuming` parameters to avoid copies",
                            props.len()
                        ),
                        file: ctx.file_path.to_string(),
                        line: struct_node.start_position().row as u32 + 1,
                        symbol: Some(struct_name.to_string()),
                        fix: Some(
                            "Use `borrowing` or `consuming` parameter ownership modifiers for large value types"
                                .into(),
                        ),
                    });
                }
            }
        }

        issues
    }
}

/// PERF-002: Excessive ARC — `[weak self]` immediately followed by `guard let self`.
pub struct ExcessiveArc;

impl AuditRule for ExcessiveArc {
    fn id(&self) -> &str {
        "PERF-002"
    }
    fn name(&self) -> &str {
        "excessive-arc"
    }
    fn category(&self) -> Category {
        Category::SwiftPerformance
    }
    fn severity(&self) -> Severity {
        Severity::Low
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let mut issues = Vec::new();

        // Text-based check: find [weak self] followed closely by guard let self
        for (i, line) in ctx.source.lines().enumerate() {
            if line.contains("[weak self]") {
                // Look ahead up to 3 lines for "guard let self"
                let remaining: String = ctx
                    .source
                    .lines()
                    .skip(i + 1)
                    .take(3)
                    .collect::<Vec<_>>()
                    .join("\n");
                if remaining.contains("guard let self") || remaining.contains("guard let `self`") {
                    issues.push(AuditIssue {
                        id: format!("{}:{}", self.id(), ctx.file_path),
                        category: self.category(),
                        severity: self.severity(),
                        rule: self.id().to_string(),
                        message: "`[weak self]` + immediate `guard let self` — ARC overhead for no benefit".into(),
                        file: ctx.file_path.to_string(),
                        line: i as u32 + 1,
                        symbol: None,
                        fix: Some(
                            "If self is always needed, capture `[self]` directly (for non-escaping closures) or use `[unowned self]` if lifetime is guaranteed"
                                .into(),
                        ),
                    });
                }
            }
        }

        issues
    }
}

/// PERF-003: Existential overhead — `any Protocol` in collections.
pub struct ExistentialOverhead;

impl AuditRule for ExistentialOverhead {
    fn id(&self) -> &str {
        "PERF-003"
    }
    fn name(&self) -> &str {
        "existential-overhead"
    }
    fn category(&self) -> Category {
        Category::SwiftPerformance
    }
    fn severity(&self) -> Severity {
        Severity::Medium
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let mut issues = Vec::new();

        // Find "any X" inside collection types like Array, Set, Dictionary
        for (i, line) in ctx.source.lines().enumerate() {
            let trimmed = line.trim();
            // Match patterns like [any Protocol], Array<any Protocol>, Set<any Protocol>
            let has_any_in_collection = (trimmed.contains("[any ") && trimmed.contains(']'))
                || (trimmed.contains("Array<any ")
                    || trimmed.contains("Set<any ")
                    || trimmed.contains("Dictionary<") && trimmed.contains("any "));

            if has_any_in_collection {
                issues.push(AuditIssue {
                    id: format!("{}:{}", self.id(), ctx.file_path),
                    category: self.category(),
                    severity: self.severity(),
                    rule: self.id().to_string(),
                    message: "Existential type (`any Protocol`) in collection — 24+ bytes per element with heap allocation".into(),
                    file: ctx.file_path.to_string(),
                    line: i as u32 + 1,
                    symbol: None,
                    fix: Some(
                        "Use generics (`some Protocol`) or a concrete wrapper type to avoid existential container overhead"
                            .into(),
                    ),
                });
            }
        }

        issues
    }
}

/// PERF-004: Collection append in loop without reserveCapacity.
pub struct CollectionNoReserve;

impl AuditRule for CollectionNoReserve {
    fn id(&self) -> &str {
        "PERF-004"
    }
    fn name(&self) -> &str {
        "collection-no-reserve"
    }
    fn category(&self) -> Category {
        Category::SwiftPerformance
    }
    fn severity(&self) -> Severity {
        Severity::Low
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        // Find for/while loops containing .append(
        let loops = find_descendants(root, ctx.source, &|node, _src| {
            node.kind() == "for_statement" || node.kind() == "while_statement"
        });

        for loop_node in loops {
            let loop_text = node_text(loop_node, ctx.source);
            if loop_text.contains(".append(") {
                // Check if reserveCapacity is called before the loop (within the same parent scope)
                let parent_text = loop_node
                    .parent()
                    .map(|p| {
                        let start = p.start_byte();
                        let end = loop_node.start_byte();
                        &ctx.source[start..end]
                    })
                    .unwrap_or("");

                if !parent_text.contains("reserveCapacity") {
                    issues.push(AuditIssue {
                        id: format!("{}:{}", self.id(), ctx.file_path),
                        category: self.category(),
                        severity: self.severity(),
                        rule: self.id().to_string(),
                        message: "`.append()` in loop without `reserveCapacity` — may cause repeated reallocations".into(),
                        file: ctx.file_path.to_string(),
                        line: loop_node.start_position().row as u32 + 1,
                        symbol: None,
                        fix: Some(
                            "Call `array.reserveCapacity(expectedCount)` before the loop if count is known or estimable"
                                .into(),
                        ),
                    });
                }
            }
        }

        issues
    }
}

/// PERF-005: Actor hop overhead — `await actor.method()` in tight loop.
pub struct ActorHopOverhead;

impl AuditRule for ActorHopOverhead {
    fn id(&self) -> &str {
        "PERF-005"
    }
    fn name(&self) -> &str {
        "actor-hop-overhead"
    }
    fn category(&self) -> Category {
        Category::SwiftPerformance
    }
    fn severity(&self) -> Severity {
        Severity::High
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        // Find for/while loops containing await
        let loops = find_descendants(root, ctx.source, &|node, _src| {
            node.kind() == "for_statement" || node.kind() == "while_statement"
        });

        for loop_node in loops {
            let loop_text = node_text(loop_node, ctx.source);
            // Count await occurrences inside the loop
            let await_count = loop_text.matches("await ").count();
            if await_count > 0 {
                // Heuristic: if the loop body has await on what looks like a property/method access
                // (e.g., `await actor.method()`), it's likely an actor hop per iteration
                if loop_text.contains("await ") && loop_text.contains('.') {
                    issues.push(AuditIssue {
                        id: format!("{}:{}", self.id(), ctx.file_path),
                        category: self.category(),
                        severity: self.severity(),
                        rule: self.id().to_string(),
                        message: format!(
                            "`await` in loop body ({await_count}x) — each iteration may suspend and context-switch to another actor"
                        ),
                        file: ctx.file_path.to_string(),
                        line: loop_node.start_position().row as u32 + 1,
                        symbol: None,
                        fix: Some(
                            "Batch operations: collect data first, then call the actor once, or move the loop inside the actor"
                                .into(),
                        ),
                    });
                }
            }
        }

        issues
    }
}

/// PERF-006: Large value type — struct with arrays or >5 properties.
pub struct LargeValueType;

impl AuditRule for LargeValueType {
    fn id(&self) -> &str {
        "PERF-006"
    }
    fn name(&self) -> &str {
        "large-value-type"
    }
    fn category(&self) -> Category {
        Category::SwiftPerformance
    }
    fn severity(&self) -> Severity {
        Severity::Low
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        let structs = find_descendants(root, ctx.source, &|node, _src| {
            node.kind() == "class_declaration"
                && node
                    .child(0)
                    .and_then(|c| c.utf8_text(ctx.source.as_bytes()).ok())
                    == Some("struct")
        });

        for struct_node in structs {
            let struct_text = node_text(struct_node, ctx.source);

            let props = find_descendants(struct_node, ctx.source, &|node, _src| {
                node.kind() == "property_declaration"
                    && node.parent().map(|p| p.kind()) == Some("class_body")
            });

            let has_array = struct_text.contains("[")
                && struct_text.contains("]")
                && (struct_text.contains(": [") || struct_text.contains("Array<"));

            // >5 properties OR contains array-typed properties
            if props.len() > 5 || (props.len() > 2 && has_array) {
                let struct_name = (0..struct_node.child_count())
                    .filter_map(|i| struct_node.child(i))
                    .find(|c| c.kind() == "simple_identifier" || c.kind() == "type_identifier")
                    .and_then(|n| n.utf8_text(ctx.source.as_bytes()).ok())
                    .unwrap_or("unknown");

                // Skip if PERF-001 already covers this (>3 props is PERF-001, >5 or arrays is PERF-006)
                // PERF-006 fires for >5 props or array-containing structs
                let reason = if has_array {
                    format!(
                        "Struct `{struct_name}` contains array properties — each copy duplicates the array buffer reference"
                    )
                } else {
                    format!(
                        "Struct `{struct_name}` has {} properties — consider class or indirect storage",
                        props.len()
                    )
                };

                issues.push(AuditIssue {
                    id: format!("{}:{}", self.id(), ctx.file_path),
                    category: self.category(),
                    severity: self.severity(),
                    rule: self.id().to_string(),
                    message: reason,
                    file: ctx.file_path.to_string(),
                    line: struct_node.start_position().row as u32 + 1,
                    symbol: Some(struct_name.to_string()),
                    fix: Some(
                        "Consider: (1) use class instead, (2) use indirect storage with a reference-type backing, or (3) use borrowing/consuming parameters"
                            .into(),
                    ),
                });
            }
        }

        issues
    }
}

/// All performance rules.
pub fn all_rules() -> Vec<Box<dyn AuditRule>> {
    vec![
        Box::new(UnnecessaryCopy),
        Box::new(ExcessiveArc),
        Box::new(ExistentialOverhead),
        Box::new(CollectionNoReserve),
        Box::new(ActorHopOverhead),
        Box::new(LargeValueType),
    ]
}
