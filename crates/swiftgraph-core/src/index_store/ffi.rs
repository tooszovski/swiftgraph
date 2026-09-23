//! Raw FFI bindings for libIndexStore.
//!
//! These are hand-written bindings matching the C header from
//! <https://github.com/swiftlang/llvm-project/blob/next/clang/include/indexstore/indexstore.h>
//!
//! We use runtime dynamic linking (`dlopen`/`dlsym`) so the binary can run
//! without Xcode installed (graceful degradation to tree-sitter).
//!
//! # Safety
//! This is the only module allowed to contain `unsafe`. Raw handles are wrapped
//! in [`IndexStore`], [`UnitReader`] and [`RecordReader`], which dispose them on
//! `Drop`; iteration callbacks copy everything into owned values
//! ([`UnitDependency`], [`Occurrence`]) so no borrowed C data escapes.

use std::borrow::Cow;
use std::ffi::{c_char, c_int, c_uint, c_void, CStr, CString};
use std::marker::PhantomData;
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
    /// Borrow as text, replacing invalid UTF-8 (paths from the store are not
    /// guaranteed to be UTF-8). Returns an empty string if null.
    ///
    /// # Safety
    /// `data` must point to `length` readable bytes that stay valid for the
    /// lifetime of the returned value.
    pub unsafe fn as_str_lossy(&self) -> Cow<'_, str> {
        if self.data.is_null() || self.length == 0 {
            return Cow::Borrowed("");
        }
        // SAFETY: the caller guarantees `data` is valid for `length` bytes.
        let bytes = unsafe { std::slice::from_raw_parts(self.data as *const u8, self.length) };
        String::from_utf8_lossy(bytes)
    }

    /// Copy into an owned `String` (lossy on invalid UTF-8).
    ///
    /// # Safety
    /// Same as [`Self::as_str_lossy`].
    pub unsafe fn to_string_owned(&self) -> String {
        // SAFETY: forwarded caller guarantee.
        unsafe { self.as_str_lossy() }.into_owned()
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
    let c_name = CString::new(name).map_err(|_| FfiError::SymbolNotFound(name.to_owned()))?;
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
            // SAFETY: dlerror returns null or a valid NUL-terminated string.
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

// --- Safe RAII wrappers ---

/// Symbol data copied out of the store.
#[derive(Debug, Clone)]
pub struct SymbolInfo {
    /// Unified Symbol Resolution string (stable symbol ID).
    pub usr: String,
    /// Symbol name, e.g. `load(id:)`.
    pub name: String,
    /// Raw [`SymbolKind`] value.
    pub kind: u32,
    /// Raw [`SymbolSubKind`] value.
    pub sub_kind: u32,
    /// Raw [`SymbolLanguage`] value.
    pub language: u32,
    /// [`symbol_property`] bitfield.
    pub properties: u64,
}

/// A relation attached to an occurrence (`calledBy`, `baseOf`, `childOf`, ...).
#[derive(Debug, Clone)]
pub struct Relation {
    /// [`symbol_role`] relation bits.
    pub roles: u64,
    /// The related symbol.
    pub symbol: SymbolInfo,
}

/// One symbol occurrence in a record, fully owned.
#[derive(Debug, Clone)]
pub struct Occurrence {
    /// The symbol that occurs.
    pub symbol: SymbolInfo,
    /// [`symbol_role`] bitfield of this occurrence.
    pub roles: u64,
    /// 1-based line.
    pub line: u32,
    /// 1-based column.
    pub column: u32,
    /// Relations of this occurrence.
    pub relations: Vec<Relation>,
}

/// A dependency of a unit (another unit, a record, or a file).
#[derive(Debug, Clone)]
pub struct UnitDependency {
    /// Raw [`UnitDependencyKind`] value.
    pub kind: u32,
    /// Record or unit name.
    pub name: String,
    /// Source file path the dependency belongs to.
    pub file_path: String,
    /// Whether the dependency is a system (SDK) one.
    pub is_system: bool,
}

/// Owned handle to an opened Index Store; disposed on drop.
pub struct IndexStore<'l> {
    lib: &'l IndexStoreLib,
    raw: IndexStoreT,
}

/// Owned unit reader; disposed on drop. Cannot outlive its [`IndexStore`].
pub struct UnitReader<'s> {
    lib: &'s IndexStoreLib,
    raw: IndexStoreUnitReaderT,
    _store: PhantomData<&'s IndexStore<'s>>,
}

/// Owned record reader; disposed on drop. Cannot outlive its [`IndexStore`].
pub struct RecordReader<'s> {
    lib: &'s IndexStoreLib,
    raw: IndexStoreRecordReaderT,
    _store: PhantomData<&'s IndexStore<'s>>,
}

