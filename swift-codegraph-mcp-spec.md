# SwiftGraph MCP Server — Техническое задание

## 1. Проблема

Существующие code graph MCP-серверы (@colbymchenry/codegraph, @optave/codegraph, CodeGraphContext, narsil-mcp) используют **tree-sitter** для парсинга. Для Swift это даёт неполную картину:

- Не разрешает protocol conformance — невозможно найти все реализации протокола
- Не мёржит extensions с типами — extension отображается как отдельный узел
- Не различает overloads — все методы с одним именем сливаются
- Не видит `@MainActor`, `Sendable`, concurrency-аннотации
- Не раскрывает макросы (`@Observable`, `#Preview`, result builders)
- Не следует за кросс-модульными зависимостями (SPM, frameworks)
- Call graph строится по именам, а не по типам — ложные связи и пропуски

Ни один из существующих инструментов не предоставляет **статический анализ паттернов** (memory leaks, concurrency violations, energy anti-patterns), который AI-агент мог бы использовать при code review.

## 2. Решение

MCP-сервер **SwiftGraph**, который:
1. Строит граф кода из **Xcode Index Store** (compiler-accurate данные)
2. Дополняет его **swift-syntax** (структурный анализ без билда)
3. Предоставляет **статические проверки** Swift-паттернов (concurrency, memory, performance, security)
4. Работает с **любым** Swift-проектом (SPM, Xcode, mixed)

---

## 3. Рекомендации по языку реализации

### Анализ вариантов

| Критерий | Swift | Rust | TypeScript (Node) |
|----------|-------|------|-------------------|
| **Доступ к Index Store** | Нативный (libIndexStore.dylib, IndexStoreDB) | Через FFI к C API | Через FFI (node-ffi / NAPI) — хрупко |
| **swift-syntax** | Нативная зависимость, полная типизация AST | Нет доступа — только tree-sitter | Нет доступа — только tree-sitter |
| **Типобезопасность** | Строгая, compile-time, exhaustive switch | Строгая, borrow checker, zero-cost abstractions | Слабая (runtime ошибки, `any`) |
| **Производительность** | Высокая (AOT, zero-overhead) | Высокая (AOT, zero-cost) | Средняя (JIT, GC паузы) |
| **Memory safety** | ARC, value types, actors | Borrow checker, zero-cost | GC, но утечки через closures |
| **Экосистема MCP** | Нет готового MCP SDK — нужно написать JSON-RPC | Есть rmcp (официальный Rust MCP SDK) | Есть @modelcontextprotocol/sdk (официальный) |
| **Порог входа для Swift-разработчиков** | Нулевой | Высокий (другая парадигма) | Средний |
| **Кросс-платформа** | macOS only (Index Store — Apple-only) | macOS only (Index Store — Apple-only) | macOS only |
| **Скорость разработки** | Средняя | Медленная (борьба с borrow checker) | Быстрая |
| **Минимизация ошибок** | ★★★★☆ — строгая типизация, но ARC retain cycles возможны | ★★★★★ — compile-time memory safety | ★★☆☆☆ — runtime ошибки |

### Рекомендация: **Rust**

Несмотря на нативность Swift для Apple-инфраструктуры, **Rust** предпочтителен по следующим причинам:

1. **Минимизация ошибок** — borrow checker исключает целый класс багов (use-after-free, data races, dangling pointers) на этапе компиляции. Для инструмента, который работает с 10000+ узлов графа и конкурентными запросами — это критично.

2. **Готовый MCP SDK** — `rmcp` (official Rust MCP SDK) избавляет от написания JSON-RPC boilerplate. На Swift пришлось бы писать MCP-транспорт с нуля.

3. **Производительность парсинга** — Rust + tree-sitter-swift для syntax-only mode работает в 2-3x быстрее чем swift-syntax (C-based parser vs Swift-based).

4. **Доступ к Index Store** — libIndexStore имеет C API, который отлично биндится через Rust FFI (`bindgen`). IndexStoreDB не нужен — raw C API достаточен и даже предпочтителен (меньше зависимостей).

5. **Единый бинарник** — Rust компилируется в статический бинарник без runtime-зависимостей. Swift требует Swift runtime (обычно есть на macOS, но может отличаться по версии).

6. **Параллелизм** — Rayon для data-parallel парсинга файлов, tokio для async MCP-сервера. В Swift аналог — structured concurrency, но без гарантий data race safety на уровне компилятора (до Swift 6 strict mode).

### Гибридный подход

Для swift-syntax AST (который доступен только в Swift):
- **Вариант A**: Встроить swift-syntax как subprocess — отдельный Swift CLI (`swiftgraph-parser`) который принимает файл, выдаёт JSON с AST. Rust-сервер вызывает его.
- **Вариант B**: Использовать tree-sitter-swift в Rust для syntax-only mode (быстрее, но менее точно для новейшего синтаксиса).
- **Вариант C**: swift-syntax через C-интерфейс (SwiftSyntax не имеет C API — нежизнеспособно).

