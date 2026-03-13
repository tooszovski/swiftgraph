# SwiftGraph — Backlog

> Last verified against code: 2026-03-13
> Status: v0.1–v0.5 all complete. 13/14 items closed. 1 remaining (swift-syntax subprocess).

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

### P1 — Quality improvements — ALL DONE ✓

- [x] **Energy audit rules broadened** — Added NRG-007 (short asyncAfter), NRG-008 (CLLocationManager without desiredAccuracy), expanded NRG-001 to <= 1s intervals, NRG-004 to detect repeatForever. 0 → 18 findings on production project.
- [x] **No critical-severity audit findings** — `audit --min-severity critical` returns 0. Project legitimately has no `Task.detached` or hardcoded secrets. Rules are correct; the project is clean.
- [x] **FTS5 BM25 ranking** — ORDER BY bm25() with weighted fields: name(10x) > qualified_name(5x) > signature(1x)
- [x] **FTS5 trigram tokenizer** — node_trigram table for substring matching ("Delegate" → "AppDelegate")

---

## Open Items — New Features

### P2 — Infrastructure — ALL DONE ✓

- [x] **Homebrew formula** — `Formula/swiftgraph.rb` with SHA256 placeholder. Needs tap repo for distribution.
- [x] **In-memory LRU cache** — 256-entry `ResponseCache` in `server.rs` for hot-path queries (search, callers, callees). Invalidated on reindex.

### P3 — Deep Analysis

- [ ] **swift-syntax subprocess** — `swiftgraph-parser` Swift CLI for deeper AST checks. Crate `crates/swiftgraph-parser/` does not exist yet. Would enable: macro expansion, type inference, full expression analysis.

---

## Closed Items — Tech Debt ✓

- [x] **Benchmark suite** — `crates/swiftgraph-core/benches/indexing.rs` with Criterion: FTS5 search, trigram search, LIKE search, tree-sitter parsing.
- [x] **Integration test fixtures** — `crates/swiftgraph-core/tests/fixtures/` with basic (4 Swift files: Models, Services, ViewModel, Extensions) and concurrency (Actors.swift) projects. 2 integration tests passing.

---

## Summary: 13 closed, 1 open

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
| ~~10~~ | ~~P2~~ | ~~Homebrew formula~~ | ~~Distribution~~ | DONE |
| ~~11~~ | ~~P2~~ | ~~In-memory LRU cache~~ | ~~Performance~~ | DONE |
| 12 | P3 | swift-syntax subprocess | New feature | DEFERRED |
| ~~13~~ | ~~P3~~ | ~~Benchmark suite~~ | ~~Testing~~ | DONE |
| ~~14~~ | ~~P3~~ | ~~Integration test fixtures~~ | ~~Testing~~ | DONE |
