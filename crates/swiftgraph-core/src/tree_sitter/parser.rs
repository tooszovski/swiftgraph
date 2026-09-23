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
            // The keyword follows `modifiers` (attributes, access level), so
            // it is not necessarily the first child.
            match declaration_keyword(node, source) {
                Some("struct") => Some(SymbolKind::Struct),
                Some("enum") => Some(SymbolKind::Enum),
                Some("extension") => Some(SymbolKind::Extension),
                _ => Some(SymbolKind::Class), // class, actor
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
    const MAX_SIGNATURE_BYTES: usize = 200;
    if trimmed.len() > MAX_SIGNATURE_BYTES {
        // Cut on a char boundary: slicing mid-codepoint panics on non-ASCII text.
        let cut = (0..=MAX_SIGNATURE_BYTES)
            .rev()
            .find(|&i| trimmed.is_char_boundary(i))
            .unwrap_or(0);
        Some(format!("{}...", &trimmed[..cut]))
    } else {
        Some(trimmed.to_string())
    }
}

/// The declaration keyword of a `class_declaration`-like node
/// (`class`, `struct`, `enum`, `actor`, `extension`).
fn declaration_keyword<'a>(node: &Node, source: &'a str) -> Option<&'a str> {
    (0..node.child_count())
        .filter_map(|i| node.child(i))
        .filter(|c| !c.is_named())
        .filter_map(|c| c.utf8_text(source.as_bytes()).ok())
        .find(|t| matches!(*t, "class" | "struct" | "enum" | "actor" | "extension"))
}

/// Children of the declaration's `modifiers` node (attributes, visibility,
/// member modifiers), plus legacy direct `attribute`/`modifier` children.
fn modifier_nodes<'t>(node: &Node<'t>) -> Vec<Node<'t>> {
    let mut out = Vec::new();
    for i in 0..node.child_count() {
        let Some(child) = node.child(i) else { continue };
        match child.kind() {
            "modifiers" => out.extend((0..child.child_count()).filter_map(|j| child.child(j))),
            "attribute" | "modifier" | "visibility_modifier" => out.push(child),
            _ => {}
        }
    }
    out
}

fn extract_attributes(node: &Node, source: &str) -> Vec<String> {
    let mut attrs = Vec::new();
    // Attributes parsed as preceding siblings (older grammar shapes)
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
    for m in modifier_nodes(node) {
        if m.kind() == "attribute" {
            if let Ok(text) = m.utf8_text(source.as_bytes()) {
                attrs.push(text.to_string());
            }
        }
    }
    attrs.dedup();
    attrs
}

fn extract_access_level(node: &Node, source: &str) -> AccessLevel {
    for m in modifier_nodes(node) {
        if !matches!(m.kind(), "visibility_modifier" | "modifier") {
            continue;
        }
        let Ok(text) = m.utf8_text(source.as_bytes()) else {
            continue;
        };
        // `private(set)` restricts only the setter; the declaration's access
        // level comes from the plain modifier.
        if text.contains("(set)") {
            continue;
        }
        match text.trim() {
            "open" => return AccessLevel::Open,
            "public" => return AccessLevel::Public,
            "package" => return AccessLevel::Package,
            "internal" => return AccessLevel::Internal,
            "fileprivate" => return AccessLevel::FilePrivate,
            "private" => return AccessLevel::Private,
            _ => {}
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

    fn parse(source: &str) -> ParseResult {
        TreeSitterParser::new()
            .unwrap()
            .parse_source(source, &PathBuf::from("test.swift"))
            .unwrap()
    }

    fn find<'a>(result: &'a ParseResult, name: &str) -> &'a GraphNode {
        result
            .nodes
            .iter()
            .find(|n| n.name == name)
            .unwrap_or_else(|| panic!("{name} not found"))
    }

    #[test]
    fn modifiers_and_attributes_on_type_declarations() {
        let r = parse(
            "@MainActor public struct Foo: View {}\n\
             @MainActor final class VM {}\n\
             public final class Store {}\n\
             fileprivate enum Hidden {}\n\
             package struct Pkg {}\n\
             @available(iOS 17, *) @MainActor open class Base {}\n",
        );
        let foo = find(&r, "Foo");
        assert_eq!(foo.kind, SymbolKind::Struct);
        assert_eq!(foo.access_level, AccessLevel::Public);
        assert!(
            foo.attributes.iter().any(|a| a == "@MainActor"),
            "{:?}",
            foo.attributes
        );

        let vm = find(&r, "VM");
        assert_eq!(vm.kind, SymbolKind::Class);
        assert!(
            vm.attributes.iter().any(|a| a == "@MainActor"),
            "{:?}",
            vm.attributes
        );

        assert_eq!(find(&r, "Store").access_level, AccessLevel::Public);
        assert_eq!(find(&r, "Hidden").access_level, AccessLevel::FilePrivate);
        assert_eq!(find(&r, "Pkg").access_level, AccessLevel::Package);
        assert_eq!(find(&r, "Pkg").kind, SymbolKind::Struct);
        let base = find(&r, "Base");
        assert_eq!(base.access_level, AccessLevel::Open);
        assert_eq!(base.attributes.len(), 2, "{:?}", base.attributes);
    }

    #[test]
    fn modifiers_on_members() {
        let r = parse(
            "struct S {\n\
                 public static func make() -> S { S() }\n\
                 public private(set) var count = 0\n\
                 private static let shared = 1\n\
                 @MainActor public func render() {}\n\
                 nonisolated public func id() -> Int { 0 }\n\
             }\n",
        );
        assert_eq!(find(&r, "make").access_level, AccessLevel::Public);
        let render = find(&r, "render");
        assert_eq!(render.access_level, AccessLevel::Public);
        assert!(
            render.attributes.iter().any(|a| a == "@MainActor"),
            "{:?}",
            render.attributes
        );
        assert_eq!(find(&r, "id").access_level, AccessLevel::Public);
    }

    #[test]
    fn long_non_ascii_signature_does_not_panic() {
        // Cyrillic chars are 2 bytes; one of the two paddings puts byte 200 mid-char.
        for pad in ["", "a"] {
            let mut parser = TreeSitterParser::new().unwrap();
            let source = format!("func x{pad}({}: Int) {{}}\n", "я".repeat(150));
            assert!(source.len() > 220);
            let result = parser
                .parse_source(&source, &PathBuf::from("test.swift"))
                .unwrap();
            let sig = result
                .nodes
                .iter()
                .find(|n| n.name == format!("x{pad}"))
                .and_then(|n| n.signature.clone())
                .unwrap();
            assert!(sig.ends_with("..."));
        }
    }

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
