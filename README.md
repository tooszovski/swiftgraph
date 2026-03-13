# SwiftGraph

A Rust-based MCP server that builds compiler-accurate code graphs from Swift projects using Xcode Index Store + tree-sitter. Provides static analysis, audit checks, and AI-oriented context tools for Swift codebases.

## Features

- **Code Graph** — Builds a full dependency graph (nodes, edges) from Swift source files
- **Index Store Integration** — Uses Xcode's Index Store for compiler-accurate symbol data, with tree-sitter fallback
- **22 MCP Tools** — Search, navigate, analyze, and audit Swift code via Model Context Protocol
- **Static Analysis** — 65+ audit rules across 12 categories (concurrency, memory, security, SwiftUI, energy, networking, etc.)
- **Architecture Analysis** — Pattern detection (MVVM/VIPER/TCA), coupling metrics, boundary enforcement, cycle detection
- **Impact Analysis** — Blast radius for symbol changes, git diff-based impact, dead code detection
- **Incremental** — SHA256-based change detection, only re-indexes modified files

## Quick Start

```bash
# Build
cargo build --workspace --release

# Initialize in a Swift project
swiftgraph init --project /path/to/ios-app

# Index the project
swiftgraph index --project /path/to/ios-app

# Run as MCP server
swiftgraph serve --mcp --project /path/to/ios-app

# Search for symbols
swiftgraph search "ViewModel"

# Run audit
swiftgraph audit --categories concurrency,memory --min-severity medium

# Watch for changes
swiftgraph watch --project /path/to/ios-app
```

## MCP Server Configuration

### Claude Code

Add to your Claude Code settings:

```json
{
  "mcpServers": {
    "swiftgraph": {
      "command": "swiftgraph",
      "args": ["serve", "--mcp", "--project", "/path/to/your/project"]
    }
  }
}
```

## MCP Tools

| Tool | Description |
|------|-------------|
| `swiftgraph_status` | Index status, project info, statistics |
| `swiftgraph_reindex` | Trigger incremental or full reindex |
| `swiftgraph_search` | FTS5 full-text search for symbols |
| `swiftgraph_node` | Get detailed info about a symbol |
| `swiftgraph_callers` | Find all callers of a symbol |
| `swiftgraph_callees` | Find all callees of a symbol |
| `swiftgraph_references` | Find all references to a symbol |
| `swiftgraph_hierarchy` | Type hierarchy (subtypes/supertypes) |
| `swiftgraph_files` | List indexed files with stats |
| `swiftgraph_extensions` | Find extensions of a type |
| `swiftgraph_conformances` | Protocol conformance queries |
| `swiftgraph_context` | Task-based context builder |
| `swiftgraph_impact` | Blast radius analysis for a symbol |
| `swiftgraph_diff_impact` | Git diff-based impact analysis |
| `swiftgraph_complexity` | Fan-in/fan-out structural complexity |
| `swiftgraph_dead_code` | Unreachable symbol detection |
| `swiftgraph_cycles` | File-level dependency cycle detection |
| `swiftgraph_coupling` | Module coupling metrics (Ca/Ce/instability) |
| `swiftgraph_architecture` | Architecture pattern detection |
| `swiftgraph_imports` | Module dependency graph |
| `swiftgraph_boundaries` | Architecture boundary enforcement |
| `swiftgraph_audit` | Static analysis audit (12 categories, 65+ rules) |

## Audit Categories

| Category | Rules | Description |
|----------|-------|-------------|
| Concurrency | CONC-001..007 | Missing @MainActor, Task captures, Sendable, actor isolation |
| Memory | MEM-001..006 | Retain cycles, strong delegates, timer leaks, KVO cleanup |
| Security | SEC-001..006 | Hardcoded secrets, insecure storage, ATS bypass, cert pinning |
| SwiftUI Performance | SUI-001..006 | Complex bodies, heavy onAppear, non-lazy lists, expensive renders |
| SwiftUI Architecture | ARCH-001..005 | Logic in views, massive bodies, @EnvironmentObject, @Published |
| Networking | NET-001..006 | Deprecated APIs, missing error handling, reachability anti-patterns |
| Codable | COD-001..005 | JSONSerialization, try? decoding, date strategy, CodingKeys |
| Energy | NRG-001..006 | Frequent timers, polling, continuous location, animation leaks |
| Storage | STR-001..004 | Wrong directories, backup exclusion, file protection |
| Accessibility | A11Y-001..004 | Missing labels, Dynamic Type, color-only info, touch targets |
| Testing | TST-001..005 | sleep() in tests, missing assertions, shared state, migration |
| Modernization | MOD-001..005 | ObservableObject → @Observable, NavigationView → NavigationStack |

### Output Formats

```bash
# Text (default)
swiftgraph audit --format text

# JSON
swiftgraph audit --format json

# SARIF (for CI/CD integration with GitHub Code Scanning)
swiftgraph audit --format sarif > results.sarif
```

## Configuration

Create `.swiftgraph/config.json` in your project root (or run `swiftgraph init`):

```json
{
  "version": 1,
  "include": ["Sources/**/*.swift", "Tests/**/*.swift"],
  "exclude": ["**/Generated/**", "**/Pods/**", "**/.build/**"],
  "index_store_path": "auto"
}
```

## Architecture

```
swiftgraph/
├── crates/
│   ├── swiftgraph-core/     # Graph model, storage, indexing pipeline, analysis
│   ├── swiftgraph-audit/    # Audit engine, 12 rule categories, output formatters
│   └── swiftgraph-mcp/      # MCP server (rmcp), CLI, tool handlers
```

| Component | Technology |
|-----------|-----------|
| Language | Rust |
| MCP SDK | rmcp v1.2 |
| Index Store | libIndexStore C FFI (dlopen) |
| AST Parsing | tree-sitter-swift v0.7 |
| Storage | SQLite + FTS5 (rusqlite) |
| Git | gitoxide (gix) |
| Parallelism | rayon + tokio |

## Development

```bash
cargo build --workspace          # Build
cargo test --workspace           # Test
cargo clippy --workspace -- -D warnings  # Lint
cargo fmt --all                  # Format
```

## License

MIT
