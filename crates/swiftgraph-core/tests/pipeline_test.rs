//! End-to-end tests for `pipeline::index_directory` in tree-sitter mode:
//! incremental reindex, deleted files, FTS consistency.

use std::path::Path;

use swiftgraph_core::pipeline;
use swiftgraph_core::storage::{self, queries};

fn index(db: &Path, root: &Path) {
    pipeline::index_directory_with_store(db, root, false, None).unwrap();
}

fn count(conn: &rusqlite::Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

fn fts_ok(conn: &rusqlite::Connection) {
    conn.execute_batch("INSERT INTO node_fts(node_fts, rank) VALUES('integrity-check', 1);")
        .expect("node_fts out of sync with nodes");
    conn.execute_batch(
        "INSERT INTO node_trigram(node_trigram, rank) VALUES('integrity-check', 1);",
    )
    .expect("node_trigram out of sync with nodes");
}

#[test]
fn reindexing_a_changed_file_does_not_duplicate_edges() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("A.swift");
    let db = dir.path().join(".swiftgraph/db.sqlite");
    std::fs::write(
        &src,
        "class A {\n    var x = 0\n    func f() { g() }\n    func g() {}\n}\n",
    )
    .unwrap();
    index(&db, dir.path());
    let conn = storage::open_db(&db).unwrap();
    let edges_before = count(&conn, "SELECT COUNT(*) FROM edges");
    let nodes_before = count(&conn, "SELECT COUNT(*) FROM nodes");
    drop(conn);

    for i in 0..3 {
        std::fs::write(
            &src,
            format!("class A {{\n    var x = 0\n    func f() {{ g() }}\n    func g() {{}}\n}}\n// {i}\n"),
        )
        .unwrap();
        index(&db, dir.path());
    }

    let conn = storage::open_db(&db).unwrap();
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM nodes"), nodes_before);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM edges"), edges_before);
    fts_ok(&conn);
}

#[test]
fn deleted_files_are_purged() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join(".swiftgraph/db.sqlite");
    std::fs::write(
        dir.path().join("Keep.swift"),
        "struct Keeper {\n    func run() { poof() }\n}\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("Gone.swift"),
        "class Vanishing {\n    func poof() {}\n}\n",
    )
    .unwrap();
    index(&db, dir.path());

    {
        let conn = storage::open_db(&db).unwrap();
        assert!(
            count(
                &conn,
                "SELECT COUNT(*) FROM edges WHERE target LIKE '%Gone.swift%'"
            ) > 0,
            "fixture should produce a cross-file call edge into Gone.swift"
        );
    }

    std::fs::remove_file(dir.path().join("Gone.swift")).unwrap();
    index(&db, dir.path());

    let conn = storage::open_db(&db).unwrap();
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM files WHERE path LIKE '%Gone.swift'"
        ),
        0
    );
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM nodes WHERE file LIKE '%Gone.swift'"
        ),
        0
    );
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM edges WHERE source LIKE '%Gone.swift%' OR target LIKE '%Gone.swift%'"
        ),
        0
    );
    assert!(queries::search_nodes(&conn, "Vanishing", 10)
        .unwrap()
        .is_empty());
    fts_ok(&conn);
}

#[test]
fn upserting_the_same_node_keeps_fts_consistent() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("db.sqlite");
    std::fs::write(dir.path().join("A.swift"), "struct Alpha {}\n").unwrap();
    index(&db, dir.path());
    // A forced reindex and a plain reindex of unchanged content.
    pipeline::index_directory_with_store(&db, dir.path(), true, None).unwrap();
    index(&db, dir.path());

    let conn = storage::open_db(&db).unwrap();
    let node = queries::search_nodes(&conn, "Alpha", 10).unwrap();
    assert_eq!(node.len(), 1);
    queries::upsert_node(&conn, &node[0]).unwrap();
    queries::upsert_node(&conn, &node[0]).unwrap();
    fts_ok(&conn);
    assert_eq!(queries::search_nodes(&conn, "Alpha", 10).unwrap().len(), 1);
}

#[test]
fn legacy_database_is_rebuilt_with_current_schema() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("db.sqlite");
    {
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE nodes (id TEXT PRIMARY KEY, name TEXT NOT NULL);
             INSERT INTO nodes VALUES ('x', 'Old');",
        )
        .unwrap();
    }
    let conn = storage::open_db(&db).unwrap();
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, storage::SCHEMA_VERSION as i64);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM nodes"), 0);
    fts_ok(&conn);
}

/// Call edges from unchanged files into a changed file must point at the
/// changed file's current node IDs (IDs contain line numbers).
#[test]
fn incremental_reindex_refreshes_incoming_edges() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("db.sqlite");
    let a = dir.path().join("Service.swift");
    std::fs::write(&a, "final class Service {\n    func run() {}\n}\n").unwrap();
    std::fs::write(
        dir.path().join("Client.swift"),
        "func use(service: Service) {\n    service.run()\n    service.stop()\n}\n",
    )
    .unwrap();
    index(&db, dir.path());

    let dangling = |conn: &rusqlite::Connection| {
        count(
            conn,
            "SELECT COUNT(*) FROM edges e WHERE e.kind = 'calls' AND NOT EXISTS (SELECT 1 FROM nodes n WHERE n.id = e.target)",
        )
    };
    let callee = |conn: &rusqlite::Connection, name: &str| {
        count(
            conn,
            &format!("SELECT COUNT(*) FROM edges e JOIN nodes n ON n.id = e.target WHERE e.kind = 'calls' AND n.name = '{name}' AND e.file LIKE '%Client.swift'"),
        )
    };
    let conn = storage::open_db(&db).unwrap();
    assert_eq!(callee(&conn, "run"), 1);
    assert_eq!(callee(&conn, "stop"), 0);
    drop(conn);

    // `run` moves down a line and `stop` appears; Client.swift is unchanged.
    std::fs::write(
        &a,
        "final class Service {\n    // moved\n    func run() {}\n    func stop() {}\n}\n",
    )
    .unwrap();
    index(&db, dir.path());

    let conn = storage::open_db(&db).unwrap();
    assert_eq!(dangling(&conn), 0);
    assert_eq!(callee(&conn, "run"), 1);
    assert_eq!(callee(&conn, "stop"), 1);
}
