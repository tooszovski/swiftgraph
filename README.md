# SwiftGraph

A Rust-based [MCP](https://modelcontextprotocol.io) server that builds a code graph from Swift projects using Xcode Index Store and tree-sitter. Provides navigation, static analysis, architecture detection, and AI-oriented context tools — all accessible as MCP tools or CLI commands.

Built for AI-assisted iOS development: give your coding agent deep understanding of a Swift codebase without reading every file.

---

## Installation

### Homebrew (recommended)

```bash
brew tap tooszovski/swiftgraph https://github.com/tooszovski/swiftgraph
brew install swiftgraph
```

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

Then restart Claude Code. Run `/mcp` to confirm `swiftgraph` is connected and 22 tools are available.

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
- **Static analysis** — 66 audit rules catch concurrency bugs, memory leaks, security issues before review

All through the Model Context Protocol — works with Claude Code, Cursor, Windsurf, or any MCP client.

### Performance

Tested on a production iOS app (943 Swift files):

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
swiftgraph audit         Static analysis (66 rules, 12 categories)
swiftgraph watch         Auto-reindex on file changes
swiftgraph serve         Start MCP server
```

## MCP Tools (22)

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

## Examples

### Search

```bash
$ swiftgraph search "ViewModel"
{
  "results": [
    {
      "name": "CalendarMainViewModel",
      "kind": "class",
      "location": { "file": "Sources/Flows/Calendar/CalendarMainViewModel.swift", "line": 10 },
      "signature": "@MainActor"
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
    { "signal": "ViewModel/VM suffix", "count": 179 },
    { "signal": "Coordinator", "count": 16 },
    { "signal": "Router", "count": 29 }
  ]
}
```

### Impact Analysis

```bash
$ swiftgraph diff-impact --git-ref "HEAD~1..HEAD"
{
  "changed_files": [
    "Sources/Core/SigurManager.swift",
    "Sources/Flows/Pass/BluetoothPassView.swift",
    "Sources/Flows/Pass/BluetoothPassViewModel.swift"
  ],
  "changed_symbols": 15,
  "total_impact": 42,
  "risk_level": "medium"
}
```

### Task Context for AI

```bash
$ swiftgraph context "add push notifications"
{
  "keywords": ["push", "notifications"],
  "nodes": [
    { "name": "MainRouterDestination", "kind": "class", "relevance": 0.85 },
    { "name": "AppDelegate", "kind": "class", "relevance": 0.72 },
    ...
  ]
}
```

### Audit

```bash
$ swiftgraph audit --categories concurrency
Audit: 24 issues (0 critical, 20 high, 4 medium, 0 low)

[HIGH] CONC-001 (MCalendarView.swift:3): `MCalendarView` inherits UIViewController
       but is missing @MainActor
  Fix: Add @MainActor to the class declaration

[HIGH] CONC-002 (CalendarSearchViewModel.swift:62): Task captures `self` strongly
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
      "target_symbol": "ServiceOrderBooleanView"
    },
    ...
  ],
  "total_violations": 127
}
```

## Audit Rules (66 rules, 12 categories)

| Category | Rules | Examples |
|----------|-------|---------|
| Concurrency | CONC-001..007 | Missing @MainActor, unsafe Task capture, Sendable violations |
| Memory | MEM-001..006 | Retain cycles, strong delegates, timer leaks, KVO cleanup |
| Security | SEC-001..006 | Hardcoded secrets, insecure storage, ATS bypass, cert pinning |
| SwiftUI Perf | SUI-001..006 | Complex bodies, heavy onAppear, non-lazy lists |
| SwiftUI Arch | ARCH-001..005 | Logic in views, massive bodies, property wrapper misuse |
| Networking | NET-001..006 | Deprecated APIs, missing error handling, reachability anti-patterns |
| Codable | COD-001..005 | JSONSerialization, `try?` swallowing errors, date handling |
| Energy | NRG-001..006 | Frequent timers, polling, continuous location, animation leaks |
| Storage | STR-001..004 | Wrong directories, backup exclusion, file protection |
| Accessibility | A11Y-001..004 | Missing labels, Dynamic Type, color-only information |
| Testing | TST-001..005 | sleep() in tests, missing assertions, shared state |
| Modernization | MOD-001..005 | ObservableObject to @Observable, NavigationView to NavigationStack |

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
  "include": ["Sources/**/*.swift", "Tests/**/*.swift"],
  "exclude": ["**/Generated/**", "**/Pods/**", "**/.build/**"],
  "index_store_path": "auto"
}
```

### Index Store

SwiftGraph works in two modes:

- **tree-sitter only** (default) — no build required, parses Swift source directly. Captures declarations, call edges, conformances, extensions.
- **Index Store + tree-sitter** — if your project has been built with Xcode, SwiftGraph reads the Index Store for compiler-accurate symbol data and augments with tree-sitter. Set `index_store_path` to `"auto"` (discovers via `xcrun`) or provide an explicit path.

## Architecture

```
swiftgraph/
├── crates/
│   ├── swiftgraph-core/     Graph model, SQLite storage, indexing pipeline, analysis
│   ├── swiftgraph-audit/    Audit engine, 12 rule categories, SARIF/JSON/text output
│   └── swiftgraph-mcp/      MCP server (rmcp), CLI (clap), tool handlers
```

| Component | Technology |
|-----------|-----------|
| Language | Rust (~11k lines) |
| MCP SDK | [rmcp](https://github.com/anthropics/rust-sdk) v1.2 |
| Index Store | libIndexStore C FFI (dlopen at runtime) |
| AST Parsing | [tree-sitter-swift](https://github.com/alex-pinkus/tree-sitter-swift) v0.7 |
| Storage | SQLite + FTS5 ([rusqlite](https://github.com/nickel-organic/rusqlite)) |
| Git | [gitoxide](https://github.com/GitoxideLabs/gitoxide) |
| Parallelism | [rayon](https://github.com/rayon-rs/rayon) (data) + [tokio](https://github.com/tokio-rs/tokio) (async) |

## Requirements

- **macOS** (Index Store is Apple-only; tree-sitter mode works on any platform but is primarily tested on macOS)
- **Rust** 1.75+ (`rustup` to install)
- **Xcode** (optional, for Index Store data)

## Building

```bash
cargo build --workspace --release
# Binary at target/release/swiftgraph (12MB, static)
```

## Development

```bash
cargo build --workspace           # Build
cargo test --workspace            # Test (17 tests)
cargo clippy --workspace -- -D warnings  # Lint (zero warnings policy)
cargo fmt --all                   # Format
```

## License

MIT
