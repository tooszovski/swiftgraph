use std::path::Path;

use thiserror::Error;
use tree_sitter::{Node, Parser};

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
    /// Create a parser for Swift sources.
    pub fn new() -> Result<Self, ParseError> {
        let mut parser = Parser::new();
        let language = tree_sitter_swift::LANGUAGE;
        parser
            .set_language(&language.into())
            .map_err(|_| ParseError::Language)?;
        Ok(Self { parser })
    }

    /// Parse a Swift file and extract nodes, edges and call sites.
    pub fn parse_file(&mut self, path: &Path) -> Result<ParseResult, ParseError> {
        let source = std::fs::read_to_string(path)?;
        self.parse_source(&source, path)
    }

    /// Parse Swift source code and extract nodes, edges and call sites.
    pub fn parse_source(&mut self, source: &str, path: &Path) -> Result<ParseResult, ParseError> {
        let tree = self
            .parser
            .parse(source, None)
            .ok_or_else(|| ParseError::Parse(path.display().to_string()))?;

        let mut result = ParseResult {
            nodes: Vec::new(),
            edges: Vec::new(),
            calls: Vec::new(),
            references: Vec::new(),
        };

        let file_path = path.to_string_lossy().to_string();
        let root = tree.root_node();
        visit_node(root, source, &file_path, None, &mut result);

        // Second pass: call sites with what this file tells about their
        // receivers, and names referenced outside of calls.
        (result.calls, result.references) =
            CallCollector::new(source, &file_path, root).collect(root);

        Ok(result)
    }
}

/// The declaration enclosing the node being visited.
#[derive(Clone, Copy)]
struct Container<'a> {
    id: &'a str,
    /// Qualified name when the container is a type or an extension.
    type_path: Option<&'a str>,
}

fn visit_node(
    node: Node,
    source: &str,
    file_path: &str,
    container: Option<Container>,
    result: &mut ParseResult,
) {
    // Local variables (inside a function, accessor or closure) are not
    // declarations of the program's structure: no node, no FTS row.
    let local_variable =
        node.kind() == "property_declaration" && container.is_some_and(|c| c.type_path.is_none());
    if let Some(symbol_kind) = map_node_kind(&node, source).filter(|_| !local_variable) {
        if let Some(name) = extract_name(&node, source) {
            let id = make_synthetic_id(file_path, &name, node.start_position().row);
            let is_type = matches!(
                symbol_kind,
                SymbolKind::Class
                    | SymbolKind::Struct
                    | SymbolKind::Enum
                    | SymbolKind::Protocol
                    | SymbolKind::Extension
            );
            // Extensions are top-level; their qualified name is the extended type.
            let mut qualified = match container.and_then(|c| c.type_path) {
                Some(prefix) if symbol_kind != SymbolKind::Extension => format!("{prefix}.{name}"),
                _ => name.clone(),
            };
            if symbol_kind == SymbolKind::Function {
                qualified.push_str(&parameter_labels(&node, source));
            }

            let graph_node = GraphNode {
                id: id.clone(),
                name: name.clone(),
                qualified_name: qualified.clone(),
                kind: symbol_kind,
                sub_kind: (node.kind() == "init_declaration")
                    .then_some(crate::graph::SymbolSubKind::Initializer),
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
                container_usr: container.map(|c| c.id.to_string()),
                doc_comment: None,
                metrics: None,
            };

            // Add containment edge
            if let Some(parent) = container {
                result.edges.push(GraphEdge {
                    source: parent.id.to_string(),
                    target: id.clone(),
                    kind: EdgeKind::Contains,
                    location: None,
                    is_implicit: true,
                    ambiguous: false,
                });
            }

            result.nodes.push(graph_node);

            // Inheritance/conformance of types, protocols and extensions
            if is_type {
                extract_inheritance(&node, source, &id, file_path, result);
            }

            // Extract extension target
            if symbol_kind == SymbolKind::Extension {
                extract_extension_target(&node, source, &id, file_path, result);
            }

            // Recurse into children with this as container
            let child = Container {
                id: &id,
                type_path: is_type.then_some(qualified.as_str()),
            };
            for i in 0..node.child_count() {
                if let Some(c) = node.child(i) {
                    visit_node(c, source, file_path, Some(child), result);
                }
            }
            return;
        }
    }

    // Recurse into children
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            visit_node(child, source, file_path, container, result);
        }
    }
}

