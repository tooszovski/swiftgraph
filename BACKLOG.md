# SwiftGraph — Backlog

## v0.1 — MVP: Graph (remaining)

### P0 — Must-have for v0.1 release

- [x] **libIndexStore C FFI bindings** — runtime-loaded (dlopen) bindings for 30+ `indexstore_*` functions, auto-discovers via xcrun
- [x] **Index Store reader** — reads units/records into GraphNode/GraphEdge with relation mapping (calledBy, baseOf, overrideOf, childOf, extendedBy)
- [x] **Semantic edge replacement** — pipeline prefers Index Store when available, falls back to tree-sitter (Hybrid/TreeSitter/IndexStore strategies)
- [x] **`swiftgraph_files` MCP tool** — list indexed files with stats, filterable by path prefix
- [x] **Real project integration test** — tested on ~/git/ios (941 files, 6824 nodes, 6140 edges)
- [x] **Cross-file call edges** — tree-sitter second-pass call extraction + name resolution pipeline

### P1 — Quality

- [x] **Error messages for missing Index Store** — clear user-facing guidance when degrading to tree-sitter-only mode
- [x] **CLI `serve` without `--mcp` flag** — provide useful help text or default behavior
- [x] **Config file loading** — read `.swiftgraph/config.json` include/exclude globs during indexing

---

## v0.2 — Intelligence

- [x] **`swiftgraph_context`** — task-based context builder: keyword extraction → FTS5 search → 2-level graph expansion → relevance ranking
- [x] **`swiftgraph_impact`** — blast radius analysis for a symbol: direct/transitive callers, affected files, affected tests, risk level
- [x] **`swiftgraph_diff_impact`** — git-based impact analysis: unstaged/staged/commit-range → changed symbols → blast radius
- [x] **`swiftgraph_extensions`** — find all extensions of a type (including cross-module)
- [x] **`swiftgraph_conformances`** — protocol conformance queries (who conforms, what does X conform to)
- [ ] **FTS5 search improvements** — trigram tokenizer, prefix queries, ranking by symbol importance

> Note: FTS5 basic search and incremental reindex (SHA256) are already implemented in v0.1.

---

## v0.3 — Audit Engine

- [x] **Audit rule framework** — AuditRule trait, parallel runner (rayon), tree-sitter pattern matching, severity filtering, category grouping
- [x] **Concurrency checks (CONC-001..004)** — missing @MainActor, unsafe Task capture, Task.detached actor isolation, actor hop in loop
- [x] **Memory checks (MEM-001..004)** — closure retain cycles, strong delegates, timer leaks, observer leaks
- [x] **Security checks (SEC-001..004)** — hardcoded secrets, insecure storage, sensitive logging, ATS bypass
- [x] **`swiftgraph_audit` MCP tool** — categories, min_severity, path_filter, max_issues
- [x] **CLI `swiftgraph audit`** — text/json/sarif output formats
- [x] **Additional CONC rules (005..007)** — Sendable violations, stored Task without cancel, nonisolated self access
- [x] **Additional MEM rules (005..006)** — KVO cleanup, PhotoKit accumulation
- [x] **Additional SEC rules (005..006)** — injectable format strings, missing certificate pinning
- [ ] **swift-syntax subprocess** — `swiftgraph-parser` Swift CLI for deeper AST checks (deferred)

---

## v0.4 — Analysis

- [x] **`swiftgraph_complexity`** — fan-in/fan-out metrics per symbol/file, sorted by structural complexity score
- [x] **`swiftgraph_dead_code`** — unreachable symbol detection (no incoming edges), excludes public API/tests/entry points
- [x] **`swiftgraph_cycles`** — file-level dependency cycle detection via DFS
- [x] **`swiftgraph_coupling`** — afferent/efferent coupling, instability, abstractness metrics between modules
- [x] **`swiftgraph_architecture`** — auto-detect architectural pattern, verify conformance
- [x] **`swiftgraph_imports`** — module dependency graph with visualization data
- [ ] **`swiftgraph_boundaries`** — configurable architecture boundary enforcement

---

## v0.5 — Production

### Additional audit categories

- [x] **SwiftUI performance (SUI-001..006)** — body complexity, unnecessary redraws, missing `Equatable`, heavy onAppear
- [x] **SwiftUI architecture (ARCH-001..005)** — logic in views, massive view bodies, improper property wrapper usage
- [x] **Energy (NRG-001..006)** — timer abuse, polling, continuous location, background mode misuse
- [x] **Networking checks (NET-001..006)** — deprecated APIs, missing error handling, hardcoded URLs
- [x] **Codable checks (COD-001..005)** — manual JSON, `try?` swallowing errors, date handling
- [x] **Storage checks (STR-001..004)** — wrong directories, missing backup exclusions, file protection
- [x] **Accessibility checks (A11Y-001..004)** — missing labels, Dynamic Type, color contrast
- [x] **Testing checks (TST-001..005)** — flaky patterns, missing assertions, shared state
- [x] **Modernization checks (MOD-001..005)** — deprecated APIs, migration opportunities (ObservableObject → @Observable)

### Infrastructure

- [x] **SARIF output** — CI/CD integration for audit results (GitHub Code Scanning, SonarQube)
- [x] **Watch mode** — FSEvents-based auto-reindex on file changes
- [ ] **Homebrew formula** — `brew install swiftgraph`
- [ ] **In-memory graph cache** — optional LRU cache for hot-path queries, bypass SQLite for repeated lookups
- [x] **Parallel audit execution** — rayon-based parallel rule evaluation across files

---

## Tech Debt / Cross-cutting

- [ ] Structured logging with `tracing` spans (currently only stderr init)
- [ ] Benchmark suite for indexing throughput and query latency
- [ ] CI pipeline (GitHub Actions) — build, test, clippy, fmt
- [ ] Integration test fixtures — small Swift projects per feature area
- [ ] Documentation — README with usage examples, architecture diagram
