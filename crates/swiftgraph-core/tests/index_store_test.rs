//! End-to-end indexing against a real Index Store produced by `swift build`.

mod common;

use swiftgraph_core::pipeline::{self, IndexStrategy};
use swiftgraph_core::storage::{self, queries};

macro_rules! fixture_or_skip {
    () => {
        match common::built_spm_fixture() {
            Some(root) => root,
            None => {
                eprintln!("skipped: swift toolchain unavailable");
                return;
            }
        }
    };
}

#[test]
fn spm_index_store_is_auto_detected() {
    let root = fixture_or_skip!();
    let info = swiftgraph_core::project::detect_project(&root).unwrap();
    assert!(
        info.index_store_path.is_some(),
        "SwiftPM store under .build/<triple>/debug/index/store not found"
    );
}

#[test]
fn index_directory_uses_index_store_automatically() {
    let root = fixture_or_skip!();
    let db_dir = tempfile::tempdir().unwrap();
    let db = db_dir.path().join("db.sqlite");

    let result = pipeline::index_directory(&db, &root, false).unwrap();
    assert_ne!(result.strategy, IndexStrategy::TreeSitter);

    let conn = storage::open_db(&db).unwrap();
    assert_eq!(
        queries::get_meta(&conn, "index_strategy")
            .unwrap()
            .as_deref(),
        Some(result.strategy.as_str())
    );
}