/// Argument labels of a function declaration in Swift notation: `(id:_:)`.
fn parameter_labels(node: &Node, source: &str) -> String {
    let mut out = String::from("(");
    let mut cursor = node.walk();
    for param in node
        .children(&mut cursor)
        .filter(|c| c.kind() == "parameter")
    {
        let label = param
            .child_by_field_name("external_name")
            .or_else(|| first_child_of_kind(&param, "simple_identifier"))
            .and_then(|n| n.utf8_text(source.as_bytes()).ok())
            .unwrap_or("_");
        out.push_str(label);
        out.push(':');
    }
    out.push(')');
    out
}

/// What a call site's receiver is known to be, as far as one file tells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Receiver {
    /// `foo()`: a member of an enclosing type, a free function or an initializer.
    Implicit,
    /// A receiver of a known type: `self.foo()`, `Type.foo()`, `Type().foo()`,
    /// `x.foo()` with `x` declared as `let x: Type` or `let x = Type(...)`.
    Typed(String),
    /// `super.foo()` inside the named type.
    Super(String),
    /// Any other expression; the type is unknown.
    Unknown,
}

/// A call found by tree-sitter. Targets are resolved after indexing, when
/// every file's declarations are in the database.
#[derive(Debug, Clone)]
pub struct CallSite {
    /// ID of the calling declaration (function or property), or the file's
    /// `__top_level__` pseudo-node.
    pub caller: String,
    /// Called name (the last component of a member chain).
    pub name: String,
    /// What is known about the receiver.
    pub receiver: Receiver,
    /// Enclosing type names, innermost first.
    pub scope: Vec<String>,
    /// Labels of the parenthesized arguments (`None` = unlabeled).
    pub labels: Vec<Option<String>>,
    /// Number of trailing closures.
    pub trailing_closures: usize,
    /// Where the call is.
    pub location: Location,
}

/// Local scope: variable name -> declared type name, if known.
type Frame = std::collections::HashMap<String, Option<String>>;

/// Type of an expression as far as the collector can infer it.
enum ExprType {
    Named(String),
    Super,
}

/// Walks declarations and function bodies and records call sites.
struct CallCollector<'s> {
    source: &'s str,
    file: &'s str,
    /// Properties declared in this file, per type name: property -> declared type.
    members: std::collections::HashMap<String, Frame>,
    /// Enclosing type names, outermost first.
    types: Vec<String>,
    /// Declaration the calls are attributed to.
    caller: Option<String>,
    /// Local scopes, innermost last.
    locals: Vec<Frame>,
    calls: Vec<CallSite>,
    /// Identifiers referenced outside of call position.
    references: std::collections::BTreeSet<String>,
    /// Callee identifiers already recorded as call sites.
    callee_ids: std::collections::HashSet<usize>,
}

impl<'s> CallCollector<'s> {
    fn new(source: &'s str, file: &'s str, root: Node) -> Self {
        let mut members = std::collections::HashMap::new();
        collect_members(root, source, &mut members);
        Self {
            source,
            file,
            members,
            types: Vec::new(),
            caller: None,
            locals: Vec::new(),
            calls: Vec::new(),
            references: std::collections::BTreeSet::new(),
            callee_ids: std::collections::HashSet::new(),
        }
    }

    fn collect(mut self, root: Node) -> (Vec<CallSite>, Vec<String>) {
        self.walk(root);
        (self.calls, self.references.into_iter().collect())
    }

