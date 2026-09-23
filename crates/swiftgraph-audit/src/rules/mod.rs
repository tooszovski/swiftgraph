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
    /// Facts gathered from every file of the project before rules run.
    pub project: &'a ProjectFacts,
}

/// Project-wide facts rules need beyond the current file, such as
/// conformances declared in other files.
#[derive(Debug, Clone, Default)]
pub struct ProjectFacts {
    /// Type, protocol or extended type name -> inherited/conformed names.
    supertypes: std::collections::HashMap<String, Vec<String>>,
    /// Type name -> declaration keyword (`class`, `struct`, `enum`, ...).
    kinds: std::collections::HashMap<String, String>,
}

impl ProjectFacts {
    /// Facts from the given sources (convenience for tests and tools).
    pub fn from_sources(sources: &[&str]) -> Self {
        let mut facts = Self::default();
        for source in sources {
            facts.merge(Self::scan(source));
        }
        facts
    }

    /// Inheritance clauses declared in one source file.
    pub fn scan(source: &str) -> Self {
        static DECL: std::sync::OnceLock<Option<regex::Regex>> = std::sync::OnceLock::new();
        static KIND: std::sync::OnceLock<Option<regex::Regex>> = std::sync::OnceLock::new();
        let mut facts = Self::default();
        if let Some(re) = KIND
            .get_or_init(|| {
                regex::Regex::new(
                    r"\b(class|struct|enum|actor|protocol)\s+([A-Za-z_][A-Za-z0-9_]*)",
                )
                .ok()
            })
            .as_ref()
        {
            for caps in re.captures_iter(source) {
                if let (Some(kind), Some(name)) = (caps.get(1), caps.get(2)) {
                    if !matches!(name.as_str(), "func" | "var" | "let" | "subscript") {
                        facts
                            .kinds
                            .entry(name.as_str().to_string())
                            .or_insert_with(|| kind.as_str().to_string());
                    }
                }
            }
        }
        let Some(re) = DECL
            .get_or_init(|| {
                regex::Regex::new(
                    r"\b(?:protocol|class|actor|struct|enum|extension)\s+([A-Za-z_][A-Za-z0-9_.]*)\s*(?:<[^>{]*>)?\s*:\s*([^{]*)\{",
                )
                .ok()
            })
            .as_ref()
        else {
            return facts;
        };
        for caps in re.captures_iter(source) {
            let (Some(name), Some(list)) = (caps.get(1), caps.get(2)) else {
                continue;
            };
            let name = name.as_str().rsplit('.').next().unwrap_or_default();
            let list = list.as_str().split(" where ").next().unwrap_or_default();
            let parents = list.split([',', '&']).filter_map(|p| {
                let p = p.trim();
                let end = p
                    .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.'))
                    .unwrap_or(p.len());
                let p = p[..end].rsplit('.').next().unwrap_or_default();
                (!p.is_empty()).then(|| p.to_string())
            });
            facts
                .supertypes
                .entry(name.to_string())
                .or_default()
                .extend(parents);
        }
        facts
    }

    /// Add another file's facts.
    pub fn merge(&mut self, other: Self) {
        for (name, kind) in other.kinds {
            self.kinds.entry(name).or_insert(kind);
        }
        for (name, parents) in other.supertypes {
            let entry = self.supertypes.entry(name).or_default();
            for p in parents {
                if !entry.contains(&p) {
                    entry.push(p);
                }
            }
        }
    }

    /// Declaration keyword of a project type, if declared anywhere.
    pub fn kind_of(&self, name: &str) -> Option<&str> {
        self.kinds.get(name).map(String::as_str)
    }

