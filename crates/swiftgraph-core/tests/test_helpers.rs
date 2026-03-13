//! Test helpers: random graph generator for property-based testing.

use rusqlite::Connection;
use swiftgraph_core::graph::*;
use swiftgraph_core::storage::{open_memory_db, queries};

/// Create an in-memory DB with N nodes and M random edges.
///
/// Nodes are named `Node_0`..`Node_{n-1}` with files `file_0.swift`..`file_{n-1}.swift`.
/// Edges are selected from the provided `(source_idx, target_idx)` pairs.
pub fn create_random_graph(node_count: usize, edges: &[(usize, usize)]) -> Connection {
    let conn = open_memory_db().expect("failed to open in-memory db");

    // Create file records
    for i in 0..node_count {
        let path = format!("Sources/file_{i}.swift");
        queries::upsert_file(&conn, &path, &format!("hash_{i}"), 1).expect("failed to upsert file");
    }

    // Create nodes
    for i in 0..node_count {
        let node = GraphNode {
            id: format!("usr:node_{i}"),
            name: format!("Node_{i}"),
            qualified_name: format!("Mod.Node_{i}"),
            kind: SymbolKind::Function,
            sub_kind: None,
            location: Location {
                file: format!("Sources/file_{i}.swift"),
                line: 1,
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
        };
        queries::upsert_node(&conn, &node).expect("failed to upsert node");
    }

    // Create edges (calls)
    for (line, &(src, tgt)) in edges.iter().enumerate() {
        if src < node_count && tgt < node_count {
            let edge = GraphEdge {
                source: format!("usr:node_{src}"),
                target: format!("usr:node_{tgt}"),
                kind: EdgeKind::Calls,
                location: Some(Location {
                    file: format!("Sources/file_{src}.swift"),
                    line: (line + 1) as u32,
                    column: 1,
                    end_line: None,
                    end_column: None,
                }),
                is_implicit: false,
            };
            // Ignore duplicates (same source/target/kind/line combo)
            let _ = queries::insert_edge(&conn, &edge);
        }
    }

    conn
}
