use std::path::Path;

use swiftgraph_core::storage::{self, queries};
use swiftgraph_core::tree_sitter::parser::TreeSitterParser;

#[test]
fn index_basic_fixture() {
    let fixture_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/basic/Sources");

    if !fixture_dir.exists() {
        panic!("Fixture dir not found: {}", fixture_dir.display());
    }

    let conn = storage::open_memory_db().expect("open memory DB");

    // Parse each Swift file
    let mut total_nodes = 0;
    let mut total_edges = 0;

    for entry in std::fs::read_dir(&fixture_dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "swift") {
            let path_str = path.to_string_lossy().to_string();
            queries::upsert_file(&conn, &path_str, "test-hash", 0).expect("insert file");

            let mut parser = TreeSitterParser::new().expect("create parser");
            let result = parser.parse_file(&path).expect("parse file");

            for node in &result.nodes {
                queries::upsert_node(&conn, node).expect("insert node");
                total_nodes += 1;
            }
            for edge in &result.edges {
                queries::insert_edge(&conn, edge).expect("insert edge");
                total_edges += 1;
            }
        }
    }

    assert!(total_nodes > 0, "should have indexed some nodes");
    assert!(total_edges > 0, "should have indexed some edges");

    // Test search
    let results = queries::search_nodes(&conn, "User*", 10).unwrap();
    assert!(!results.is_empty(), "should find User-related symbols");

    // Test LIKE search
    let results = queries::find_nodes_by_name(&conn, "Service", None, 10).unwrap();
    assert!(!results.is_empty(), "should find Service symbols via LIKE");

    // Test kind filter
    let results = queries::find_nodes_by_name(&conn, "", Some("protocol"), 10).unwrap();
    assert!(!results.is_empty(), "should find protocols");

    // Test stats
    let stats = queries::get_stats(&conn).unwrap();
    assert!(stats.node_count > 0);
}

#[test]
fn index_concurrency_fixture() {
    let fixture_dir =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/concurrency/Sources");

    if !fixture_dir.exists() {
        panic!("Fixture dir not found: {}", fixture_dir.display());
    }

    let conn = storage::open_memory_db().expect("open memory DB");
    let mut total_nodes = 0;

    for entry in std::fs::read_dir(&fixture_dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "swift") {
            let path_str = path.to_string_lossy().to_string();
            queries::upsert_file(&conn, &path_str, "test-hash", 0).expect("insert file");

            let mut parser = TreeSitterParser::new().expect("create parser");
            let result = parser.parse_file(&path).expect("parse file");

            for node in &result.nodes {
                queries::upsert_node(&conn, node).expect("insert node");
                total_nodes += 1;
            }
        }
    }

    assert!(total_nodes > 0, "should have indexed concurrency fixture");

    // Should find actor
    let results = queries::search_nodes(&conn, "DataStore*", 10).unwrap();
    assert!(!results.is_empty(), "should find DataStore actor");
}
