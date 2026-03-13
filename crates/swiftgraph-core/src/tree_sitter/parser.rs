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

/// Tree-sitter based Swift parser for extracting declarations, call sites, and type references.
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
        let root = tree.root_node();
        self.visit_node(root, source, &file_path, None, &mut result);

        // Second pass: extract call edges and type references from function bodies
        self.extract_calls(&tree, source, &file_path, &mut result);

        Ok(result)
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

                // Extract extension target
                if symbol_kind == SymbolKind::Extension {
                    extract_extension_target(&node, source, &id, file_path, result);
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

    /// Second pass: find all call_expression nodes and create Calls edges.
    fn extract_calls(&self, tree: &Tree, source: &str, file_path: &str, result: &mut ParseResult) {
        let root = tree.root_node();
        // Build a map: line range → containing function node ID
        let func_ranges: Vec<(u32, u32, String)> = result
            .nodes
            .iter()
            .filter(|n| {
                matches!(
                    n.kind,
                    SymbolKind::Function | SymbolKind::Method | SymbolKind::Property
                )
            })
            .filter_map(|n| Some((n.location.line, n.location.end_line?, n.id.clone())))
            .collect();

        self.visit_calls(root, source, file_path, &func_ranges, result);
    }

    fn visit_calls(
        &self,
        node: Node,
        source: &str,
        file_path: &str,
        func_ranges: &[(u32, u32, String)],
        result: &mut ParseResult,
    ) {
        if node.kind() == "call_expression" {
            let callee_name = extract_call_target(&node, source);
            if let Some(name) = callee_name {
                // Skip trivial calls (operators, very short names)
                if name.len() >= 2 && !name.starts_with('_') {
                    let call_line = node.start_position().row as u32 + 1;

                    // Find the containing function
                    let caller_id = func_ranges
                        .iter()
                        .find(|(start, end, _)| call_line >= *start && call_line <= *end)
                        .map(|(_, _, id)| id.clone());

                    // Create a Calls edge: caller → callee (by name, resolved later)
                    let source_id =
                        caller_id.unwrap_or_else(|| format!("ts::{file_path}::__top_level__::0"));
                    let target_id = format!("name::{name}");

                    result.edges.push(GraphEdge {
                        source: source_id,
                        target: target_id,
                        kind: EdgeKind::Calls,
                        location: Some(Location {
                            file: file_path.to_string(),
                            line: call_line,
                            column: node.start_position().column as u32 + 1,
                            end_line: None,
                            end_column: None,
                        }),
                        is_implicit: false,
                    });
                }
            }
        }

        // Recurse
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                self.visit_calls(child, source, file_path, func_ranges, result);
            }
        }
    }
}

/// Extract the callee name from a call_expression node.
/// Returns the function/method name (last identifier in the chain).
fn extract_call_target(node: &Node, source: &str) -> Option<String> {
    // call_expression has:
    // - simple_identifier (direct call: fetchItems())
    // - navigation_expression (member call: service.performRequest())
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            match child.kind() {
                "simple_identifier" => {
                    return child.utf8_text(source.as_bytes()).ok().map(String::from);
                }
                "navigation_expression" => {
                    // Get the last identifier in the chain (the method name)
                    return extract_nav_call_name(&child, source);
                }
                _ => {}
            }
        }
    }
    None
}

/// Extract the method name from a navigation_expression.
/// e.g., `service.performRequest` → "performRequest"
/// e.g., `NetworkManager.shared.fetch` → "fetch"
/// e.g., `self.process` → "process"
fn extract_nav_call_name(node: &Node, source: &str) -> Option<String> {
    // navigation_expression contains navigation_suffix children
    // The last navigation_suffix has the actual method name
    let mut last_name: Option<String> = None;

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "navigation_suffix" {
                // navigation_suffix > "." > simple_identifier
                for j in 0..child.child_count() {
                    if let Some(inner) = child.child(j) {
                        if inner.kind() == "simple_identifier" {
                            last_name = inner.utf8_text(source.as_bytes()).ok().map(String::from);
                        }
                    }
                }
            }
        }
    }

    last_name
}

/// Result of parsing a single file.
#[derive(Debug)]
pub struct ParseResult {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

/// Map a tree-sitter-swift node to a SymbolKind.
fn map_node_kind(node: &Node, source: &str) -> Option<SymbolKind> {
    match node.kind() {
        "class_declaration" => {
            let keyword = node
                .child(0)
                .and_then(|c| c.utf8_text(source.as_bytes()).ok());
            match keyword {
                Some("struct") => Some(SymbolKind::Struct),
                Some("actor") => Some(SymbolKind::Class),
                Some("extension") => Some(SymbolKind::Extension),
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
            // For extensions: `class_declaration > user_type > type_identifier`
            if kind == "user_type" {
                return find_type_name(&child, source);
            }
        }
    }
    None
}

fn extract_signature(node: &Node, source: &str) -> Option<String> {
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
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "inheritance_specifier" {
                if let Some(name) = find_type_name(&child, source) {
                    let target_id = format!("synthetic::{}", name.trim().replace(['<', '>'], ""));
                    result.edges.push(GraphEdge {
                        source: type_id.to_string(),
                        target: target_id,
                        kind: EdgeKind::ConformsTo,
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

/// Extract extension target: `extension MyType` → ExtendsType edge.
fn extract_extension_target(
    node: &Node,
    source: &str,
    ext_id: &str,
    file_path: &str,
    result: &mut ParseResult,
) {
    // extension_declaration > type_identifier
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "type_identifier" || child.kind() == "user_type" {
                if let Some(name) = find_type_name(&child, source) {
                    let target_id = format!("synthetic::{}", name.trim().replace(['<', '>'], ""));
                    result.edges.push(GraphEdge {
                        source: ext_id.to_string(),
                        target: target_id,
                        kind: EdgeKind::ExtendsType,
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
                break;
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

    #[test]
    fn extract_call_edges() {
        let mut parser = TreeSitterParser::new().unwrap();
        let source = r#"
class MyService {
    func loadData() {
        fetchItems()
        let x = helper.process()
        self.update()
    }
    func fetchItems() {}
    func update() {}
}
"#;
        let result = parser
            .parse_source(source, &PathBuf::from("test.swift"))
            .unwrap();

        let call_edges: Vec<_> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Calls)
            .collect();

        assert!(
            call_edges.len() >= 3,
            "Should find at least 3 call edges, found {}",
            call_edges.len()
        );

        let targets: Vec<&str> = call_edges.iter().map(|e| e.target.as_str()).collect();
        assert!(
            targets.contains(&"name::fetchItems"),
            "Should find fetchItems call"
        );
        assert!(
            targets.contains(&"name::process"),
            "Should find process call"
        );
        assert!(targets.contains(&"name::update"), "Should find update call");
    }

    #[test]
    fn extract_extension_edges() {
        let mut parser = TreeSitterParser::new().unwrap();
        let source = r#"
extension String {
    func trimmed() -> String { self.trimmingCharacters(in: .whitespaces) }
}
"#;
        let result = parser
            .parse_source(source, &PathBuf::from("test.swift"))
            .unwrap();

        let ext_edges: Vec<_> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::ExtendsType)
            .collect();

        assert_eq!(ext_edges.len(), 1, "Should find extension edge");
        assert_eq!(ext_edges[0].target, "synthetic::String");
    }
}
