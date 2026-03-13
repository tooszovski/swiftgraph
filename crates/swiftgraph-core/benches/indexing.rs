use std::path::Path;

use criterion::{criterion_group, criterion_main, Criterion};
use swiftgraph_core::storage;
use swiftgraph_core::storage::queries;

fn bench_search(c: &mut Criterion) {
    // Use the test fixture DB if it exists, otherwise skip
    let db_path = std::path::Path::new("../../tests/fixtures/bench.sqlite");
    if !db_path.exists() {
        eprintln!("Skipping benchmark: no fixture DB at {}", db_path.display());
        return;
    }

    let conn = storage::open_db(db_path).expect("open bench DB");

    c.bench_function("fts5_search", |b| {
        b.iter(|| {
            let _ = queries::search_nodes(&conn, "ViewModel*", 20);
        });
    });

    c.bench_function("like_search", |b| {
        b.iter(|| {
            let _ = queries::find_nodes_by_name(&conn, "ViewModel", None, 20);
        });
    });

    c.bench_function("trigram_search", |b| {
        b.iter(|| {
            let _ = queries::search_nodes_trigram(&conn, "Manager", 20);
        });
    });
}

fn bench_queries(c: &mut Criterion) {
    let db_path = std::path::Path::new("../../tests/fixtures/bench.sqlite");
    if !db_path.exists() {
        return;
    }

    let conn = storage::open_db(db_path).expect("open bench DB");

    c.bench_function("get_callers", |b| {
        b.iter(|| {
            let _ = queries::get_callers(&conn, "AppDelegate", 30);
        });
    });

    c.bench_function("get_stats", |b| {
        b.iter(|| {
            let _ = queries::get_stats(&conn);
        });
    });
}

fn bench_tree_sitter(c: &mut Criterion) {
    use swiftgraph_core::tree_sitter::parser::TreeSitterParser;

    let source = r#"
import Foundation
import SwiftUI

@MainActor
class AppDelegate: NSObject, UIApplicationDelegate {
    var window: UIWindow?

    func application(_ application: UIApplication,
                     didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]?) -> Bool {
        setupLogger()
        configureAppearance()
        return true
    }

    private func setupLogger() {
        Logger.shared.configure(level: .debug)
    }

    private func configureAppearance() {
        UINavigationBar.appearance().tintColor = .systemBlue
    }
}

struct ContentView: View {
    @StateObject var viewModel = ContentViewModel()

    var body: some View {
        NavigationStack {
            List(viewModel.items) { item in
                Text(item.title)
            }
            .onAppear { viewModel.loadItems() }
        }
    }
}

class ContentViewModel: ObservableObject {
    @Published var items: [Item] = []

    func loadItems() {
        Task {
            items = try await fetchItems()
        }
    }

    func fetchItems() async throws -> [Item] {
        return []
    }
}

struct Item: Identifiable {
    let id: UUID
    let title: String
}
"#;

    c.bench_function("tree_sitter_parse", |b| {
        b.iter(|| {
            let mut parser = TreeSitterParser::new().unwrap();
            let _ = parser.parse_source(source, Path::new("bench.swift"));
        });
    });
}

criterion_group!(benches, bench_search, bench_queries, bench_tree_sitter);
criterion_main!(benches);
