# SwiftGraph — Work Log

## Session 1: 2026-03-12 — Project Bootstrap & v0.1 Scaffold

### Environment Setup
- Installed Rust 1.94.0 (stable, aarch64-apple-darwin)
- Initialized git repository on `main` branch
- Created `CLAUDE.md` for agent-based development

### Research Completed
- **rmcp** 1.2.0 — confirmed API: `#[tool_router]` + `#[tool_handler]` + `#[tool]` macros, `serve_server()`, `transport::io::stdio()`
- **tree-sitter-swift** 0.7.1 — requires tree-sitter >= 0.25 (ABI version 15). Both `struct` and `class` map to `class_declaration` node; disambiguated by first child keyword
- **libIndexStore** — 81 exported C symbols at `/Applications/Xcode.app/.../usr/lib/libIndexStore.dylib`. Header not shipped locally, available at `swiftlang/llvm-project`. Reference Swift wrapper: `MobileNativeFoundation/swift-index-store`
- **Swift** 6.2.4 available on system
- GPG signing disabled for repo (key not available locally)

### Code Written — 7 commits, ~2400 lines

| Commit | Scope | Description |
|--------|-------|-------------|
| `909ef0d` | chore | Initial setup — spec, CLAUDE.md, .gitignore |
| `28e5959` | core/graph | GraphNode, GraphEdge, SymbolKind (14), EdgeKind (14), Location, AccessLevel, NodeMetrics |
| `440cc79` | core/storage | SQLite schema (5 tables), FTS5 with sync triggers, CRUD queries, callers/callees/hierarchy/stats |
| `0a9eee9` | core/tree_sitter | tree-sitter-swift parser — declarations, inheritance, attributes, access levels |
| `c75e78e` | core/project+pipeline | Project detection (SPM/Xcode/XcodeGen/Tuist), parallel indexing pipeline (rayon), SHA256 incremental |
| `0370baf` | audit | AuditIssue, Severity, Category (13 types), AuditResult. Rule/output stubs |
| `93bda16` | mcp | rmcp 1.2.0 server (8 tools), CLI (init/index/serve/search/callers/hierarchy) |

### Tests — 8/8 passing

| # | Test | Module | Verifies |
|---|------|--------|----------|
| 1 | `symbol_kind_as_str` | graph/node | SymbolKind string serialization |
| 2 | `node_serialization_roundtrip` | graph/node | GraphNode JSON encode/decode |
| 3 | `edge_serialization_roundtrip` | graph/edge | GraphEdge JSON encode/decode |
| 4 | `project_type_as_str` | project | ProjectType string representation |
| 5 | `create_db_and_insert_node` | storage | SQLite DB creation, node upsert+read, FK constraints |
| 6 | `insert_edge_and_query_callers` | storage | Edge insert, callers/callees query correctness |
| 7 | `parse_simple_struct` | tree_sitter | Parses `struct User {}`, correct SymbolKind::Struct |
| 8 | `parse_class_with_inheritance` | tree_sitter | Parses `class Foo: Bar`, detects conformance edges |

### Quality Gates
- `cargo clippy --workspace -- -D warnings` — clean
- `cargo fmt --all -- --check` — clean
- `cargo test --workspace` — 8/8 pass

### Issues Encountered & Resolved
1. **rmcp API mismatch** — `#[tool(tool_box)]` doesn't exist in 1.2.0. Fixed: use `#[tool_router]` + `#[tool_handler]` + `Parameters<T>` pattern
2. **tree-sitter ABI mismatch** — tree-sitter 0.23/0.24 support ABI 14, but tree-sitter-swift 0.7 compiles to ABI 15. Fixed: upgraded to tree-sitter 0.25
3. **tree-sitter-swift node kinds** — `struct` and `class` both produce `class_declaration`. Fixed: check first child keyword (`struct`/`class`/`actor`)
4. **ServerInfo non-exhaustive** — can't use struct literal with `..Default::default()`. Fixed: `let mut info = Default::default(); info.instructions = ...`
5. **GPG key not found** — `commit.gpgsign` was enabled globally. Fixed: disabled locally

---

## Session 2: 2026-03-12 — Index Store FFI, Pipeline Integration, Real Project Test

### libIndexStore FFI Bindings
- Fetched official C header from `swiftlang/llvm-project` (`indexstore.h`, v0.16)
- Hand-written Rust FFI bindings in `index_store/ffi.rs` (~500 lines):
  - Runtime dynamic linking via `dlopen`/`dlsym` (no build-time Xcode dependency)
  - 30+ function pointers: store lifecycle, unit enumeration, record reading, symbol/occurrence/relation access
  - Auto-discovers dylib via `xcrun --find swift` → toolchain lib path, or well-known Xcode paths
  - Supports `INDEXSTORE_LIB_PATH` env override
  - `Send + Sync` — thread-safe for concurrent reads

