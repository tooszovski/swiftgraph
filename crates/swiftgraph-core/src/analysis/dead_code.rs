//! Dead code detection.
//!
//! Finds symbols with no incoming edges (no callers, no references).
//! Excludes: public API, tests, entry points, @main, protocols, extensions.
//!
//! "Possibly used" wins over "dead": ambiguous call edges count as uses, and
//! so does a call site whose receiver could not be resolved but whose name
//! matches the symbol (`name_refs`). Tree-sitter mode cannot prove a
//! symbol unused, so it only reports symbols nothing could be calling.
//!
//! With Index Store data the compiler recorded every reference, call, read
//! and write, so members are checked too: a declaration is dead when nothing
//! but its container points at it, it neither overrides nor witnesses a
//! requirement, carries no runtime attribute (`@objc`, `@IBAction`, `@main`)
//! and is not used implicitly (Codable members, `CodingKeys`, raw-value and
//! CaseIterable enum cases). Members of a dead type are reported through the
//! type.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::Serialize;
use thiserror::Error;

use crate::storage::{self, queries};

#[derive(Debug, Error)]
pub enum DeadCodeError {
    #[error("storage error: {0}")]
    Storage(#[from] crate::storage::StorageError),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

/// A potentially dead symbol.
#[derive(Debug, Serialize)]
pub struct DeadSymbol {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub file: String,
    pub line: u32,
    pub access_level: String,
    /// Why it's considered dead.
    pub reason: String,
}

/// Dead code analysis result.
#[derive(Debug, Serialize)]
pub struct DeadCodeResult {
    /// Dead symbols, at most `limit`.
    pub dead_symbols: Vec<DeadSymbol>,
    /// Symbols matching the path filter.
    pub total_symbols_checked: usize,
    /// All dead symbols found (may exceed `dead_symbols.len()`).
    pub dead_count: usize,
    pub dead_percentage: f64,
    /// `dead_symbols` was cut to `limit`.
    pub truncated: bool,
}

/// Find dead code: symbols with no incoming edges.
pub fn find_dead_code(
    db_path: &Path,
    path_filter: Option<&str>,
    include_tests: bool,
    limit: u32,
) -> Result<DeadCodeResult, DeadCodeError> {
    let conn = storage::open_db(db_path)?;
    find_dead_code_from_conn(&conn, path_filter, include_tests, limit)
}

/// Find dead code from an existing connection.
pub fn find_dead_code_from_conn(
    conn: &rusqlite::Connection,
    path_filter: Option<&str>,
    include_tests: bool,
    limit: u32,
) -> Result<DeadCodeResult, DeadCodeError> {
    let nodes = queries::get_nodes_by_path_prefix(conn, path_filter.unwrap_or(""), u32::MAX)?;
    let total_checked = nodes.len();
    let facts = GraphFacts::load(conn)?;
    let mut dead: Vec<DeadSymbol> = Vec::new();

    // Types first: members of a dead type are reported through the type.
    let is_type = |kind: &str| matches!(kind, "class" | "struct" | "enum" | "typeAlias");
    let mut ordered: Vec<&crate::graph::GraphNode> = nodes.iter().collect();
    ordered.sort_by_key(|n| !is_type(n.kind.as_str()));
    let mut dead_ids: HashSet<String> = HashSet::new();

    for node in ordered {
        // Skip excluded kinds
        let kind = node.kind.as_str();
        if matches!(
            kind,
            "protocol" | "extension" | "import" | "associatedType" | "module"
        ) {
            continue;
        }

        // Skip test files unless requested
        if !include_tests && super::is_test_or_manifest(&node.location.file) {
            continue;
        }

        // Skip public/open API (may be used externally)
        let access = format!("{:?}", node.access_level);
        if matches!(
            node.access_level,
            crate::graph::AccessLevel::Public | crate::graph::AccessLevel::Open
        ) {
            continue;
        }

        // Skip entry points
        if node.name == "body"
            || node.name == "main"
            || node.name == "deinit"
            || node.name.starts_with("application(")
            || node.name.starts_with("scene(")
        {
            continue;
        }

        // Compiler (Index Store) symbols have USRs; everything else is
        // tree-sitter data judged by names.
        let compiler = node.id.starts_with("s:") || node.id.starts_with("c:");
        // Compiler symbols with no source declaration (synthesized members,
        // macro expansions) cannot be removed by hand
        if compiler && node.location.end_line.is_none() {
            continue;
        }
        let used = if compiler {
            facts.used_by_compiler_data(node)
        } else {
            facts.possibly_used_by_name(node)
        };
        if used {
            continue;
        }
        if facts.has_dead_ancestor(node, &dead_ids) {
            continue;
        }

        dead_ids.insert(node.id.clone());
        dead.push(DeadSymbol {
            id: node.id.clone(),
            name: node.name.clone(),
            kind: kind.to_string(),
            file: node.location.file.clone(),
            line: node.location.line,
            access_level: access.to_string(),
            reason: "No incoming edges (no callers or references)".into(),
        });
    }

    dead.sort_by(|a, b| (&a.file, a.line, &a.id).cmp(&(&b.file, b.line, &b.id)));
    let dead_count = dead.len();
    let truncated = dead_count > limit as usize;
    dead.truncate(limit as usize);
    let dead_percentage = if total_checked > 0 {
        (dead_count as f64 / total_checked as f64) * 100.0
    } else {
        0.0
    };

    Ok(DeadCodeResult {
        dead_symbols: dead,
        total_symbols_checked: total_checked,
        dead_count,
        dead_percentage,
        truncated,
    })
}

/// Standard-library conformances that make members used implicitly:
/// Decodable, Encodable (synthesized coding), RawRepresentable, CaseIterable,
/// and `String`/`Int` raw types of enums.
const IMPLICIT_USE_CONFORMANCES: &[&str] = &[
    "s:Se",
    "s:SE",
    "s:SY",
    "s:s12CaseIterableP",
    "s:SS",
    "s:Si",
    "synthetic::Codable",
    "synthetic::Decodable",
    "synthetic::Encodable",
    "synthetic::CaseIterable",
    "synthetic::String",
    "synthetic::Int",
];

/// Attributes that expose a declaration to the runtime or the system.
const RUNTIME_ATTRIBUTES: &[&str] = &[
    "@objc",
    "@IBAction",
    "@IBOutlet",
    "@IBInspectable",
    "@main",
    "@UIApplicationMain",
    "@NSApplicationMain",
    "@NSManaged",
    "@UIApplicationDelegateAdaptor",
    "@NSApplicationDelegateAdaptor",
    "@WKApplicationDelegateAdaptor",
    "@_cdecl",
    "@_dynamicReplacement",
];

/// Edge and name data dead-code decisions need, loaded once.
struct GraphFacts {
    /// Incoming edges of any kind (tree-sitter semantics).
    incoming: HashMap<String, u32>,
    /// Incoming edges other than containment (compiler data).
    uses: HashMap<String, u32>,
    /// Outgoing edges of any kind.
    outgoing: HashMap<String, u32>,
    /// Declarations that override or witness another one.
    overriders: HashSet<String>,
    /// Type id → conformance/inheritance targets.
    conformances: HashMap<String, Vec<String>>,
    /// Node id → (name, container id).
    names: HashMap<String, (String, Option<String>)>,
    /// Names referenced without a confident edge (tree-sitter mode).
    unresolved: HashSet<String>,
}

impl GraphFacts {
    fn load(conn: &rusqlite::Connection) -> rusqlite::Result<Self> {
        let counts = |sql: &str| -> rusqlite::Result<HashMap<String, u32>> {
            let mut stmt = conn.prepare(sql)?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?)))?;
            rows.collect()
        };
        // Ambiguous edges are included on purpose: a possible caller keeps a
        // symbol alive.
        let incoming = counts("SELECT target, COUNT(*) FROM edges GROUP BY target")?;
        let outgoing = counts("SELECT source, COUNT(*) FROM edges GROUP BY source")?;
        let uses = counts(
            "SELECT target, COUNT(*) FROM edges
             WHERE kind != 'contains' AND source != target GROUP BY target",
        )?;
        let overriders = {
            let mut stmt =
                conn.prepare("SELECT DISTINCT source FROM edges WHERE kind = 'overrides'")?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            rows.collect::<Result<_, _>>()?
        };
        let mut conformances: HashMap<String, Vec<String>> = HashMap::new();
        {
            let mut stmt = conn.prepare(
                "SELECT source, target FROM edges WHERE kind IN ('conformsTo', 'inheritsFrom')",
            )?;
            let rows =
                stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            for row in rows {
                let (source, target) = row?;
                conformances.entry(source).or_default().push(target);
            }
        }
        let mut names = HashMap::new();
        {
            let mut stmt = conn.prepare("SELECT id, name, container_usr FROM nodes")?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                ))
            })?;
            for row in rows {
                let (id, name, container) = row?;
                names.insert(id, (name, container));
            }
        }
        let unresolved = {
            let mut stmt = conn.prepare("SELECT DISTINCT name FROM name_refs")?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            rows.collect::<Result<_, _>>()?
        };
        Ok(Self {
            incoming,
            uses,
            outgoing,
            overriders,
            conformances,
            names,
            unresolved,
        })
    }

    /// Tree-sitter nodes: any incoming edge (containment included, since
    /// member uses are not recorded) or a name seen at an unresolved use.
    fn possibly_used_by_name(&self, node: &crate::graph::GraphNode) -> bool {
        let base_name = node.name.split('(').next().unwrap_or(&node.name);
        if self.unresolved.contains(base_name) {
            return true;
        }
        if self.incoming.get(&node.id).copied().unwrap_or(0) > 0 {
            return true;
        }
        // Containers (types with members) are structural, not dead
        let is_container = matches!(node.kind.as_str(), "class" | "struct" | "enum");
        is_container && self.outgoing.get(&node.id).copied().unwrap_or(0) > 0
    }

    /// Index Store nodes: real references, calls and reads count; so do
    /// overriding or witnessing a requirement, runtime attributes and
    /// members used implicitly by Codable, raw values or CaseIterable.
    fn used_by_compiler_data(&self, node: &crate::graph::GraphNode) -> bool {
        if self.uses.get(&node.id).copied().unwrap_or(0) > 0 || self.overriders.contains(&node.id) {
            return true;
        }
        if node
            .attributes
            .iter()
            .any(|a| RUNTIME_ATTRIBUTES.iter().any(|r| a.starts_with(r)))
        {
            return true;
        }
        let container = node.container_usr.as_deref();
        let container_name = container
            .and_then(|c| self.names.get(c))
            .map(|(n, _)| n.as_str());
        if node.name == "CodingKeys" || container_name == Some("CodingKeys") {
            return true;
        }
        // Stored properties of Codable types and cases of raw-value,
        // Codable or CaseIterable enums are used by synthesized code
        if matches!(node.kind.as_str(), "property" | "enumCase") {
            if let Some(targets) = container.and_then(|c| self.conformances.get(c)) {
                if targets
                    .iter()
                    .any(|t| IMPLICIT_USE_CONFORMANCES.contains(&t.as_str()))
                {
                    return true;
                }
            }
        }
        false
    }

    /// Whether a containing type was already reported dead.
    fn has_dead_ancestor(&self, node: &crate::graph::GraphNode, dead: &HashSet<String>) -> bool {
        let mut current = node.container_usr.clone();
        let mut depth = 0;
        while let Some(id) = current {
            if dead.contains(&id) {
                return true;
            }
            depth += 1;
            if depth > 16 {
                break;
            }
            current = self.names.get(&id).and_then(|(_, c)| c.clone());
        }
        false
    }
}
