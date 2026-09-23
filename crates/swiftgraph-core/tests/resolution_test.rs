//! Receiver-aware call resolution in tree-sitter mode.
//!
//! The fixture has several types with same-named methods (`update`, `sync`,
//! `load`), stdlib and SwiftUI calls (`.map {}`, `.frame()`, `.padding()`)
//! with project symbols of the same name, private functions with the same
//! name in two files, and calls through `self.`, typed variables, static
//! members and inherited/protocol-extension members.

use std::path::{Path, PathBuf};

use rusqlite::Connection;
use swiftgraph_core::pipeline::{self, SwiftSyntaxMode};
use swiftgraph_core::storage;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/resolution")
}

fn indexed() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("db.sqlite");
    pipeline::index_directory_with_options(&db, &fixture(), true, None, &SwiftSyntaxMode::Disabled)
        .unwrap();
    let conn = storage::open_db(&db).unwrap();
    (dir, conn)
}

/// Node ID by qualified name, optionally restricted to a file name.
fn id(conn: &Connection, qualified: &str, file: Option<&str>) -> String {
    let pattern = file.map_or_else(|| "%".to_string(), |f| format!("%/{f}"));
    conn.query_row(
        "SELECT id FROM nodes WHERE qualified_name = ?1 AND file LIKE ?2 ORDER BY id LIMIT 1",
        rusqlite::params![qualified, pattern],
        |r| r.get(0),
    )
    .unwrap_or_else(|e| panic!("{qualified} not found: {e}"))
}

/// `(target qualified name, ambiguous)` of call edges from `source`.
fn calls(conn: &Connection, source: &str) -> Vec<(String, bool)> {
    let mut stmt = conn
        .prepare(
            "SELECT n.qualified_name, e.ambiguous FROM edges e JOIN nodes n ON n.id = e.target
             WHERE e.source = ?1 AND e.kind = 'calls' ORDER BY n.qualified_name, n.file",
        )
        .unwrap();
    let rows = stmt
        .query_map([source], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    rows.map(|r| r.unwrap()).collect()
}

fn confident(conn: &Connection, source: &str) -> Vec<String> {
    let mut v: Vec<String> = calls(conn, source)
        .into_iter()
        .filter(|(_, ambiguous)| !ambiguous)
        .map(|(q, _)| q)
        .collect();
    v.dedup();
    v
}

#[test]
fn self_and_implicit_calls_stay_in_the_container() {
    let (_d, conn) = indexed();
    let reload = id(&conn, "CartStore.reload()", None);
    let mut edges = calls(&conn, &reload);
    edges.dedup();
    assert_eq!(edges, vec![("CartStore.update()".to_string(), false)]);
}

#[test]
fn typed_receivers_resolve_to_members_of_the_declared_type() {
    let (_d, conn) = indexed();
    let refresh = id(&conn, "ViewModel.refresh(items:)", None);
    assert_eq!(
        confident(&conn, &refresh),
        vec![
            "CartStore",
            "CartStore.update()",
            "Helper.make()",
            "ProfileService.update(id:)",
            "WalletService.update(model:)",
        ]
    );
    assert!(
        calls(&conn, &refresh)
            .iter()
            .all(|(_, ambiguous)| !ambiguous),
        "{:?}",
        calls(&conn, &refresh)
    );

    let reset = id(&conn, "ViewModel.reset(store:)", None);
    assert_eq!(confident(&conn, &reset), vec!["CartStore.update()"]);
}

#[test]
fn stdlib_and_swiftui_calls_do_not_hit_project_symbols() {
    let (_d, conn) = indexed();
    let map = id(&conn, "Mapper.map(_:)", None);
    let frame = id(&conn, "Screen.frame", None);
    let padding = id(&conn, "Screen.padding()", None);
    for target in [map, frame, padding] {
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM edges WHERE target = ?1 AND kind = 'calls'",
                [&target],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 0, "unexpected calls into {target}");
    }
}