### Index Store Reader
- `index_store/reader.rs` (~350 lines): converts Index Store data → `GraphNode`/`GraphEdge`
  - Unit enumeration → filter Swift non-system units → record reading
  - Occurrence processing with relation mapping:
    - `REL_CALLEDBY` → `EdgeKind::Calls`
    - `REL_BASEOF` → `EdgeKind::ConformsTo` / `EdgeKind::InheritsFrom` (protocol vs class)
    - `REL_OVERRIDEOF` → `EdgeKind::Overrides`
    - `REL_CHILDOF` → `EdgeKind::Contains`
    - `REL_EXTENDEDBY` → `EdgeKind::ExtendsType`
  - Symbol kind mapping (28 Index Store kinds → 15 graph kinds)
  - Access level from properties bitfield (public/package/internal/fileprivate/private)

### Pipeline Integration
- `index_directory_with_store()` — tries Index Store first, tree-sitter fallback
- Reports `IndexStrategy`: `IndexStore`, `TreeSitter`, or `Hybrid`
- CLI `index` command auto-detects Index Store via `project::detect_project()`

### New MCP Tool: `swiftgraph_files`
- Lists indexed files with stats (path, hash, last_indexed, symbol_count)
- Filterable by path prefix (e.g. `Sources/Features/`)

### Data Model Additions
- `SymbolKind::Module` — for Index Store module symbols
- `AccessLevel::Package` — Swift 5.9+ package access control
- `NodeMetrics` derives `Default`

### Schema Fix
- Removed FK constraints on `edges.source`/`edges.target` → `nodes.id`
- Reason: edges often reference SDK symbols (UIKit, Foundation) not in our index
- `INSERT OR IGNORE` now works correctly on real projects

### Integration Test: Production Project 
- XcodeGen project, 941 Swift files
- **6824 nodes, 6140 edges** indexed via tree-sitter
- Search, hierarchy, callers all verified
- MCP server added to `.mcp.json`, JSON-RPC handshake verified
- Index Store path not found (project not built) → graceful degradation to tree-sitter

### Commits — 4 new (11 total)

| Commit | Scope | Description |
|--------|-------|-------------|
| `af61ae1` | fix(storage) | Remove FK constraints on edges for SDK symbol compatibility |
| `5b8879c` | feat(core) | libIndexStore FFI bindings + Index Store reader |
| `c5f7e97` | feat(core) | Integrate Index Store into indexing pipeline |
| `ec49c9d` | feat(mcp) | swiftgraph_files tool, Module kind, Package access level |

### Tests — 10/10 passing

| # | Test | Module | Verifies |
|---|------|--------|----------|
| 9 | `get_files_query` | storage | get_files with path prefix filter |
| 10 | `index_store_lib_loads` | storage | libIndexStore.dylib discovery via xcrun |

### Quality Gates
- `cargo clippy --workspace -- -D warnings` — clean
- `cargo fmt --all -- --check` — clean
- `cargo test --workspace` — 10/10 pass

### Dependency Versions (locked)
| Crate | Version |
|-------|---------|
| rmcp | 1.2.0 |
| tree-sitter | 0.25.10 |
| tree-sitter-swift | 0.7.1 |
| rusqlite | 0.32.1 (bundled SQLite) |
| tokio | 1.50.0 |
| rayon | 1.11.0 |
| clap | 4.5.60 |
| serde | 1.0.228 |
| serde_json | 1.0.149 |
| sha2 | 0.10.9 |
| walkdir | 2.5.0 |

---

## Session 3: 2026-03-12 — v0.2 Intelligence

### Analysis Module — 3 new modules (~500 lines)
- `analysis/context.rs` — task-based context builder
  - Keyword extraction (stop word filtering, min 3 chars)
  - FTS5 search per keyword with LIKE fallback
  - 2-level BFS expansion via incoming/outgoing edges
  - Scoring: `incoming * 2 + outgoing + 50 seed bonus + 10 name boost`
  - Architecture detection (VIPER, TCA/Redux, MVVM+Coordinator, MVVM+Router, MVVM)
- `analysis/impact.rs` — blast radius analysis
  - Direct dependents categorized by edge kind
  - BFS transitive dependents up to configurable depth
  - Risk levels: low/medium/high/critical
- `analysis/diff_impact.rs` — git diff-based impact
  - Uses `git diff --name-only --diff-filter=ACMR`
  - Finds symbols in changed files, aggregates impact

### New Queries — 8 functions
- `get_extensions()`, `get_conformances(direction)`, `get_all_incoming()`, `get_all_outgoing()`, `count_incoming()`, `count_outgoing()`, `get_nodes_in_file()`, `find_nodes_by_name_pattern()`

### MCP Tools — 5 new (14 total)
- `swiftgraph_extensions`, `swiftgraph_conformances`, `swiftgraph_context`, `swiftgraph_impact`, `swiftgraph_diff_impact`

### CLI Subcommands — 3 new
- `context`, `impact`, `diff-impact`

### Integration Test: Production Project
- Context, impact, diff-impact all verified on 941-file project

### Commits — 2 new (13 total)

| Commit | Scope | Description |
|--------|-------|-------------|
| `17c6ea1` | feat(core) | Context builder, impact analysis, diff-impact, 8 new queries |
| `0a59aba` | feat(mcp) | 5 new MCP tools + 3 CLI subcommands + name resolution |

### Tests — 11/11 passing

### Quality Gates
- clippy clean, fmt clean, 11/11 tests pass
