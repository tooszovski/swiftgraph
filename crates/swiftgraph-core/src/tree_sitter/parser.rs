use std::path::Path;

use thiserror::Error;
use tree_sitter::{Node, Parser, Tree};

use crate::graph::{AccessLevel, EdgeKind, GraphEdge, GraphNode, Location, SymbolKind};

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("tree-sitter language error")]
    Language,
    #[error("failed to parse file: {0}")]
    Parse(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Tree-sitter based Swift parser for extracting declarations and basic call sites.
pub struct TreeSitterParser {
    parser: Parser,
}

impl TreeSitterParser {
    pub fn new() -> Result<Self, ParseError> {
        let mut parser = Parser::new();
        let language = tree_sitter_swift::LANGUAGE;
        parser
            .set_language(&language.into())
            .map_err(|_| ParseError::Language)?;
        Ok(Self { parser })
    }

    /// Parse a Swift file and extract nodes and edges.
    pub fn parse_file(&mut self, path: &Path) -> Result<ParseResult, ParseError> {
        let source = std::fs::read_to_string(path)?;
        self.parse_source(&source, path)
    }

    /// Parse Swift source code and extract nodes and edges.
    pub fn parse_source(&mut self, source: &str, path: &Path) -> Result<ParseResult, ParseError> {
        let tree = self
            .parser
            .parse(source, None)
            .ok_or_else(|| ParseError::Parse(path.display().to_string()))?;

        let mut result = ParseResult {
            nodes: Vec::new(),
            edges: Vec::new(),
        };

        let file_path = path.to_string_lossy().to_string();
        self.extract_declarations(&tree, source, &file_path, &mut result);

        Ok(result)
    }

    fn extract_declarations(
        &self,
        tree: &Tree,
        source: &str,
        file_path: &str,
        result: &mut ParseResult,
    ) {
        let root = tree.root_node();
        self.visit_node(root, source, file_path, None, result);
    }

    fn visit_node(
        &self,
        node: Node,
        source: &str,
        file_path: &str,
        container_id: Option<&str>,
        result: &mut ParseResult,
    ) {
        // Extract declarations
        if let Some(symbol_kind) = map_node_kind(&node, source) {
            if let Some(name) = extract_name(&node, source) {
                let id = make_synthetic_id(file_path, &name, node.start_position().row);

                let graph_node = GraphNode {
                    id: id.clone(),
                    name: name.clone(),
                    qualified_name: name.clone(),
                    kind: symbol_kind,
                    sub_kind: None,
                    location: Location {
                        file: file_path.to_string(),
                        line: node.start_position().row as u32 + 1,
                        column: node.start_position().column as u32 + 1,
                        end_line: Some(node.end_position().row as u32 + 1),
                        end_column: Some(node.end_position().column as u32 + 1),
                    },
                    signature: extract_signature(&node, source),
                    attributes: extract_attributes(&node, source),
                    access_level: extract_access_level(&node, source),
                    container_usr: container_id.map(String::from),
                    doc_comment: None,
                    metrics: None,
                };

                // Add containment edge
                if let Some(parent_id) = container_id {
                    result.edges.push(GraphEdge {
                        source: parent_id.to_string(),
                        target: id.clone(),
                        kind: EdgeKind::Contains,
                        location: None,
                        is_implicit: true,
                    });
                }

                result.nodes.push(graph_node);

                // Extract inheritance/conformance from type declarations
                if matches!(
                    symbol_kind,
                    SymbolKind::Class | SymbolKind::Struct | SymbolKind::Enum
                ) {
                    extract_inheritance(&node, source, &id, file_path, result);
                }

                // Recurse into children with this as container
                let child_container = id.clone();
                for i in 0..node.child_count() {
                    if let Some(child) = node.child(i) {
                        self.visit_node(child, source, file_path, Some(&child_container), result);
                    }
                }
                return;
            }
        }

        // Recurse into children
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                self.visit_node(child, source, file_path, container_id, result);
            }
        }
    }
}

/// Result of parsing a single file.
#[derive(Debug)]
pub struct ParseResult {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

/// Map a tree-sitter-swift node to a SymbolKind.
///
/// In tree-sitter-swift, `class_declaration` is used for both `class` and `struct`.
/// The actual keyword is the first child node (`struct` or `class`).
fn map_node_kind(node: &Node, source: &str) -> Option<SymbolKind> {
    match node.kind() {
        "class_declaration" => {
            // Distinguish struct vs class vs actor by looking at the keyword child
            let keyword = node
                .child(0)
                .and_then(|c| c.utf8_text(source.as_bytes()).ok());
            match keyword {
                Some("struct") => Some(SymbolKind::Struct),
                Some("actor") => Some(SymbolKind::Class), // treat actor as class for now
                _ => Some(SymbolKind::Class),
            }
        }
        "protocol_declaration" => Some(SymbolKind::Protocol),
        "enum_declaration" => Some(SymbolKind::Enum),
        "function_declaration" => Some(SymbolKind::Function),
        "property_declaration" => Some(SymbolKind::Property),
        "typealias_declaration" => Some(SymbolKind::TypeAlias),
        "extension_declaration" => Some(SymbolKind::Extension),
        "enum_entry" => Some(SymbolKind::EnumCase),
        "import_declaration" => Some(SymbolKind::Import),
        "associatedtype_declaration" => Some(SymbolKind::AssociatedType),
        _ => None,
    }
}

fn extract_name(node: &Node, source: &str) -> Option<String> {
    // Try to find a name child node
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            let kind = child.kind();
            if kind == "simple_identifier"
                || kind == "type_identifier"
                || kind == "identifier"
                || kind == "name"
            {
                return Some(child.utf8_text(source.as_bytes()).ok()?.to_string());
            }
        }
    }
    None
}

