# SwiftGraph

A Rust-based [MCP](https://modelcontextprotocol.io) server that builds a code graph from Swift projects using Xcode Index Store and tree-sitter. Provides navigation, static analysis, architecture detection, and AI-oriented context tools — all accessible as MCP tools or CLI commands.

Built for AI-assisted iOS development: give your coding agent deep understanding of a Swift codebase without reading every file.

---

## Installation

### Homebrew (recommended)

```bash
brew tap tooszovski/tap
brew install swiftgraph
```

The formula lives in [tooszovski/homebrew-tap](https://github.com/tooszovski/homebrew-tap) and builds from source with the committed `Cargo.lock`. Releases published by the release workflow also carry a prebuilt Apple Silicon binary on the [GitHub releases](https://github.com/tooszovski/swiftgraph/releases) page.

After installation, verify:

```bash
swiftgraph --help
```

### MCP Setup for Claude Code

Add to `.mcp.json` in your iOS project root:

**Apple Silicon (M1/M2/M3/M4):**

```json
{
  "mcpServers": {
    "swiftgraph": {
      "command": "/opt/homebrew/bin/swiftgraph",
      "args": ["serve", "--mcp"]
    }
  }
}
```

**Intel Mac:**

```json
{
  "mcpServers": {
    "swiftgraph": {
      "command": "/usr/local/bin/swiftgraph",
      "args": ["serve", "--mcp"]
    }
  }
}
```

> **Note:** Full path is required — Claude Code spawns MCP servers without loading your shell profile, so `PATH` may not include Homebrew directories.

To point at a specific project (e.g. from a global config `~/.claude/mcp.json`):

```json
{
  "mcpServers": {
    "swiftgraph": {
      "command": "/opt/homebrew/bin/swiftgraph",
      "args": ["serve", "--mcp", "--project", "/path/to/ios-project"]
    }
  }
}
```

Then restart Claude Code. Run `/mcp` to confirm `swiftgraph` is connected and 23 tools are available.

### First Run

```bash
cd /path/to/ios-project
swiftgraph init      # creates .swiftgraph/config.json
swiftgraph index     # indexes Swift files (tree-sitter; adds Index Store if Xcode build exists)
```

### Cursor / Windsurf

Add to MCP settings with the same command and args. Use the full Homebrew path as shown above.

### Build from Source

```bash
git clone https://github.com/tooszovski/swiftgraph.git
cd swiftgraph
cargo build --workspace --release
# Binary at target/release/swiftgraph
```

---

## Why

LLMs working with large Swift codebases need more than text search. SwiftGraph gives them:

- **A dependency graph** — who calls what, who conforms to what, who extends what
- **Targeted context** — "I need to add push notifications" → here are the 25 most relevant symbols
- **Impact analysis** — "If I change this class, what breaks?" → blast radius with affected files and tests
- **Architecture awareness** — auto-detects MVVM/VIPER/TCA, enforces layer boundaries
- **Static analysis** — 73 audit rules catch concurrency bugs, memory leaks, security issues before review

All through the Model Context Protocol — works with Claude Code, Cursor, Windsurf, or any MCP client.

### Performance

Measured with v0.5 on a production iOS app (943 Swift files):

| Operation | Time |
|-----------|------|
| Full index | 5.3s |
| Incremental reindex | 0.5s |
| Search query | <120ms |
| Full audit (12 categories) | 0.8s |

## CLI Commands

```
swiftgraph init          Initialize .swiftgraph/ config
swiftgraph index         Index Swift files (--force for full reindex)
swiftgraph search        Search symbols by name, filter by kind
swiftgraph callers       Find callers of a symbol
swiftgraph callees       Find callees of a symbol
swiftgraph hierarchy     Type hierarchy (subtypes/supertypes)
swiftgraph context       Build task-relevant context for AI
swiftgraph impact        Blast radius analysis for a symbol
swiftgraph diff-impact   Impact analysis from git diff
swiftgraph complexity    Fan-in/fan-out structural complexity
swiftgraph dead-code     Find unreachable symbols
swiftgraph cycles        Detect dependency cycles
swiftgraph coupling      Module coupling metrics (Ca/Ce/instability)
swiftgraph architecture  Detect or validate architecture pattern
swiftgraph imports       Module dependency graph
swiftgraph boundaries    Check architecture boundary rules
swiftgraph audit         Static analysis (73 rules, 13 categories)
swiftgraph watch         Auto-reindex on file changes
swiftgraph serve         Start MCP server
```

Symbol arguments accept a USR/node ID or a plain name (`UserService`, `load` or `load(id:)`); names resolve to the best exact match, then to a prefix/substring match.

## MCP Tools (23)

All CLI commands are also available as MCP tools with `swiftgraph_` prefix, plus a few extras:

| Tool | Description |
|------|-------------|
| `swiftgraph_status` | Index status and project statistics |
| `swiftgraph_reindex` | Trigger incremental or full reindex |
| `swiftgraph_search` | Full-text search for symbols |
| `swiftgraph_node` | Detailed info about a specific symbol |
| `swiftgraph_callers` | Find all callers of a symbol |
| `swiftgraph_callees` | Find all callees of a symbol |
| `swiftgraph_references` | Find all references to a symbol |
| `swiftgraph_hierarchy` | Type hierarchy (subtypes/supertypes) |
| `swiftgraph_files` | List indexed files with stats |
| `swiftgraph_extensions` | Find extensions of a type |
| `swiftgraph_conformances` | Protocol conformance queries |
| `swiftgraph_context` | Task-based context builder for AI |
| `swiftgraph_impact` | Blast radius for symbol changes |
| `swiftgraph_diff_impact` | Git diff-based impact analysis |
| `swiftgraph_complexity` | Structural complexity metrics |
| `swiftgraph_dead_code` | Unreachable symbol detection |
| `swiftgraph_cycles` | File-level dependency cycles |
| `swiftgraph_coupling` | Module coupling (Ca/Ce/instability/abstractness) |
| `swiftgraph_architecture` | Architecture pattern detection |
| `swiftgraph_imports` | Module dependency graph |
| `swiftgraph_boundaries` | Architecture boundary enforcement |
| `swiftgraph_audit` | Static analysis audit |
| `swiftgraph_concurrency` | Isolation, Sendable, cross-actor calls and mutable state of a symbol |

## Examples

### Search

```bash
$ swiftgraph search "ViewModel"
{
  "results": [
    {
      "name": "ProfileViewModel",
      "kind": "class",
      "location": { "file": "Sources/Features/Profile/ProfileViewModel.swift", "line": 10 },
      "attributes": ["@MainActor"]
    },
    ...
  ],
  "total": 179
}
```

### Architecture Detection

```bash
$ swiftgraph architecture
{
  "detected_pattern": "MVVM+Coordinator",
  "confidence": 0.48,
  "evidence": [
    { "pattern": "MVVM+Coordinator", "signal": "ViewModel/VM suffix", "count": 179 },
    { "pattern": "MVVM+Coordinator", "signal": "Coordinator", "count": 16 }
  ],
  "violations": [],
  "layer_stats": [ ... ]
}
```

### Impact Analysis

```bash
$ swiftgraph diff-impact --git-ref "HEAD~1..HEAD"
{
  "git_ref": "HEAD~1..HEAD",
  "changed_files": [
    "Sources/Core/AccessManager.swift",
    "Sources/Features/Pass/PassView.swift",
    "Sources/Features/Pass/PassViewModel.swift"
  ],
  "changed_symbols": ["s:3App13AccessManagerC", ...],
  "total_direct_impact": 15,
  "total_transitive_impact": 42,
  "affected_files": [ ... ],
  "affected_tests": [ ... ],
  "risk_level": "medium"
}
```

### Task Context for AI

```bash
$ swiftgraph context "add push notifications"
{
  "keywords": ["push", "notifications"],
  "nodes": [
    { "name": "AppDelegate", "kind": "class", "score": 64.0, ... },
    { "name": "NotificationService", "kind": "class", "score": 57.0, ... },
    ...
  ],
  "files": [ ... ],
  "architecture": "MVVM"
}
```

### Audit

```bash
$ swiftgraph audit --categories concurrency
Audit: 24 issues (0 critical, 20 high, 4 medium, 0 low)

[HIGH] CONC-001 (CalendarViewController.swift:3): `CalendarViewController` inherits UIViewController
       but is missing @MainActor
  Fix: Add @MainActor to the class declaration

[HIGH] CONC-002 (SearchViewModel.swift:62): Task captures `self` strongly
       — may cause retain cycle
  Fix: Use `[weak self]` capture list
```

### Boundary Enforcement

Define layer rules in a JSON config:

```json
{
  "layers": [
    { "name": "Views", "pattern": "**/Views/**" },
    { "name": "Models", "pattern": "**/Models/**" },
    { "name": "Services", "pattern": "**/Services/**" }
  ],
  "rules": [
    { "from": "Models", "to": "Views", "allowed": false },
    { "from": "Services", "to": "Views", "allowed": false }
  ]
}
```

```bash
$ swiftgraph boundaries --config boundaries.json
{
  "violations": [
    {
      "source_layer": "Services",
      "target_layer": "Views",
      "source_symbol": "makeContentView",
      "target_symbol": "OrderSummaryView"
    },
    ...
  ],
  "total_violations": 127
}
```

## Audit Rules (73 rules, 13 categories)

| Category | Rules | Examples |
|----------|-------|---------|
| Concurrency | CONC-001..007 | Missing @MainActor, unsafe Task capture, Sendable violations |
| Memory | MEM-001..006 | Retain cycles, strong delegates, timer leaks, KVO cleanup |
| Security | SEC-001..006 | Hardcoded secrets, insecure storage, ATS bypass, cert pinning |
| SwiftUI Perf | SUI-001..006 | Complex bodies, heavy onAppear, non-lazy lists |
| SwiftUI Arch | ARCH-001..005 | Logic in views, massive bodies, property wrapper misuse |
| Networking | NET-001..006 | Deprecated APIs, missing error handling, reachability anti-patterns |
| Codable | COD-001..005 | JSONSerialization, `try?` swallowing errors, date handling |
| Energy | NRG-001..008 | Frequent timers, polling, continuous location, animation leaks, short asyncAfter |
| Storage | STR-001..004 | Wrong directories, backup exclusion, file protection |
| Accessibility | A11Y-001..004 | Missing labels, Dynamic Type, color-only information |
| Testing | TST-001..005 | sleep() in tests, missing assertions, shared state |
| Modernization | MOD-001..005 | ObservableObject to @Observable, NavigationView to NavigationStack |
| Swift Performance | PERF-001..006 | Large value copies, excessive ARC, existentials in collections, actor hops in loops |

Rules whose findings are style or optimization suggestions rather than defects have severity `advisory` and are hidden by default; `swiftgraph audit --include-advisory` (MCP `include_advisory: true`, or `--min-severity advisory`) shows them, and `audit.severity` can promote one, e.g. `{"PERF-006": "low"}`. Migrations to iOS 17 APIs (`@Observable`, two-parameter `onChange`) are skipped when the deployment target read from `*.pbxproj`, `Package.swift` or `project.yml` is lower. Rules that depend on conventions report their confidence through severity: CONC-001 is high only when the class has asynchronous code, SUI-005 is low outside a `ScrollView`. Conformances are resolved across files, so ARCH-005 and PERF-001 see `protocol CoordinatorObject: ObservableObject` or `protocol ApiResponse: Decodable` declared elsewhere. When `max_issues` cuts the list, JSON output has `"truncated": true` and `found_issues`.

### Output Formats

```bash
swiftgraph audit                          # Text (human-readable)
swiftgraph audit --format json            # JSON (for tooling)
swiftgraph audit --format sarif > out.sarif  # SARIF (GitHub Code Scanning, SonarQube)
```

## Configuration

`swiftgraph init` creates `.swiftgraph/config.json`:

```json
{
  "version": 1,
  "include": [],
  "exclude": ["**/Generated/**", "**/Pods/**", "**/.build/**", "**/DerivedData/**"],
  "index_store_path": "auto",
  "resolution": { "max_candidates": 3 },
  "audit": { "disabled_rules": [], "severity": {} }
}
```

`audit.disabled_rules` turns rules off for the project (for example `["CONC-001"]` when view models hop to the main actor explicitly); `audit.severity` replaces a rule's severity, e.g. `{"SUI-005": "low"}`. The older `exclude_rules` key is read as `disabled_rules`.

An empty `include` means every `.swift` file under the project root that no `exclude` glob matches. Globs are matched against paths relative to the root, for example `"include": ["App/**/*.swift"]`.

If the Xcode project or `Package.swift` is not in the root (for example, it lives in `./ios`), SwiftGraph finds it up to three levels deep, skipping `.build`, `Pods`, `DerivedData` and `node_modules`. To pin it explicitly, add `"project_dir": "ios"`.

### Index Store

SwiftGraph works in two modes:

- **tree-sitter only** (default) — no build required, parses Swift source directly. Captures declarations, call edges, conformances, extensions.

  Without a compiler, call targets are resolved by receiver: `self.`/implicit calls go to the enclosing type, its extensions and supertypes; `Type.foo()` and variables with a declared type (`let x: T`, `let x = T()`, parameters, stored properties) go to members of that type; `private`/`fileprivate` symbols only match within their file, and argument labels must fit. A call on a receiver of unknown type gets edges only when at most `resolution.max_candidates` project members share the name; such edges are marked `ambiguous` and shown by `callers`/`callees`, but ignored by `cycles`, `complexity`, `impact`, `coupling` and `boundaries`. Common standard library, SwiftUI, UIKit and Combine names (`map`, `filter`, `frame`, `padding`, `sink`, ...) never resolve to project symbols through an unknown receiver. `dead-code` counts ambiguous callers and names referenced without a confident edge as possible uses, so it reports only symbols nothing could be referring to.
- **Index Store + tree-sitter** — if your project has been built with Xcode, SwiftGraph reads the Index Store for compiler-accurate symbol data and augments with tree-sitter. `index_store_path` accepts `"auto"` (default: `.build/index/store` or SwiftPM's `.build/<triple>/debug/index/store`, `~/Library/Developer/Xcode/DerivedData/<Project>-*/Index.noindex/DataStore` for Xcode), `"none"` to force tree-sitter only, or an explicit path (relative to the project root). `swiftgraph index --index-store-path` overrides it. `swiftgraph index`, `swiftgraph watch` and the `swiftgraph_reindex` tool all use the same resolution. The backend used by the last run is reported by `swiftgraph_status` as `indexStrategy` (`index-store`, `hybrid` or `tree-sitter`); switching backends triggers a full rebuild so tree-sitter and Index Store symbol IDs never mix.

## Architecture

```
swiftgraph/
├── crates/
│   ├── swiftgraph-core/     Graph model, SQLite storage, indexing pipeline, analysis
│   ├── swiftgraph-audit/    Audit engine, 13 rule categories, SARIF/JSON/text output
│   ├── swiftgraph-mcp/      MCP server (rmcp), CLI (clap), tool handlers
│   └── swiftgraph-parser/   Optional Swift CLI on swift-syntax (SwiftPM package, not a Cargo crate)
```

`swiftgraph-parser` enriches tree-sitter declarations and their nested members with attributes, doc comments, access levels and signatures, and adds import attributes such as `@testable`. SwiftGraph runs it once per index in batch mode (`--stdin`) and checks its protocol version with `--version`; `swiftgraph_status` shows the parser in use as `swiftSyntaxParser`. It does not yet run on files covered by the Index Store. It is optional: without it indexing works the same, minus that enrichment. Build it with `cd crates/swiftgraph-parser && swift build -c release` (Xcode 16+ / Swift 6 toolchain) and put the binary next to `swiftgraph`, on `PATH`, or point `SWIFTGRAPH_PARSER_PATH` at it.

| Component | Technology |
|-----------|-----------|
| Language | Rust (~15k lines incl. tests) |
| MCP SDK | [rmcp](https://github.com/modelcontextprotocol/rust-sdk) v1.8 |
| Index Store | libIndexStore C FFI (dlopen at runtime) |
| AST Parsing | [tree-sitter-swift](https://github.com/alex-pinkus/tree-sitter-swift) v0.7 |
| Storage | SQLite + FTS5 ([rusqlite](https://github.com/rusqlite/rusqlite)) |
| Git | `git` CLI (diff-impact) |
| Parallelism | [rayon](https://github.com/rayon-rs/rayon) (data) + [tokio](https://github.com/tokio-rs/tokio) (async) |

## Requirements

- **macOS** (Index Store is Apple-only; tree-sitter mode works on any platform but is primarily tested on macOS)
- **Rust** 1.96+ (`rustup` to install; the minimum comes from the locked dependency tree)
- **Xcode** (optional, for Index Store data; Xcode 16+ to build `swiftgraph-parser`)

## Building

```bash
cargo build --workspace --release
# Binary at target/release/swiftgraph
```

## Development

```bash
cargo build --workspace           # Build
cargo test --workspace            # Test (97 Rust tests; Index Store and real-parser tests are skipped without Xcode)
cd crates/swiftgraph-parser && swift test  # swift-syntax parser tests
cargo clippy --workspace --all-targets -- -D warnings  # Lint (zero warnings policy)
cargo fmt --all                   # Format
```

## License

MIT