#[test]
fn switching_backend_rebuilds_instead_of_mixing() {
    let root = fixture_or_skip!();
    let db_dir = tempfile::tempdir().unwrap();
    let db = db_dir.path().join("db.sqlite");

    // First pass: tree-sitter only.
    let first = pipeline::index_directory_with_store(&db, &root, false, None).unwrap();
    assert_eq!(first.strategy, IndexStrategy::TreeSitter);

    // Second pass: Index Store available — must not keep stale ts:: nodes
    // for sources the store covers (Package.swift is not compiled, so it
    // legitimately stays on tree-sitter).
    let second = pipeline::index_directory(&db, &root, false).unwrap();
    assert!(second.strategy.uses_index_store());

    let conn = storage::open_db(&db).unwrap();
    let ts_nodes: i64 = conn
        .query_row(
            // Imports stay tree-sitter nodes: the store does not record them
            "SELECT COUNT(*) FROM nodes WHERE id LIKE 'ts::%' AND kind != 'import' AND file LIKE '%/Sources/%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(ts_nodes, 0, "tree-sitter nodes left over after switching");
}

#[test]
fn relative_root_does_not_duplicate_files() {
    let root = fixture_or_skip!();
    let db_dir = tempfile::tempdir().unwrap();
    let db = db_dir.path().join("db.sqlite");
    let cwd = std::env::current_dir().unwrap();
    let relative = pathdiff(&root, &cwd);

    pipeline::index_directory(&db, &relative, false).unwrap();
    let conn = storage::open_db(&db).unwrap();
    let models: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM files WHERE path LIKE '%Sources/Fixture/Models.swift'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(models, 1, "same file stored under two different paths");
}

/// `target` relative to `base` (both absolute), via `..` components.
fn pathdiff(target: &std::path::Path, base: &std::path::Path) -> std::path::PathBuf {
    let base = base.canonicalize().unwrap();
    let t: Vec<_> = target.components().collect();
    let b: Vec<_> = base.components().collect();
    let common = t.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let mut out = std::path::PathBuf::new();
    for _ in common..b.len() {
        out.push("..");
    }
    for c in &t[common..] {
        out.push(c);
    }
    out
}

#[test]
fn reader_extracts_symbols_from_real_store() {
    use swiftgraph_core::index_store::{ffi::IndexStoreLib, reader};
    let root = fixture_or_skip!();
    let Ok(lib) = IndexStoreLib::load() else {
        eprintln!("skipped: libIndexStore unavailable");
        return;
    };
    let store = swiftgraph_core::project::detect_project(&root)
        .unwrap()
        .index_store_path
        .unwrap();
    let data = reader::read_index_store(&lib, &store).unwrap();
    assert!(data.units_read > 0);
    for name in [
        "User",
        "UserStore",
        "MemoryStore",
        "Screen",
        "greeting(name:)",
    ] {
        assert!(
            data.nodes
                .iter()
                .any(|n| n.name == name && n.id.starts_with("s:")),
            "missing {name}"
        );
    }
    assert!(data
        .edges
        .iter()
        .any(|e| e.kind == swiftgraph_core::graph::EdgeKind::Calls));
}

#[test]
fn conformance_edges_point_from_type_to_protocol() {
    use swiftgraph_core::graph::EdgeKind;
    use swiftgraph_core::index_store::{ffi::IndexStoreLib, reader};
    let root = fixture_or_skip!();
    let Ok(lib) = IndexStoreLib::load() else {
        return;
    };
    let store = swiftgraph_core::project::detect_project(&root)
        .unwrap()
        .index_store_path
        .unwrap();
    let data = reader::read_index_store(&lib, &store).unwrap();
    let usr = |name: &str| {
        data.nodes
            .iter()
            .find(|n| n.name == name)
            .map(|n| n.id.clone())
            .unwrap()
    };
    let (store_usr, proto_usr) = (usr("MemoryStore"), usr("UserStore"));
    assert!(
        data.edges.iter().any(|e| e.kind == EdgeKind::ConformsTo
            && e.source == store_usr
            && e.target == proto_usr),
        "expected MemoryStore -conformsTo-> UserStore"
    );
    assert!(!data
        .edges
        .iter()
        .any(|e| e.kind == EdgeKind::InheritsFrom && e.source == proto_usr));
}

fn store_data() -> Option<swiftgraph_core::index_store::reader::IndexStoreData> {
    use swiftgraph_core::index_store::{ffi::IndexStoreLib, reader};
    let root = common::built_spm_fixture()?;
    let lib = IndexStoreLib::load().ok()?;
    let store = swiftgraph_core::project::detect_project(&root)
        .ok()?
        .index_store_path?;
    reader::read_index_store(&lib, &store).ok()
}

#[test]
fn accessors_fold_into_their_property() {
    let Some(data) = store_data() else { return };
    assert!(
        !data.nodes.iter().any(|n| n.name.contains(':')
            && ["getter:", "setter:", "_modify:", "_read:", "willSet:", "didSet:"]
                .iter()
                .any(|p| n.name.starts_with(p))),
        "accessor nodes: {:?}",
        data.nodes
            .iter()
            .filter(|n| n.name.contains("etter:"))
            .map(|n| &n.name)
            .collect::<Vec<_>>()
    );
    let value = data.nodes.iter().find(|n| n.name == "value").unwrap();
    // `counter.value = 1` writes through the setter: the reference lands on the property
    assert!(data
        .edges
        .iter()
        .any(|e| e.target == value.id && e.source.contains("makeCounter")));
    // The getter reads `storage`: attributed to the property
    let storage = data.nodes.iter().find(|n| n.name == "storage").unwrap();
    assert!(data
        .edges
        .iter()
        .any(|e| e.source == value.id && e.target == storage.id));
}

#[test]
fn members_are_contained_by_their_type() {
    use swiftgraph_core::graph::EdgeKind;
    let Some(data) = store_data() else { return };
    let store = data.nodes.iter().find(|n| n.name == "MemoryStore").unwrap();
    let load = data
        .nodes
        .iter()
        .find(|n| n.name == "load(id:)" && n.location.file.ends_with("Store.swift"))
        .unwrap();
    assert!(data
        .edges
        .iter()
        .any(|e| e.kind == EdgeKind::Contains && e.source == store.id && e.target == load.id));
    assert_eq!(load.container_usr.as_deref(), Some(store.id.as_str()));
    assert!(
        load.qualified_name.contains("MemoryStore"),
        "{}",
        load.qualified_name
    );
}

#[test]
fn type_references_become_edges() {
    use swiftgraph_core::graph::EdgeKind;
    let Some(data) = store_data() else { return };
    let tag = data.nodes.iter().find(|n| n.name == "Tag").unwrap();
    assert!(data
        .edges
        .iter()
        .any(|e| e.target == tag.id && e.kind == EdgeKind::References));
}

#[test]
fn hybrid_nodes_are_stitched_with_tree_sitter_details() {
    let root = fixture_or_skip!();
    let db_dir = tempfile::tempdir().unwrap();
    let db = db_dir.path().join("db.sqlite");
    let result = pipeline::index_directory_with_options(
        &db,
        &root,
        true,
        swiftgraph_core::project::detect_project(&root)
            .unwrap()
            .index_store_path
            .as_deref(),
        &pipeline::SwiftSyntaxMode::Disabled,
    )
    .unwrap();
    assert!(result.strategy.uses_index_store());
    let conn = storage::open_db(&db).unwrap();

    let vm = queries::resolve_symbol(&conn, "CounterViewModel")
        .unwrap()
        .unwrap();
    assert!(vm.id.starts_with("s:"), "{}", vm.id);
    assert!(
        vm.attributes.iter().any(|a| a == "@MainActor"),
        "{:?}",
        vm.attributes
    );
    assert!(vm.location.end_line.is_some());
    assert!(vm.signature.is_some());

    let reset = conn
        .query_row(
            "SELECT access_level FROM nodes WHERE name = 'reset()' AND id LIKE 's:%'",
            [],
            |r| r.get::<_, String>(0),
        )
        .unwrap();
    assert_eq!(reset, "Private");

    // Imports come from tree-sitter for files the store covers
    let imports: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM nodes WHERE kind = 'import' AND name = 'Foundation' AND file LIKE '%Counter.swift'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(imports, 1);
    // No tree-sitter duplicates of declarations the store has
    let dupes: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM nodes WHERE id LIKE 'ts::%' AND kind != 'import' AND file LIKE '%/Sources/%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(dupes, 0);
}