**Рекомендация**: Вариант A для полного режима, Вариант B для быстрого fallback.

### Итоговый стек

| Компонент | Технология | Обоснование |
|-----------|-----------|-------------|
| Основной сервер | **Rust** | Memory safety, производительность, rmcp SDK |
| MCP transport | **rmcp** (Rust MCP SDK) | Официальный SDK, JSON-RPC over stdin/stdout |
| Index Store reader | **libIndexStore C API** через bindgen | Compiler-accurate данные, стабильный C ABI |
| AST parser (full) | **swift-syntax** subprocess (Swift CLI) | Полная грамматика Swift |
| AST parser (fast) | **tree-sitter-swift** (Rust native) | Быстрый fallback без Swift toolchain |
| Storage | **SQLite** (rusqlite) + FTS5 | Быстро, один файл, zero deps |
| Git | **gix** (gitoxide, pure Rust) | Для diff-impact, без libgit2 |
| Parallelism | **rayon** (parsing) + **tokio** (server) | Data-parallel + async I/O |

---

## 4. Архитектура

```
┌──────────────────────────────────────────────────────────┐
│                   MCP Server (Rust + rmcp)                │
│                 JSON-RPC over stdin/stdout                 │
├──────────────────────────────────────────────────────────┤
│                      Tool Router                          │
├──────────┬───────────┬───────────┬───────────┬───────────┤
│  Graph   │ Analysis  │  Audit    │  Search   │ Workspace │
│  Query   │ Engine    │  Engine   │  Engine   │  Manager  │
├──────────┴───────────┴───────────┴───────────┴───────────┤
│                  Unified Graph Model                      │
│       (nodes: symbols, edges: relations, metadata)        │
├──────────────────────────────────────────────────────────┤
│  Index Store   │  swift-syntax    │  tree-sitter  │  Git │
│  Reader (C FFI)│  (subprocess)    │  (native)     │ (gix)│
└──────────────────────────────────────────────────────────┘
```

### Источники данных (приоритет)

| Источник | Что даёт | Когда используется |
|----------|---------|-------------------|
| **Index Store** (primary) | Call graph, references, conformances, type hierarchy, USR-ы — compiler-accurate | После `xcodebuild build` / `swift build` |
| **swift-syntax subprocess** (secondary) | Полный AST: тела функций, атрибуты, import-ы, doc-comments, сигнатуры | Когда нужен точный AST (аудит-проверки) |
| **tree-sitter-swift** (fast fallback) | Базовый AST: declarations, call sites, структура | Когда Index Store и swift-syntax недоступны |
| **Git** (supplementary) | Blame, co-change analysis, diff impact | По запросу |

### Стратегия деградации

```
Index Store + swift-syntax  →  полный режим (граф + аудит)
Index Store only            →  граф без AST-аудитов
swift-syntax only           →  структурный граф + аудит (без compiler-accurate связей)
tree-sitter only            →  базовый граф (аналог существующих инструментов)
```

### Автодетекция проекта

При `swiftgraph init` сервер автоматически определяет тип проекта:

| Маркер | Тип проекта | Index Store path |
|--------|-------------|-----------------|
| `Package.swift` | SPM | `.build/index/store/` |
| `*.xcodeproj` | Xcode | `~/Library/Developer/Xcode/DerivedData/<hash>/Index.noindex/DataStore/` |
| `*.xcworkspace` | Xcode workspace | аналогично, по scheme name |
| `project.yml` (XcodeGen) | XcodeGen → Xcode | аналогично `.xcodeproj` |
| `Tuist/` | Tuist → Xcode | аналогично `.xcodeproj` |

---

## 5. Data Model

### Node (символ)

```rust
struct GraphNode {
    id: String,               // USR (Unified Symbol Resolution) или synthetic ID
    name: String,             // "AppRouter"
    qualified_name: String,   // "MyApp.AppRouter"
    kind: SymbolKind,         // Class, Struct, Enum, Protocol, Method, Property, ...
    sub_kind: Option<SymbolSubKind>, // Getter, Setter, Subscript, Initializer, Deinitializer
    location: Location,       // file, line, column, end_line, end_column
    signature: Option<String>,// "func perform(request: IHTTPRequest) async throws -> Data"
    attributes: Vec<String>,  // ["@MainActor", "@Published", "@ObservedObject"]
    access_level: AccessLevel,// Public, Internal, Private, FilePrivate, Open
    container_usr: Option<String>, // USR родительского символа
    doc_comment: Option<String>,   // /// documentation
    metrics: Option<NodeMetrics>,  // lines, complexity, parameter_count
}

enum SymbolKind {
    Class, Struct, Enum, Protocol, Method, Property,
    Function, TypeAlias, Extension, EnumCase, Macro,
    AssociatedType, Import, File,
}
```