    fn text(&self, node: Node) -> &'s str {
        node.utf8_text(self.source.as_bytes()).unwrap_or("")
    }

    fn walk_children(&mut self, node: Node) {
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();
        for child in children {
            self.walk(child);
        }
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "class_declaration" | "protocol_declaration" => {
                let name = extract_name(&node, self.source);
                let caller = self.caller.take();
                let locals = std::mem::take(&mut self.locals);
                if let Some(name) = &name {
                    self.types.push(name.clone());
                }
                self.walk_children(node);
                if name.is_some() {
                    self.types.pop();
                }
                self.caller = caller;
                self.locals = locals;
            }
            "function_declaration"
            | "init_declaration"
            | "deinit_declaration"
            | "subscript_declaration" => {
                let caller = self.caller.clone();
                if self.caller.is_none()
                    && matches!(node.kind(), "function_declaration" | "init_declaration")
                {
                    self.caller = extract_name(&node, self.source)
                        .map(|name| make_synthetic_id(self.file, &name, node.start_position().row));
                }
                self.locals.push(parameter_frame(&node, self.source));
                self.walk_children(node);
                self.locals.pop();
                self.caller = caller;
            }
            "property_declaration" if self.caller.is_none() => {
                // A property of a type or a global: calls in its initializer
                // or accessors belong to it.
                self.caller = extract_name(&node, self.source)
                    .map(|name| make_synthetic_id(self.file, &name, node.start_position().row));
                self.walk_children(node);
                self.caller = None;
            }
            "property_declaration" => {
                let bindings = property_bindings(&node, self.source);
                if self.locals.is_empty() {
                    self.locals.push(Frame::new());
                }
                if let Some(frame) = self.locals.last_mut() {
                    frame.extend(bindings);
                }
                self.walk_children(node);
            }
            "lambda_literal" => {
                self.locals.push(lambda_frame(&node, self.source));
                self.walk_children(node);
                self.locals.pop();
            }
            "call_expression" => {
                self.record(node);
                self.walk_children(node);
            }
            "simple_identifier" | "type_identifier" => {
                if !self.callee_ids.contains(&node.id()) && !is_declared_name(&node) {
                    // `$name` reads the projected value of property `name`.
                    let name = self.text(node);
                    self.references
                        .insert(name.strip_prefix('$').unwrap_or(name).to_string());
                }
            }
            _ => self.walk_children(node),
        }
    }

    /// `Some(type)` if `name` is a local variable or parameter in scope.
    fn local(&self, name: &str) -> Option<Option<String>> {
        self.locals
            .iter()
            .rev()
            .find_map(|frame| frame.get(name).cloned())
    }

    fn expr_type(&self, expr: Node) -> Option<ExprType> {
        match expr.kind() {
            "self_expression" => self.types.last().cloned().map(ExprType::Named),
            "super_expression" => self.types.last().map(|_| ExprType::Super),
            "simple_identifier" => {
                let name = self.text(expr);
                if let Some(local) = self.local(name) {
                    return local.map(ExprType::Named);
                }
                if let Some(ty) = self
                    .types
                    .last()
                    .and_then(|t| self.members.get(t))
                    .and_then(|m| m.get(name))
                {
                    return ty.clone().map(ExprType::Named);
                }
                starts_uppercase(name).then(|| ExprType::Named(name.to_string()))
            }
            "navigation_expression" => {
                let ExprType::Named(base) = self.expr_type(expr.child_by_field_name("target")?)?
                else {
                    return None;
                };
                let property = nav_suffix(&expr).map(|n| self.text(n))?;
                self.members
                    .get(&base)?
                    .get(property)?
                    .clone()
                    .map(ExprType::Named)
            }
            "postfix_expression" => self.expr_type(expr.child_by_field_name("target")?),
            "call_expression" => {
                let ty = constructor_type(&expr, self.source)?;
                self.local(&ty).is_none().then_some(ExprType::Named(ty))
            }
            _ => None,
        }
    }

    fn record(&mut self, node: Node) {
        let Some(callee) = node.named_child(0) else {
            return;
        };
        let (name, receiver) = match callee.kind() {
            "simple_identifier" => {
                let name = self.text(callee);
                // Calling a local closure or a parameter, not a declaration.
                if self.local(name).is_some() {
                    return;
                }
                self.callee_ids.insert(callee.id());
                (name, Receiver::Implicit)
            }
            "navigation_expression" => {
                let Some(suffix) = nav_suffix(&callee) else {
                    return;
                };
                self.callee_ids.insert(suffix.id());
                let name = self.text(suffix);
                let receiver = match callee
                    .child_by_field_name("target")
                    .and_then(|t| self.expr_type(t))
                {
                    Some(ExprType::Named(ty)) => Receiver::Typed(ty),
                    Some(ExprType::Super) => self
                        .types
                        .last()
                        .map_or(Receiver::Unknown, |t| Receiver::Super(t.clone())),
                    None => Receiver::Unknown,
                };
                (name, receiver)
            }
            _ => return,
        };
        // Skip trivial calls (operators, very short names); `.init(...)`
        // counts only with a known receiver (`self.init`, `super.init`).
        if name.len() < 2
            || name.starts_with('_')
            || (name == "init" && matches!(receiver, Receiver::Implicit | Receiver::Unknown))
        {
            return;
        }

        let mut labels = Vec::new();
        let mut trailing_closures = 0;
        let mut cursor = node.walk();
        for suffix in node
            .children(&mut cursor)
            .filter(|c| c.kind() == "call_suffix")
        {
            let mut inner = suffix.walk();
            for part in suffix.children(&mut inner) {
                match part.kind() {
                    "value_arguments" => {
                        let mut args = part.walk();
                        for arg in part
                            .children(&mut args)
                            .filter(|a| a.kind() == "value_argument")
                        {
                            labels.push(
                                first_child_of_kind(&arg, "value_argument_label")
                                    .map(|l| self.text(l).to_string()),
                            );
                        }
                    }
                    "lambda_literal" => trailing_closures += 1,
                    _ => {}
                }
            }
        }

        let caller = self
            .caller
            .clone()
            .unwrap_or_else(|| format!("ts::{}::__top_level__::0", self.file));
        self.calls.push(CallSite {
            caller,
            name: name.to_string(),
            receiver,
            scope: self.types.iter().rev().cloned().collect(),
            labels,
            trailing_closures,
            location: Location {
                file: self.file.to_string(),
                line: node.start_position().row as u32 + 1,
                column: node.start_position().column as u32 + 1,
                end_line: None,
                end_column: None,
            },
        });
    }
}

