//! False-positive patterns found by a precision pass (random samples checked
//! against sources), each next to the true positive that must stay.

use swiftgraph_audit::engine::{AuditIssue, Severity};
use swiftgraph_audit::rules::{self, AuditRule, FileContext, ProjectFacts};

fn check(rule: &dyn AuditRule, source: &str) -> Vec<AuditIssue> {
    let mut parser = rules::swift_parser().unwrap();
    let tree = parser.parse(source, None).unwrap();
    let project = ProjectFacts::default();
    let ctx = FileContext {
        file_path: "Sample.swift",
        source,
        tree: &tree,
        project: &project,
    };
    rule.check(&ctx)
}

fn found(issues: &[AuditIssue]) -> Vec<(u32, Severity)> {
    issues.iter().map(|i| (i.line, i.severity)).collect()
}

#[test]
fn conc001_trusts_uikit_isolation_and_explicit_main_hops() {
    let rule = rules::concurrency::MissingMainActor;
    let source = r#"
final class Sheet: UIViewController {
    func load() async {}
}
final class HopsViewModel: ObservableObject {
    @Published var items: [Int] = []
    func bind() {
        service.itemsPublisher.receiveOnMain().assign(to: &$items)
        Task { await runOnMain { self.items = [] } }
    }
}
final class RacyViewModel: ObservableObject {
    @Published var items: [Int] = []
}
extension RacyViewModel {
    func reload() {
        Task { items = await fetch() }
    }
}
"#;
    assert_eq!(
        found(&check(&rule, source)),
        vec![(5, Severity::Advisory), (12, Severity::High)]
    );
}

#[test]
fn conc005_links_tasks_to_the_state_they_touch() {
    let rule = rules::concurrency::SendableViolation;
    let source = r#"
final class Presenter {
    var title = ""
    func close() { Task { @MainActor in self.title = "" } }
}
final class Grouped {
    var results: [Int] = []
    func run() async { await withTaskGroup(of: Int.self) { group in group.addTask { 1 } } }
}
final class Local {
    let service: Service
    func run() { var count = 0; count += 1; Task { await service.load() } }
}
final class Counter {
    var count = 0
    func increment() { Task { self.count += 1 } }
}
"#;
    let lines: Vec<u32> = check(&rule, source).iter().map(|i| i.line).collect();
    assert_eq!(lines, vec![14]);
}

#[test]
fn mem001_needs_self_to_own_the_closure() {
    let rule = rules::memory::ClosureRetainCycle;
    let source = r#"
final class Manager {
    var bag = Set<AnyCancellable>()
    func send(tx: String) -> AnyPublisher<Result, Error> {
        Future.async { try await self.provider.send(tx) }
            .eraseToAnyPublisher()
    }
    var balance: AnyPublisher<String, Never> {
        balancePublisher.map { self.format($0) }.eraseToAnyPublisher()
    }
    func copy() {
        worker.run { [repo = self.repo] in repo.save() }
    }
    func bind() {
        publisher.sink { self.update($0) }.store(in: &bag)
        api.load(completion: { self.finish() })
    }
}
"#;
    assert_eq!(
        found(&check(&rule, source)),
        vec![(15, Severity::High), (16, Severity::Low)]
    );
}

#[test]
fn sec002_ignores_asset_tokens() {
    let rule = rules::security::InsecureStorage;
    let source = "UserDefaults.standard.set($0, forKey: tokenSymbolKey)\n@AppStorage(StorageType.withdrawTokenDisplayed) var shown = false\nUserDefaults.standard.set(accessToken, forKey: \"access_token\")\n";
    let lines: Vec<u32> = check(&rule, source).iter().map(|i| i.line).collect();
    assert_eq!(lines, vec![3]);
}

#[test]
fn sec004_needs_a_host_after_the_scheme() {
    let rule = rules::security::AtsBypass;
    let source = "let schemes = [\"http://\", \"https://\"]\nlet node = URL(string: \"http://api.node.io/v2\")\n";
    let lines: Vec<u32> = check(&rule, source).iter().map(|i| i.line).collect();
    assert_eq!(lines, vec![2]);
}

#[test]
fn cod002_skips_probe_chains_and_non_codable_decode() {
    let rule = rules::codable::TryOptionalDecoding;
    let source = r#"
init(from decoder: Decoder) throws {
    let container = try decoder.singleValueContainer()
    if let object = try? container.decode([String: JSON].self) {
        self = .object(object)
    } else if let array = try? container.decode([JSON].self) {
        self = .array(array)
    } else {
        throw DecodingError.dataCorrupted(.init(codingPath: [], debugDescription: ""))
    }
}
let script = try? segWitBuilder.decode(address: address)
guard let payload = try? JSONDecoder().decode(Payload.self, from: data) else { throw ParseError.invalid }
guard let tx = try? JSONDecoder().decode(Tx.self, from: source) else { items = []; return }
"#;
    let lines: Vec<u32> = check(&rule, source).iter().map(|i| i.line).collect();
    assert_eq!(lines, vec![14]);
}

// Second round: patterns left after the first fix.