### Edge (связь)

```rust
struct GraphEdge {
    source: String,           // USR источника
    target: String,           // USR цели
    kind: EdgeKind,
    location: Option<Location>, // где в коде эта связь
    is_implicit: bool,        // synthesized by compiler
}

enum EdgeKind {
    // Вызовы
    Calls,                    // source вызывает target
    // Типы
    ConformsTo,               // struct X: Protocol
    InheritsFrom,             // class X: BaseClass
    ExtendsType,              // extension X { ... }
    Overrides,                // override func ...
    ImplementsRequirement,    // concrete method → protocol requirement
    // Зависимости
    References,               // использует символ (чтение)
    Mutates,                  // модифицирует (запись)
    Imports,                  // import Module
    DependsOn,                // модуль зависит от модуля
    // Содержание
    Contains,                 // тип содержит метод/свойство
    // Данные
    Returns,                  // функция возвращает тип
    ParameterOf,              // тип параметра функции
    PropertyType,             // свойство имеет тип
}
```

### Storage

SQLite с FTS5 для полнотекстового поиска. Таблицы:

- `nodes` — символы
- `edges` — связи
- `files` — файлы с метаданными (language, hash, last_indexed)
- `node_fts` — FTS5 виртуальная таблица для поиска
- `diagnostics` — результаты аудит-проверок (кешированные)

---

## 6. MCP Tools

### 6.1 Индексация и статус

#### `swiftgraph_status`
Статус индекса, статистика, режим работы, информация о проекте.

```json
// Response
{
    "projectName": "MyApp",
    "projectType": "xcode",        // "spm" | "xcode" | "xcodegen" | "tuist"
    "mode": "full",                // "full" | "syntax-only" | "tree-sitter" | "stale"
    "indexStoreAge": "2h ago",
    "files": 948,
    "nodes": 12500,
    "edges": 45000,
    "nodesByKind": { "class": 634, "method": 5200, "..." : "..." },
    "edgesByKind": { "calls": 15000, "conformsTo": 800, "..." : "..." },
    "lastReindex": "2026-03-12T10:30:00Z",
    "dbSize": "42 MB",
    "targets": ["MyApp", "MyAppTests"],
    "swiftVersion": "6.0"
}
```

#### `swiftgraph_reindex`
Переиндексация. Параметры: `force: bool`, `files_only: [String]`.

---

### 6.2 Навигация по графу

#### `swiftgraph_search`
Поиск символов по имени / паттерну. Поддерживает fuzzy matching.

```json
{ "query": "Router", "kind": "class", "limit": 20 }
```

#### `swiftgraph_node`
Детальная информация о символе.

```json
{ "symbol": "AppRouter", "include_code": true, "include_relations": true }
```

Возвращает: node + опционально исходный код + список связей (conformances, extensions, container).

#### `swiftgraph_callers`
Кто вызывает символ. **Compiler-accurate** — разрешает по типу, а не по имени.

```json
{ "symbol": "HTTPTransport.perform", "limit": 30, "transitive": false }
```

#### `swiftgraph_callees`
Что вызывает символ. Обратное направление от callers.

#### `swiftgraph_references`
Все использования символа (шире чем callers — включает чтение свойств, type annotations, generic constraints).

```json
{ "symbol": "AppRouter", "roles": ["call", "reference", "conformsTo"], "limit": 50 }
```

#### `swiftgraph_hierarchy`
Иерархия типов и протоколов.

```json
// Params
{ "symbol": "Codable", "direction": "subtypes", "depth": 3 }

// Response
{
    "root": "Codable",
    "kind": "protocol",
    "subtypes": [
        { "name": "UserModel", "kind": "struct", "file": "...", "line": 12 },
        { "name": "EventModel", "kind": "struct", "file": "...", "line": 5 }
    ]
}
```

#### `swiftgraph_extensions`
Все extension-ы типа, мёрженые в единое представление.

```json
{ "symbol": "String" }
// Response — все методы/свойства добавленные через extensions, с location каждого
```

#### `swiftgraph_conformances`
Какие протоколы реализует тип (и обратно — кто реализует протокол).

```json
{ "symbol": "ObservableObject", "direction": "conformedBy", "limit": 50 }
```

---

### 6.3 Контекст для задач (AI-oriented)

#### `swiftgraph_context`
**Главный инструмент.** По описанию задачи собирает релевантный контекст.

```json
// Params
{
    "task": "add search functionality to schedule screen",
    "max_nodes": 25,
    "include_code": true,
    "include_tests": true
}

// Response — structured context:
// - entry points (views, routers, builders)
// - related models and protocols
// - related networking / data layer
// - related tests
// - suggested files to modify
// - detected architecture pattern
```

