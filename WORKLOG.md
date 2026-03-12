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
