//! Audit rules organized by category.
//!
//! Each rule operates on Swift source text + tree-sitter AST and returns findings.

pub mod accessibility;
pub mod codable;
pub mod concurrency;
pub mod energy;
pub mod memory;
pub mod modernization;
pub mod networking;
pub mod performance;
pub mod security;
pub mod storage;
pub mod swiftui_arch;
pub mod swiftui_perf;
pub mod testing;

use crate::engine::{AuditIssue, Category, Severity};
use tree_sitter::{Node, Parser, Tree};

/// Context passed to each rule when checking a file.
pub struct FileContext<'a> {
    pub file_path: &'a str,
    pub source: &'a str,
    pub tree: &'a Tree,
}

/// Trait for audit rules.
pub trait AuditRule: Send + Sync {
    /// Unique rule ID (e.g. "CONC-001").
    fn id(&self) -> &str;
    /// Human-readable rule name.
    fn name(&self) -> &str;
    /// Rule category.
    fn category(&self) -> Category;
    /// Default severity.
    fn severity(&self) -> Severity;
    /// Check a file and return findings.
    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue>;
}

/// Create a tree-sitter Swift parser.
pub fn swift_parser() -> Result<Parser, tree_sitter::LanguageError> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_swift::LANGUAGE.into())?;
    Ok(parser)
}

/// Helper: find all descendant nodes matching a predicate.
pub fn find_descendants<'a>(
    node: Node<'a>,
    source: &'a str,
    predicate: &dyn Fn(Node<'a>, &str) -> bool,
) -> Vec<Node<'a>> {
    let mut results = Vec::new();
    find_descendants_inner(node, source, predicate, &mut results);
    results
}

fn find_descendants_inner<'a>(
    node: Node<'a>,
    source: &'a str,
    predicate: &dyn Fn(Node<'a>, &str) -> bool,
    results: &mut Vec<Node<'a>>,
) {
    if predicate(node, source) {
        results.push(node);
    }
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            find_descendants_inner(child, source, predicate, results);
        }
    }
}

/// Helper: get the text of a node.
pub fn node_text<'a>(node: Node<'a>, source: &'a str) -> &'a str {
    node.utf8_text(source.as_bytes()).unwrap_or("")
}

/// Helper: check if a declaration has an attribute containing `attr`.
///
/// Looks at direct `attribute` children and inside the `modifiers` node,
/// where tree-sitter-swift puts attributes such as `@MainActor`.
pub fn has_attribute(node: Node, source: &str, attr: &str) -> bool {
    for i in 0..node.child_count() {
        let Some(child) = node.child(i) else { continue };
        match child.kind() {
            "attribute" => {
                if node_text(child, source).contains(attr) {
                    return true;
                }
            }
            "modifiers" => {
                for j in 0..child.child_count() {
                    if let Some(m) = child.child(j) {
                        if m.kind() == "attribute" && node_text(m, source).contains(attr) {
                            return true;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    false
}

/// Helper: get the keyword of a class_declaration (class/struct/enum/actor/extension).
///
/// The keyword follows the optional `modifiers` node, so it is not
/// necessarily the first child.
pub fn class_keyword<'a>(node: Node<'a>, source: &'a str) -> &'a str {
    (0..node.child_count())
        .filter_map(|i| node.child(i))
        .filter(|c| !c.is_named())
        .filter_map(|c| c.utf8_text(source.as_bytes()).ok())
        .find(|t| matches!(*t, "class" | "struct" | "enum" | "actor" | "extension"))
        .unwrap_or("class")
}

/// Helper: get the name of a declaration node.
pub fn decl_name(node: Node, source: &str) -> Option<String> {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            let kind = child.kind();
            if kind == "simple_identifier" || kind == "type_identifier" || kind == "identifier" {
                return child.utf8_text(source.as_bytes()).ok().map(String::from);
            }
        }
    }
    None
}
