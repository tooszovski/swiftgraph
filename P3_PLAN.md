# P3 Implementation Plan

## P3-1: Fix TODOs in queries.rs (sub_kind / access_level parsing)

**Complexity: S | Dependencies: None | Risk: Low**

### Problem
`row_to_node()` in `queries.rs` discards `sub_kind` (always `None`) and hardcodes `access_level` to `Internal`. Data IS stored correctly on write via `format!("{:?}", ...)` — just never parsed back. This causes `find_dead_code` to incorrectly flag `Public`/`Open` symbols.

### Implementation
- Add `fn parse_sub_kind(s: &str) -> Option<SymbolSubKind>` — match on Debug strings: "Getter", "Setter", "Subscript", "Initializer", "Deinitializer"
- Add `fn parse_access_level(s: &str) -> AccessLevel` — match on "Open", "Public", "Package", "Internal", "FilePrivate", "Private"
- Wire into `row_to_node()` at lines 250 and 260
- Add unit test for round-trip (upsert → read → verify fields)

### Files
| File | Change |
|------|--------|
| `crates/swiftgraph-core/src/storage/queries.rs` | Add 2 parsing functions, update `row_to_node` |

---

## P3-2: Request-ID Tracing for MCP Requests

**Complexity: S | Dependencies: `uuid` crate | Risk: Low**

### Problem
22 MCP tool handlers have no correlation IDs in logs. Cannot trace a specific request through the system.

### Implementation
Add `with_request_span` helper on `SwiftGraphServer`:

```rust
fn with_request_span<F: FnOnce() -> String>(&self, tool: &str, f: F) -> String {
    let request_id = uuid::Uuid::new_v4();
    let span = tracing::info_span!("mcp_tool", %request_id, tool);
    let _guard = span.enter();
    tracing::info!("started");
    let start = std::time::Instant::now();
    let result = f();
    tracing::info!(elapsed_ms = start.elapsed().as_millis(), "completed");
    result
}
```

Wrap each of the 22 tool handler bodies in `self.with_request_span(...)`.

### Files
| File | Change |
|------|--------|
| `Cargo.toml` | Add `uuid = { version = "1", features = ["v4"] }` |
| `crates/swiftgraph-mcp/Cargo.toml` | Add `uuid.workspace = true` |
| `crates/swiftgraph-mcp/src/server.rs` | Add helper, wrap 22 handlers |
| `crates/swiftgraph-mcp/src/main.rs` | Add `.with_span_events(FmtSpan::CLOSE)` |

### Alternative
Use `AtomicU64` counter instead of UUID to avoid new dependency. Non-globally-unique but simpler.

---

## P3-3: Fuzz / Property-Based Testing

**Complexity: M | Dependencies: `proptest` | Risk: Low**

### Problem
Graph algorithms (cycles, impact, dead_code, complexity) have no edge-case testing for degenerate inputs (empty graphs, self-loops, massive fan-out, disconnected components).

### Target Algorithms (priority order)
1. `cycles.rs` — DFS cycle detection
2. `impact.rs` — BFS blast radius
3. `dead_code.rs` — no incoming edges
4. `complexity.rs` — fan-in/fan-out

### Implementation

**Step 1: Refactor for testability** — Extract `_from_conn(&Connection)` variants from each analysis function (currently they take `db_path: &Path` and open their own connection). The public API stays the same; the `_from_conn` variants are `pub(crate)`.

**Step 2: Random graph generator** — Helper that populates an in-memory DB with N nodes and M random edges.

**Step 3: Property tests:**
- `no_crash_on_arbitrary_graph` — any graph → no panic
- `known_cycle_is_detected` — deliberate cycle → found
- `acyclic_graph_has_no_cycles` — DAG → empty result
- `impact_includes_direct_callers` — A→B ⇒ impact(B) ∋ A
- `impact_is_monotonic_with_depth` — depth=3 ≥ depth=2
- `impact_on_leaf_node_is_zero` — no incoming → 0