#[test]
fn private_functions_resolve_only_within_their_file() {
    let (_d, conn) = indexed();
    let run_a = id(&conn, "runA()", Some("FormatA.swift"));
    let format_a = id(&conn, "format()", Some("FormatA.swift"));
    let targets: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT target, ambiguous FROM edges WHERE source = ?1 AND kind = 'calls'")
            .unwrap();
        stmt.query_map([&run_a], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?))
        })
        .unwrap()
        .map(|r| r.unwrap())
        .inspect(|(_, ambiguous)| assert!(!ambiguous))
        .map(|(t, _)| t)
        .collect()
    };
    assert_eq!(targets, vec![format_a]);
}

#[test]
fn unknown_receivers_are_ambiguous_up_to_the_threshold_and_dropped_above() {
    let (_d, conn) = indexed();
    let user = id(&conn, "useUnknown()", None);
    let edges = calls(&conn, &user);
    // `sync` has 2 candidates (<= default threshold 3): kept, flagged ambiguous.
    assert_eq!(
        edges,
        vec![
            ("SyncA.sync()".to_string(), true),
            ("SyncB.sync()".to_string(), true),
        ]
    );
    // `load` has 4 candidates: no edges, but the name is recorded as used.
    let recorded: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM name_refs WHERE name = 'load'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(recorded, 1);
}

#[test]
fn inherited_and_protocol_extension_members_resolve() {
    let (_d, conn) = indexed();
    let close = id(&conn, "DetailScreen.close()", None);
    assert_eq!(
        confident(&conn, &close),
        vec!["BaseScreen.track()", "Coordinating.dismiss()"]
    );
}

#[test]
fn threshold_is_configurable() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("proj");
    std::fs::create_dir_all(root.join(".swiftgraph")).unwrap();
    std::fs::copy(
        fixture().join("Sources/Ambiguous.swift"),
        root.join("Ambiguous.swift"),
    )
    .unwrap();
    std::fs::write(
        root.join(".swiftgraph/config.json"),
        r#"{"resolution": {"max_candidates": 1}}"#,
    )
    .unwrap();
    let db = dir.path().join("db.sqlite");
    pipeline::index_directory_with_options(&db, &root, true, None, &SwiftSyntaxMode::Disabled)
        .unwrap();
    let conn = storage::open_db(&db).unwrap();
    let user = id(&conn, "useUnknown()", None);
    assert!(calls(&conn, &user).is_empty(), "{:?}", calls(&conn, &user));
}

#[test]
fn dead_code_keeps_symbols_referenced_by_name_and_reports_unreferenced_ones() {
    let (_d, conn) = indexed();
    let result =
        swiftgraph_core::analysis::dead_code::find_dead_code_from_conn(&conn, None, true, 500)
            .unwrap();
    let dead: Vec<&str> = result
        .dead_symbols
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    // Read as a member (`Units.secondInMillis`) or used as a type annotation.
    // `$previewFlag` reads the projected value of a top-level `@State`.
    for alive in [
        "Units",
        "secondInMillis",
        "ApiRequest",
        "previewFlag",
        "macroFlag",
    ] {
        assert!(!dead.contains(&alive), "{alive} reported dead: {dead:?}");
    }
    for unreferenced in ["Orphan", "orphanHelper"] {
        assert!(
            dead.contains(&unreferenced),
            "{unreferenced} not reported: {dead:?}"
        );
    }
}

#[test]
fn initializers_are_declarations_and_constructor_targets() {
    let (_d, conn) = indexed();
    let init = id(&conn, "Session.init(token:)", None);
    // Calls inside init belong to the initializer, not the file top level.
    assert_eq!(confident(&conn, &init), vec!["Session.configure()"]);
    // `Session(token:)` resolves to the matching initializer.
    let open = id(&conn, "openSession()", None);
    assert_eq!(confident(&conn, &open), vec!["Session.init(token:)"]);
    // `self.init(token:)` from a convenience initializer.
    let convenience = id(&conn, "Session.init()", None);
    assert_eq!(confident(&conn, &convenience), vec!["Session.init(token:)"]);
}

#[test]
fn property_types_declared_in_other_files_type_the_receiver() {
    let (_d, conn) = indexed();
    let total = id(&conn, "Checkout.total()", None);
    let mut edges = calls(&conn, &total);
    edges.dedup();
    assert_eq!(
        edges,
        vec![
            ("PricingService.compute()".to_string(), false),
            ("TaxService.compute()".to_string(), false),
        ]
    );
}
