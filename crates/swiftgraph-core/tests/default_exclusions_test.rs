//! Package manifests and test targets are excluded from dead-code and
//! complexity unless `include_tests` is set.

use std::path::Path;

use swiftgraph_core::analysis::{complexity, dead_code};
use swiftgraph_core::pipeline::{self, SwiftSyntaxMode};
use swiftgraph_core::storage;

fn excluded(file: &str) -> bool {
    file.ends_with("/Package.swift") || file.contains("UITests/") || file.contains("TestKit/")
}

#[test]
fn manifests_and_test_targets_are_excluded_by_default() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("db.sqlite");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/resolution");
    pipeline::index_directory_with_options(&db, &root, true, None, &SwiftSyntaxMode::Disabled)
        .unwrap();
    let conn = storage::open_db(&db).unwrap();

    let dead = dead_code::find_dead_code_from_conn(&conn, None, false, 1000).unwrap();
    assert!(
        dead.dead_symbols.iter().all(|s| !excluded(&s.file)),
        "{:?}",
        dead.dead_symbols
    );
    let dead_all = dead_code::find_dead_code_from_conn(&conn, None, true, 1000).unwrap();
    for name in ["package", "makeStubProfile"] {
        assert!(
            dead_all.dead_symbols.iter().any(|s| s.name == name),
            "{name} missing with include_tests"
        );
    }

    let cx = complexity::analyze_complexity_from_conn(&conn, None, 1000, "score", false).unwrap();
    assert!(
        cx.symbols.iter().all(|s| !excluded(&s.file)),
        "{:?}",
        cx.symbols
    );
    let cx_all =
        complexity::analyze_complexity_from_conn(&conn, None, 1000, "score", true).unwrap();
    assert!(cx_all.symbols.iter().any(|s| excluded(&s.file)));
    assert!(cx_all.total_symbols > cx.total_symbols);
}

#[test]
fn test_path_predicate() {
    use swiftgraph_core::analysis::is_test_or_manifest;
    for path in [
        "/p/App/Package.swift",
        "/p/AppTests/FooTests.swift",
        "/p/AppUITests/Screens/Main.swift",
        "/p/Tests/AppTests/A.swift",
        "/p/Modules/TangoTestKit/Stub.swift",
        "/p/App/LoginTests.swift",
    ] {
        assert!(is_test_or_manifest(path), "{path}");
    }
    for path in [
        "/p/App/TestsHelper/Feature.swift",
        "/p/App/Contest/Entry.swift",
        "/p/App/Testimonials/View.swift",
        "/p/App/PackageList.swift",
    ] {
        assert!(!is_test_or_manifest(path), "{path}");
    }
}

#[test]
fn cycles_skip_test_targets_by_default() {
    use swiftgraph_core::analysis::cycles;
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("db.sqlite");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/resolution");
    pipeline::index_directory_with_options(&db, &root, true, None, &SwiftSyntaxMode::Disabled)
        .unwrap();
    let conn = storage::open_db(&db).unwrap();

    let all = cycles::detect_cycles_from_conn(&conn, None, 100, true).unwrap();
    assert!(
        all.cycles
            .iter()
            .any(|c| c.files.iter().all(|f| f.contains("AppUITests/"))),
        "fixture cycle not found: {:?}",
        all.cycles
    );
    let default = cycles::detect_cycles_from_conn(&conn, None, 100, false).unwrap();
    assert!(
        default
            .cycles
            .iter()
            .all(|c| c.files.iter().all(|f| !excluded(f))),
        "{:?}",
        default.cycles
    );
}