### Files
| File | Change |
|------|--------|
| `Cargo.toml` | Add `proptest = "1"` to workspace dev-deps |
| `crates/swiftgraph-core/Cargo.toml` | Add `proptest.workspace = true` |
| `crates/swiftgraph-core/src/analysis/cycles.rs` | Extract `detect_cycles_from_conn` |
| `crates/swiftgraph-core/src/analysis/impact.rs` | Extract `analyze_impact_from_conn` |
| `crates/swiftgraph-core/src/analysis/dead_code.rs` | Extract `find_dead_code_from_conn` |
| `crates/swiftgraph-core/tests/proptest_graph.rs` (new) | ~15 property tests |
| `crates/swiftgraph-core/tests/test_helpers.rs` (new) | Random graph generator |

---

## P3-4: swift-syntax Subprocess (swiftgraph-parser)

**Complexity: XL | Dependencies: Swift toolchain, swift-syntax 600+ | Risk: High**

### Problem
tree-sitter cannot handle macro expansion, type inference, or full expression analysis. The spec calls for a Swift CLI subprocess for deeper AST checks.

### Architecture
```
Rust (pipeline) → spawns → swiftgraph-parser (Swift CLI) → JSON stdout → Rust parses
```

### JSON Protocol (per file)
```json
{
  "version": 1,
  "file": "/path/to/File.swift",
  "declarations": [{
    "name": "MyClass",
    "kind": "class",
    "line": 5,
    "attributes": ["@MainActor", "@Observable"],
    "access_level": "public",
    "signature": "class MyClass: BaseClass, SomeProtocol",
    "doc_comment": "/// A class that does things.",
    "body_analysis": { "has_async": true, "captures_self": false, "complexity": 4 }
  }],
  "imports": ["Foundation", "SwiftUI"],
  "macros_expanded": 0
}
```

### Phasing
1. **Phase A** — Basic declaration extraction (attributes, access levels, signatures). ~L
2. **Phase B** — Body analysis (captures_self, has_async, complexity). ~M
3. **Phase C** — Macro expansion + batch mode. ~L

### Files to Create
| File | Purpose |
|------|---------|
| `crates/swiftgraph-parser/Package.swift` | Swift package, depends on swift-syntax 600+ |
| `crates/swiftgraph-parser/Sources/SwiftGraphParser/main.swift` | Entry: file arg → parse → JSON |
| `crates/swiftgraph-parser/Sources/SwiftGraphParser/ASTVisitor.swift` | SyntaxVisitor subclass |
| `crates/swiftgraph-parser/Sources/SwiftGraphParser/Models.swift` | Codable output types |

### Files to Modify
| File | Change |
|------|--------|
| `crates/swiftgraph-core/src/lib.rs` | Add `pub mod swift_syntax;` |
| `crates/swiftgraph-core/src/swift_syntax/mod.rs` (new) | `find_parser()`, `parse_file()` |
| `crates/swiftgraph-core/src/pipeline/mod.rs` | Add step 4b: swift-syntax enrichment |
| `crates/swiftgraph-core/src/config.rs` | Add `swift_syntax_path: String` |

### Risks
- **Build complexity**: Swift CLI built separately (`swift build`), not part of `cargo build`
- **Subprocess overhead**: ~50ms/file. Mitigate with batch mode (Phase C)
- **SwiftSyntax version pinning**: Tied to Swift compiler version
- **Graceful degradation preserved**: If parser not found, system works as today

---

## Recommended Order

| # | Item | Size | Impact |
|---|------|------|--------|
| 1 | P3-1: TODOs in queries.rs | S | Fixes dead_code bug |
| 2 | P3-2: Request-ID tracing | S | Observability |
| 3 | P3-3: Property testing | M | Algorithm correctness |
| 4 | P3-4: swift-syntax subprocess | XL | Deep analysis capability |
