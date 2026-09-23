//! False positives seen on a large production app, each next to a true
//! positive that must keep firing.

use swiftgraph_audit::engine::{AuditIssue, Severity};
use swiftgraph_audit::rules::{self, AuditRule, FileContext, ProjectFacts};
use swiftgraph_audit::runner::{run_audit, AuditOptions};

fn check_with(rule: &dyn AuditRule, source: &str, project: &ProjectFacts) -> Vec<AuditIssue> {
    let mut parser = rules::swift_parser().unwrap();
    let tree = parser.parse(source, None).unwrap();
    let ctx = FileContext {
        file_path: "Sample.swift",
        source,
        tree: &tree,
        project,
    };
    rule.check(&ctx)
}

fn check(rule: &dyn AuditRule, source: &str) -> Vec<AuditIssue> {
    check_with(rule, source, &ProjectFacts::default())
}

fn lines(issues: &[AuditIssue]) -> Vec<u32> {
    issues.iter().map(|i| i.line).collect()
}

// CONC-001

#[test]
fn conc001_is_low_without_async_code_and_high_with_it() {
    let rule = rules::concurrency::MissingMainActor;
    let plain = check(
        &rule,
        "final class SettingsViewModel: ObservableObject {\n    @Published var title = \"\"\n    func rename(_ t: String) { title = t }\n}\n",
    );
    assert_eq!(plain.len(), 1);
    assert_eq!(plain[0].severity, Severity::Advisory);

    let racy = check(
        &rule,
        "final class FeedViewModel: ObservableObject {\n    @Published var items: [Int] = []\n    func load() async {\n        items = await fetch()\n    }\n}\n",
    );
    assert_eq!(racy.len(), 1);
    assert_eq!(racy[0].severity, Severity::High);
}

#[test]
fn audit_config_disables_rules_and_overrides_severity() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".swiftgraph")).unwrap();
    std::fs::write(
        dir.path().join("VM.swift"),
        "final class VM: ObservableObject {\n    func load() async {}\n}\nclass Holder: NSObject {\n    var delegate: HolderDelegate?\n}\n",
    )
    .unwrap();
    let run = || {
        let result = run_audit(dir.path(), &AuditOptions::default()).unwrap();
        result
            .issues
            .into_iter()
            .map(|i| (i.rule, i.severity))
            .collect::<Vec<_>>()
    };
    let before = run();
    assert!(
        before.contains(&("CONC-001".to_string(), Severity::High)),
        "{before:?}"
    );
    assert!(
        before.contains(&("MEM-002".to_string(), Severity::High)),
        "{before:?}"
    );

    std::fs::write(
        dir.path().join(".swiftgraph/config.json"),
        r#"{"audit": {"disabled_rules": ["CONC-001"], "severity": {"MEM-002": "low"}}}"#,
    )
    .unwrap();
    let after = run();
    assert!(
        after.iter().all(|(rule, _)| rule != "CONC-001"),
        "{after:?}"
    );
    assert!(
        after.contains(&("MEM-002".to_string(), Severity::Low)),
        "{after:?}"
    );

    // The legacy key written by older `init` keeps working.
    std::fs::write(
        dir.path().join(".swiftgraph/config.json"),
        r#"{"audit": {"exclude_rules": ["MEM-002"]}}"#,
    )
    .unwrap();
    assert!(run().iter().all(|(rule, _)| rule != "MEM-002"));
}

// PERF-001

#[test]
fn perf001_skips_codable_dtos_and_swiftui_views() {
    let rule = rules::performance::UnnecessaryCopy;
    let dto = "public struct AccountDTO: Decodable {\n    let id: String\n    let name: String\n    let balance: String\n    let currency: String\n}\n";
    assert!(check(&rule, dto).is_empty());
    let codable = "struct Settings: Codable, Equatable {\n    var a = 1\n    var b = 2\n    var c = 3\n    var d = 4\n}\n";
    assert!(check(&rule, codable).is_empty());
    let view = "struct BalanceView: View {\n    let a: String\n    let b: String\n    let c: String\n    let d: String\n    var body: some View { Text(a) }\n}\n";
    assert!(check(&rule, view).is_empty());
    let facts = ProjectFacts::from_sources(&["protocol ApiResponse: Decodable {}\n"]);
    let indirect = "struct BalanceResponse: ApiResponse {\n    let a: String\n    let b: String\n    let c: String\n    let d: String\n}\n";
    assert!(check_with(&rule, indirect, &facts).is_empty());
    // Modifiers used to hide the `struct` keyword.
    let model = "public struct WalletModel {\n    let id: String\n    let name: String\n    let balance: Decimal\n    let tokens: [String]\n}\n";
    assert_eq!(check(&rule, model).len(), 1);
}

