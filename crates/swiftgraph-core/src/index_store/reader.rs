//! High-level Index Store reader.
//!
//! Reads compilation units and records from an Xcode Index Store,
//! converting them into `GraphNode` and `GraphEdge` values.

use std::collections::HashMap;
use std::path::Path;

use thiserror::Error;
use tracing::{debug, trace};

use super::ffi::{
    symbol_property, symbol_role, FfiError, IndexStore, IndexStoreLib, Occurrence, SymbolKind,
    SymbolLanguage, UnitDependencyKind,
};
use crate::graph::{
    AccessLevel, EdgeKind, GraphEdge, GraphNode, Location, NodeMetrics, SymbolKind as GSymbolKind,
};

/// Errors while reading an Index Store.
#[derive(Debug, Error)]
pub enum ReaderError {
    /// Failure in the libIndexStore layer.
    #[error("FFI error: {0}")]
    Ffi(#[from] FfiError),
    /// Store-level error message.
    #[error("IndexStore error: {0}")]
    Store(String),
}

/// Result of reading an entire Index Store.
#[derive(Debug, Default)]
pub struct IndexStoreData {
    /// Symbol definitions/declarations.
    pub nodes: Vec<GraphNode>,
    /// Relations between symbols.
    pub edges: Vec<GraphEdge>,
    /// Map from file path → list of node IDs in that file.
    pub file_nodes: HashMap<String, Vec<String>>,
    /// Number of non-system Swift units read.
    pub units_read: usize,
    /// Number of records read.
    pub records_read: usize,
    /// Accessor USR → USR of its property (accessors are folded).
    aliases: HashMap<String, String>,
    /// Child USR → parent USR (`childOf` relations).
    containers: HashMap<String, String>,
}

/// Open an Index Store and read all Swift units/records into graph data.
pub fn read_index_store(
    lib: &IndexStoreLib,
    store_path: &Path,
) -> Result<IndexStoreData, ReaderError> {
    let store = IndexStore::open(lib, store_path)?;
    let mut data = IndexStoreData::default();

    let unit_names = store.unit_names();
    debug!("Found {} units in Index Store", unit_names.len());

    for unit_name in &unit_names {
        match read_unit(&store, unit_name, &mut data) {
            Ok(true) => data.units_read += 1,
            Ok(false) => {}
            Err(e) => trace!("Skipping unit {unit_name}: {e}"),
        }
    }

    finalize(&mut data);

    debug!(
        "IndexStore read complete: {} nodes, {} edges from {} units, {} records",
        data.nodes.len(),
        data.edges.len(),
        data.units_read,
        data.records_read
    );

    Ok(data)
}

/// Read a single unit and its records. Returns `false` for skipped units
/// (system units, non-Swift units).
fn read_unit(
    store: &IndexStore<'_>,
    unit_name: &str,
    data: &mut IndexStoreData,
) -> Result<bool, ReaderError> {
    let unit = store.unit_reader(unit_name)?;
    if unit.is_system_unit() {
        return Ok(false);
    }

    let main_file = unit.main_file();
    if !main_file.ends_with(".swift") {
        return Ok(false);
    }

    let record_names: Vec<String> = unit
        .dependencies()
        .into_iter()
        .filter(|d| d.kind == UnitDependencyKind::Record as u32)
        .map(|d| d.name)
        .collect();
    drop(unit);

    for record_name in record_names {
        match read_record(store, &record_name, &main_file, data) {
            Ok(()) => data.records_read += 1,
            Err(e) => trace!("Skipping record {record_name}: {e}"),
        }
    }

    Ok(true)
}

/// Read a record and extract symbols + relations.
fn read_record(
    store: &IndexStore<'_>,
    record_name: &str,
    file_path: &str,
    data: &mut IndexStoreData,
) -> Result<(), ReaderError> {
    let occurrences = store.record_reader(record_name)?.occurrences();
    let mut seen: HashMap<String, usize> = HashMap::new(); // USR → index in data.nodes

    for occ in &occurrences {
        process_occurrence(occ, file_path, data, &mut seen);
    }
    Ok(())
}

/// Name prefixes of Swift accessor symbols (`getter:value`, `setter:value`...).
const ACCESSOR_PREFIXES: &[&str] = &[
    "getter:",
    "setter:",
    "_modify:",
    "_read:",
    "modify:",
    "read:",
    "willSet:",
    "didSet:",
    "init:",
    "unsafeAddress:",
    "unsafeMutableAddress:",
];

fn is_accessor(occ: &Occurrence) -> bool {
    occ.relations
        .iter()
        .any(|r| r.roles & symbol_role::REL_ACCESSOROF != 0)
        || ACCESSOR_PREFIXES
            .iter()
            .any(|p| occ.symbol.name.starts_with(p))
}

fn process_occurrence(
    occ: &Occurrence,
    file_path: &str,
    data: &mut IndexStoreData,
    seen: &mut HashMap<String, usize>,
) {
    let usr = &occ.symbol.usr;
    if usr.is_empty() || occ.symbol.language != SymbolLanguage::Swift as u32 {
        return;
    }
    let declares = occ.roles & (symbol_role::DEFINITION | symbol_role::DECLARATION) != 0;

    // Accessors are part of their property: no node, relations are
    // attributed to the property.
    if is_accessor(occ) {
        if let Some(property) = occ
            .relations
            .iter()
            .find(|r| r.roles & symbol_role::REL_ACCESSOROF != 0)
        {
            data.aliases
                .insert(usr.clone(), property.symbol.usr.clone());
        }
    } else if declares && !seen.contains_key(usr) {
        // Definitions/declarations become nodes
        let node = GraphNode {
            id: usr.clone(),
            name: occ.symbol.name.clone(),
            qualified_name: occ.symbol.name.clone(),
            kind: map_symbol_kind(occ.symbol.kind),
            sub_kind: (occ.symbol.kind == SymbolKind::Constructor as u32)
                .then_some(crate::graph::SymbolSubKind::Initializer),
            location: Location {
                file: file_path.to_owned(),
                line: occ.line,
                column: occ.column,
                end_line: None,
                end_column: None,
            },
            signature: None,
            attributes: Vec::new(),
            access_level: map_access_level(occ.symbol.properties),
            container_usr: None,
            doc_comment: None,
            metrics: Some(NodeMetrics::default()),
        };
        seen.insert(usr.clone(), data.nodes.len());
        data.nodes.push(node);
        data.file_nodes
            .entry(file_path.to_owned())
            .or_default()
            .push(usr.clone());
    }

    // Relations (calledBy, baseOf, childOf, ...). In the Index Store they are
    // inverted: "calledBy X" on an occurrence of S means X calls S.
    let location = Some(Location {
        file: file_path.to_owned(),
        line: occ.line,
        column: occ.column,
        end_line: None,
        end_column: None,
    });
    let implicit = occ.roles & symbol_role::IMPLICIT != 0;
    let mut called = false;
    for rel in &occ.relations {
        let rel_usr = &rel.symbol.usr;
        if rel_usr.is_empty() {
            continue;
        }
        let mut push = |source: &str, target: &str, kind: EdgeKind| {
            data.edges.push(GraphEdge {
                source: source.to_owned(),
                target: target.to_owned(),
                kind,
                location: location.clone(),
                is_implicit: implicit,
                ambiguous: false,
            });
        };
        if rel.roles & symbol_role::REL_CALLEDBY != 0 {
            push(rel_usr, usr, EdgeKind::Calls);
            called = true;
        }
        if rel.roles & symbol_role::REL_BASEOF != 0 {
            // "S baseOf R": the occurrence symbol S is the base, R the subtype.
            let kind = if occ.symbol.kind == SymbolKind::Protocol as u32 {
                EdgeKind::ConformsTo
            } else {
                EdgeKind::InheritsFrom
            };
            push(rel_usr, usr, kind);
        }
        if rel.roles & symbol_role::REL_OVERRIDEOF != 0 {
            // Also protocol witnesses: the implementation "overrides" the requirement
            push(usr, rel_usr, EdgeKind::Overrides);
        }
        if rel.roles & symbol_role::REL_CHILDOF != 0 && declares {
            // "S childOf P": P contains S
            push(rel_usr, usr, EdgeKind::Contains);
            data.containers
                .entry(usr.clone())
                .or_insert_with(|| rel_usr.clone());
        }
        if rel.roles & symbol_role::REL_EXTENDEDBY != 0 {
            push(rel_usr, usr, EdgeKind::ExtendsType);
        }
    }

    // Uses that are not calls: type annotations, property reads/writes,
    // enum cases, initializer references
    if !declares && !called && occ.roles & symbol_role::REFERENCE != 0 {
        if let Some(container) = occ
            .relations
            .iter()
            .find(|r| r.roles & symbol_role::REL_CONTAINEDBY != 0)
        {
            data.edges.push(GraphEdge {
                source: container.symbol.usr.clone(),
                target: usr.clone(),
                kind: EdgeKind::References,
                location,
                is_implicit: implicit,
                ambiguous: false,
            });
        }
    }
}

/// Fold accessors into properties, set containers and qualified names,
/// drop duplicate and self edges.
fn finalize(data: &mut IndexStoreData) {
    let alias = |usr: &str| -> String {
        data.aliases
            .get(usr)
            .cloned()
            .unwrap_or_else(|| usr.to_owned())
    };
    let mut seen = std::collections::HashSet::new();
    let edges = std::mem::take(&mut data.edges);
    for mut edge in edges {
        edge.source = alias(&edge.source);
        edge.target = alias(&edge.target);
        if edge.source == edge.target && edge.kind != EdgeKind::Overrides {
            continue;
        }
        let key = (
            edge.source.clone(),
            edge.target.clone(),
            edge.kind,
            edge.location.as_ref().map(|l| (l.line, l.column)),
        );
        if seen.insert(key) {
            data.edges.push(edge);
        }
    }

    let names: HashMap<String, String> = data
        .nodes
        .iter()
        .map(|n| (n.id.clone(), n.name.clone()))
        .collect();
    let containers = &data.containers;
    let qualified = |usr: &str| -> String {
        let mut parts = vec![names.get(usr).cloned().unwrap_or_default()];
        let mut current = usr;
        let mut depth = 0;
        while let Some(parent) = containers.get(current) {
            depth += 1;
            if depth > 16 {
                break;
            }
            match names.get(parent) {
                Some(name) => parts.push(name.clone()),
                None => break,
            }
            current = parent;
        }
        parts.reverse();
        parts.join(".")
    };
    let updates: Vec<(Option<String>, String)> = data
        .nodes
        .iter()
        .map(|n| (containers.get(&n.id).cloned(), qualified(&n.id)))
        .collect();
    for (node, (container, qualified)) in data.nodes.iter_mut().zip(updates) {
        node.container_usr = container;
        node.qualified_name = qualified;
    }
}

/// Map IndexStore symbol kind to our SymbolKind.
fn map_symbol_kind(raw: u32) -> GSymbolKind {
    match raw {
        x if x == SymbolKind::Enum as u32 => GSymbolKind::Enum,
        x if x == SymbolKind::Struct as u32 => GSymbolKind::Struct,
        x if x == SymbolKind::Class as u32 => GSymbolKind::Class,
        x if x == SymbolKind::Protocol as u32 => GSymbolKind::Protocol,
        x if x == SymbolKind::Extension as u32 => GSymbolKind::Extension,
        x if x == SymbolKind::TypeAlias as u32 => GSymbolKind::TypeAlias,
        x if x == SymbolKind::Function as u32 => GSymbolKind::Function,
        x if x == SymbolKind::Variable as u32 => GSymbolKind::Property,
        x if x == SymbolKind::Field as u32 => GSymbolKind::Property,
        x if x == SymbolKind::EnumConstant as u32 => GSymbolKind::EnumCase,
        x if x == SymbolKind::InstanceMethod as u32 => GSymbolKind::Function,
        x if x == SymbolKind::ClassMethod as u32 => GSymbolKind::Function,
        x if x == SymbolKind::StaticMethod as u32 => GSymbolKind::Function,
        x if x == SymbolKind::InstanceProperty as u32 => GSymbolKind::Property,
        x if x == SymbolKind::ClassProperty as u32 => GSymbolKind::Property,
        x if x == SymbolKind::StaticProperty as u32 => GSymbolKind::Property,
        x if x == SymbolKind::Constructor as u32 => GSymbolKind::Function,
        x if x == SymbolKind::Destructor as u32 => GSymbolKind::Function,
        x if x == SymbolKind::Module as u32 => GSymbolKind::Module,
        _ => GSymbolKind::Function, // fallback
    }
}

/// Map IndexStore symbol properties bitfield to AccessLevel.
///
/// Swift access control is a multi-bit field (bits 17-19), not independent
/// flags: e.g. PUBLIC = FILEPRIVATE | PACKAGE bits. Compare the whole field.
/// Stores written without access control info yield `Internal`.
fn map_access_level(properties: u64) -> AccessLevel {
    use symbol_property::*;
    const MASK: u64 = SWIFT_AC_LESS_THAN_FILEPRIVATE | SWIFT_AC_FILEPRIVATE | SWIFT_AC_PACKAGE;
    match properties & MASK {
        SWIFT_AC_PUBLIC => AccessLevel::Public,
        SWIFT_AC_PACKAGE => AccessLevel::Package,
        SWIFT_AC_INTERNAL => AccessLevel::Internal,
        SWIFT_AC_FILEPRIVATE => AccessLevel::FilePrivate,
        SWIFT_AC_LESS_THAN_FILEPRIVATE => AccessLevel::Private,
        _ => AccessLevel::Internal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_control_is_an_exact_field_not_overlapping_bits() {
        use symbol_property::*;
        let async_flag = SWIFT_ASYNC;
        assert_eq!(map_access_level(SWIFT_AC_PUBLIC), AccessLevel::Public);
        assert_eq!(map_access_level(SWIFT_AC_PACKAGE), AccessLevel::Package);
        assert_eq!(map_access_level(SWIFT_AC_INTERNAL), AccessLevel::Internal);
        assert_eq!(
            map_access_level(SWIFT_AC_FILEPRIVATE),
            AccessLevel::FilePrivate
        );
        assert_eq!(
            map_access_level(SWIFT_AC_LESS_THAN_FILEPRIVATE),
            AccessLevel::Private
        );
        assert_eq!(
            map_access_level(SWIFT_AC_FILEPRIVATE | async_flag),
            AccessLevel::FilePrivate
        );
        assert_eq!(map_access_level(0), AccessLevel::Internal);
    }
}
