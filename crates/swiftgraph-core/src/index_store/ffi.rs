//! Raw FFI bindings for libIndexStore.
//!
//! These are hand-written bindings matching the C header from
//! <https://github.com/swiftlang/llvm-project/blob/next/clang/include/indexstore/indexstore.h>
//!
//! We use runtime dynamic linking (`dlopen`/`dlsym`) so the binary can run
//! without Xcode installed (graceful degradation to tree-sitter).
//!
//! # Safety
//! All functions in this module are unsafe C FFI wrappers. The `IndexStoreLib`
//! struct provides a safe(r) loading interface.

use std::ffi::{c_char, c_int, c_uint, c_void, CStr, CString};
use std::path::Path;

use thiserror::Error;

// --- Opaque pointer types ---

pub type IndexStoreT = *mut c_void;
pub type IndexStoreSymbolT = *mut c_void;
pub type IndexStoreOccurrenceT = *mut c_void;
pub type IndexStoreSymbolRelationT = *mut c_void;
pub type IndexStoreRecordReaderT = *mut c_void;
pub type IndexStoreUnitReaderT = *mut c_void;
pub type IndexStoreUnitDependencyT = *mut c_void;
pub type IndexStoreErrorT = *mut c_void;

// --- String ref ---

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct IndexStoreStringRef {
    pub data: *const c_char,
    pub length: usize,
}

impl IndexStoreStringRef {
    /// Convert to a Rust `&str`. Returns empty string if null.
    ///
    /// # Safety
    /// The pointer must be valid for the lifetime of the returned `&str`.
    pub unsafe fn as_str(&self) -> &str {
        if self.data.is_null() || self.length == 0 {
            return "";
        }
        let bytes = std::slice::from_raw_parts(self.data as *const u8, self.length);
        std::str::from_utf8_unchecked(bytes)
    }

    /// Convert to owned String.
    ///
    /// # Safety
    /// The pointer must be valid.
    pub unsafe fn to_string_owned(&self) -> String {
        self.as_str().to_owned()
    }
}

// --- Enums ---

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Unknown = 0,
    Module = 1,
    Namespace = 2,
    NamespaceAlias = 3,
    Macro = 4,
    Enum = 5,
    Struct = 6,
    Class = 7,
    Protocol = 8,
    Extension = 9,
    Union = 10,
    TypeAlias = 11,
    Function = 12,
    Variable = 13,
    Field = 14,
    EnumConstant = 15,
    InstanceMethod = 16,
    ClassMethod = 17,
    StaticMethod = 18,
    InstanceProperty = 19,
    ClassProperty = 20,
    StaticProperty = 21,
    Constructor = 22,
    Destructor = 23,
    ConversionFunction = 24,
    Parameter = 25,
    Using = 26,
    Concept = 27,
    CommentTag = 1000,
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolSubKind {
    None = 0,
    SwiftAccessorWillSet = 1000,
    SwiftAccessorDidSet = 1001,
    SwiftExtensionOfStruct = 1004,
    SwiftExtensionOfClass = 1005,
    SwiftExtensionOfEnum = 1006,
    SwiftExtensionOfProtocol = 1007,
    SwiftSubscript = 1011,
    SwiftAssociatedType = 1012,
    SwiftGenericTypeParam = 1013,
}

/// Symbol roles (bitfield).
#[allow(non_upper_case_globals)]
pub mod symbol_role {
    pub const DECLARATION: u64 = 1 << 0;
    pub const DEFINITION: u64 = 1 << 1;
    pub const REFERENCE: u64 = 1 << 2;
    pub const READ: u64 = 1 << 3;
    pub const WRITE: u64 = 1 << 4;
    pub const CALL: u64 = 1 << 5;
    pub const DYNAMIC: u64 = 1 << 6;
    pub const IMPLICIT: u64 = 1 << 8;

    // Relation roles
    pub const REL_CHILDOF: u64 = 1 << 9;
    pub const REL_BASEOF: u64 = 1 << 10;
    pub const REL_OVERRIDEOF: u64 = 1 << 11;
    pub const REL_RECEIVEDBY: u64 = 1 << 12;
    pub const REL_CALLEDBY: u64 = 1 << 13;
    pub const REL_EXTENDEDBY: u64 = 1 << 14;
    pub const REL_ACCESSOROF: u64 = 1 << 15;
    pub const REL_CONTAINEDBY: u64 = 1 << 16;
}

