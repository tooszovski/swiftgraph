//! Codable audit rules (COD-001 through COD-005).

use crate::engine::{AuditIssue, Category, Severity};
use crate::rules::{find_descendants, node_text, AuditRule, FileContext};

/// COD-001: Manual JSON building with dictionaries instead of Codable.
pub struct ManualJsonBuilding;

impl AuditRule for ManualJsonBuilding {
    fn id(&self) -> &str {
        "COD-001"
    }
    fn name(&self) -> &str {
        "manual-json-building"
    }
    fn category(&self) -> Category {
        Category::Codable
    }
    fn severity(&self) -> Severity {
        Severity::Medium
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        let calls = find_descendants(root, ctx.source, &|node, src| {
            if node.kind() != "call_expression" {
                return false;
            }
            let text = node_text(node, src);
            text.contains("JSONSerialization.data(")
                || text.contains("JSONSerialization.jsonObject(")
        });

        for call in calls {
            issues.push(AuditIssue {
                id: format!("{}:{}", self.id(), ctx.file_path),
                category: self.category(),
                severity: self.severity(),
                rule: self.id().to_string(),
                message: "Using JSONSerialization instead of Codable — less type-safe".into(),
                file: ctx.file_path.to_string(),
                line: call.start_position().row as u32 + 1,
                symbol: None,
                fix: Some("Define Codable structs and use JSONEncoder/JSONDecoder".into()),
            });
        }

        issues
    }
}

/// COD-002: `try?` swallowing Codable decoding errors.
pub struct TryOptionalDecoding;

impl AuditRule for TryOptionalDecoding {
    fn id(&self) -> &str {
        "COD-002"
    }
    fn name(&self) -> &str {
        "try-optional-decoding"
    }
    fn category(&self) -> Category {
        Category::Codable
    }
    fn severity(&self) -> Severity {
        Severity::High
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        let try_exprs = find_descendants(root, ctx.source, &|node, src| {
            if node.kind() != "try_expression" {
                return false;
            }
            let text = node_text(node, src);
            text.starts_with("try?") && (text.contains(".decode(") || text.contains("JSONDecoder"))
        });

        for expr in try_exprs {
            issues.push(AuditIssue {
                id: format!("{}:{}", self.id(), ctx.file_path),
                category: self.category(),
                severity: self.severity(),
                rule: self.id().to_string(),
                message: "`try?` on decode silently loses decoding errors — data corruption goes unnoticed".into(),
                file: ctx.file_path.to_string(),
                line: expr.start_position().row as u32 + 1,
                symbol: None,
                fix: Some("Use try/catch and log the DecodingError for debugging".into()),
            });
        }

        issues
    }
}

/// COD-003: Date decoding without explicit strategy.
pub struct DateDecodingStrategy;

impl AuditRule for DateDecodingStrategy {
    fn id(&self) -> &str {
        "COD-003"
    }
    fn name(&self) -> &str {
        "date-decoding-strategy"
    }
    fn category(&self) -> Category {
        Category::Codable
    }
    fn severity(&self) -> Severity {
        Severity::Medium
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        // Find JSONDecoder() usage without dateDecodingStrategy
        let decoders = find_descendants(root, ctx.source, &|node, src| {
            if node.kind() != "call_expression" {
                return false;
            }
            let text = node_text(node, src);
            text.contains("JSONDecoder()")
        });

        for decoder in decoders {
            // Check surrounding context for dateDecodingStrategy
            let parent = decoder.parent();
            let context = parent
                .and_then(|p| p.parent())
                .map(|p| node_text(p, ctx.source))
                .unwrap_or("");

            // Also check if Date is used in Codable structs in this file
            let has_date = ctx.source.contains(": Date")
                || ctx.source.contains(": Date?")
                || ctx.source.contains("[Date]");

            if has_date
                && !context.contains("dateDecodingStrategy")
                && !ctx.source.contains("dateDecodingStrategy")
            {
                issues.push(AuditIssue {
                    id: format!("{}:{}", self.id(), ctx.file_path),
                    category: self.category(),
                    severity: self.severity(),
                    rule: self.id().to_string(),
                    message:
                        "JSONDecoder without dateDecodingStrategy — Date fields may fail to decode"
                            .into(),
                    file: ctx.file_path.to_string(),
                    line: decoder.start_position().row as u32 + 1,
                    symbol: None,
                    fix: Some(
                        "Set decoder.dateDecodingStrategy = .iso8601 or appropriate strategy"
                            .into(),
                    ),
                });
            }
        }

        issues
    }
}

