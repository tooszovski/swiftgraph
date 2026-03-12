//! High-level Index Store reader.
//!
//! Reads compilation units and records from an Xcode Index Store,
//! converting them into `GraphNode` and `GraphEdge` values.

use std::collections::HashMap;
use std::ffi::{c_uint, c_void, CString};
use std::path::Path;
use std::ptr;

use thiserror::Error;
use tracing::{debug, trace};

use super::ffi::{
    self, symbol_property, symbol_role, FfiError, IndexStoreLib, IndexStoreStringRef, SymbolKind,
    SymbolLanguage, UnitDependencyKind,
};
use crate::graph::{
    AccessLevel, EdgeKind, GraphEdge, GraphNode, Location, NodeMetrics, SymbolKind as GSymbolKind,
};

#[derive(Debug, Error)]
pub enum ReaderError {
    #[error("FFI error: {0}")]
    Ffi(#[from] FfiError),
    #[error("IndexStore error: {0}")]
    Store(String),
}

/// Result of reading an entire Index Store.
#[derive(Debug, Default)]
pub struct IndexStoreData {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    /// Map from file path → list of node IDs in that file.
    pub file_nodes: HashMap<String, Vec<String>>,
    pub units_read: usize,
    pub records_read: usize,
}

/// Open an Index Store and read all Swift units/records into graph data.
pub fn read_index_store(
    lib: &IndexStoreLib,
    store_path: &Path,
) -> Result<IndexStoreData, ReaderError> {
    let store_path_c = CString::new(store_path.to_string_lossy().as_ref())
        .map_err(|_| ReaderError::Store("invalid store path".into()))?;

    let mut error: ffi::IndexStoreErrorT = ptr::null_mut();

    // SAFETY: C API call with valid CString.
    let store = unsafe { (lib.store_create)(store_path_c.as_ptr(), &mut error) };
    if store.is_null() {
        let msg = unsafe { lib.get_error_message(error) };
        return Err(ReaderError::Store(msg));
    }

    let mut data = IndexStoreData::default();

    // 1. Enumerate all units
    let unit_names = enumerate_units(lib, store);
    debug!("Found {} units in Index Store", unit_names.len());

    // 2. For each unit, read the record
    for unit_name in &unit_names {
        match read_unit(lib, store, unit_name, &mut data) {
            Ok(()) => data.units_read += 1,
            Err(e) => {
                trace!("Skipping unit {unit_name}: {e}");
            }
        }
    }

    // SAFETY: Disposing a valid store handle.
    unsafe {
        (lib.store_dispose)(store);
    }

    debug!(
        "IndexStore read complete: {} nodes, {} edges from {} units, {} records",
        data.nodes.len(),
        data.edges.len(),
        data.units_read,
        data.records_read
    );

    Ok(data)
}

/// Collect all unit names from the store.
fn enumerate_units(lib: &IndexStoreLib, store: ffi::IndexStoreT) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let ctx = &mut names as *mut Vec<String> as *mut c_void;

    // SAFETY: Callback writes into our Vec through the context pointer.
    unsafe {
        (lib.store_units_apply_f)(store, 0, ctx, unit_applier);
    }

    names
}

unsafe extern "C" fn unit_applier(ctx: *mut c_void, name: IndexStoreStringRef) -> bool {
    // SAFETY: ctx is a valid *mut Vec<String> from enumerate_units.
    let names = &mut *(ctx as *mut Vec<String>);
    let s = name.to_string_owned();
    names.push(s);
    true // continue iteration
}

