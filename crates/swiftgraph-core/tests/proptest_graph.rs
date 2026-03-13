//! Property-based tests for graph algorithms.

mod test_helpers;

use proptest::prelude::*;
use swiftgraph_core::analysis::{complexity, cycles, dead_code, impact};

/// Strategy: generate a graph with N nodes and M random edges.
fn arb_graph(
    max_nodes: usize,
    max_edges: usize,
) -> impl Strategy<Value = (usize, Vec<(usize, usize)>)> {
    (1..=max_nodes).prop_flat_map(move |n| {
        let edge_count = 0..=max_edges;
        let edges = prop::collection::vec((0..n, 0..n), edge_count);
        (Just(n), edges)
    })
}

proptest! {
    // --- Cycles ---

    #[test]
    fn cycles_no_crash_on_arbitrary_graph((n, edges) in arb_graph(30, 50)) {
        let conn = test_helpers::create_random_graph(n, &edges);
        let result = cycles::detect_cycles_from_conn(&conn, None, 100);
        prop_assert!(result.is_ok());
    }

    #[test]
    fn known_cycle_is_detected(_dummy in 0..1u32) {
        // A → B → C → A
        let conn = test_helpers::create_random_graph(3, &[(0, 1), (1, 2), (2, 0)]);
        let result = cycles::detect_cycles_from_conn(&conn, None, 100).unwrap();
        // At least one cycle should be detected
        prop_assert!(!result.cycles.is_empty(), "Expected cycle A→B→C→A to be detected");
    }

    #[test]
    fn acyclic_graph_has_no_cycles(_dummy in 0..1u32) {
        // Linear DAG: 0 → 1 → 2 → 3
        let conn = test_helpers::create_random_graph(4, &[(0, 1), (1, 2), (2, 3)]);
        let result = cycles::detect_cycles_from_conn(&conn, None, 100).unwrap();
        prop_assert!(result.cycles.is_empty(), "DAG should have no cycles, found {:?}", result.cycles);
    }

    // --- Impact ---

    #[test]
    fn impact_no_crash_on_arbitrary_graph((n, edges) in arb_graph(20, 40)) {
        let conn = test_helpers::create_random_graph(n, &edges);
        // Pick node 0 as target
        let result = impact::analyze_impact_from_conn(&conn, "usr:node_0", 3);
        prop_assert!(result.is_ok());
    }

    #[test]
    fn impact_includes_direct_callers(_dummy in 0..1u32) {
        // A → B (A calls B), so impact of B should include A
        let conn = test_helpers::create_random_graph(2, &[(0, 1)]);
        let result = impact::analyze_impact_from_conn(&conn, "usr:node_1", 3).unwrap();
        prop_assert!(result.direct_impact >= 1, "Expected A in impact of B");
        prop_assert!(result.breakdown.callers.contains(&"usr:node_0".to_string()));
    }

    #[test]
    fn impact_is_monotonic_with_depth(_dummy in 0..1u32) {
        // Chain: 0 → 1 → 2 → 3
        let conn = test_helpers::create_random_graph(4, &[(0, 1), (1, 2), (2, 3)]);
        let depth2 = impact::analyze_impact_from_conn(&conn, "usr:node_3", 2).unwrap();
        let depth3 = impact::analyze_impact_from_conn(&conn, "usr:node_3", 3).unwrap();
        prop_assert!(
            depth3.transitive_impact >= depth2.transitive_impact,
            "depth=3 ({}) should be >= depth=2 ({})",
            depth3.transitive_impact,
            depth2.transitive_impact
        );
    }

    #[test]
    fn impact_on_leaf_node_is_zero(_dummy in 0..1u32) {
        // 0 → 1, node 0 has no incoming edges
        let conn = test_helpers::create_random_graph(2, &[(0, 1)]);
        let result = impact::analyze_impact_from_conn(&conn, "usr:node_0", 3).unwrap();
        prop_assert_eq!(result.direct_impact, 0);
        prop_assert_eq!(result.transitive_impact, 0);
    }

    // --- Dead code ---

    #[test]
    fn dead_code_no_crash_on_arbitrary_graph((n, edges) in arb_graph(20, 40)) {
        let conn = test_helpers::create_random_graph(n, &edges);
        let result = dead_code::find_dead_code_from_conn(&conn, None, false, 100);
        prop_assert!(result.is_ok());
    }

    #[test]
    fn dead_code_detects_unreferenced_node(_dummy in 0..1u32) {
        // Node 0 → Node 1, Node 2 is orphan
        let conn = test_helpers::create_random_graph(3, &[(0, 1)]);
        let result = dead_code::find_dead_code_from_conn(&conn, None, false, 100).unwrap();
        let dead_ids: Vec<&str> = result.dead_symbols.iter().map(|s| s.id.as_str()).collect();
        // Node 2 has no incoming edges and is not a container with outgoing edges
        prop_assert!(dead_ids.contains(&"usr:node_2"), "Expected node_2 to be dead, found: {:?}", dead_ids);
    }

    // --- Complexity ---

    #[test]
    fn complexity_no_crash_on_arbitrary_graph((n, edges) in arb_graph(20, 40)) {
        let conn = test_helpers::create_random_graph(n, &edges);
        let result = complexity::analyze_complexity_from_conn(&conn, None, 100, "score");
        prop_assert!(result.is_ok());
    }

    #[test]
    fn complexity_fan_out_matches_edges(_dummy in 0..1u32) {
        // Node 0 calls nodes 1, 2, 3 — fan_out should be 3
        let conn = test_helpers::create_random_graph(4, &[(0, 1), (0, 2), (0, 3)]);
        let result = complexity::analyze_complexity_from_conn(&conn, None, 100, "fan_out").unwrap();
        let node0 = result.symbols.iter().find(|s| s.id == "usr:node_0").unwrap();
        prop_assert_eq!(node0.fan_out, 3, "Expected fan_out=3 for node_0");
    }
}