    /// Whether `name` is `target` or inherits it through declarations
    /// anywhere in the project.
    pub fn inherits(&self, name: &str, target: &str) -> bool {
        let mut seen = std::collections::HashSet::new();
        let mut stack = vec![name];
        while let Some(current) = stack.pop() {
            if current == target {
                return true;
            }
            if !seen.insert(current) {
                continue;
            }
            if let Some(parents) = self.supertypes.get(current) {
                stack.extend(parents.iter().map(String::as_str));
            }
        }
        false
    }
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
    /// Minimum iOS major version the suggested fix needs. The rule is skipped
    /// when the project's deployment target is known and lower.
    fn min_ios_major(&self) -> Option<u32> {
        None
    }
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

/// Names in the inheritance clause of a type declaration (`: A, B<C>`).
pub fn inheritance_names(node: Node, source: &str) -> Vec<String> {
    let mut out = Vec::new();
    for i in 0..node.child_count() {
        let Some(child) = node.child(i) else { continue };
        if child.kind() != "inheritance_specifier" {
            continue;
        }
        let text = node_text(child, source).trim();
        let end = text
            .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.'))
            .unwrap_or(text.len());
        if let Some(name) = text[..end].rsplit('.').next().filter(|n| !n.is_empty()) {
            out.push(name.to_string());
        }
    }
    out
}

/// Nearest enclosing type or extension declaration.
pub fn enclosing_type(node: Node) -> Option<Node> {
    let mut current = node.parent();
    while let Some(n) = current {
        if matches!(n.kind(), "class_declaration" | "protocol_declaration") {
            return Some(n);
        }
        current = n.parent();
    }
    None
}

/// Whether `node` sits in a struct or enum (directly or in an extension of
/// a project struct/enum), where `self` is a value.
pub fn in_value_type(node: Node, source: &str, project: &ProjectFacts) -> bool {
    let Some(decl) = enclosing_type(node) else {
        return false;
    };
    match class_keyword(decl, source) {
        "struct" | "enum" => true,
        "extension" => decl_type_name(decl, source)
            .and_then(|name| project.kind_of(&name))
            .is_some_and(|k| matches!(k, "struct" | "enum")),
        _ => false,
    }
}

/// Name of a type declaration; for `extension A.B` it is `B`.
pub fn decl_type_name(node: Node, source: &str) -> Option<String> {
    if let Some(name) = decl_name(node, source) {
        return Some(name);
    }
    (0..node.child_count())
        .filter_map(|i| node.child(i))
        .find(|c| c.kind() == "user_type")
        .map(|t| node_text(t, source))
        .and_then(|t| t.split('<').next())
        .and_then(|t| t.rsplit('.').next())
        .map(|t| t.trim().to_string())
}

/// Called name of a `call_expression` (`a.b.name(...)` → `name`).
pub fn callee_name<'a>(call: Node<'a>, source: &'a str) -> Option<&'a str> {
    let callee = call.named_child(0)?;
    match callee.kind() {
        "simple_identifier" => Some(node_text(callee, source)),
        "navigation_expression" => {
            let suffix = callee.child_by_field_name("suffix")?;
            let name = suffix
                .child_by_field_name("suffix")
                .or_else(|| suffix.named_child(0))?;
            Some(node_text(name, source))
        }
        _ => None,
    }
}

/// 1-based line of a declaration's keyword (`class`, `struct`, ...), which
/// follows attributes such as `@available(...)` on earlier lines.
pub fn declaration_line(node: Node, source: &str) -> u32 {
    (0..node.child_count())
        .filter_map(|i| node.child(i))
        .find(|c| {
            !c.is_named()
                && matches!(
                    node_text(*c, source),
                    "class" | "struct" | "enum" | "actor" | "extension" | "protocol"
                )
        })
        .unwrap_or(node)
        .start_position()
        .row as u32
        + 1
}

/// Lowest iOS deployment target declared under `root`: Xcode build settings
/// (`IPHONEOS_DEPLOYMENT_TARGET` in `*.pbxproj`), SwiftPM `platforms`
/// (`.iOS(.v16)`, `.iOS("16.4")`) and XcodeGen `project.yml` (`iOS: "16.0"`).
/// `None` when nothing declares one.
pub fn ios_deployment_target(root: &std::path::Path) -> Option<(u32, u32)> {
    static PATTERNS: std::sync::OnceLock<Option<[regex::Regex; 3]>> = std::sync::OnceLock::new();
    let [pbx, spm, yml] = PATTERNS
        .get_or_init(|| {
            Some([
                regex::Regex::new(r#"IPHONEOS_DEPLOYMENT_TARGET\s*=\s*"?(\d+)(?:\.(\d+))?"#)
                    .ok()?,
                regex::Regex::new(r#"\.iOS\(\s*(?:\.v(\d+)(?:_(\d+))?|"(\d+)(?:\.(\d+))?")"#)
                    .ok()?,
                regex::Regex::new(r#"\biOS:\s*"?(\d+)(?:\.(\d+))?"#).ok()?,
            ])
        })
        .as_ref()?;
    let mut lowest: Option<(u32, u32)> = None;
    let mut note = |major: Option<&str>, minor: Option<&str>| {
        let Some(major) = major.and_then(|m| m.parse::<u32>().ok()) else {
            return;
        };
        let minor = minor.and_then(|m| m.parse::<u32>().ok()).unwrap_or(0);
        if lowest.is_none_or(|l| (major, minor) < l) {
            lowest = Some((major, minor));
        }
    };
    let walker = walkdir::WalkDir::new(root).into_iter().filter_entry(|e| {
        let name = e.file_name().to_string_lossy();
        !matches!(
            name.as_ref(),
            ".build" | "Pods" | "DerivedData" | "node_modules" | ".git" | "Carthage"
        )
    });
    for entry in walker.filter_map(Result::ok) {
        let name = entry.file_name().to_string_lossy();
        let re = match name.as_ref() {
            "project.pbxproj" => pbx,
            "Package.swift" => spm,
            "project.yml" => yml,
            _ => continue,
        };
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        for caps in re.captures_iter(&text) {
            let get = |i: usize| caps.get(i).map(|m| m.as_str());
            if get(1).is_some() {
                note(get(1), get(2));
            } else {
                note(get(3), get(4));
            }
        }
    }
    lowest
}

/// A modifier call `expr.name(...)`: the call and the `name` identifier.
pub struct ModifierCall<'a> {
    pub call: Node<'a>,
    pub name: Node<'a>,
}

impl ModifierCall<'_> {
    /// 1-based (line, column) of the modifier name.
    pub fn position(&self) -> (u32, u32) {
        let p = self.name.start_position();
        (p.row as u32 + 1, p.column as u32 + 1)
    }
}

/// Every `.name(...)` modifier call under `root`.
pub fn modifier_calls<'a>(root: Node<'a>, source: &'a str, name: &str) -> Vec<ModifierCall<'a>> {
    find_descendants(root, source, &|n, _| n.kind() == "call_expression")
        .into_iter()
        .filter_map(|call| {
            let nav = call.named_child(0)?;
            if nav.kind() != "navigation_expression" {
                return None;
            }
            let suffix = nav.child_by_field_name("suffix")?;
            let ident = suffix
                .child_by_field_name("suffix")
                .or_else(|| suffix.named_child(0))?;
            (node_text(ident, source) == name).then_some(ModifierCall { call, name: ident })
        })
        .collect()
}

/// Walk up a modifier chain from `expr` (`expr.a().b()`): the names of the
/// modifiers applied to it, innermost first, and the outermost call.
pub fn modifier_chain<'a>(expr: Node<'a>, source: &'a str) -> (Vec<&'a str>, Node<'a>) {
    let mut names = Vec::new();
    let mut current = expr;
    while let Some(nav) = current.parent() {
        if nav.kind() != "navigation_expression"
            || nav.child_by_field_name("target").map(|t| t.id()) != Some(current.id())
        {
            break;
        }
        let Some(call) = nav.parent() else { break };
        if call.kind() != "call_expression" || call.named_child(0).map(|c| c.id()) != Some(nav.id())
        {
            break;
        }
        if let Some(name) = callee_name(call, source) {
            names.push(name);
        }
        current = call;
    }
    (names, current)
}

/// Whether `node` is preview code: inside `#Preview { }` or a
/// `PreviewProvider` / `*_Previews` type.
pub fn in_preview(node: Node, source: &str) -> bool {
    let mut current = node.parent();
    while let Some(n) = current {
        match n.kind() {
            "class_declaration" => {
                if decl_name(n, source).is_some_and(|name| name.ends_with("_Previews"))
                    || inheritance_names(n, source)
                        .iter()
                        .any(|i| i == "PreviewProvider")
                {
                    return true;
                }
            }
            _ if node_text(n, source).starts_with("#Preview") => return true,
            _ => {}
        }
        current = n.parent();
    }
    false
}

/// The control (`Button`, `NavigationLink`, `Link`, `Menu`, `Toggle`) whose
/// label closure is `statements`, if any.
pub fn label_owner<'a>(statements: Node<'a>, source: &'a str) -> Option<Node<'a>> {
    let lambda = statements
        .parent()
        .filter(|l| l.kind() == "lambda_literal")?;
    let mut up = lambda.parent()?;
    // `label: { }` → value_argument → value_arguments → call_suffix
    while matches!(up.kind(), "value_argument" | "value_arguments") {
        up = up.parent()?;
    }
    if up.kind() != "call_suffix" {
        return None;
    }
    let call = up.parent().filter(|c| c.kind() == "call_expression")?;
    matches!(
        callee_name(call, source),
        Some("Button" | "NavigationLink" | "Link" | "Menu" | "Toggle")
    )
    .then_some(call)
}
