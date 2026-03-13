//! Unit tests for MCP tool functions (navigation + status layer).
//!
//! Uses a temp-file SQLite database populated with test data to exercise
//! the search, callers, callees, hierarchy, files, and audit-option parsing.

use swiftgraph_core::graph::edge::{EdgeKind, GraphEdge};
use swiftgraph_core::graph::node::{AccessLevel, GraphNode, Location, NodeMetrics, SymbolKind};
use swiftgraph_core::storage::{self, queries};
use swiftgraph_mcp::tools::navigation;

/// Create a temp DB file and populate it with test data.
fn setup_test_db() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("create temp dir");
    let db_path = dir.path().join("test.sqlite");

    let conn = storage::open_db(&db_path).expect("open test DB");

    // Insert test files
    queries::upsert_file(&conn, "Sources/Models/User.swift", "hash1", 3).unwrap();
    queries::upsert_file(&conn, "Sources/Services/UserService.swift", "hash2", 2).unwrap();
    queries::upsert_file(&conn, "Sources/ViewModels/UserVM.swift", "hash3", 1).unwrap();

    // Insert test nodes
    let nodes = vec![
        make_node(
            "usr:User",
            "User",
            "App.User",
            SymbolKind::Struct,
            "Sources/Models/User.swift",
        ),
        make_node(
            "usr:UserService",
            "UserService",
            "App.UserService",
            SymbolKind::Protocol,
            "Sources/Services/UserService.swift",
        ),
        make_node(
            "usr:NetworkUserService",
            "NetworkUserService",
            "App.NetworkUserService",
            SymbolKind::Class,
            "Sources/Services/UserService.swift",
        ),
        make_node(
            "usr:UserVM",
            "UserListViewModel",
            "App.UserListViewModel",
            SymbolKind::Class,
            "Sources/ViewModels/UserVM.swift",
        ),
        make_node(
            "usr:fetchUser",
            "fetchUser",
            "App.UserService.fetchUser",
            SymbolKind::Method,
            "Sources/Services/UserService.swift",
        ),
        make_node(
            "usr:loadUsers",
            "loadUsers",
            "App.UserListViewModel.loadUsers",
            SymbolKind::Method,
            "Sources/ViewModels/UserVM.swift",
        ),
    ];

    for node in &nodes {
        queries::upsert_node(&conn, node).unwrap();
    }

    // Insert edges
    let edges = vec![
        // loadUsers calls fetchUser
        make_edge("usr:loadUsers", "usr:fetchUser", EdgeKind::Calls),
        // NetworkUserService conforms to UserService
        make_edge(
            "usr:NetworkUserService",
            "usr:UserService",
            EdgeKind::ConformsTo,
        ),
        // UserVM references User
        make_edge("usr:UserVM", "usr:User", EdgeKind::References),
    ];

    for edge in &edges {
        queries::insert_edge(&conn, edge).unwrap();
    }

    (dir, db_path)
}

fn make_node(id: &str, name: &str, qname: &str, kind: SymbolKind, file: &str) -> GraphNode {
    GraphNode {
        id: id.into(),
        name: name.into(),
        qualified_name: qname.into(),
        kind,
        sub_kind: None,
        location: Location {
            file: file.into(),
            line: 1,
            column: 1,
            end_line: Some(10),
            end_column: Some(1),
        },
        signature: None,
        attributes: vec![],
        access_level: AccessLevel::Internal,
        container_usr: None,
        doc_comment: None,
        metrics: Some(NodeMetrics {
            lines: Some(10),
            complexity: Some(1),
            parameter_count: None,
        }),
    }
}

fn make_edge(source: &str, target: &str, kind: EdgeKind) -> GraphEdge {
    GraphEdge {
        source: source.into(),
        target: target.into(),
        kind,
        location: None,
        is_implicit: false,
    }
}

// --- Search tests ---

#[test]
fn search_by_name_returns_results() {
    let (_dir, db_path) = setup_test_db();
    let params = navigation::SearchParams {
        query: "User".into(),
        kind: None,
        limit: Some(10),
    };
    let resp = navigation::search(&db_path, params).unwrap();
    assert!(resp.total > 0, "should find User-related symbols");
    assert!(resp.results.iter().any(|n| n.name == "User"));
}

#[test]
fn search_with_kind_filter() {
    let (_dir, db_path) = setup_test_db();
    let params = navigation::SearchParams {
        query: "User".into(),
        kind: Some("protocol".into()),
        limit: Some(10),
    };
    let resp = navigation::search(&db_path, params).unwrap();
    assert!(resp.total > 0, "should find UserService protocol");
    for node in &resp.results {
        assert_eq!(node.kind.as_str(), "protocol");
    }
}

#[test]
fn search_wildcard_lists_all() {
    let (_dir, db_path) = setup_test_db();
    let params = navigation::SearchParams {
        query: "*".into(),
        kind: None,
        limit: Some(100),
    };
    let resp = navigation::search(&db_path, params).unwrap();
    assert!(resp.total >= 6, "should list all nodes");
}

