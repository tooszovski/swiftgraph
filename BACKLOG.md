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

- [x] **Energy audit rules broadened** — Added NRG-007 (short asyncAfter), NRG-008 (CLLocationManager without desiredAccuracy), expanded NRG-001 to <= 1s intervals, NRG-004 to detect repeatForever. 0 → 18 findings on production project.
- [ ] **No critical-severity audit findings** — `audit --min-severity critical` returns 0. Project legitimately has no `Task.detached` or hardcoded secrets. Rules are correct; the project is clean.

---

## Open Items — New Features

### P1 — Search & Intelligence (ALL DONE)

- [x] **FTS5 BM25 ranking** — ORDER BY bm25() with weighted fields: name(10x) > qualified_name(5x) > signature(1x)
- [x] **FTS5 trigram tokenizer** — node_trigram table for substring matching ("Delegate" → "AppDelegate")

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

## Summary: 5 open items (9 closed)

| # | Priority | Item | Type | Status |
|---|----------|------|------|--------|
| ~~1~~ | ~~P0~~ | ~~FTS5 prefix matching~~ | ~~Bug~~ | DONE |
| ~~2~~ | ~~P0~~ | ~~FTS5 --kind filter~~ | ~~Bug~~ | DONE |
| ~~3~~ | ~~P0~~ | ~~FTS5 wildcard *~~ | ~~Bug~~ | DONE |
| ~~4~~ | ~~P0~~ | ~~callees CLI subcommand~~ | ~~Missing feature~~ | DONE |
| ~~5~~ | ~~P0~~ | ~~Audit per-category cap~~ | ~~Bug~~ | DONE |
| ~~6~~ | ~~P1~~ | ~~Energy audit rules~~ | ~~Quality~~ | DONE |
| ~~7~~ | ~~P1~~ | ~~Critical-severity rules~~ | ~~Quality~~ | OK (project clean) |
| ~~8~~ | ~~P1~~ | ~~FTS5 BM25 ranking~~ | ~~Enhancement~~ | DONE |
| ~~9~~ | ~~P1~~ | ~~FTS5 trigram tokenizer~~ | ~~Enhancement~~ | DONE |
| 10 | P2 | Homebrew formula | Distribution | |
| 11 | P2 | In-memory LRU cache | Performance | |
| 12 | P3 | swift-syntax subprocess | New feature | |
| 13 | P3 | Benchmark suite (criterion) | Testing | |
| 14 | P3 | Integration test fixtures | Testing | |