Алгоритм:
1. Извлечение ключевых слов из task description
2. FTS5 поиск по именам символов
3. Расширение графа: от найденных символов по callers/callees/conformances на 2 уровня
4. Ранжирование по PageRank + relevance score
5. Отсечение по max_nodes
6. Опциональная подгрузка исходного кода ключевых символов

#### `swiftgraph_impact`
Анализ blast radius изменения символа.

```json
// Params
{ "symbol": "NetworkError", "depth": 3 }

// Response
{
    "direct_impact": 15,
    "transitive_impact": 87,
    "affected_files": ["Sources/Core/Network/...", "..."],
    "affected_tests": ["Tests/NetworkTests/..."],
    "risk_level": "high",
    "breakdown": {
        "callers": ["..."],
        "conforming_types": ["..."],
        "extensions": ["..."]
    }
}
```

#### `swiftgraph_diff_impact`
Анализ impact на основе git diff (unstaged, staged, или между коммитами).

```json
{ "ref": "HEAD~3..HEAD" }
// или
{ "ref": "staged" }
// Response — какие символы изменены, их blast radius, affected tests
```

---

### 6.4 Метрики и анализ

#### `swiftgraph_complexity`
Cyclomatic complexity, cognitive complexity, fan-in/fan-out для символа, файла или директории.

```json
{ "target": "Sources/Features/", "sort_by": "complexity", "limit": 20 }
```

#### `swiftgraph_dead_code`
Символы без входящих ссылок (потенциально неиспользуемый код). Использует граф — accurate, не grep.

```json
{ "path": "Sources/", "exclude_public": true, "exclude_tests": true, "exclude_entry_points": true }
```

#### `swiftgraph_cycles`
Обнаружение циклических зависимостей между модулями/файлами/типами.

```json
{ "level": "file" }  // "file" | "type" | "module"
```

#### `swiftgraph_coupling`
Метрики связанности между модулями/директориями.

```json
{ "source": "Sources/Features/Auth/", "target": "Sources/Core/" }
// Response — afferent/efferent coupling, instability, abstractness
```

---

### 6.5 Architecture-aware

#### `swiftgraph_architecture`
Определение архитектурного паттерна и проверка его соблюдения. Автоматически распознаёт: MVVM, MVC, VIPER, Clean Architecture, TCA, MVVM+C (Coordinator), MVVM+Router.

```json
// Params
{ "path": "Sources/Features/Auth/" }

// Response
{
    "pattern": "MVVM+Router",
    "components": {
        "view": "AuthView.swift",
        "viewModel": "AuthViewModel.swift",
        "router": "→ AuthRouter (shared)"
    },
    "violations": [
        { "rule": "view-imports-networking", "file": "AuthView.swift", "line": 3 }
    ]
}
```

#### `swiftgraph_boundaries`
Проверка архитектурных границ (задаются в конфигурации или передаются в параметрах).

```json
// Params
{
    "rules": [
        { "from": "Features/*", "deny": "Features/*/", "allow": "Core/**,Models/**" },
        { "from": "Models/*", "deny": "Features/**,Core/**" }
    ]
}
// Response — список нарушений с location
```

---

### 6.6 Статические проверки (Audit Engine)

Встроенные проверки Swift-паттернов, работающие на данных графа + swift-syntax AST. Каждая проверка имеет severity (critical/high/medium/low) и категорию.

#### `swiftgraph_audit`
Запуск проверок по категории или всех сразу.

```json
// Params
{
    "categories": ["concurrency", "memory", "performance"],  // или ["all"]
    "path": "Sources/",           // опционально — scope
    "severity_min": "medium",     // опционально — фильтр severity
    "fix_suggestions": true       // включить рекомендации по исправлению
}

// Response
{
    "total_issues": 23,
    "by_severity": { "critical": 2, "high": 8, "medium": 13 },
    "issues": [
        {
            "id": "CONC-001",
            "category": "concurrency",
            "severity": "critical",
            "rule": "missing-main-actor",
            "message": "UIViewController subclass without @MainActor",
            "file": "Sources/Features/Auth/AuthViewController.swift",
            "line": 5,
            "symbol": "AuthViewController",
            "fix": "Add @MainActor attribute to class declaration"
        }
    ]
}
```

#### Категории и правила проверок

##### Concurrency (`concurrency`)

| ID | Severity | Правило | Что проверяет |
|----|----------|---------|---------------|
| CONC-001 | CRITICAL | `missing-main-actor` | UIViewController, ObservableObject, View subclasses без `@MainActor` |
| CONC-002 | HIGH | `unsafe-task-capture` | `Task { self.property }` без `[weak self]` |
| CONC-003 | CRITICAL | `nonisolated-self-access` | `nonisolated func` с `Task { self.property }` внутри |
| CONC-004 | HIGH | `sendable-violation` | Non-Sendable типы передаваемые через actor boundary (через граф) |
| CONC-005 | HIGH | `detached-mainactor-access` | `@MainActor` свойства из `Task.detached` без `await MainActor.run` |
| CONC-006 | MEDIUM | `stored-task-no-weak` | `var task: Task<...>? = Task { self... }` без weak capture |
| CONC-007 | MEDIUM | `actor-hop-in-loop` | `await actorMethod()` внутри for/while — каждый hop ~100μs |

