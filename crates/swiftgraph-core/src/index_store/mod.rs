/// Raw FFI bindings for libIndexStore C API (runtime-loaded via `dlopen`).
pub mod ffi;
/// High-level reader that converts Index Store data into `GraphNode`/`GraphEdge`.
pub mod reader;