#[test]
fn conc001_is_high_only_when_tasks_touch_state() {
    let rule = rules::concurrency::MissingMainActor;
    let source = r#"
final class RoutingViewModel: ObservableObject {
    @Published var route: Route?
    // Loads asynchronously
    func open() { Task { await coordinator.open() } }
}
final class LoadingViewModel: ObservableObject {
    @Published var isLoading = false
    func load() { Task { isLoading = await service.load() } }
}
"#;
    assert_eq!(
        found(&check(&rule, source)),
        vec![(2, Severity::Advisory), (7, Severity::High)]
    );
}

#[test]
fn conc005_ignores_wrappers_hops_and_main_actor_methods() {
    let rule = rules::concurrency::SendableViolation;
    let source = r#"
final class Injected {
    @Injected(\.service) var service: Service
    func run() { Task { await service.load() } }
}
final class Hopping {
    var title = ""
    func run() { Task { let t = await load(); await MainActor.run { title = t } } }
}
final class MainMethod {
    var title = ""
    @MainActor func run() { Task { title = await load() } }
}
final class Caching {
    var task: Task<Void, Never>?
    func run() { task = Task { defer { task = nil }; await work() } }
}
"#;
    let lines: Vec<u32> = check(&rule, source).iter().map(|i| i.line).collect();
    assert_eq!(lines, vec![14]);
}

#[test]
fn mem001_weak_capture_helpers_and_stored_call_results() {
    let rule = rules::memory::ClosureRetainCycle;
    let source = r#"
final class Coordinator {
    var bag = Set<AnyCancellable>()
    var handle: Handle?
    func bind() {
        publisher.withWeakCaptureOf(self).sink { (self, value) in self.apply(value) }.store(in: &bag)
        handle = safari.openURL(url, onSuccess: { _ in self.resume() })
    }
}
"#;
    assert_eq!(found(&check(&rule, source)), vec![(7, Severity::High)]);
}

#[test]
fn cod002_optional_results_and_fallback_decodes_are_intended() {
    let rule = rules::codable::TryOptionalDecoding;
    let source = r#"
func cached(_ data: Data) -> Item? {
    return try? JSONDecoder().decode(Item.self, from: data)
}
func parse(_ c: SingleValueDecodingContainer) throws -> Value {
    if let number = try? c.decode(Int.self) { return .number(number) }
    return .string(try c.decode(String.self))
}
func details(_ d: Data) {
    guard let tx = try? JSONDecoder().decode(Tx.self, from: d) else { items = []; return }
}
"#;
    let lines: Vec<u32> = check(&rule, source).iter().map(|i| i.line).collect();
    assert_eq!(lines, vec![10]);
}

#[test]
fn sec004_skips_links_opened_by_the_user() {
    let rule = rules::security::AtsBypass;
    let source = "extension DashExternalLinkProvider: ExternalLinkProvider {\n    var testnetFaucetURL: URL? { URL(string: \"http://faucet.test.dash.io/\") }\n}\nlet config = BlockBookConfig(restNode: \"http://bsc-blockbook.io\")\n";
    let lines: Vec<u32> = check(&rule, source).iter().map(|i| i.line).collect();
    assert_eq!(lines, vec![4]);
}

#[test]
fn a11y_second_round_patterns() {
    let label = rules::accessibility::MissingAccessibilityLabel;
    let src = r#"
struct Avatar: View {
    var body: some View {
        if let image { Image(uiImage: image) } else { Placeholder() }
    }
}
"#;
    assert!(check(&label, src).is_empty(), "{:?}", check(&label, src));

    let font = rules::accessibility::FixedFontSize;
    let src = r#"
struct Close: View {
    var body: some View {
        Image(systemName: "xmark").font(.system(size: 17))
        Text("Title").font(.system(size: 17))
    }
}
"#;
    assert_eq!(
        check(&font, src).iter().map(|i| i.line).collect::<Vec<_>>(),
        vec![5]
    );

    let target = rules::accessibility::SmallTouchTarget;
    let src = r#"
struct Bar: View {
    var body: some View {
        Button { a() } label: { Image("a").frame(width: 20, height: 20) }
            .allowsHitTesting(false)
        ToolbarItem(placement: .navigationBarLeading) {
            Button { b() } label: { Image("b").frame(width: 24, height: 24) }
        }
        Image("c").frame(width: 10, height: 10).frame(width: 30, height: 30).onTapGesture { c() }
    }
}
"#;
    let lines: Vec<(u32, Option<u32>)> = check(&target, src)
        .iter()
        .map(|i| (i.line, i.column))
        .collect();
    assert_eq!(lines, vec![(9, Some(49))]);
}

#[test]
fn sui005_is_advisory_and_skips_previews() {
    let rule = rules::swiftui_perf::NonLazyList;
    let src = r#"
struct V: View {
    var body: some View {
        ScrollView { VStack { ForEach(model.items) { Text($0.title) } } }
    }
}
#Preview {
    ScrollView { VStack { ForEach(items) { Text($0) } } }
}
"#;
    assert_eq!(found(&check(&rule, src)), vec![(4, Severity::Advisory)]);
}