##### Memory (`memory`)

| ID | Severity | Правило | Что проверяет |
|----|----------|---------|---------------|
| MEM-001 | CRITICAL | `timer-leak` | `Timer.scheduledTimer(repeats: true)` без `.invalidate()` в deinit/onDisappear |
| MEM-002 | HIGH | `observer-leak` | `NotificationCenter.addObserver(self,` без `removeObserver` |
| MEM-003 | HIGH | `closure-retain-cycle` | Closures в `.append`, `DispatchQueue`, `URLSession` capturing `self` без `[weak self]` |
| MEM-004 | MEDIUM | `strong-delegate` | `var delegate: SomeDelegate` без `weak` |
| MEM-005 | MEDIUM | `view-callback-leak` | Stored closures в `.onAppear`/`.onDisappear` без weak self |
| MEM-006 | LOW | `photokit-no-cancel` | `PHImageManager.request*` без `cancelImageRequest` в reuse/disappear |

##### Swift Performance (`swift-performance`)

| ID | Severity | Правило | Что проверяет |
|----|----------|---------|---------------|
| PERF-001 | HIGH | `unnecessary-copy` | Большие struct (>64 bytes / 3+ свойств) без `borrowing`/`consuming` |
| PERF-002 | CRITICAL | `excessive-arc` | `[weak self]` + immediate `guard let self` (unowned безопаснее здесь) |
| PERF-003 | HIGH | `existential-overhead` | `any Protocol` в коллекциях — heap alloc + witness table на каждый элемент |
| PERF-004 | MEDIUM | `collection-no-reserve` | `.append(` в циклах без `reserveCapacity` |
| PERF-005 | HIGH | `actor-hop-overhead` | `await actor.method()` в tight loop |
| PERF-006 | MEDIUM | `large-value-type` | Struct с arrays или >5-6 свойств — implicit copying overhead |

##### SwiftUI Performance (`swiftui-performance`)

| ID | Severity | Правило | Что проверяет |
|----|----------|---------|---------------|
| SUI-001 | CRITICAL | `file-io-in-body` | `Data(contentsOf:`, `String(contentsOf:` внутри `var body` |
| SUI-002 | CRITICAL | `formatter-in-body` | `DateFormatter()`, `NumberFormatter()` внутри body (1-2ms каждый) |
| SUI-003 | HIGH | `image-processing-in-body` | `.resized`, `UIGraphicsBeginImageContext`, `CIFilter` внутри body |
| SUI-004 | HIGH | `missing-lazy-container` | `VStack { ForEach(largeCollection) }` без Lazy |
| SUI-005 | MEDIUM | `whole-collection-dependency` | `.contains(`, `.filter(` в body — view updates при любом изменении |
| SUI-006 | LOW | `legacy-observable` | `ObservableObject` + `@Published` вместо `@Observable` (iOS 17+) |

##### SwiftUI Architecture (`swiftui-architecture`)

| ID | Severity | Правило | Что проверяет |
|----|----------|---------|---------------|
| ARCH-001 | HIGH | `logic-in-view-body` | Бизнес-логика, фильтрация, вычисления внутри `var body` |
| ARCH-002 | CRITICAL | `async-in-view` | Multi-step `Task { }` с state mutation внутри View |
| ARCH-003 | HIGH | `state-as-binding-source` | `@State var item: Item` (non-private) для переданного объекта — теряет обновления |
| ARCH-004 | MEDIUM | `god-viewmodel` | `@Observable class` с >20 свойств или смешанными доменами |
| ARCH-005 | MEDIUM | `swiftui-import-in-model` | `import SwiftUI` в файлах моделей/сервисов |

##### Security & Privacy (`security`)

| ID | Severity | Правило | Что проверяет |
|----|----------|---------|---------------|
| SEC-001 | CRITICAL | `hardcoded-secret` | API keys, tokens по regex: `AKIA[0-9A-Z]{16}`, `sk-[a-zA-Z0-9]{24,}`, `ghp_`, etc. |
| SEC-002 | CRITICAL | `missing-privacy-manifest` | Отсутствие `PrivacyInfo.xcprivacy` при использовании Required Reason APIs |
| SEC-003 | HIGH | `insecure-token-storage` | Tokens/secrets в `@AppStorage` или `UserDefaults` вместо Keychain |
| SEC-004 | HIGH | `http-ats-violation` | `http://` URLs (кроме localhost) |
| SEC-005 | MEDIUM | `sensitive-data-in-logs` | `print(password`, `Logger.*token`, `NSLog.*secret` |
| SEC-006 | HIGH | `missing-att-description` | `ATTrackingManager` без `NSUserTrackingUsageDescription` в Info.plist |