/// Symbol properties (bitfield) — includes access control.
#[allow(non_upper_case_globals)]
pub mod symbol_property {
    pub const GENERIC: u64 = 1 << 0;
    pub const UNITTEST: u64 = 1 << 3;
    pub const LOCAL: u64 = 1 << 7;

    pub const SWIFT_ASYNC: u64 = 1 << 16;
    pub const SWIFT_AC_LESS_THAN_FILEPRIVATE: u64 = 1 << 17;
    pub const SWIFT_AC_FILEPRIVATE: u64 = 1 << 18;
    pub const SWIFT_AC_INTERNAL: u64 = (1 << 18) | (1 << 17);
    pub const SWIFT_AC_PACKAGE: u64 = 1 << 19;
    pub const SWIFT_AC_PUBLIC: u64 = (1 << 19) | (1 << 18);
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolLanguage {
    C = 0,
    ObjC = 1,
    CXX = 2,
    Swift = 100,
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitDependencyKind {
    Unit = 1,
    Record = 2,
    File = 3,
}

// --- Errors ---

#[derive(Debug, Error)]
pub enum FfiError {
    #[error("libIndexStore not found at {0}")]
    LibNotFound(String),
    #[error("symbol not found: {0}")]
    SymbolNotFound(String),
    #[error("IndexStore error: {0}")]
    StoreError(String),
    #[error("null pointer from IndexStore API")]
    NullPointer,
}

// --- Function pointer types ---

type FnStoreCreate = unsafe extern "C" fn(*const c_char, *mut IndexStoreErrorT) -> IndexStoreT;
type FnStoreDispose = unsafe extern "C" fn(IndexStoreT);
type FnErrorGetDescription = unsafe extern "C" fn(IndexStoreErrorT) -> *const c_char;
type FnErrorDispose = unsafe extern "C" fn(IndexStoreErrorT);

type FnStoreUnitsApplyF = unsafe extern "C" fn(
    IndexStoreT,
    c_uint,                                                         // sorted
    *mut c_void,                                                    // context
    unsafe extern "C" fn(*mut c_void, IndexStoreStringRef) -> bool, // applier
) -> bool;

// Record reader
type FnRecordReaderCreate = unsafe extern "C" fn(
    IndexStoreT,
    *const c_char,
    *mut IndexStoreErrorT,
) -> IndexStoreRecordReaderT;
type FnRecordReaderDispose = unsafe extern "C" fn(IndexStoreRecordReaderT);

type FnRecordReaderOccurrencesApplyF = unsafe extern "C" fn(
    IndexStoreRecordReaderT,
    *mut c_void,
    unsafe extern "C" fn(*mut c_void, IndexStoreOccurrenceT) -> bool,
) -> bool;

type FnRecordReaderSymbolsApplyF = unsafe extern "C" fn(
    IndexStoreRecordReaderT,
    bool, // nocache
    *mut c_void,
    unsafe extern "C" fn(*mut c_void, IndexStoreSymbolT) -> bool,
) -> bool;

// Symbol accessors
type FnSymbolGetKind = unsafe extern "C" fn(IndexStoreSymbolT) -> u32;
type FnSymbolGetSubKind = unsafe extern "C" fn(IndexStoreSymbolT) -> u32;
type FnSymbolGetLanguage = unsafe extern "C" fn(IndexStoreSymbolT) -> u32;
type FnSymbolGetProperties = unsafe extern "C" fn(IndexStoreSymbolT) -> u64;
type FnSymbolGetRoles = unsafe extern "C" fn(IndexStoreSymbolT) -> u64;
type FnSymbolGetName = unsafe extern "C" fn(IndexStoreSymbolT) -> IndexStoreStringRef;
type FnSymbolGetUsr = unsafe extern "C" fn(IndexStoreSymbolT) -> IndexStoreStringRef;

// Occurrence accessors
type FnOccurrenceGetSymbol = unsafe extern "C" fn(IndexStoreOccurrenceT) -> IndexStoreSymbolT;
type FnOccurrenceGetRoles = unsafe extern "C" fn(IndexStoreOccurrenceT) -> u64;
type FnOccurrenceGetLineCol = unsafe extern "C" fn(IndexStoreOccurrenceT, *mut c_uint, *mut c_uint);
type FnOccurrenceRelationsApplyF = unsafe extern "C" fn(
    IndexStoreOccurrenceT,
    *mut c_void,
    unsafe extern "C" fn(*mut c_void, IndexStoreSymbolRelationT) -> bool,
) -> bool;

// Symbol relation accessors
type FnSymbolRelationGetRoles = unsafe extern "C" fn(IndexStoreSymbolRelationT) -> u64;
type FnSymbolRelationGetSymbol =
    unsafe extern "C" fn(IndexStoreSymbolRelationT) -> IndexStoreSymbolT;

// Unit reader
type FnUnitReaderCreate = unsafe extern "C" fn(
    IndexStoreT,
    *const c_char,
    *mut IndexStoreErrorT,
) -> IndexStoreUnitReaderT;
type FnUnitReaderDispose = unsafe extern "C" fn(IndexStoreUnitReaderT);
type FnUnitReaderGetMainFile = unsafe extern "C" fn(IndexStoreUnitReaderT) -> IndexStoreStringRef;
type FnUnitReaderGetModuleName = unsafe extern "C" fn(IndexStoreUnitReaderT) -> IndexStoreStringRef;
type FnUnitReaderIsSystemUnit = unsafe extern "C" fn(IndexStoreUnitReaderT) -> bool;
type FnUnitReaderDependenciesApplyF = unsafe extern "C" fn(
    IndexStoreUnitReaderT,
    *mut c_void,
    unsafe extern "C" fn(*mut c_void, IndexStoreUnitDependencyT) -> bool,
) -> bool;

// Unit dependency accessors
type FnUnitDependencyGetKind = unsafe extern "C" fn(IndexStoreUnitDependencyT) -> u32;
type FnUnitDependencyGetName =
    unsafe extern "C" fn(IndexStoreUnitDependencyT) -> IndexStoreStringRef;
type FnUnitDependencyGetFilepath =
    unsafe extern "C" fn(IndexStoreUnitDependencyT) -> IndexStoreStringRef;
type FnUnitDependencyIsSystem = unsafe extern "C" fn(IndexStoreUnitDependencyT) -> bool;

/// Handle to the dynamically loaded libIndexStore.
///
/// All Index Store operations go through this struct. It is `Send + Sync` because
/// the underlying C library is thread-safe for read operations.
#[allow(dead_code)] // Fields loaded for completeness; some used only by future features.
pub struct IndexStoreLib {
    _lib: *mut c_void, // dlopen handle

    // Store lifecycle
    pub(crate) store_create: FnStoreCreate,
    pub(crate) store_dispose: FnStoreDispose,
    pub(crate) error_get_description: FnErrorGetDescription,
    pub(crate) error_dispose: FnErrorDispose,

    // Unit enumeration
    pub(crate) store_units_apply_f: FnStoreUnitsApplyF,

    // Record reader
    pub(crate) record_reader_create: FnRecordReaderCreate,
    pub(crate) record_reader_dispose: FnRecordReaderDispose,
    pub(crate) record_reader_occurrences_apply_f: FnRecordReaderOccurrencesApplyF,
    pub(crate) record_reader_symbols_apply_f: FnRecordReaderSymbolsApplyF,

    // Symbol
    pub(crate) symbol_get_kind: FnSymbolGetKind,
    pub(crate) symbol_get_sub_kind: FnSymbolGetSubKind,
    pub(crate) symbol_get_language: FnSymbolGetLanguage,
    pub(crate) symbol_get_properties: FnSymbolGetProperties,
    pub(crate) symbol_get_roles: FnSymbolGetRoles,
    pub(crate) symbol_get_name: FnSymbolGetName,
    pub(crate) symbol_get_usr: FnSymbolGetUsr,

    // Occurrence
    pub(crate) occurrence_get_symbol: FnOccurrenceGetSymbol,
    pub(crate) occurrence_get_roles: FnOccurrenceGetRoles,
    pub(crate) occurrence_get_line_col: FnOccurrenceGetLineCol,
    pub(crate) occurrence_relations_apply_f: FnOccurrenceRelationsApplyF,

    // Symbol relation
    pub(crate) symbol_relation_get_roles: FnSymbolRelationGetRoles,
    pub(crate) symbol_relation_get_symbol: FnSymbolRelationGetSymbol,

    // Unit reader
    pub(crate) unit_reader_create: FnUnitReaderCreate,
    pub(crate) unit_reader_dispose: FnUnitReaderDispose,
    pub(crate) unit_reader_get_main_file: FnUnitReaderGetMainFile,
    pub(crate) unit_reader_get_module_name: FnUnitReaderGetModuleName,
    pub(crate) unit_reader_is_system_unit: FnUnitReaderIsSystemUnit,
    pub(crate) unit_reader_dependencies_apply_f: FnUnitReaderDependenciesApplyF,

    // Unit dependency
    pub(crate) unit_dependency_get_kind: FnUnitDependencyGetKind,
    pub(crate) unit_dependency_get_name: FnUnitDependencyGetName,
    pub(crate) unit_dependency_get_filepath: FnUnitDependencyGetFilepath,
    pub(crate) unit_dependency_is_system: FnUnitDependencyIsSystem,
}

// SAFETY: libIndexStore is thread-safe for concurrent reads.
unsafe impl Send for IndexStoreLib {}
// SAFETY: All operations are behind shared references.
unsafe impl Sync for IndexStoreLib {}

// --- dlopen/dlsym helpers ---

extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> c_int;
    fn dlerror() -> *const c_char;
}

const RTLD_LAZY: c_int = 0x1;

/// Load a symbol from a dylib handle, returning an error if not found.
unsafe fn load_sym<T>(handle: *mut c_void, name: &str) -> Result<T, FfiError> {
    let c_name = CString::new(name).unwrap();
    let ptr = dlsym(handle, c_name.as_ptr());
    if ptr.is_null() {
        return Err(FfiError::SymbolNotFound(name.to_owned()));
    }
    // SAFETY: We checked for null, and the caller guarantees T matches the symbol type.
    Ok(std::mem::transmute_copy(&ptr))
}

/// Known paths to search for libIndexStore.dylib.
const DYLIB_SEARCH_PATHS: &[&str] = &[
    // Xcode default toolchain
    "/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib/libIndexStore.dylib",
    // Xcode beta
    "/Applications/Xcode-beta.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib/libIndexStore.dylib",
    // Command Line Tools
    "/Library/Developer/CommandLineTools/usr/lib/libIndexStore.dylib",
];

impl IndexStoreLib {
    /// Try to load libIndexStore from a specific path.
    pub fn load_from(dylib_path: &Path) -> Result<Self, FfiError> {
        let path_str = CString::new(dylib_path.to_string_lossy().as_ref())
            .map_err(|_| FfiError::LibNotFound(dylib_path.to_string_lossy().into_owned()))?;

        // SAFETY: dlopen with a valid C string path.
        let handle = unsafe { dlopen(path_str.as_ptr(), RTLD_LAZY) };
        if handle.is_null() {
            let err = unsafe {
                let e = dlerror();
                if e.is_null() {
                    "unknown error".to_owned()
                } else {
                    CStr::from_ptr(e).to_string_lossy().into_owned()
                }
            };
            return Err(FfiError::LibNotFound(format!(
                "{}: {err}",
                dylib_path.display()
            )));
        }

        // SAFETY: Each load_sym transmutes a function pointer from dlsym.
        // The types must match the C API exactly.
        unsafe {
            Ok(Self {
                _lib: handle,
                store_create: load_sym(handle, "indexstore_store_create")?,
                store_dispose: load_sym(handle, "indexstore_store_dispose")?,
                error_get_description: load_sym(handle, "indexstore_error_get_description")?,
                error_dispose: load_sym(handle, "indexstore_error_dispose")?,
                store_units_apply_f: load_sym(handle, "indexstore_store_units_apply_f")?,
                record_reader_create: load_sym(handle, "indexstore_record_reader_create")?,
                record_reader_dispose: load_sym(handle, "indexstore_record_reader_dispose")?,
                record_reader_occurrences_apply_f: load_sym(
                    handle,
                    "indexstore_record_reader_occurrences_apply_f",
                )?,
                record_reader_symbols_apply_f: load_sym(
                    handle,
                    "indexstore_record_reader_symbols_apply_f",
                )?,
                symbol_get_kind: load_sym(handle, "indexstore_symbol_get_kind")?,
                symbol_get_sub_kind: load_sym(handle, "indexstore_symbol_get_subkind")?,
                symbol_get_language: load_sym(handle, "indexstore_symbol_get_language")?,
                symbol_get_properties: load_sym(handle, "indexstore_symbol_get_properties")?,
                symbol_get_roles: load_sym(handle, "indexstore_symbol_get_roles")?,
                symbol_get_name: load_sym(handle, "indexstore_symbol_get_name")?,
                symbol_get_usr: load_sym(handle, "indexstore_symbol_get_usr")?,
                occurrence_get_symbol: load_sym(handle, "indexstore_occurrence_get_symbol")?,
                occurrence_get_roles: load_sym(handle, "indexstore_occurrence_get_roles")?,
                occurrence_get_line_col: load_sym(handle, "indexstore_occurrence_get_line_col")?,
                occurrence_relations_apply_f: load_sym(
                    handle,
                    "indexstore_occurrence_relations_apply_f",
                )?,
                symbol_relation_get_roles: load_sym(
                    handle,
                    "indexstore_symbol_relation_get_roles",
                )?,
                symbol_relation_get_symbol: load_sym(
                    handle,
                    "indexstore_symbol_relation_get_symbol",
                )?,
                unit_reader_create: load_sym(handle, "indexstore_unit_reader_create")?,
                unit_reader_dispose: load_sym(handle, "indexstore_unit_reader_dispose")?,
                unit_reader_get_main_file: load_sym(
                    handle,
                    "indexstore_unit_reader_get_main_file",
                )?,
                unit_reader_get_module_name: load_sym(
                    handle,
                    "indexstore_unit_reader_get_module_name",
                )?,
                unit_reader_is_system_unit: load_sym(
                    handle,
                    "indexstore_unit_reader_is_system_unit",
                )?,
                unit_reader_dependencies_apply_f: load_sym(
                    handle,
                    "indexstore_unit_reader_dependencies_apply_f",
                )?,
                unit_dependency_get_kind: load_sym(handle, "indexstore_unit_dependency_get_kind")?,
                unit_dependency_get_name: load_sym(handle, "indexstore_unit_dependency_get_name")?,
                unit_dependency_get_filepath: load_sym(
                    handle,
                    "indexstore_unit_dependency_get_filepath",
                )?,
                unit_dependency_is_system: load_sym(
                    handle,
                    "indexstore_unit_dependency_is_system",
                )?,
            })
        }
    }