/// Read a single unit and its associated record.
fn read_unit(
    lib: &IndexStoreLib,
    store: ffi::IndexStoreT,
    unit_name: &str,
    data: &mut IndexStoreData,
) -> Result<(), ReaderError> {
    let unit_name_c =
        CString::new(unit_name).map_err(|_| ReaderError::Store("invalid unit name".into()))?;

    let mut error: ffi::IndexStoreErrorT = ptr::null_mut();

    // SAFETY: C API call with valid store and unit name.
    let unit_reader = unsafe { (lib.unit_reader_create)(store, unit_name_c.as_ptr(), &mut error) };
    if unit_reader.is_null() {
        let msg = unsafe { lib.get_error_message(error) };
        return Err(ReaderError::Store(msg));
    }

    // Skip system units
    // SAFETY: unit_reader is valid.
    let is_system = unsafe { (lib.unit_reader_is_system_unit)(unit_reader) };
    if is_system {
        unsafe { (lib.unit_reader_dispose)(unit_reader) };
        return Ok(());
    }

    // Get main file path
    // SAFETY: unit_reader is valid.
    let main_file = unsafe {
        let sr = (lib.unit_reader_get_main_file)(unit_reader);
        sr.to_string_owned()
    };

    // Only process Swift files
    if !main_file.ends_with(".swift") {
        unsafe { (lib.unit_reader_dispose)(unit_reader) };
        return Ok(());
    }

    // Find record dependencies for this unit
    let record_names = enumerate_record_deps(lib, unit_reader);

    // SAFETY: Done with unit reader.
    unsafe { (lib.unit_reader_dispose)(unit_reader) };

    // Read each record
    for record_name in record_names {
        match read_record(lib, store, &record_name, &main_file, data) {
            Ok(()) => data.records_read += 1,
            Err(e) => {
                trace!("Skipping record {record_name}: {e}");
            }
        }
    }

    Ok(())
}

/// Get the record dependency names from a unit reader.
fn enumerate_record_deps(
    lib: &IndexStoreLib,
    unit_reader: ffi::IndexStoreUnitReaderT,
) -> Vec<String> {
    struct DepCtx<'a> {
        lib: &'a IndexStoreLib,
        records: Vec<String>,
    }

    let mut ctx = DepCtx {
        lib,
        records: Vec::new(),
    };
    let ctx_ptr = &mut ctx as *mut DepCtx as *mut c_void;

    unsafe extern "C" fn dep_applier(
        ctx: *mut c_void,
        dep: ffi::IndexStoreUnitDependencyT,
    ) -> bool {
        // SAFETY: ctx is a valid *mut DepCtx.
        let ctx = &mut *(ctx as *mut DepCtx);
        let kind = (ctx.lib.unit_dependency_get_kind)(dep);
        if kind == UnitDependencyKind::Record as u32 {
            let name = (ctx.lib.unit_dependency_get_name)(dep);
            ctx.records.push(name.to_string_owned());
        }
        true
    }

    // SAFETY: Callback uses our DepCtx through context pointer.
    unsafe {
        (lib.unit_reader_dependencies_apply_f)(unit_reader, ctx_ptr, dep_applier);
    }

    ctx.records
}

