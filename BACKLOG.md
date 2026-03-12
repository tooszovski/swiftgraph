# SwiftGraph — Backlog

## v0.1 — MVP: Graph (remaining)

### P0 — Must-have for v0.1 release

- [ ] **libIndexStore C FFI bindings** — bindgen-generated Rust bindings for 81 `indexstore_*` symbols from Xcode's `libIndexStore.dylib`
- [ ] **Index Store reader** — parse Index Store data into `GraphNode`/`GraphEdge`, write to SQLite. USR-based identifiers for compiler-accurate edges
- [ ] **Semantic edge replacement** — when Index Store is available, replace tree-sitter-inferred edges with compiler-accurate edges (calls, conformances, inheritance)
- [ ] **`swiftgraph_files` MCP tool** — list indexed files with stats (node count, edge count, last indexed timestamp)
- [ ] **Real project integration test** — test full pipeline on a real SPM project (e.g., swift-collections or Alamofire)

### P1 — Quality

- [ ] **Error messages for missing Index Store** — clear user-facing guidance when degrading to tree-sitter-only mode
- [ ] **CLI `serve` without `--mcp` flag** — provide useful help text or default behavior
- [ ] **Config file loading** — read `.swiftgraph/config.json` include/exclude globs during indexing

---

## v0.2 — Intelligence

- [ ] **`swiftgraph_context`** — task-based context builder: keyword extraction → FTS5 search → 2-level graph expansion → PageRank ranking → source code attachment
- [ ] **`swiftgraph_impact`** — blast radius analysis for a symbol: direct/transitive callers, affected files, affected tests, risk level
- [ ] **`swiftgraph_diff_impact`** — git-based impact analysis via `gix`: unstaged/staged/commit-range → changed symbols → blast radius
- [ ] **`swiftgraph_extensions`** — find all extensions of a type (including cross-module)
- [ ] **`swiftgraph_conformances`** — protocol conformance queries (who conforms, what does X conform to)
- [ ] **FTS5 search improvements** — trigram tokenizer, prefix queries, ranking by symbol importance

> Note: FTS5 basic search and incremental reindex (SHA256) are already implemented in v0.1.

---

## v0.3 — Audit Engine

- [ ] **swift-syntax subprocess** — `swiftgraph-parser` Swift CLI using SwiftSyntax for AST-level checks (concurrency, memory, security patterns)
- [ ] **Audit rule framework** — rule registration, severity filtering, category grouping, fix suggestions
- [ ] **Concurrency checks (CONC-001..007)**
  - CONC-001: missing `@MainActor` on UIViewController/ObservableObject/View
  - CONC-002: unsafe Task capture (`Task { self.property }` without `[weak self]`)
  - CONC-003: nonisolated self access
  - CONC-004: Sendable violations across actor boundaries (graph-based)
  - CONC-005: `@MainActor` property from `Task.detached`
  - CONC-006: stored Task without weak capture
  - CONC-007: actor hop in loop
- [ ] **Memory checks (MEM-001..006)** — retain cycles, timer leaks, delegate strong refs, closure captures, NotificationCenter observers, KVO cleanup
- [ ] **Security checks (SEC-001..006)** — hardcoded secrets, insecure storage, ATS bypass, plain-text logging, injectable format strings, missing certificate pinning
- [ ] **`swiftgraph_audit` MCP tool** — run checks by category/severity/path scope
- [ ] **CLI `swiftgraph audit`** — text/json output modes

---

## v0.4 — Analysis

- [ ] **`swiftgraph_complexity`** — cyclomatic + cognitive complexity, fan-in/fan-out per symbol/file/directory
- [ ] **`swiftgraph_dead_code`** — USR-based unreachable symbol detection (no incoming edges), with exclusions for public API, tests, entry points
- [ ] **`swiftgraph_cycles`** — cycle detection at file/type/module level
- [ ] **`swiftgraph_coupling`** — afferent/efferent coupling, instability, abstractness metrics between modules
- [ ] **`swiftgraph_architecture`** — auto-detect architectural pattern (MVVM, MVC, VIPER, Clean, TCA, MVVM+C, MVVM+Router), verify conformance
- [ ] **`swiftgraph_boundaries`** — configurable architecture boundary enforcement (from/deny/allow rules)
- [ ] **`swiftgraph_concurrency`** — deep concurrency analysis combining graph + AST data
- [ ] **`swiftgraph_imports`** — module dependency graph with visualization data

---

## v0.5 — Production

### Additional audit categories

- [ ] **SwiftUI performance (SUI-001..006)** — body complexity, unnecessary redraws, missing `Equatable`, heavy onAppear
- [ ] **SwiftUI architecture (ARCH-001..005)** — logic in views, massive view bodies, improper property wrapper usage
- [ ] **Energy (NRG-001..006)** — timer abuse, polling, continuous location, background mode misuse
- [ ] **Networking checks** — deprecated APIs, missing error handling, hardcoded URLs
- [ ] **Codable checks** — manual JSON, `try?` swallowing errors, date handling
- [ ] **Storage checks** — wrong directories, missing backup exclusions, file protection
- [ ] **Accessibility checks** — missing labels, Dynamic Type, color contrast
- [ ] **Testing checks** — flaky patterns, missing assertions, shared state
- [ ] **Modernization checks** — deprecated APIs, migration opportunities (ObservableObject → @Observable)

### Infrastructure

- [ ] **SARIF output** — CI/CD integration for audit results (GitHub Code Scanning, SonarQube)
- [ ] **Watch mode** — FSEvents-based auto-reindex on file changes
- [ ] **Homebrew formula** — `brew install swiftgraph`
- [ ] **In-memory graph cache** — optional LRU cache for hot-path queries, bypass SQLite for repeated lookups
- [ ] **Parallel audit execution** — rayon-based parallel rule evaluation across files

---

## Tech Debt / Cross-cutting

- [ ] Structured logging with `tracing` spans (currently only stderr init)
- [ ] Benchmark suite for indexing throughput and query latency
- [ ] CI pipeline (GitHub Actions) — build, test, clippy, fmt
- [ ] Integration test fixtures — small Swift projects per feature area
- [ ] Documentation — README with usage examples, architecture diagram
