# SwiftGraph — CLAUDE.md

## Project Overview

SwiftGraph is a **Rust-based MCP server** that builds compiler-accurate code graphs from Swift projects using Xcode Index Store + tree-sitter + swift-syntax. It provides static analysis, audit checks, and AI-oriented context tools for Swift codebases.

**Full spec**: `swift-codegraph-mcp-spec.md` (always read before major decisions)

## Tech Stack

| Component | Technology | Version/Notes |
|-----------|-----------|---------------|
| Language | **Rust** | Install via `rustup` if missing |
| MCP SDK | **rmcp** | v1.2.0+, official Rust MCP SDK, tokio async |
| Index Store | **libIndexStore C API** via bindgen | 81 C symbols, dylib at Xcode toolchain |
| AST (fast) | **tree-sitter-swift** | v0.7.1, tree-sitter v0.23.0 |
| AST (full) | **swift-syntax subprocess** | Separate Swift CLI (`swiftgraph-parser`) |
| Storage | **rusqlite** + FTS5 | Single SQLite file |
| Git | **gix** (gitoxide) | Pure Rust, for diff-impact |
| Parallelism | **rayon** (data) + **tokio** (async) | |

## Environment

- **macOS** only (Index Store is Apple-only)
- **libIndexStore.dylib**: `/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib/libIndexStore.dylib`
- **indexstore.h**: NOT shipped locally — use from `swiftlang/llvm-project` repo or write manual bindings from the 81 exported C symbols (see `nm -gU` output in spec research)
- **Swift**: 6.2.4 available at `/usr/bin/swift`
- **Reference implementation**: [MobileNativeFoundation/swift-index-store](https://github.com/MobileNativeFoundation/swift-index-store) — Swift wrapper over same C API, good for understanding data structures

## Project Structure (Target)

```
swiftgraph/
├── CLAUDE.md                          # This file
├── swift-codegraph-mcp-spec.md        # Full technical specification
├── Cargo.toml                         # Workspace root
├── crates/
│   ├── swiftgraph-core/               # Graph model, storage, indexing pipeline
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── graph/                 # GraphNode, GraphEdge, SymbolKind, EdgeKind
│   │   │   │   ├── mod.rs
│   │   │   │   ├── node.rs
│   │   │   │   └── edge.rs
│   │   │   ├── index_store/           # libIndexStore FFI bindings + reader
│   │   │   │   ├── mod.rs
│   │   │   │   ├── ffi.rs             # bindgen or manual C FFI
│   │   │   │   └── reader.rs          # High-level IndexStore reader
│   │   │   ├── tree_sitter/           # tree-sitter-swift parser
│   │   │   │   ├── mod.rs
│   │   │   │   └── parser.rs
│   │   │   ├── storage/               # SQLite (rusqlite) + FTS5
│   │   │   │   ├── mod.rs
│   │   │   │   ├── schema.rs          # CREATE TABLE statements
│   │   │   │   └── queries.rs         # Prepared queries
│   │   │   ├── pipeline/              # Indexing pipeline (scan → parse → enrich → store)
│   │   │   │   └── mod.rs
│   │   │   ├── analysis/              # Metrics (complexity, coupling, dead code, cycles)
│   │   │   │   └── mod.rs
│   │   │   └── project/               # Project detection (SPM/Xcode/XcodeGen/Tuist)
│   │   │       └── mod.rs
│   │   └── Cargo.toml
│   ├── swiftgraph-audit/              # Audit engine: rules, categories, checks
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── engine.rs              # Rule registry, runner
│   │   │   ├── rules/                 # One module per category
│   │   │   │   ├── concurrency.rs     # CONC-001..007
│   │   │   │   ├── memory.rs          # MEM-001..006
│   │   │   │   ├── security.rs        # SEC-001..006
│   │   │   │   ├── performance.rs     # PERF-001..006
│   │   │   │   ├── swiftui_perf.rs    # SUI-001..006
│   │   │   │   ├── swiftui_arch.rs    # ARCH-001..005
│   │   │   │   ├── energy.rs          # NRG-001..006
│   │   │   │   ├── networking.rs      # NET-001..006
│   │   │   │   ├── codable.rs         # COD-001..005
│   │   │   │   ├── storage.rs         # STR-001..004
│   │   │   │   ├── accessibility.rs   # A11Y-001..004
│   │   │   │   ├── testing.rs         # TST-001..005
│   │   │   │   └── modernization.rs   # MOD-001..005
│   │   │   └── output/                # Formatters: JSON, SARIF, text, markdown
│   │   │       └── mod.rs
│   │   └── Cargo.toml
│   ├── swiftgraph-mcp/                # MCP server (rmcp) + tool handlers
│   │   ├── src/
│   │   │   ├── main.rs                # Entry: CLI parsing + MCP serve
│   │   │   ├── server.rs              # rmcp server setup
│   │   │   └── tools/                 # One module per tool group
│   │   │       ├── mod.rs
│   │   │       ├── status.rs          # swiftgraph_status, swiftgraph_reindex
│   │   │       ├── navigation.rs      # search, node, callers, callees, references
│   │   │       ├── hierarchy.rs       # hierarchy, extensions, conformances
│   │   │       ├── context.rs         # context, impact, diff_impact
│   │   │       ├── metrics.rs         # complexity, dead_code, cycles, coupling
│   │   │       ├── architecture.rs    # architecture, boundaries
│   │   │       ├── audit.rs           # swiftgraph_audit
│   │   │       ├── concurrency.rs     # swiftgraph_concurrency
│   │   │       └── workspace.rs       # files, imports
│   │   └── Cargo.toml
│   └── swiftgraph-parser/             # Swift CLI subprocess (swift-syntax)
│       ├── Package.swift
│       └── Sources/
│           └── SwiftGraphParser/
│               └── main.swift         # Reads .swift file → outputs JSON AST
├── tests/                             # Integration tests
│   ├── fixtures/                      # Sample Swift projects for testing
│   └── integration/
└── .swiftgraph/                       # Runtime config (created by `swiftgraph init`)
    └── config.json
```

## Build & Run Commands

```bash
# Build everything
cargo build --workspace

# Build release
cargo build --workspace --release

# Run tests
cargo test --workspace

# Run specific crate tests
cargo test -p swiftgraph-core
cargo test -p swiftgraph-audit

# Run the MCP server
cargo run -p swiftgraph-mcp -- serve --mcp

# Run CLI commands
cargo run -p swiftgraph-mcp -- init
cargo run -p swiftgraph-mcp -- index
cargo run -p swiftgraph-mcp -- audit --categories concurrency,memory

# Clippy (MUST pass before commit)
cargo clippy --workspace -- -D warnings

# Format
cargo fmt --all
```

## Development Rules

### Code Quality
- **`cargo clippy -- -D warnings` must pass** — no warnings allowed
- **`cargo fmt --all` before every commit** — consistent formatting
- **All public APIs must have doc comments** (`///`)
- **Error handling**: use `thiserror` for library errors, `anyhow` for binary. Never `unwrap()` in library code — only in tests
- **No `unsafe` without a `// SAFETY:` comment** explaining invariants. The only `unsafe` should be in `index_store/ffi.rs` for C FFI calls

### Architecture Principles
- **Crate boundaries are API boundaries** — `swiftgraph-core` must not depend on `rmcp`. `swiftgraph-audit` depends on `swiftgraph-core`. `swiftgraph-mcp` depends on both
- **Data model is the contract** — `GraphNode`, `GraphEdge`, `SymbolKind`, `EdgeKind` from the spec are the source of truth. Match the spec's Rust structs exactly
- **Graceful degradation** — code must work at every level: full (IndexStore+swift-syntax), IndexStore-only, swift-syntax-only, tree-sitter-only. Never panic on missing data source
- **Incremental by default** — SHA256 hash comparison for reindexing. Full reindex only when `--force`

### Performance Targets
| Operation | Target |
|-----------|--------|
| Full index 1000 files | < 10s |
| Incremental 1-10 files | < 1s |
| MCP tool response | < 200ms |
| Audit 1000 files | < 15s |
| FTS5 search | < 50ms |

### Testing Strategy
- **Unit tests** in each module (`#[cfg(test)] mod tests`)
- **Integration tests** in `tests/` with fixture Swift projects
- **Test fixtures**: create minimal Swift projects in `tests/fixtures/` that exercise specific patterns (conformances, extensions, concurrency, etc.)
- For IndexStore tests: generate index data by running `swift build -index-store-path` on fixtures

### libIndexStore FFI
- The C API has 81 exported symbols (prefixed `indexstore_`)
- Key entry points: `indexstore_store_create`, `indexstore_store_units_apply`, `indexstore_unit_reader_create`, `indexstore_record_reader_create`
- Symbol data: `indexstore_symbol_get_usr`, `indexstore_symbol_get_kind`, `indexstore_symbol_get_name`
- Occurrence data: `indexstore_occurrence_get_roles`, `indexstore_occurrence_get_line_col`
- Use `bindgen` with a manually written header or write manual FFI bindings from nm output
- **Always wrap raw pointers in safe Rust types** with proper Drop implementations
- Link with: `println!("cargo:rustc-link-lib=dylib=IndexStore"); println!("cargo:rustc-link-search=native=/Applications/Xcode.app/.../usr/lib/");`

### Roadmap Phases (implement in order)
1. **v0.1 — MVP: Graph** — scaffold, IndexStore FFI, tree-sitter fallback, basic MCP tools (status/search/node/callers/callees/references/hierarchy/files), CLI (init/index/serve)
2. **v0.2 — Intelligence** — context, impact, diff_impact, extensions, conformances, FTS5, incremental reindex
3. **v0.3 — Audit Engine** — swift-syntax subprocess, audit framework, concurrency/memory/security checks
4. **v0.4 — Analysis** — complexity, dead_code, cycles, coupling, architecture, boundaries
5. **v0.5 — Production** — all remaining audit categories, SARIF, watch mode, homebrew, performance optimization

### Commit Convention
```
type(scope): description

feat(index-store): add unit reader FFI bindings
fix(storage): handle FTS5 special characters in search
refactor(graph): extract edge resolution into separate module
test(audit): add concurrency rule fixtures
```
Types: `feat`, `fix`, `refactor`, `test`, `docs`, `perf`, `ci`, `chore`

## Key Design Decisions

1. **Why Rust over Swift**: borrow checker for memory safety at graph scale, `rmcp` SDK availability, single static binary, rayon+tokio parallelism
2. **Why SQLite over in-memory only**: persistence across MCP server restarts, FTS5 for search, single-file simplicity
3. **Why tree-sitter as fallback**: works without build, fast C-based parser; swift-syntax is more accurate but requires Swift toolchain
4. **Why workspace of crates**: separation of concerns, independent testability, clear dependency direction
5. **Why C API over IndexStoreDB**: fewer dependencies, stable C ABI, sufficient for our needs (81 functions cover units/records/symbols/occurrences)

## Reference Links

- [rmcp (Rust MCP SDK)](https://github.com/modelcontextprotocol/rust-sdk)
- [libIndexStore source (swiftlang/llvm-project)](https://github.com/swiftlang/llvm-project)
- [swift-index-store (Swift reference impl)](https://github.com/MobileNativeFoundation/swift-index-store)
- [tree-sitter-swift](https://crates.io/crates/tree-sitter-swift)
- [MCP Protocol spec](https://spec.modelcontextprotocol.io/)