    /// Try to load libIndexStore from well-known Xcode paths.
    pub fn load() -> Result<Self, FfiError> {
        // Check INDEXSTORE_LIB_PATH env var first
        if let Ok(path) = std::env::var("INDEXSTORE_LIB_PATH") {
            return Self::load_from(Path::new(&path));
        }

        // Try xcrun to find the active toolchain
        if let Ok(output) = std::process::Command::new("xcrun")
            .args(["--find", "swift"])
            .output()
        {
            if output.status.success() {
                let swift_path = String::from_utf8_lossy(&output.stdout).trim().to_owned();
                // swift is at ...toolchain/usr/bin/swift → lib is at ...toolchain/usr/lib/
                if let Some(bin_dir) = Path::new(&swift_path).parent() {
                    let lib_path = bin_dir
                        .parent()
                        .unwrap_or(bin_dir)
                        .join("lib/libIndexStore.dylib");
                    if lib_path.exists() {
                        return Self::load_from(&lib_path);
                    }
                }
            }
        }

        // Fall back to known paths
        for path in DYLIB_SEARCH_PATHS {
            let p = Path::new(path);
            if p.exists() {
                return Self::load_from(p);
            }
        }

        Err(FfiError::LibNotFound(
            "libIndexStore.dylib not found — install Xcode or set INDEXSTORE_LIB_PATH".into(),
        ))
    }

    /// Get the IndexStore error message and dispose it.
    ///
    /// # Safety
    /// `error` must be a valid IndexStore error pointer.
    pub(crate) unsafe fn get_error_message(&self, error: IndexStoreErrorT) -> String {
        if error.is_null() {
            return "unknown error".into();
        }
        let desc = (self.error_get_description)(error);
        let msg = if desc.is_null() {
            "unknown error".into()
        } else {
            CStr::from_ptr(desc).to_string_lossy().into_owned()
        };
        (self.error_dispose)(error);
        msg
    }
}

impl Drop for IndexStoreLib {
    fn drop(&mut self) {
        // SAFETY: _lib is a valid dlopen handle.
        unsafe {
            dlclose(self._lib);
        }
    }
}