#[test]
fn search_empty_query_lists_all() {
    let (_dir, db_path) = setup_test_db();
    let params = navigation::SearchParams {
        query: "".into(),
        kind: None,
        limit: Some(100),
    };
    let resp = navigation::search(&db_path, params).unwrap();
    assert!(resp.total >= 6);
}

// --- Node lookup ---

#[test]
fn get_node_by_id() {
    let (_dir, db_path) = setup_test_db();
    let params = navigation::NodeParams {
        symbol: "usr:User".into(),
        ..Default::default()
    };
    let node = navigation::get_node(&db_path, params).unwrap();
    assert!(node.is_some());
    assert_eq!(node.unwrap().name, "User");
}

#[test]
fn get_node_not_found() {
    let (_dir, db_path) = setup_test_db();
    let params = navigation::NodeParams {
        symbol: "usr:NonExistent".into(),
        ..Default::default()
    };
    let node = navigation::get_node(&db_path, params).unwrap();
    assert!(node.is_none());
}

// --- Callers / Callees ---

#[test]
fn get_callers_returns_edges() {
    let (_dir, db_path) = setup_test_db();
    let params = navigation::CallersParams {
        symbol: "usr:fetchUser".into(),
        limit: Some(10),
    };
    let resp = navigation::get_callers(&db_path, params).unwrap();
    assert_eq!(resp.count, 1);
    assert_eq!(resp.edges[0].source, "usr:loadUsers");
}

#[test]
fn get_callees_returns_edges() {
    let (_dir, db_path) = setup_test_db();
    let params = navigation::CallersParams {
        symbol: "usr:loadUsers".into(),
        limit: Some(10),
    };
    let resp = navigation::get_callees(&db_path, params).unwrap();
    assert_eq!(resp.count, 1);
    assert_eq!(resp.edges[0].target, "usr:fetchUser");
}

#[test]
fn get_callers_empty_for_root() {
    let (_dir, db_path) = setup_test_db();
    let params = navigation::CallersParams {
        symbol: "usr:loadUsers".into(),
        limit: Some(10),
    };
    let resp = navigation::get_callers(&db_path, params).unwrap();
    assert_eq!(resp.count, 0);
}

// --- References ---

#[test]
fn get_references_includes_all_edge_kinds() {
    let (_dir, db_path) = setup_test_db();
    let params = navigation::CallersParams {
        symbol: "usr:User".into(),
        limit: Some(10),
    };
    let resp = navigation::get_references(&db_path, params).unwrap();
    assert!(resp.count >= 1, "should find references to User");
}

// --- Hierarchy ---

#[test]
fn get_hierarchy_subtypes() {
    let (_dir, db_path) = setup_test_db();
    let params = navigation::HierarchyParams {
        symbol: "usr:UserService".into(),
        direction: Some("subtypes".into()),
        depth: Some(3),
    };
    let resp = navigation::get_hierarchy(&db_path, params).unwrap();
    assert_eq!(resp.direction, "subtypes");
    assert!(
        resp.related.iter().any(|n| n.name == "NetworkUserService"),
        "NetworkUserService should be a subtype of UserService"
    );
}

#[test]
fn get_hierarchy_supertypes() {
    let (_dir, db_path) = setup_test_db();
    let params = navigation::HierarchyParams {
        symbol: "usr:NetworkUserService".into(),
        direction: Some("supertypes".into()),
        depth: Some(3),
    };
    let resp = navigation::get_hierarchy(&db_path, params).unwrap();
    assert_eq!(resp.direction, "supertypes");
    assert!(
        resp.related.iter().any(|n| n.name == "UserService"),
        "UserService should be a supertype of NetworkUserService"
    );
}

// --- Files ---

#[test]
fn get_files_all() {
    let (_dir, db_path) = setup_test_db();
    let params = navigation::FilesParams {
        path: None,
        limit: Some(100),
    };
    let resp = navigation::get_files(&db_path, params).unwrap();
    assert_eq!(resp.count, 3);
}

#[test]
fn get_files_with_path_filter() {
    let (_dir, db_path) = setup_test_db();
    let params = navigation::FilesParams {
        path: Some("Sources/Services".into()),
        limit: Some(100),
    };
    let resp = navigation::get_files(&db_path, params).unwrap();
    assert_eq!(resp.count, 1);
}

// --- Audit option parsing ---

#[test]
fn parse_audit_options_all_defaults() {
    let opts = navigation::parse_audit_options(None, None, None, None);
    assert!(opts.categories.is_empty()); // empty = all
    assert_eq!(opts.max_issues, 100);
}

#[test]
fn parse_audit_options_with_categories() {
    let opts =
        navigation::parse_audit_options(Some("concurrency,memory"), Some("high"), None, Some(50));
    assert_eq!(opts.categories.len(), 2);
    assert_eq!(opts.max_issues, 50);
}

#[test]
fn parse_audit_options_unknown_category_ignored() {
    let opts = navigation::parse_audit_options(Some("concurrency,bogus,memory"), None, None, None);
    assert_eq!(opts.categories.len(), 2);
}

// --- Boundaries ---

#[test]
fn get_boundaries_invalid_json_returns_error() {
    let (_dir, db_path) = setup_test_db();
    let result = navigation::get_boundaries(&db_path, "not json");
    assert!(result.is_err());
}