// MEM-001

#[test]
fn mem001_ignores_non_escaping_collection_closures() {
    let rule = rules::memory::ClosureRetainCycle;
    let source = r#"
final class Loader {
    var handler: (() -> Void)?
    func run(items: [Int]) {
        let a = items.map { self.transform($0) }
        let b = items.filter { self.isValid($0) }.compactMap { self.convert($0) }
        items.forEach { self.handle($0) }
        let found = items.first(where: { self.isValid($0) })
        let sorted = items.sorted { self.rank($0) < self.rank($1) }
        let has = items.contains(where: { self.isValid($0) })
        let sum = items.reduce(0) { $0 + self.weight($1) }
        service.fetch(completion: { result in self.apply(result) })
        handler = { self.finish() }
    }
}
"#;
    assert_eq!(lines(&check(&rule, source)), vec![12, 13]);
}

#[test]
fn mem001_keeps_combine_map_closures() {
    let rule = rules::memory::ClosureRetainCycle;
    let source = r#"
final class Store {
    func bind() {
        $query
            .map { self.normalize($0) }
            .sink { _ in }
            .store(in: &bag)
    }
}
"#;
    assert_eq!(check(&rule, source).len(), 1);
}

// MEM-002

#[test]
fn mem002_only_flags_stored_delegate_properties_of_classes() {
    let rule = rules::memory::StrongDelegate;
    let source = r#"
final class Router {
    var delegate: RouterDelegate? = nil
    weak var weakDelegate: RouterDelegate?
    var dataSource: UITableViewDataSource!
    let delegateData: Data
    var delegateName: String
    var onDelegate: (() -> Void)?
    var hasDelegate: Bool { delegate != nil }
    private lazy var tableDataSource: DataSource = makeDataSource()
    func handle(message: Message) {
        let delegateData = message.delegateData
        var delegate: RouterDelegate? = nil
    }
}
struct Config {
    var delegate: ConfigDelegate?
}
"#;
    assert_eq!(lines(&check(&rule, source)), vec![3, 5]);
}

// SEC-001

#[test]
fn sec001_ignores_raw_values_headers_addresses_and_event_names() {
    let rule = rules::security::HardcodedSecrets;
    let noise = r#"
enum Event: String {
    case buttonRemoveToken = "[Token] Remove Token"
}
enum Header {
    static let xApiKey = "X-API-KEY"
}
let polToken = "0x455e53CBB86018Ac2B8092FdCd39d8444aFFC3F6"
let tokenEvent = "Token Screen Opened"
"#;
    assert!(check(&rule, noise).is_empty(), "{:?}", check(&rule, noise));
    let secret = "let apiKey = \"sk_live_51H8xY2eZvKYlo2C9\"\nlet password = \"hunter2hunter2\"\n";
    assert_eq!(lines(&check(&rule, secret)), vec![1, 2]);
}

// SEC-003

#[test]
fn sec003_ignores_asset_tokens_and_flags_credentials() {
    let rule = rules::security::SensitiveLogging;
    let noise = r#"
Logger.info("\(token.name) allowance")
print("tokenItem: \(tokenItem.symbol)")
Logger.debug("Loaded \(tokens.count) tokens")
AppLogger.error("Failed to save web credential", error: error)
VisaLogger.error("missing access token", error: HandlerError.missingAccessToken)
print("Password reset tapped")
"#;
    assert!(check(&rule, noise).is_empty(), "{:?}", check(&rule, noise));
    let leaks = r#"
print("refresh: \(refreshToken)")
Logger.info("mnemonic \(wallet.mnemonic)")
NSLog("key %@", privateKey)
print("pwd \(userPassword)")
"#;
    assert_eq!(lines(&check(&rule, leaks)), vec![2, 3, 4, 5]);
}

// ARCH-005

