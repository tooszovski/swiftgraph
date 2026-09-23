//! Ambiguous call edges (receiver unknown, several candidates) are kept for
//! navigation but must not drive cycles, complexity or impact. For dead-code
//! they count as "possibly used", and so do names recorded in
//! `name_refs`.

use rusqlite::Connection;
use swiftgraph_core::analysis::{complexity, cycles, dead_code, impact};
use swiftgraph_core::graph::*;
use swiftgraph_core::storage::{open_memory_db, queries};

fn node(id: &str, name: &str, file: &str) -> GraphNode {
    GraphNode {
        id: id.into(),
        name: name.into(),
        qualified_name: name.into(),
        kind: SymbolKind::Function,
        sub_kind: None,
        location: Location {
            file: file.into(),
            line: 1,
            column: 1,
            end_line: Some(3),
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

fn call(conn: &Connection, source: &str, target: &str, file: &str, ambiguous: bool) {
    queries::insert_edge(
        conn,
        &GraphEdge {
            source: source.into(),
            target: target.into(),
            kind: EdgeKind::Calls,
            location: Some(Location {
                file: file.into(),
                line: 2,
                column: 5,
                end_line: None,
                end_column: None,
            }),
            is_implicit: false,
            ambiguous,
        },
    )
    .unwrap();
}

/// a <-> b through ambiguous edges only; c -> d confident; e never called
/// but its name was seen at an unresolved call site; f never called.
fn graph() -> Connection {
    let conn = open_memory_db().unwrap();
    for (id, file) in [
        ("a", "A.swift"),
        ("b", "B.swift"),
        ("c", "C.swift"),
        ("d", "D.swift"),
        ("e", "E.swift"),
        ("f", "F.swift"),
    ] {
        queries::upsert_file(&conn, file, "h", 1).unwrap();
        queries::upsert_node(&conn, &node(id, &format!("{id}Func"), file)).unwrap();
    }
    call(&conn, "a", "b", "A.swift", true);
    call(&conn, "b", "a", "B.swift", true);
    call(&conn, "c", "d", "C.swift", false);
    conn.execute(
        "INSERT INTO name_refs (file, name) VALUES ('C.swift', 'eFunc')",
        [],
    )
    .unwrap();
    conn
}

#[test]
fn cycles_ignore_ambiguous_edges() {
    let conn = graph();
    let result = cycles::detect_cycles_from_conn(&conn, None, 100, true).unwrap();
    assert!(result.cycles.is_empty(), "{:?}", result.cycles);
    assert!(!result.truncated);
}

#[test]
fn complexity_ignores_ambiguous_edges() {
    let conn = graph();
    let result = complexity::analyze_complexity_from_conn(&conn, None, 100, "score", true).unwrap();
    let get = |id: &str| result.symbols.iter().find(|s| s.id == id).unwrap();
    assert_eq!((get("a").fan_in, get("a").fan_out), (0, 0));
    assert_eq!((get("d").fan_in, get("c").fan_out), (1, 1));
    assert_eq!(result.total_symbols, 6);
    assert!(!result.truncated);

    let top1 = complexity::analyze_complexity_from_conn(&conn, None, 1, "score", true).unwrap();
    assert_eq!(top1.symbols.len(), 1);
    assert_eq!(top1.total_symbols, 6);
    assert!(top1.truncated);
}

#[test]
fn impact_ignores_ambiguous_edges() {
    let conn = graph();
    let result = impact::analyze_impact_from_conn(&conn, "a", 3).unwrap();
    assert_eq!(result.direct_impact, 0);
    assert_eq!(result.transitive_impact, 0);
    assert_eq!(result.ambiguous_callers, vec!["b".to_string()]);
    let d = impact::analyze_impact_from_conn(&conn, "d", 3).unwrap();
    assert_eq!(d.direct_impact, 1);
}

#[test]
fn dead_code_treats_ambiguous_and_unresolved_uses_as_possibly_used() {
    let conn = graph();
    let result = dead_code::find_dead_code_from_conn(&conn, None, false, 100).unwrap();
    let mut dead: Vec<&str> = result.dead_symbols.iter().map(|s| s.id.as_str()).collect();
    dead.sort();
    // a, b: ambiguous callers; d: confident caller; e: name seen at an
    // unresolved call site. c and f have no callers at all.
    assert_eq!(dead, vec!["c", "f"]);
    assert_eq!(result.dead_count, 2);
    assert!(!result.truncated);

    let one = dead_code::find_dead_code_from_conn(&conn, None, false, 1).unwrap();
    assert_eq!(one.dead_symbols.len(), 1);
    assert_eq!(one.dead_count, 2);
    assert!(one.truncated);
}