/// Whether an identifier is the name being declared (not a reference).
fn is_declared_name(node: &Node) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    match parent.kind() {
        "class_declaration"
        | "protocol_declaration"
        | "function_declaration"
        | "protocol_function_declaration"
        | "typealias_declaration"
        | "associatedtype_declaration"
        | "enum_entry"
        | "parameter"
        | "lambda_parameter"
        | "value_argument_label" => true,
        // `let name` / `var name` in property declarations
        "pattern" => parent.parent().is_some_and(|g| {
            matches!(
                g.kind(),
                "property_declaration" | "protocol_property_declaration"
            )
        }),
        _ => false,
    }
}

/// Record declared property types of every type (and extension) in the file.
fn collect_members(
    node: Node,
    source: &str,
    members: &mut std::collections::HashMap<String, Frame>,
) {
    if matches!(node.kind(), "class_declaration" | "protocol_declaration") {
        if let (Some(name), Some(body)) = (
            extract_name(&node, source),
            node.child_by_field_name("body"),
        ) {
            let mut cursor = body.walk();
            for decl in body.children(&mut cursor).filter(|c| {
                matches!(
                    c.kind(),
                    "property_declaration" | "protocol_property_declaration"
                )
            }) {
                members
                    .entry(name.clone())
                    .or_default()
                    .extend(property_bindings(&decl, source));
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_members(child, source, members);
    }
}

/// Names bound by a property declaration with their declared type, from a
/// type annotation or an initializer call (`let x = Type(...)`).
fn property_bindings(decl: &Node, source: &str) -> Vec<(String, Option<String>)> {
    let mut out: Vec<(String, Option<String>)> = Vec::new();
    let mut cursor = decl.walk();
    for child in decl.children(&mut cursor) {
        match child.kind() {
            "pattern" => {
                // Tuple patterns bind several names of unknown types.
                if let Some(id) = first_child_of_kind(&child, "simple_identifier") {
                    if let Ok(name) = id.utf8_text(source.as_bytes()) {
                        out.push((name.to_string(), None));
                    }
                }
            }
            "type_annotation" => {
                if let Some(last) = out.last_mut() {
                    last.1 = child.named_child(0).and_then(|t| type_name(&t, source));
                }
            }
            "call_expression" => {
                if let Some(last) = out.last_mut() {
                    if last.1.is_none() {
                        last.1 = constructor_type(&child, source);
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Parameters of a function, initializer or subscript with their types.
fn parameter_frame(node: &Node, source: &str) -> Frame {
    let mut frame = Frame::new();
    let mut cursor = node.walk();
    for param in node
        .children(&mut cursor)
        .filter(|c| c.kind() == "parameter")
    {
        let mut name = None;
        let mut ty = None;
        let mut after_colon = false;
        let mut inner = param.walk();
        for part in param.children(&mut inner) {
            if !part.is_named() {
                after_colon |= part.kind() == ":";
                continue;
            }
            if after_colon {
                if ty.is_none() && part.kind() != "parameter_modifiers" {
                    ty = Some(type_name(&part, source));
                }
            } else if part.kind() == "simple_identifier" {
                // The last identifier before `:` is the internal name.
                name = part.utf8_text(source.as_bytes()).ok();
            }
        }
        if let Some(name) = name {
            frame.insert(name.to_string(), ty.flatten());
        }
    }
    frame
}

/// Closure parameters (`{ item in ... }`, `{ (item: T) in ... }`).
fn lambda_frame(node: &Node, source: &str) -> Frame {
    let mut frame = Frame::new();
    let mut stack = vec![*node];
    while let Some(n) = stack.pop() {
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            match child.kind() {
                "lambda_function_type" | "lambda_function_type_parameters" => stack.push(child),
                "lambda_parameter" => {
                    let mut inner = child.walk();
                    let named: Vec<Node> = child.named_children(&mut inner).collect();
                    if let Some(name) = named
                        .iter()
                        .find(|c| c.kind() == "simple_identifier")
                        .and_then(|c| c.utf8_text(source.as_bytes()).ok())
                    {
                        let ty = named
                            .iter()
                            .rev()
                            .find(|c| c.kind() != "simple_identifier")
                            .and_then(|t| type_name(t, source));
                        frame.insert(name.to_string(), ty);
                    }
                }
                _ => {}
            }
        }
    }
    frame
}

/// Nominal type named by a type node; `None` for arrays, functions, tuples.
fn type_name(node: &Node, source: &str) -> Option<String> {
    match node.kind() {
        "type_identifier" => node.utf8_text(source.as_bytes()).ok().map(String::from),
        "user_type" => {
            let mut cursor = node.walk();
            let last = node
                .children(&mut cursor)
                .filter(|c| c.kind() == "type_identifier")
                .last();
            last.and_then(|t| t.utf8_text(source.as_bytes()).ok())
                .map(String::from)
        }
        "optional_type" | "opaque_type" | "existential_type" | "implicitly_unwrapped_type" => node
            .child_by_field_name("wrapped")
            .or_else(|| node.named_child(0))
            .and_then(|t| type_name(&t, source)),
        _ => None,
    }
}

/// `Type(...)` → `Type` (capitalized callee without a receiver).
fn constructor_type(call: &Node, source: &str) -> Option<String> {
    let callee = call.named_child(0)?;
    if callee.kind() != "simple_identifier" {
        return None;
    }
    let name = callee.utf8_text(source.as_bytes()).ok()?;
    starts_uppercase(name).then(|| name.to_string())
}

/// The member name of a navigation expression (`a.b.name` → `name`).
fn nav_suffix<'t>(nav: &Node<'t>) -> Option<Node<'t>> {
    let suffix = nav.child_by_field_name("suffix").or_else(|| {
        let mut cursor = nav.walk();
        let last = nav
            .children(&mut cursor)
            .filter(|c| c.kind() == "navigation_suffix")
            .last();
        last
    })?;
    suffix
        .child_by_field_name("suffix")
        .or_else(|| first_child_of_kind(&suffix, "simple_identifier"))
        .filter(|n| n.kind() == "simple_identifier")
}

fn first_child_of_kind<'t>(node: &Node<'t>, kind: &str) -> Option<Node<'t>> {
    let mut cursor = node.walk();
    let found = node.children(&mut cursor).find(|c| c.kind() == kind);
    found
}

fn starts_uppercase(name: &str) -> bool {
    name.chars().next().is_some_and(char::is_uppercase)
}

/// Result of parsing a single file.
#[derive(Debug)]
pub struct ParseResult {
    /// Declarations.
    pub nodes: Vec<GraphNode>,
    /// Containment, inheritance and extension edges.
    pub edges: Vec<GraphEdge>,
    /// Call sites, resolved to call edges by the indexing pipeline.
    pub calls: Vec<CallSite>,
    /// Distinct identifiers referenced outside of call position (member
    /// reads, type annotations, arguments), sorted.
    pub references: Vec<String>,
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
        "function_declaration" | "protocol_function_declaration" | "init_declaration" => {
            Some(SymbolKind::Function)
        }
        "property_declaration" | "protocol_property_declaration" => Some(SymbolKind::Property),
        "typealias_declaration" => Some(SymbolKind::TypeAlias),
        "extension_declaration" => Some(SymbolKind::Extension),
        "enum_entry" => Some(SymbolKind::EnumCase),
        "import_declaration" => Some(SymbolKind::Import),
        "associatedtype_declaration" => Some(SymbolKind::AssociatedType),
        _ => None,
    }
}

fn extract_name(node: &Node, source: &str) -> Option<String> {
    if node.kind() == "init_declaration" {
        return Some("init".to_string());
    }
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
            // For extensions: `class_declaration > user_type > type_identifier`;
            // `extension Outer.Inner` extends `Inner`.
            if kind == "user_type" {
                return type_name(&child, source).or_else(|| find_type_name(&child, source));
            }
            // For properties: `property_declaration > pattern > simple_identifier`
            if kind == "pattern" {
                if let Some(id) = (0..child.child_count())
                    .filter_map(|j| child.child(j))
                    .find(|c| c.kind() == "simple_identifier")
                {
                    return Some(id.utf8_text(source.as_bytes()).ok()?.to_string());
                }
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
                        ambiguous: false,
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
                if let Some(name) =
                    type_name(&child, source).or_else(|| find_type_name(&child, source))
                {
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
                        ambiguous: false,
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
    fn property_declarations_are_extracted() {
        let r = parse(
            "struct S {\n\
                 public private(set) var count = 0\n\
                 private static let shared = 1\n\
                 @Published var items: [String] = []\n\
                 let name: String\n\
             }\n",
        );
        let count = find(&r, "count");
        assert_eq!(count.kind, SymbolKind::Property);
        assert_eq!(count.access_level, AccessLevel::Public);
        assert_eq!(find(&r, "shared").access_level, AccessLevel::Private);
        assert!(find(&r, "items")
            .attributes
            .iter()
            .any(|a| a == "@Published"));
        assert_eq!(find(&r, "name").kind, SymbolKind::Property);
    }

    #[test]
    fn local_variables_are_not_declarations() {
        let r = parse(
            "let globalValue = 1\n\
             struct S {\n\
                 let member = 2\n\
                 var body: Int { let inBody = 3; return inBody }\n\
                 func run() {\n\
                     let local = 4\n\
                     var counter: Int = 0\n\
                     items.forEach { item in let inClosure = item }\n\
                     func nested() {}\n\
                 }\n\
             }\n",
        );
        let names: Vec<&str> = r.nodes.iter().map(|n| n.name.as_str()).collect();
        for kept in ["globalValue", "S", "member", "body", "run", "nested"] {
            assert!(names.contains(&kept), "{kept} missing: {names:?}");
        }
        for local in ["inBody", "local", "counter", "inClosure"] {
            assert!(!names.contains(&local), "{local} is a node: {names:?}");
        }
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
    fn extract_call_sites_with_receivers() {
        let r = parse(
            r#"
class MyService {
    let api: ApiClient
    private var cache = Cache()
    func loadData(items: [Int], store: Store) {
        fetchItems()
        let x = helper.process()
        self.update(id: 1, force: true)
        api.send(request) { _ in }
        cache.clear()
        store.save()
        Logger.info("x")
        items.map { $0 }
        let local: Store = make()
        local.flush()
        let run = { }
        run()
        super.viewDidLoad()
        UIView().layoutIfNeeded()
        items.forEach { item in item.go() }
    }
    func fetchItems() {}
    func update(id: Int, force: Bool) {}
}
"#,
        );
        let site = |name: &str| {
            r.calls
                .iter()
                .find(|c| c.name == name)
                .unwrap_or_else(|| panic!("{name} not recorded: {:?}", r.calls))
        };
        let typed = |t: &str| Receiver::Typed(t.to_string());

        let load_data = find(&r, "loadData");
        assert_eq!(load_data.qualified_name, "MyService.loadData(items:store:)");
        // `Cache()` belongs to the property initializer, the rest to `loadData`.
        assert_eq!(site("Cache").caller, find(&r, "cache").id);
        assert!(r
            .calls
            .iter()
            .filter(|c| c.name != "Cache")
            .all(|c| c.caller == load_data.id));

        assert_eq!(site("fetchItems").receiver, Receiver::Implicit);
        assert_eq!(site("fetchItems").scope, vec!["MyService".to_string()]);
        assert_eq!(site("process").receiver, Receiver::Unknown);
        let update = site("update");
        assert_eq!(update.receiver, typed("MyService"));
        assert_eq!(
            update.labels,
            vec![Some("id".to_string()), Some("force".to_string())]
        );
        let send = site("send");
        assert_eq!(send.receiver, typed("ApiClient"));
        assert_eq!(
            (send.labels.clone(), send.trailing_closures),
            (vec![None], 1)
        );
        assert_eq!(site("clear").receiver, typed("Cache"));
        assert_eq!(site("save").receiver, typed("Store"));
        assert_eq!(site("info").receiver, typed("Logger"));
        assert_eq!(site("map").receiver, Receiver::Unknown);
        assert_eq!(site("flush").receiver, typed("Store"));
        assert_eq!(
            site("viewDidLoad").receiver,
            Receiver::Super("MyService".to_string())
        );
        assert_eq!(site("layoutIfNeeded").receiver, typed("UIView"));
        assert_eq!(site("go").receiver, Receiver::Unknown);
        // Calling a local closure is not a call to a declaration.
        assert!(r.calls.iter().all(|c| c.name != "run"));
    }

    #[test]
    fn protocol_requirements_and_nested_extensions_are_declarations() {
        let r = parse(
            "protocol Service: AnyObject {\n\
                 var name: String { get }\n\
                 func load(id: Int, _ force: Bool)\n\
             }\n\
             extension Outer.Inner: Service {\n\
                 func load(id: Int, _ force: Bool) {}\n\
             }\n",
        );
        assert_eq!(find(&r, "name").qualified_name, "Service.name");
        let requirements: Vec<&str> = r
            .nodes
            .iter()
            .filter(|n| n.name == "load")
            .map(|n| n.qualified_name.as_str())
            .collect();
        assert_eq!(
            requirements,
            vec!["Service.load(id:_:)", "Inner.load(id:_:)"]
        );
        assert_eq!(find(&r, "Inner").kind, SymbolKind::Extension);
        let supertypes: Vec<&str> = r
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::ConformsTo)
            .map(|e| e.target.as_str())
            .collect();
        assert_eq!(
            supertypes,
            vec!["synthetic::AnyObject", "synthetic::Service"]
        );
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
