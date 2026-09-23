//! Every analysis must return byte-identical JSON for the same database.
//! HashSet/HashMap iteration order differs between instances, so repeated
//! calls in one process expose any unsorted hash-based output.

use std::path::{Path, PathBuf};

use swiftgraph_core::analysis::{
    architecture, boundaries, complexity, context, coupling, cycles, dead_code, impact, imports,
};
use swiftgraph_core::graph::*;
use swiftgraph_core::storage::{self, queries};

const DIRS: [&str; 4] = ["Views", "Services", "Models", "Utils"];
const N: usize = 16;

fn file(i: usize) -> String {
    format!("/p/Sources/{}/F{i}.swift", DIRS[i % DIRS.len()])
}

fn node(id: String, name: String, kind: SymbolKind, file: String, line: u32) -> GraphNode {
    GraphNode {
        qualified_name: name.clone(),
        id,
        name,
        kind,
        sub_kind: None,
        location: Location {
            file,
            line,
            column: 1,
            end_line: None,
            end_column: None,
        },
        signature: None,
        attributes: vec![],
        access_level: AccessLevel::Internal,
        container_usr: None,
        doc_comment: None,
        metrics: None,
    }
}

fn edge(s: String, t: String, kind: EdgeKind, file: String, line: u32) -> GraphEdge {
    GraphEdge {
        source: s,
        target: t,
        kind,
        location: Some(Location {
            file,
            line,
            column: 1,
            end_line: None,
            end_column: None,
        }),
        is_implicit: false,
        ambiguous: false,
    }
}

fn build_db() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("db.sqlite");
    let conn = storage::open_db(&db).unwrap();
    for i in 0..N {
        let f = file(i);
        queries::upsert_file(&conn, &f, "h", 3).unwrap();
        let ty = ["ViewModel", "Service", "Model", "View"][i % 4];
        let nodes = [
            node(
                format!("t{i}"),
                format!("T{i}{ty}"),
                SymbolKind::Class,
                f.clone(),
                1,
            ),
            node(
                format!("f{i}"),
                format!("load{i}"),
                SymbolKind::Function,
                f.clone(),
                2,
            ),
            node(
                format!("p{i}"),
                format!("Proto{i}"),
                SymbolKind::Protocol,
                f.clone(),
                3,
            ),
            node(
                format!("i{i}"),
                ["Foundation", "UIKit", "Combine", "SwiftUI"][i % 4].into(),
                SymbolKind::Import,
                f.clone(),
                0,
            ),
        ];
        for n in &nodes {
            queries::upsert_node(&conn, n).unwrap();
        }
    }
    for i in 0..N {
        let j = (i + 1) % N;
        let k = (i + N / 2) % N;
        for e in [
            edge(
                format!("f{i}"),
                format!("f{j}"),
                EdgeKind::Calls,
                file(i),
                10,
            ),
            edge(
                format!("f{i}"),
                format!("f{k}"),
                EdgeKind::Calls,
                file(i),
                11,
            ),
            edge(
                format!("t{i}"),
                format!("p{k}"),
                EdgeKind::ConformsTo,
                file(i),
                1,
            ),
            edge(
                format!("t{i}"),
                format!("t{j}"),
                EdgeKind::References,
                file(i),
                12,
            ),
        ] {
            queries::insert_edge(&conn, &e).unwrap();
        }
    }
    (dir, db)
}

fn assert_stable<T: serde::Serialize>(name: &str, mut f: impl FnMut() -> T) {
    let first = serde_json::to_string(&f()).unwrap();
    for _ in 0..8 {
        assert_eq!(
            serde_json::to_string(&f()).unwrap(),
            first,
            "{name} output is unstable"
        );
    }
}

#[test]
fn analyses_are_deterministic() {
    let (_dir, db) = build_db();
    let db: &Path = &db;

    assert_stable("context", || {
        context::build_context(db, "load view model service", 40, true).unwrap()
    });
    assert_stable("impact", || impact::analyze_impact(db, "f0", 4).unwrap());
    assert_stable("complexity", || {
        complexity::analyze_complexity(db, None, 20, "score", true).unwrap()
    });
    assert_stable("complexity fan_in", || {
        complexity::analyze_complexity(db, None, 7, "fan_in", true).unwrap()
    });
    assert_stable("cycles", || {
        cycles::detect_cycles(db, None, 5, true).unwrap()
    });
    assert_stable("dead_code", || {
        dead_code::find_dead_code(db, None, true, 50).unwrap()
    });
    assert_stable("coupling", || {
        coupling::analyze_coupling(db, 3, Some("/p")).unwrap()
    });
    assert_stable("architecture", || {
        architecture::analyze_architecture(db, None).unwrap()
    });
    assert_stable("imports", || imports::analyze_imports(db, None).unwrap());
    let cfg: boundaries::BoundaryConfig = serde_json::from_str(
        r#"{"layers": [
              {"name": "Views", "pattern": "**/Views/**"},
              {"name": "Services", "pattern": "**/Services/**"},
              {"name": "Models", "pattern": "**/Models/**"},
              {"name": "Utils", "pattern": "**/Utils/**"}],
            "rules": [{"from": "Models", "to": "Views", "allowed": false},
                      {"from": "Utils", "to": "Services", "allowed": false}]}"#,
    )
    .unwrap();
    assert_stable("boundaries", || {
        boundaries::check_boundaries(db, &cfg).unwrap()
    });
}
