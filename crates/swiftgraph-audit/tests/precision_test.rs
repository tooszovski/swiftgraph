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