##### Energy (`energy`)

| ID | Severity | Правило | Что проверяет |
|----|----------|---------|---------------|
| NRG-001 | CRITICAL | `timer-abuse` | `Timer.scheduledTimer` с interval < 1s и `repeats: true` без tolerance |
| NRG-002 | CRITICAL | `polling-not-push` | URLSession на Timer (периодический опрос вместо push) |
| NRG-003 | CRITICAL | `continuous-location` | `startUpdatingLocation` без `stopUpdatingLocation`, `kCLLocationAccuracyBest` без необходимости |
| NRG-004 | HIGH | `animation-leak` | `CADisplayLink`, `withAnimation` без stop в disappear handlers |
| NRG-005 | HIGH | `background-mode-unused` | `UIBackgroundModes` в plist без matching runtime usage |
| NRG-006 | MEDIUM | `network-no-discretionary` | `URLSession.shared` для non-urgent requests без `isDiscretionary` |

##### Networking (`networking`)

| ID | Severity | Правило | Что проверяет |
|----|----------|---------|---------------|
| NET-001 | HIGH | `deprecated-reachability` | `SCNetworkReachability*` — race conditions, misses proxy/VPN |
| NET-002 | MEDIUM | `deprecated-cfsocket` | `CFSocketCreate`, `CFSocketConnectToAddress` — 30% CPU penalty |
| NET-003 | MEDIUM | `deprecated-nsstream` | `NSInputStream`, `NSOutputStream`, `CFStreamCreatePairWithSocket` |
| NET-004 | HIGH | `reachability-before-connect` | Reachability check followed by connection start — race condition |
| NET-005 | MEDIUM | `hardcoded-ip` | IP литералы типа `"192.168.1.1"` вместо hostnames |
| NET-006 | MEDIUM | `blocking-socket-call` | Synchronous `connect()`, `send()`, `recv()` on main thread |

##### Codable (`codable`)

| ID | Severity | Правило | Что проверяет |
|----|----------|---------|---------------|
| COD-001 | HIGH | `manual-json-building` | String interpolation для JSON: `"\"{key\": \"\(value)\"}` |
| COD-002 | HIGH | `try-question-mark-decode` | `try? JSONDecoder().decode` — глушит ошибки декодирования |
| COD-003 | MEDIUM | `json-serialization-legacy` | `JSONSerialization` вместо Codable |
| COD-004 | MEDIUM | `date-no-strategy` | `Date` в Codable struct без `dateDecodingStrategy` |
| COD-005 | MEDIUM | `date-formatter-no-locale` | `DateFormatter()` без `locale` — locale-dependent bugs |

##### Storage (`storage`)

| ID | Severity | Правило | Что проверяет |
|----|----------|---------|---------------|
| STR-001 | CRITICAL | `data-in-tmp` | Важные данные в `NSTemporaryDirectory` — iOS может удалить |
| STR-002 | HIGH | `no-backup-exclusion` | Файлы >1MB в Documents/ без `isExcludedFromBackup` |
| STR-003 | MEDIUM | `no-file-protection` | Запись файлов без `FileProtectionType` |
| STR-004 | MEDIUM | `userdefaults-abuse` | Объекты >1MB в UserDefaults |

##### Accessibility (`accessibility`)

| ID | Severity | Правило | Что проверяет |
|----|----------|---------|---------------|
| A11Y-001 | CRITICAL | `missing-accessibility-label` | Image/Button без `accessibilityLabel` или `accessibilityHidden` |
| A11Y-002 | HIGH | `fixed-font-size` | `.font(.system(size:))` без `relativeTo:` — ломает Dynamic Type |
| A11Y-003 | MEDIUM | `small-touch-target` | `.frame(width/height:)` < 44pt |
| A11Y-004 | MEDIUM | `animation-no-reduce-motion` | `withAnimation` без `isReduceMotionEnabled` check |

##### Testing (`testing`)

| ID | Severity | Правило | Что проверяет |
|----|----------|---------|---------------|
| TST-001 | CRITICAL | `sleep-in-test` | `sleep(`, `Thread.sleep`, `usleep(` в тестах — timing-dependent flakiness |
| TST-002 | HIGH | `shared-mutable-state` | `static var` в test classes — data race при параллельных тестах |
| TST-003 | MEDIUM | `test-no-assertion` | Тест без assert/expect — ложно-зелёный |
| TST-004 | MEDIUM | `xctest-to-swift-testing` | `XCTestCase` который можно мигрировать на `@Suite/@Test` |
| TST-005 | HIGH | `mainactor-test-violation` | Тест обращается к `@MainActor` типу без `@MainActor` аннотации |