fn extract_signature(node: &Node, source: &str) -> Option<String> {
    // For function declarations, capture the first line as signature
    let start = node.start_byte();
    let text = &source[start..];
    let first_line = text.lines().next()?;
    let trimmed = first_line.trim();
    if trimmed.len() > 200 {
        Some(format!("{}...", &trimmed[..200]))
    } else {
        Some(trimmed.to_string())
    }
}

fn extract_attributes(node: &Node, source: &str) -> Vec<String> {
    let mut attrs = Vec::new();
    // Look for attribute nodes before the declaration
    if let Some(parent) = node.parent() {
        for i in 0..parent.child_count() {
            if let Some(sibling) = parent.child(i) {
                if sibling.id() == node.id() {
                    break;
                }
                if sibling.kind() == "attribute" {
                    if let Ok(text) = sibling.utf8_text(source.as_bytes()) {
                        attrs.push(text.to_string());
                    }
                }
            }
        }
    }
    // Also check direct children
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "attribute" {
                if let Ok(text) = child.utf8_text(source.as_bytes()) {
                    attrs.push(text.to_string());
                }
            }
        }
    }
    attrs
}

fn extract_access_level(node: &Node, source: &str) -> AccessLevel {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "modifiers" || child.kind() == "modifier" {
                if let Ok(text) = child.utf8_text(source.as_bytes()) {
                    match text.trim() {
                        "public" => return AccessLevel::Public,
                        "private" => return AccessLevel::Private,
                        "fileprivate" => return AccessLevel::FilePrivate,
                        "open" => return AccessLevel::Open,
                        "internal" => return AccessLevel::Internal,
                        _ => {}
                    }
                }
            }
        }
    }
    AccessLevel::Internal
}

fn extract_inheritance(
    node: &Node,
    source: &str,
    type_id: &str,
    file_path: &str,
    result: &mut ParseResult,
) {
    // In tree-sitter-swift, inheritance is represented as:
    // class_declaration > ":" > inheritance_specifier > user_type > type_identifier
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "inheritance_specifier" {
                // Find the type_identifier inside
                if let Some(name) = find_type_name(&child, source) {
                    let target_id = format!("synthetic::{}", name.trim().replace(['<', '>'], ""));
                    result.edges.push(GraphEdge {
                        source: type_id.to_string(),
                        target: target_id,
                        kind: EdgeKind::ConformsTo, // refined later with Index Store data
                        location: Some(Location {
                            file: file_path.to_string(),
                            line: child.start_position().row as u32 + 1,
                            column: child.start_position().column as u32 + 1,
                            end_line: None,
                            end_column: None,
                        }),
                        is_implicit: false,
                    });
                }
            }
        }
    }
}

fn find_type_name(node: &Node, source: &str) -> Option<String> {
    if node.kind() == "type_identifier" || node.kind() == "simple_identifier" {
        return node.utf8_text(source.as_bytes()).ok().map(String::from);
    }
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if let Some(name) = find_type_name(&child, source) {
                return Some(name);
            }
        }
    }
    None
}

fn make_synthetic_id(file: &str, name: &str, line: usize) -> String {
    format!("ts::{file}::{name}::{line}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn parse_simple_struct() {
        let mut parser = TreeSitterParser::new().unwrap();
        let source = r#"
struct User {
    let name: String
    let age: Int
}
"#;
        let result = parser
            .parse_source(source, &PathBuf::from("test.swift"))
            .unwrap();

        assert!(!result.nodes.is_empty());
        let struct_node = result.nodes.iter().find(|n| n.name == "User");
        assert!(struct_node.is_some(), "Should find User struct");
        assert_eq!(struct_node.unwrap().kind, SymbolKind::Struct);
    }

    #[test]
    fn parse_class_with_inheritance() {
        let mut parser = TreeSitterParser::new().unwrap();
        let source = r#"
class ViewController: UIViewController, UITableViewDelegate {
    func viewDidLoad() {
        super.viewDidLoad()
    }
}
"#;
        let result = parser
            .parse_source(source, &PathBuf::from("test.swift"))
            .unwrap();

        let class_node = result.nodes.iter().find(|n| n.name == "ViewController");
        assert!(class_node.is_some());
        assert_eq!(class_node.unwrap().kind, SymbolKind::Class);

        // Should have conformance/inheritance edges
        let conformance_edges: Vec<_> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::ConformsTo)
            .collect();
        assert!(
            !conformance_edges.is_empty(),
            "Should detect inheritance/conformance"
        );
    }
}