/// Calls `f` for each item passed to a C applier callback. `f` returns
/// `false` to stop iteration.
unsafe extern "C" fn trampoline<T, F: FnMut(T) -> bool>(ctx: *mut c_void, item: T) -> bool {
    // SAFETY: `ctx` is the `&mut F` passed by the `*_apply_f` caller below and
    // is valid for the duration of that synchronous call.
    let f = unsafe { &mut *(ctx as *mut F) };
    f(item)
}

/// The C callback for closure `f` (infers the unnameable closure type).
fn trampoline_for<T, F: FnMut(T) -> bool>(_f: &F) -> unsafe extern "C" fn(*mut c_void, T) -> bool {
    trampoline::<T, F>
}

fn c_string(s: &str) -> Result<CString, FfiError> {
    CString::new(s).map_err(|_| FfiError::StoreError(format!("interior NUL in {s:?}")))
}

impl IndexStoreLib {
    /// Copy a symbol's data. `symbol` must come from a live occurrence/relation.
    fn symbol_info(&self, symbol: IndexStoreSymbolT) -> SymbolInfo {
        // SAFETY: `symbol` is valid for the duration of the enclosing applier
        // callback; strings are copied before returning.
        unsafe {
            SymbolInfo {
                usr: (self.symbol_get_usr)(symbol).to_string_owned(),
                name: (self.symbol_get_name)(symbol).to_string_owned(),
                kind: (self.symbol_get_kind)(symbol),
                sub_kind: (self.symbol_get_sub_kind)(symbol),
                language: (self.symbol_get_language)(symbol),
                properties: (self.symbol_get_properties)(symbol),
            }
        }
    }
}

impl<'l> IndexStore<'l> {
    /// Open the Index Store at `path`.
    pub fn open(lib: &'l IndexStoreLib, path: &Path) -> Result<Self, FfiError> {
        let path_c = c_string(&path.to_string_lossy())?;
        let mut error: IndexStoreErrorT = std::ptr::null_mut();
        // SAFETY: valid C string and out-pointer; ownership of the result is
        // taken by `Self` and released in `Drop`.
        let raw = unsafe { (lib.store_create)(path_c.as_ptr(), &mut error) };
        if raw.is_null() {
            // SAFETY: `error` was set by the failed call (or is null).
            return Err(FfiError::StoreError(unsafe {
                lib.get_error_message(error)
            }));
        }
        Ok(Self { lib, raw })
    }

    /// Names of all units in the store.
    pub fn unit_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        let mut f = |name: IndexStoreStringRef| {
            // SAFETY: `name` is valid during the callback; copied immediately.
            names.push(unsafe { name.to_string_owned() });
            true
        };
        // SAFETY: `self.raw` is a live store; `f` outlives the synchronous call.
        unsafe {
            (self.lib.store_units_apply_f)(
                self.raw,
                0,
                &mut f as *mut _ as *mut c_void,
                trampoline_for::<IndexStoreStringRef, _>(&f),
            );
        }
        names
    }

    /// Open a reader for the unit named `name`.
    pub fn unit_reader(&self, name: &str) -> Result<UnitReader<'_>, FfiError> {
        let name_c = c_string(name)?;
        let mut error: IndexStoreErrorT = std::ptr::null_mut();
        // SAFETY: live store, valid C string; result owned by `UnitReader`.
        let raw = unsafe { (self.lib.unit_reader_create)(self.raw, name_c.as_ptr(), &mut error) };
        if raw.is_null() {
            // SAFETY: `error` was set by the failed call (or is null).
            return Err(FfiError::StoreError(unsafe {
                self.lib.get_error_message(error)
            }));
        }
        Ok(UnitReader {
            lib: self.lib,
            raw,
            _store: PhantomData,
        })
    }

    /// Open a reader for the record named `name`.
    pub fn record_reader(&self, name: &str) -> Result<RecordReader<'_>, FfiError> {
        let name_c = c_string(name)?;
        let mut error: IndexStoreErrorT = std::ptr::null_mut();
        // SAFETY: live store, valid C string; result owned by `RecordReader`.
        let raw = unsafe { (self.lib.record_reader_create)(self.raw, name_c.as_ptr(), &mut error) };
        if raw.is_null() {
            // SAFETY: `error` was set by the failed call (or is null).
            return Err(FfiError::StoreError(unsafe {
                self.lib.get_error_message(error)
            }));
        }
        Ok(RecordReader {
            lib: self.lib,
            raw,
            _store: PhantomData,
        })
    }
}

impl Drop for IndexStore<'_> {
    fn drop(&mut self) {
        // SAFETY: `raw` came from `store_create` and is disposed exactly once.
        unsafe { (self.lib.store_dispose)(self.raw) }
    }
}