##### Modernization (`modernization`)

| ID | Severity | Правило | Что проверяет |
|----|----------|---------|---------------|
| MOD-001 | HIGH | `observable-object-legacy` | `ObservableObject` + `@Published` → `@Observable` (iOS 17+) |
| MOD-002 | HIGH | `state-object-legacy` | `@StateObject` → `@State` (при использовании `@Observable`) |
| MOD-003 | HIGH | `observed-object-legacy` | `@ObservedObject` → прямое свойство или `@Bindable` |
| MOD-004 | HIGH | `environment-object-legacy` | `@EnvironmentObject` → `@Environment` |
| MOD-005 | MEDIUM | `completion-handler-legacy` | Completion handlers → `async`/`await` |

---

### 6.7 Concurrency-aware (Swift-specific)

#### `swiftgraph_concurrency`
Глубокий анализ concurrency-аннотаций в графе. Использует и граф (кто кого вызывает), и AST (какие аннотации).

```json
// Params
{ "symbol": "SomeRouter" }

// Response
{
    "isolation": "@MainActor",
    "sendable_conformance": false,
    "called_from_non_isolated": [
        { "caller": "BackgroundService.handle", "file": "...", "line": 42, "issue": "cross-actor call without await" }
    ],
    "mutable_state": [
        { "property": "path", "wrapper": "@Published", "accessed_from": ["SomeRouter", "TabView"] }
    ]
}
```

---

### 6.8 Файлы и workspace

#### `swiftgraph_files`
Дерево файлов проекта с метаданными.

```json
{ "path": "Sources/", "max_depth": 2, "include_metadata": true }
// Response — дерево с language, symbol_count, last_modified, complexity per file
```

#### `swiftgraph_imports`
Граф импортов (module-level dependencies).

```json
{ "module": "MyApp" }
// Response — модули которые MyApp импортирует, и кто импортирует MyApp (если multi-target)
```

---

## 7. Индексация: Pipeline

```
1. Scan workspace → list .swift files (respect include/exclude globs)
2. Detect project type → find Index Store path
3. Hash each file (SHA256) → compare with stored hashes
4. For changed/new files:
   a. Parse with tree-sitter-swift → extract declarations, basic call sites → nodes + preliminary edges
   b. If swift-syntax subprocess available:
      - Send file → get full AST JSON → extract attributes, signatures, doc-comments, body analysis
5. If Index Store available:
   a. Read all units + records via libIndexStore C API
   b. Enrich nodes with USRs, resolved types, access levels
   c. Replace syntactic edges with semantic edges (accurate call graph)
   d. Add conformance, inheritance, override, extension edges
6. Compute metrics (cyclomatic complexity via AST, fan-in/fan-out via graph)
7. Run audit checks on changed files → cache results in diagnostics table
8. Build/update FTS5 index
9. Persist to SQLite (single transaction)
```

### Инкрементальная переиндексация

- SHA256 каждого файла в `files` таблице
- При reindex: сканировать файлы, сравнивать хеши
- Переиндексировать только изменённые файлы + файлы зависящие от изменённых (через граф)
- Index Store: проверять `mtime` unit-файлов

### Целевая производительность

| Операция | Цель | Метод |
|----------|------|-------|
| Полная индексация, 1000 файлов | < 10 сек | rayon parallel parse + batch SQLite insert |
| Инкрементальная, 1-10 файлов | < 1 сек | hash diff + targeted reindex |
| MCP tool response | < 200 мс | SQLite indexed queries + in-memory graph cache |
| Аудит, 1000 файлов | < 15 сек | rayon parallel AST analysis |
| FTS5 search | < 50 мс | SQLite FTS5 |

---

## 8. Интерфейс CLI

```bash
# Инициализация (создаёт .swiftgraph/ в корне проекта)
swiftgraph init

# Индексация (автодетекция Index Store)
swiftgraph index
swiftgraph index --force
swiftgraph index --index-store-path /path/to/DataStore

# Запуск MCP-сервера
swiftgraph serve --mcp

# Аудит из CLI (для CI/CD)
swiftgraph audit --categories concurrency,memory,security
swiftgraph audit --severity-min high --format json
swiftgraph audit --format sarif  # для GitHub Code Scanning

# Интерактивные запросы (для отладки)
swiftgraph search "Router"
swiftgraph callers "AppRouter.logout"
swiftgraph hierarchy "Codable" --direction subtypes
swiftgraph impact "NetworkError" --depth 2
swiftgraph dead-code --path Sources/
```

---

## 9. Конфигурация

Файл `.swiftgraph/config.json` (создаётся при `init`, можно редактировать):

