//! Task-based context builder.
//!
//! Given a task description, extracts keywords, searches the graph via FTS5,
//! expands via callers/callees/conformances up to 2 levels, then ranks
//! by a simple relevance score (PageRank-lite: incoming edge count).

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::Serialize;
use thiserror::Error;

use crate::graph::GraphNode;
use crate::storage::{self, queries};

#[derive(Debug, Error)]
pub enum ContextError {
    #[error("storage error: {0}")]
    Storage(#[from] crate::storage::StorageError),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

/// Result of context building.
#[derive(Debug, Serialize)]
pub struct ContextResult {
    /// The extracted keywords from the task.
    pub keywords: Vec<String>,
    /// Relevant nodes ranked by importance.
    pub nodes: Vec<RankedNode>,
    /// Files that should be examined.
    pub files: Vec<String>,
    /// Suggested architecture pattern (if detectable).
    pub architecture: Option<String>,
}

/// A node with its relevance score.
#[derive(Debug, Serialize)]
pub struct RankedNode {
    #[serde(flatten)]
    pub node: GraphNode,
    /// Relevance score (higher = more relevant).
    pub score: f64,
}

/// Build context for a task.
pub fn build_context(
    db_path: &Path,
    task: &str,
    max_nodes: u32,
    include_tests: bool,
) -> Result<ContextResult, ContextError> {
    let conn = storage::open_db(db_path)?;

    // 1. Extract keywords from task description
    let keywords = extract_keywords(task);

    // 2. FTS5 search for each keyword
    let mut seed_nodes: Vec<GraphNode> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();

    for keyword in &keywords {
        // Try FTS5 first
        let results = queries::search_nodes(&conn, keyword, 10)
            .or_else(|_| queries::find_nodes_by_name_pattern(&conn, keyword, 10))?;

        for node in results {
            if seen_ids.insert(node.id.clone()) {
                seed_nodes.push(node);
            }
        }
    }

    // 3. Expand graph: from seed nodes, follow edges 2 levels deep
    let mut all_node_ids: HashSet<String> = seed_nodes.iter().map(|n| n.id.clone()).collect();
    let mut frontier: Vec<String> = seed_nodes.iter().map(|n| n.id.clone()).collect();

    for _depth in 0..2 {
        let mut next_frontier = Vec::new();
        for node_id in &frontier {
            // Get callers + callees + conformances
            let incoming = queries::get_all_incoming(&conn, node_id, 10).unwrap_or_default();
            let outgoing = queries::get_all_outgoing(&conn, node_id, 10).unwrap_or_default();

            for edge in incoming.iter().chain(outgoing.iter()) {
                let other = if edge.source == *node_id {
                    &edge.target
                } else {
                    &edge.source
                };
                if all_node_ids.insert(other.clone()) {
                    next_frontier.push(other.clone());
                }
            }
        }
        frontier = next_frontier;
    }

    // 4. Resolve all nodes and compute scores
    let mut scored: HashMap<String, (GraphNode, f64)> = HashMap::new();

    for node_id in &all_node_ids {
        if let Ok(Some(node)) = queries::get_node(&conn, node_id) {
            // Skip test files unless requested
            if !include_tests && node.location.file.contains("/Tests/") {
                continue;
            }

            // Score: seed nodes get base score + bonus for incoming edges
            let is_seed = seed_nodes.iter().any(|s| s.id == *node_id);
            let incoming_count = queries::count_incoming(&conn, node_id).unwrap_or(0) as f64;
            let outgoing_count = queries::count_outgoing(&conn, node_id).unwrap_or(0) as f64;

            let mut score = incoming_count * 2.0 + outgoing_count;
            if is_seed {
                score += 50.0; // seed bonus
            }

            // Boost entry points (views, routers, builders)
            let name_lower = node.name.to_lowercase();
            if name_lower.contains("view")
                || name_lower.contains("router")
                || name_lower.contains("builder")
                || name_lower.contains("controller")
            {
                score += 10.0;
            }

            scored.insert(node_id.clone(), (node, score));
        }
    }

    // 5. Sort by score, limit
    let mut ranked: Vec<RankedNode> = scored
        .into_values()
        .map(|(node, score)| RankedNode { node, score })
        .collect();
    ranked.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    ranked.truncate(max_nodes as usize);

    // 6. Collect files
    let files: Vec<String> = ranked
        .iter()
        .map(|r| r.node.location.file.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    // 7. Detect architecture pattern
    let architecture = detect_architecture(&ranked);

    Ok(ContextResult {
        keywords,
        nodes: ranked,
        files,
        architecture,
    })
}

/// Extract meaningful keywords from a task description.
fn extract_keywords(task: &str) -> Vec<String> {
    let stop_words: HashSet<&str> = [
        "a",
        "an",
        "the",
        "to",
        "in",
        "for",
        "of",
        "and",
        "or",
        "is",
        "it",
        "on",
        "at",
        "by",
        "add",
        "fix",
        "update",
        "implement",
        "create",
        "make",
        "change",
        "modify",
        "remove",
        "delete",
        "new",
        "from",
        "with",
        "this",
        "that",
        "screen",
        "page",
        "feature",
        "functionality",
        "function",
        "method",
        "should",
        "need",
        "want",
        "bug",
    ]
    .into_iter()
    .collect();

    task.split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| w.len() >= 3)
        .map(|w| w.to_string())
        .filter(|w| !stop_words.contains(w.to_lowercase().as_str()))
        .collect()
}

/// Simple architecture pattern detection from ranked nodes.
fn detect_architecture(nodes: &[RankedNode]) -> Option<String> {
    let names: Vec<&str> = nodes.iter().map(|n| n.node.name.as_str()).collect();
    let kinds: Vec<&str> = nodes.iter().map(|n| n.node.kind.as_str()).collect();

    let has_viewmodel = names.iter().any(|n| n.contains("ViewModel"));
    let has_view = names
        .iter()
        .any(|n| n.contains("View") && !n.contains("ViewModel"));
    let has_router = names.iter().any(|n| n.contains("Router"));
    let has_coordinator = names.iter().any(|n| n.contains("Coordinator"));
    let has_interactor = names.iter().any(|n| n.contains("Interactor"));
    let has_presenter = names.iter().any(|n| n.contains("Presenter"));
    let has_store = names
        .iter()
        .any(|n| n.contains("Store") || n.contains("Reducer"));
    let has_protocol = kinds.contains(&"protocol");

    if has_interactor && has_presenter && has_router {
        Some("VIPER".into())
    } else if has_store {
        Some("TCA/Redux".into())
    } else if has_viewmodel && has_coordinator {
        Some("MVVM+Coordinator".into())
    } else if has_viewmodel && has_router {
        Some("MVVM+Router".into())
    } else if has_viewmodel && has_view {
        Some("MVVM".into())
    } else if has_view && has_protocol {
        Some("Protocol-oriented".into())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyword_extraction() {
        let keywords = extract_keywords("add search functionality to schedule screen");
        assert!(keywords.contains(&"search".to_string()));
        assert!(keywords.contains(&"schedule".to_string()));
        assert!(!keywords.contains(&"add".to_string()));
        assert!(!keywords.contains(&"to".to_string()));
    }
}