/// Read a record and extract symbols + occurrences.
fn read_record(
    lib: &IndexStoreLib,
    store: ffi::IndexStoreT,
    record_name: &str,
    file_path: &str,
    data: &mut IndexStoreData,
) -> Result<(), ReaderError> {
    let record_name_c =
        CString::new(record_name).map_err(|_| ReaderError::Store("invalid record name".into()))?;

    let mut error: ffi::IndexStoreErrorT = ptr::null_mut();

    // SAFETY: C API call with valid store and record name.
    let reader = unsafe { (lib.record_reader_create)(store, record_name_c.as_ptr(), &mut error) };
    if reader.is_null() {
        let msg = unsafe { lib.get_error_message(error) };
        return Err(ReaderError::Store(msg));
    }

    // First pass: collect all symbol definitions → GraphNode
    let mut symbols_by_usr: HashMap<String, usize> = HashMap::new(); // USR → index in data.nodes

    struct SymCtx<'a> {
        lib: &'a IndexStoreLib,
        file_path: &'a str,
        data: &'a mut IndexStoreData,
        symbols_by_usr: &'a mut HashMap<String, usize>,
    }

    let mut sym_ctx = SymCtx {
        lib,
        file_path,
        data,
        symbols_by_usr: &mut symbols_by_usr,
    };

    // Read occurrences (which include both definitions and references)
    let sym_ctx_ptr = &mut sym_ctx as *mut SymCtx as *mut c_void;

    unsafe extern "C" fn occurrence_applier(
        ctx: *mut c_void,
        occurrence: ffi::IndexStoreOccurrenceT,
    ) -> bool {
        // SAFETY: ctx is a valid *mut SymCtx.
        let ctx = &mut *(ctx as *mut SymCtx);
        let lib = ctx.lib;

        let symbol = (lib.occurrence_get_symbol)(occurrence);
        let roles = (lib.occurrence_get_roles)(occurrence);
        let usr = (lib.symbol_get_usr)(symbol).to_string_owned();

        if usr.is_empty() {
            return true;
        }

        let mut line: c_uint = 0;
        let mut col: c_uint = 0;
        (lib.occurrence_get_line_col)(occurrence, &mut line, &mut col);

        let kind_raw = (lib.symbol_get_kind)(symbol);
        let properties = (lib.symbol_get_properties)(symbol);
        let language = (lib.symbol_get_language)(symbol);

        // Only process Swift symbols
        if language != SymbolLanguage::Swift as u32 {
            return true;
        }

        // If this is a definition, create/update a GraphNode
        if roles & symbol_role::DEFINITION != 0 || roles & symbol_role::DECLARATION != 0 {
            let name = (lib.symbol_get_name)(symbol).to_string_owned();
            let graph_kind = map_symbol_kind(kind_raw);
            let access = map_access_level(properties);

            if !ctx.symbols_by_usr.contains_key(&usr) {
                let node = GraphNode {
                    id: usr.clone(),
                    name: name.clone(),
                    qualified_name: name,
                    kind: graph_kind,
                    sub_kind: None,
                    location: Location {
                        file: ctx.file_path.to_owned(),
                        line: line as u32,
                        column: col as u32,
                        end_line: None,
                        end_column: None,
                    },
                    signature: None,
                    attributes: Vec::new(),
                    access_level: access,
                    container_usr: None,
                    doc_comment: None,
                    metrics: Some(NodeMetrics::default()),
                };
                let idx = ctx.data.nodes.len();
                ctx.data.nodes.push(node);
                ctx.symbols_by_usr.insert(usr.clone(), idx);

                ctx.data
                    .file_nodes
                    .entry(ctx.file_path.to_owned())
                    .or_default()
                    .push(usr.clone());
            }
        }

        // Process relations (calledBy, baseOf, childOf, etc.)
        struct RelCtx<'a> {
            lib: &'a IndexStoreLib,
            source_usr: String,
            file_path: String,
            line: u32,
            col: u32,
            edges: Vec<GraphEdge>,
        }

        let mut rel_ctx = RelCtx {
            lib,
            source_usr: usr.clone(),
            file_path: ctx.file_path.to_owned(),
            line: line as u32,
            col: col as u32,
            edges: Vec::new(),
        };
        let rel_ptr = &mut rel_ctx as *mut RelCtx as *mut c_void;

        unsafe extern "C" fn relation_applier(
            ctx: *mut c_void,
            relation: ffi::IndexStoreSymbolRelationT,
        ) -> bool {
            let ctx = &mut *(ctx as *mut RelCtx);
            let lib = ctx.lib;

            let rel_roles = (lib.symbol_relation_get_roles)(relation);
            let rel_symbol = (lib.symbol_relation_get_symbol)(relation);
            let rel_usr = (lib.symbol_get_usr)(rel_symbol).to_string_owned();

            if rel_usr.is_empty() {
                return true;
            }

            let location = Some(Location {
                file: ctx.file_path.clone(),
                line: ctx.line,
                column: ctx.col,
                end_line: None,
                end_column: None,
            });

            // Map relation roles to EdgeKind
            // Note: In Index Store, relations are inverted:
            // - "calledBy" on an occurrence means rel_usr calls source_usr
            // - "baseOf" means source_usr inherits from rel_usr

            if rel_roles & symbol_role::REL_CALLEDBY != 0 {
                ctx.edges.push(GraphEdge {
                    source: rel_usr.clone(),
                    target: ctx.source_usr.clone(),
                    kind: EdgeKind::Calls,
                    location: location.clone(),
                    is_implicit: false,
                });
            }

            if rel_roles & symbol_role::REL_BASEOF != 0 {
                // source_usr conforms to / inherits from rel_usr
                let rel_kind_raw = (lib.symbol_get_kind)(rel_symbol);
                let edge_kind = if rel_kind_raw == SymbolKind::Protocol as u32 {
                    EdgeKind::ConformsTo
                } else {
                    EdgeKind::InheritsFrom
                };
                ctx.edges.push(GraphEdge {
                    source: ctx.source_usr.clone(),
                    target: rel_usr.clone(),
                    kind: edge_kind,
                    location: location.clone(),
                    is_implicit: false,
                });
            }

            if rel_roles & symbol_role::REL_OVERRIDEOF != 0 {
                ctx.edges.push(GraphEdge {
                    source: ctx.source_usr.clone(),
                    target: rel_usr.clone(),
                    kind: EdgeKind::Overrides,
                    location: location.clone(),
                    is_implicit: false,
                });
            }

            if rel_roles & symbol_role::REL_CHILDOF != 0 {
                ctx.edges.push(GraphEdge {
                    source: ctx.source_usr.clone(),
                    target: rel_usr.clone(),
                    kind: EdgeKind::Contains,
                    location: location.clone(),
                    is_implicit: false,
                });
            }

            if rel_roles & symbol_role::REL_EXTENDEDBY != 0 {
                ctx.edges.push(GraphEdge {
                    source: rel_usr.clone(),
                    target: ctx.source_usr.clone(),
                    kind: EdgeKind::ExtendsType,
                    location: location.clone(),
                    is_implicit: false,
                });
            }

            true
        }

        (lib.occurrence_relations_apply_f)(occurrence, rel_ptr, relation_applier);

        ctx.data.edges.extend(rel_ctx.edges);

        // If this is a reference (call), also create a direct edge
        if roles & symbol_role::CALL != 0 {
            // This occurrence calls the symbol identified by `usr`
            // We don't have the caller USR here directly — relations handle that
        }

        if roles & symbol_role::REFERENCE != 0 && roles & symbol_role::CALL == 0 {
            // A plain reference (not a call) — we track these separately
            // The "source" is whoever contains this reference, which requires
            // relation data to determine properly
        }

        true // continue
    }

    // SAFETY: Callback operates through our SymCtx context.
    unsafe {
        (lib.record_reader_occurrences_apply_f)(reader, sym_ctx_ptr, occurrence_applier);
    }

    // SAFETY: Done with record reader.
    unsafe {
        (lib.record_reader_dispose)(reader);
    }

    Ok(())
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
fn map_access_level(properties: u64) -> AccessLevel {
    // Check from most to least restrictive
    if properties & symbol_property::SWIFT_AC_PUBLIC != 0 {
        AccessLevel::Public
    } else if properties & symbol_property::SWIFT_AC_PACKAGE != 0 {
        AccessLevel::Package
    } else if properties & symbol_property::SWIFT_AC_INTERNAL != 0 {
        AccessLevel::Internal
    } else if properties & symbol_property::SWIFT_AC_FILEPRIVATE != 0 {
        AccessLevel::FilePrivate
    } else if properties & symbol_property::SWIFT_AC_LESS_THAN_FILEPRIVATE != 0 {
        AccessLevel::Private
    } else {
        AccessLevel::Internal // default
    }
}
