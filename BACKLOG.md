# SwiftGraph — Backlog

> Last verified against code: 2026-03-13
> Status: v0.1–v0.5 all complete. Remaining items are improvements, infra, and bugs from testing.

## Completed Milestones

- [x] **v0.1 — MVP: Graph** — IndexStore FFI, tree-sitter, pipeline, 8 MCP tools, CLI
- [x] **v0.2 — Intelligence** — context, impact, diff-impact, extensions, conformances
- [x] **v0.3 — Audit Engine** — 19 rules (CONC/MEM/SEC), parallel runner, text/json/sarif
- [x] **v0.4 — Analysis** — complexity, dead-code, cycles, coupling, architecture, imports, boundaries
- [x] **v0.5 — Production** — 9 audit categories (SUI/ARCH/NRG/NET/COD/STR/A11Y/TST/MOD), SARIF, watch mode

**Current state**: 22 MCP tools, 19 CLI subcommands, 943 files indexed (~/git/ios), 7202 nodes, 43567 edges

---

## Open Items — Bugs & Quick Fixes

### P0 — Bugs found in testing (2026-03-13) — ALL FIXED ✓

- [x] **FTS5 search: prefix matching** — auto-`*` suffix + LIKE fallback on 0 results
- [x] **FTS5 search: `--kind` filter** — post-filter applied in FTS5 path
- [x] **FTS5 search: wildcard `*`** — treated as "list all"
- [x] **Add `callees` CLI subcommand** — added, parity with MCP tool
- [x] **Audit max_issues per-category** — even distribution across categories

### P1 — Quality improvements

- [ ] **Energy audit rules too narrow** — NRG-001..006 exist but find 0 issues on 943-file production project. Rules check literal patterns (e.g., `Timer` with interval < 1s, `startUpdatingLocation` without `activityType`) but real code uses wrappers/abstractions. Need broader matching: Timer.publish, DispatchSourceTimer, CLLocationManager patterns, background fetch intervals.
  - File: `crates/swiftgraph-audit/src/rules/energy.rs` (6 rules, 333 lines)

- [ ] **No critical-severity audit findings** — `audit --min-severity critical` returns 0 on real project. Only CONC-003 (Task.detached) and SEC-001 (hardcoded secrets) are critical. Verify these rules fire on real patterns.
  - Files: `crates/swiftgraph-audit/src/rules/concurrency.rs`, `security.rs`

---

## Open Items — New Features

### P1 — Search & Intelligence

- [ ] **FTS5 ranking by symbol importance** — ORDER BY rank (BM25), weighted fields (name > qualified_name > signature), boost by fan-in/complexity score.
  - File: `crates/swiftgraph-core/src/storage/queries.rs:68-82`

- [ ] **FTS5 trigram tokenizer** — for substring matching without prefix requirement. Requires `tokenize='trigram'` in CREATE VIRTUAL TABLE.
  - File: `crates/swiftgraph-core/src/storage/schema.rs:70-77`

### P2 — Infrastructure

- [ ] **Homebrew formula** — `brew install swiftgraph`. Need Formula .rb file + tap repo.

- [ ] **In-memory LRU cache** — optional cache layer for hot-path queries (search, callers, callees). Bypass SQLite for repeated lookups. No dependency added yet.

### P3 — Deep Analysis

- [ ] **swift-syntax subprocess** — `swiftgraph-parser` Swift CLI for deeper AST checks. Crate `crates/swiftgraph-parser/` does not exist yet. Would enable: macro expansion, type inference, full expression analysis.

---

## Open Items — Tech Debt

- [ ] **Benchmark suite** — criterion-based benchmarks for indexing throughput and query latency. No `benches/` dir or criterion dependency exists.

- [ ] **Integration test fixtures** — `tests/fixtures/` exists but is empty. Need small Swift projects per feature area (conformances, extensions, concurrency patterns, etc.)

---

## Summary: 9 open items (5 P0 closed)

| # | Priority | Item | Type | Status |
|---|----------|------|------|--------|
| ~~1~~ | ~~P0~~ | ~~FTS5 prefix matching~~ | ~~Bug~~ | DONE |
| ~~2~~ | ~~P0~~ | ~~FTS5 --kind filter~~ | ~~Bug~~ | DONE |
| ~~3~~ | ~~P0~~ | ~~FTS5 wildcard *~~ | ~~Bug~~ | DONE |
| ~~4~~ | ~~P0~~ | ~~callees CLI subcommand~~ | ~~Missing feature~~ | DONE |
| ~~5~~ | ~~P0~~ | ~~Audit per-category cap~~ | ~~Bug~~ | DONE |
| 6 | P1 | Energy audit rules — broader patterns | Quality | |
| 7 | P1 | Verify critical-severity rules fire | Quality | |
| 8 | P1 | FTS5 ranking (BM25 + importance) | Enhancement | |
| 9 | P1 | FTS5 trigram tokenizer | Enhancement | |
| 10 | P2 | Homebrew formula | Distribution | |
| 11 | P2 | In-memory LRU cache | Performance | |
| 12 | P3 | swift-syntax subprocess | New feature | |
| 13 | P3 | Benchmark suite (criterion) | Testing | |
| 14 | P3 | Integration test fixtures | Testing | |