/// COD-004: CodingKeys enum missing cases.
pub struct ManualCodingKeys;

impl AuditRule for ManualCodingKeys {
    fn id(&self) -> &str {
        "COD-004"
    }
    fn name(&self) -> &str {
        "manual-coding-keys"
    }
    fn category(&self) -> Category {
        Category::Codable
    }
    fn severity(&self) -> Severity {
        Severity::Low
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        // Find enums named CodingKeys with raw string values (potential maintenance burden)
        let enums = find_descendants(root, ctx.source, &|node, src| {
            if node.kind() != "class_declaration" {
                return false;
            }
            let text = node_text(node, src);
            text.contains("enum CodingKeys") && text.contains("CodingKey")
        });

        for e in enums {
            let text = node_text(e, ctx.source);
            let case_count = text.matches("case ").count();
            if case_count > 10 {
                issues.push(AuditIssue {
                    id: format!("{}:{}", self.id(), ctx.file_path),
                    category: self.category(),
                    severity: self.severity(),
                    rule: self.id().to_string(),
                    message: format!(
                        "CodingKeys enum has {case_count} cases — consider using keyDecodingStrategy.convertFromSnakeCase"
                    ),
                    file: ctx.file_path.to_string(),
                    line: e.start_position().row as u32 + 1,
                    symbol: None,
                    fix: Some("Use keyDecodingStrategy if keys follow a consistent pattern".into()),
                });
            }
        }

        issues
    }
}

/// COD-005: Encoding/decoding without key-not-found handling.
pub struct MissingKeyHandling;

impl AuditRule for MissingKeyHandling {
    fn id(&self) -> &str {
        "COD-005"
    }
    fn name(&self) -> &str {
        "missing-key-handling"
    }
    fn category(&self) -> Category {
        Category::Codable
    }
    fn severity(&self) -> Severity {
        Severity::Medium
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        // Find custom init(from decoder:) with container.decode but no decodeIfPresent
        let inits = find_descendants(root, ctx.source, &|node, src| {
            if node.kind() != "function_declaration" {
                return false;
            }
            let text = node_text(node, src);
            text.contains("init(from decoder") || text.contains("init(from: Decoder")
        });

        for init in inits {
            let text = node_text(init, ctx.source);
            let decode_count = text.matches(".decode(").count();
            let decode_if_present = text.matches(".decodeIfPresent(").count();

            // If all fields use .decode() and none use .decodeIfPresent(), flag it
            if decode_count > 3 && decode_if_present == 0 {
                issues.push(AuditIssue {
                    id: format!("{}:{}", self.id(), ctx.file_path),
                    category: self.category(),
                    severity: self.severity(),
                    rule: self.id().to_string(),
                    message: format!(
                        "Custom decoder uses {decode_count} .decode() calls but no .decodeIfPresent() — fragile to API changes"
                    ),
                    file: ctx.file_path.to_string(),
                    line: init.start_position().row as u32 + 1,
                    symbol: None,
                    fix: Some("Use decodeIfPresent for optional fields to handle missing keys gracefully".into()),
                });
            }
        }

        issues
    }
}

/// All codable rules.
pub fn all_rules() -> Vec<Box<dyn AuditRule>> {
    vec![
        Box::new(ManualJsonBuilding),
        Box::new(TryOptionalDecoding),
        Box::new(DateDecodingStrategy),
        Box::new(ManualCodingKeys),
        Box::new(MissingKeyHandling),
    ]
}