```json
{
    "version": 1,
    "include": ["Sources/**/*.swift", "Tests/**/*.swift"],
    "exclude": ["**/Generated/**", "**/Pods/**", "**/.build/**"],
    "index_store_path": "auto",
    "swift_syntax_path": "auto",
    "audit": {
        "enabled_categories": ["all"],
        "severity_min": "medium",
        "exclude_rules": [],
        "custom_boundaries": [
            { "from": "Features/*", "deny": "Features/*/", "allow": "Core/**,Models/**" }
        ]
    }
}
```

- `"index_store_path": "auto"` — поиск в DerivedData по имени проекта / Package.swift
- `"swift_syntax_path": "auto"` — поиск `swiftgraph-parser` рядом с основным бинарником

---

## 10. MCP-конфигурация

```json
{
    "mcpServers": {
        "swiftgraph": {
            "command": "swiftgraph",
            "args": ["serve", "--mcp", "--project", "/path/to/project"]
        }
    }
}
```

Если `--project` не указан — используется cwd.

---

## 11. CI/CD интеграция

```yaml
# GitHub Actions example
- name: SwiftGraph Audit
  run: |
    swiftgraph index
    swiftgraph audit --format sarif --output swiftgraph-results.sarif

- name: Upload SARIF
  uses: github/codeql-action/upload-sarif@v3
  with:
    sarif_file: swiftgraph-results.sarif
```

Поддерживаемые форматы вывода аудита:
- `json` — для программного потребления
- `sarif` — для GitHub Code Scanning / VS Code
- `text` — для терминала (с цветами)
- `markdown` — для PR-комментариев

---

## 12. Сравнение с существующими решениями

| Capability | colbymchenry | optave | narsil | **SwiftGraph** |
|---|---|---|---|---|
| Swift call graph accuracy | ~30% (name) | ~30% (name) | ~30% (name) | **~98% (USR)** |
| Protocol conformances | Нет | Нет | Нет | **Да** |
| Type hierarchy | Нет | Нет | Нет | **Да** |
| Extension merging | Нет | Нет | Нет | **Да** |
| Concurrency analysis | Нет | Нет | Нет | **Да (graph + AST)** |
| Static audit checks | Нет | Нет | Partial (security) | **80+ rules, 12 categories** |
| Dead code detection | Нет | Да (name-based) | Да | **Да (USR-based, accurate)** |
| Impact analysis | Базовый | Git-aware | Базовый | **Semantic + Git** |
| Architecture enforcement | Нет | Да | Нет | **Да + Swift pattern detection** |
| CI/CD (SARIF) | Нет | Нет | Нет | **Да** |
| Требует билда | Нет | Нет | Нет | Частично (degraded без) |
| Языки | 14 | 11 | 32 | **Только Swift** |
| Memory safety (impl) | JS (GC) | JS (GC) | Rust | **Rust** |

---

## 13. Roadmap

### v0.1 — MVP: Graph
- [ ] Rust project scaffold: rmcp server, rusqlite, tree-sitter-swift
- [ ] libIndexStore C FFI bindings (bindgen)
- [ ] Index Store reader → nodes + edges в SQLite
- [ ] tree-sitter-swift fallback parser
- [ ] MCP tools: `status`, `search`, `node`, `callers`, `callees`, `references`
- [ ] `hierarchy` (type + protocol)
- [ ] `files`
- [ ] CLI: `init`, `index`, `serve --mcp`
- [ ] Auto-detect project type (SPM / Xcode / XcodeGen / Tuist)

### v0.2 — Intelligence
- [ ] `context` (task-based context builder с PageRank)
- [ ] `impact` + `diff_impact` (gix integration)
- [ ] `extensions`, `conformances`
- [ ] FTS5 full-text search
- [ ] Incremental reindex (SHA256 diff)

### v0.3 — Audit Engine
- [ ] swift-syntax subprocess (`swiftgraph-parser` Swift CLI)
- [ ] Audit framework: rule registration, severity, category
- [ ] Concurrency checks (CONC-001..007)
- [ ] Memory checks (MEM-001..006)
- [ ] Security checks (SEC-001..006)
- [ ] `swiftgraph_audit` MCP tool
- [ ] CLI: `swiftgraph audit` с text/json output

### v0.4 — Analysis
- [ ] `complexity`, `dead_code`, `cycles`, `coupling`
- [ ] `architecture` (pattern detection), `boundaries`
- [ ] `concurrency` deep analysis (graph + AST combined)
- [ ] `imports` (module dependency graph)

### v0.5 — Production
- [ ] SwiftUI performance checks (SUI-001..006)
- [ ] SwiftUI architecture checks (ARCH-001..005)
- [ ] Energy checks (NRG-001..006)
- [ ] Networking, Codable, Storage, Accessibility, Testing, Modernization checks
- [ ] SARIF output for CI/CD
- [ ] Watch mode (FSEvents → auto-reindex)
- [ ] Homebrew formula
- [ ] Performance: parallel file parsing (rayon), in-memory graph cache