#[test]
fn arch005_sees_conformance_through_project_protocols_and_extensions() {
    let rule = rules::swiftui_arch::PublishedWithoutObservable;
    let facts = ProjectFacts::from_sources(&[
        "protocol CoordinatorObject: ObservableObject {}\n",
        "protocol FlowCoordinator: CoordinatorObject, AnyObject {}\n",
        "extension LegacyStore: ObservableObject {}\n",
    ]);
    let source = r#"
final class AppCoordinator: FlowCoordinator {
    @Published var route: Route?
}
final class LegacyStore {
    @Published var items: [Int] = []
}
final class Broken: NSObject {
    @Published var value = 0
}
"#;
    assert_eq!(lines(&check_with(&rule, source, &facts)), vec![8]);
}

// SUI-005

#[test]
fn sui005_skips_small_literal_collections_and_lowers_non_scrolling_lists() {
    let rule = rules::swiftui_perf::NonLazyList;
    let source = r#"
struct V: View {
    var body: some View {
        VStack {
            ForEach(["a", "b", "c"], id: \.self) { Text($0) }
            ForEach(0..<3) { Text("\($0)") }
            ForEach(model.rows) { Text($0.title) }
        }
        ScrollView {
            VStack {
                ForEach(model.items) { Text($0.title) }
            }
        }
    }
}
"#;
    let issues = check(&rule, source);
    let found: Vec<(u32, Severity)> = issues.iter().map(|i| (i.line, i.severity)).collect();
    assert_eq!(found, vec![(7, Severity::Advisory), (11, Severity::Medium)]);
}

#[test]
fn mem001_ignores_value_types_including_their_extensions() {
    let rule = rules::memory::ClosureRetainCycle;
    let facts = ProjectFacts::from_sources(&["struct RowView: View {}\n"]);
    let source = r#"
struct Card {
    func bind(store: Store) {
        store.onChange = { self.render() }
    }
}
extension RowView {
    func bind(store: Store) {
        store.onChange = { self.render() }
    }
}
final class Screen {
    func bind(store: Store) {
        store.onChange = { self.render() }
    }
}
"#;
    assert_eq!(lines(&check_with(&rule, source, &facts)), vec![14]);
}

// Runner

#[test]
fn runner_reports_each_finding_once_per_line() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("Close.swift"),
        "struct Close: View {\n    var body: some View {\n        Button(action: close) {\n            Image(systemName: \"xmark\")\n                .font(.system(size: 17, weight: .semibold))\n                .foregroundColor(.white)\n        }\n    }\n}\n",
    )
    .unwrap();
    let result = run_audit(dir.path(), &AuditOptions::default()).unwrap();
    let mut keys: Vec<_> = result
        .issues
        .iter()
        .map(|i| (i.file.clone(), i.line, i.rule.clone(), i.message.clone()))
        .collect();
    let a11y = keys.iter().filter(|k| k.2 == "A11Y-001").count();
    assert_eq!(a11y, 1, "{:?}", result.issues);
    let before = keys.len();
    keys.sort();
    keys.dedup();
    assert_eq!(keys.len(), before, "duplicates: {:?}", result.issues);
}

#[test]
fn runner_respects_config_include_and_exclude() {
    let dir = tempfile::tempdir().unwrap();
    let delegate = "class Holder: NSObject {\n    var delegate: HolderDelegate?\n}\n";
    std::fs::create_dir_all(dir.path().join(".swiftgraph")).unwrap();
    std::fs::create_dir_all(dir.path().join("App/Preview Content")).unwrap();
    std::fs::write(dir.path().join("App/Holder.swift"), delegate).unwrap();
    std::fs::write(dir.path().join("App/Message.pb.swift"), delegate).unwrap();
    std::fs::write(dir.path().join("App/Preview Content/Mock.swift"), delegate).unwrap();
    std::fs::write(
        dir.path().join(".swiftgraph/config.json"),
        r#"{"exclude": ["**/*.pb.swift", "**/Preview Content/**"]}"#,
    )
    .unwrap();
    let result = run_audit(dir.path(), &AuditOptions::default()).unwrap();
    let mut files: Vec<String> = result
        .issues
        .iter()
        .map(|i| i.file.rsplit('/').next().unwrap_or_default().to_string())
        .collect();
    files.dedup();
    assert_eq!(files, vec!["Holder.swift".to_string()]);
}

#[test]
fn conc001_points_at_the_class_line_not_its_attributes() {
    let rule = rules::concurrency::MissingMainActor;
    let source = "@usableFromInline\nfinal class VM: ObservableObject {}\n\n@available(iOS, deprecated: 100000, message: \"x\")\n// note\nclass Screen: UIViewController {}\n";
    assert_eq!(lines(&check(&rule, source)), vec![2, 6]);
}