impl UnitReader<'_> {
    /// Whether this is a system (SDK) unit.
    pub fn is_system_unit(&self) -> bool {
        // SAFETY: `raw` is a live unit reader.
        unsafe { (self.lib.unit_reader_is_system_unit)(self.raw) }
    }

    /// Main source file of the unit.
    pub fn main_file(&self) -> String {
        // SAFETY: live reader; string copied while the reader is alive.
        unsafe { (self.lib.unit_reader_get_main_file)(self.raw).to_string_owned() }
    }

    /// Module the unit belongs to.
    pub fn module_name(&self) -> String {
        // SAFETY: live reader; string copied while the reader is alive.
        unsafe { (self.lib.unit_reader_get_module_name)(self.raw).to_string_owned() }
    }

    /// All dependencies of the unit.
    pub fn dependencies(&self) -> Vec<UnitDependency> {
        let lib = self.lib;
        let mut deps = Vec::new();
        let mut f = |dep: IndexStoreUnitDependencyT| {
            // SAFETY: `dep` is valid during the callback; data copied immediately.
            unsafe {
                deps.push(UnitDependency {
                    kind: (lib.unit_dependency_get_kind)(dep),
                    name: (lib.unit_dependency_get_name)(dep).to_string_owned(),
                    file_path: (lib.unit_dependency_get_filepath)(dep).to_string_owned(),
                    is_system: (lib.unit_dependency_is_system)(dep),
                });
            }
            true
        };
        // SAFETY: live reader; `f` outlives the synchronous call.
        unsafe {
            (self.lib.unit_reader_dependencies_apply_f)(
                self.raw,
                &mut f as *mut _ as *mut c_void,
                trampoline_for::<IndexStoreUnitDependencyT, _>(&f),
            );
        }
        deps
    }
}

impl Drop for UnitReader<'_> {
    fn drop(&mut self) {
        // SAFETY: `raw` came from `unit_reader_create` and is disposed exactly once.
        unsafe { (self.lib.unit_reader_dispose)(self.raw) }
    }
}

impl RecordReader<'_> {
    /// All occurrences in the record, with their relations.
    pub fn occurrences(&self) -> Vec<Occurrence> {
        let lib = self.lib;
        let mut out = Vec::new();
        let mut f = |occ: IndexStoreOccurrenceT| {
            let mut relations = Vec::new();
            let mut on_relation = |rel: IndexStoreSymbolRelationT| {
                // SAFETY: `rel` is valid during the callback; data copied immediately.
                let (roles, symbol) = unsafe {
                    (
                        (lib.symbol_relation_get_roles)(rel),
                        (lib.symbol_relation_get_symbol)(rel),
                    )
                };
                relations.push(Relation {
                    roles,
                    symbol: lib.symbol_info(symbol),
                });
                true
            };
            let mut line: c_uint = 0;
            let mut column: c_uint = 0;
            // SAFETY: `occ` is valid during the callback; nested apply is
            // synchronous and `on_relation` outlives it.
            let (symbol, roles) = unsafe {
                (lib.occurrence_get_line_col)(occ, &mut line, &mut column);
                (lib.occurrence_relations_apply_f)(
                    occ,
                    &mut on_relation as *mut _ as *mut c_void,
                    trampoline_for::<IndexStoreSymbolRelationT, _>(&on_relation),
                );
                (
                    (lib.occurrence_get_symbol)(occ),
                    (lib.occurrence_get_roles)(occ),
                )
            };
            out.push(Occurrence {
                symbol: lib.symbol_info(symbol),
                roles,
                line,
                column,
                relations,
            });
            true
        };
        // SAFETY: live reader; `f` outlives the synchronous call.
        unsafe {
            (self.lib.record_reader_occurrences_apply_f)(
                self.raw,
                &mut f as *mut _ as *mut c_void,
                trampoline_for::<IndexStoreOccurrenceT, _>(&f),
            );
        }
        out
    }
}

impl Drop for RecordReader<'_> {
    fn drop(&mut self) {
        // SAFETY: `raw` came from `record_reader_create` and is disposed exactly once.
        unsafe { (self.lib.record_reader_dispose)(self.raw) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_utf8_string_ref_is_replaced_not_trusted() {
        let bytes: &[u8] = b"Sources/\xff\xfeName.swift";
        let sr = IndexStoreStringRef {
            data: bytes.as_ptr() as *const c_char,
            length: bytes.len(),
        };
        // SAFETY: `bytes` outlives the call and `length` matches the slice.
        let s = unsafe { sr.to_string_owned() };
        assert!(std::str::from_utf8(s.as_bytes()).is_ok());
        assert!(s.contains('\u{FFFD}'));
        assert!(s.starts_with("Sources/") && s.ends_with("Name.swift"));
    }
}
