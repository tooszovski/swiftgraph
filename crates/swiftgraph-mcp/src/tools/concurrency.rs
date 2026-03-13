//! Concurrency analysis tool — deep inspection of a symbol's concurrency annotations.
//!
//! Analyzes: actor isolation, Sendable conformance, cross-actor calls,
//! mutable state access, and async boundaries.

use std::path::Path;

use anyhow::Result;
use serde::Serialize;
use swiftgraph_core::graph::GraphNode;
use swiftgraph_core::storage::{self, queries};

/// Parameters for concurrency analysis.
#[derive(Debug)]
pub struct ConcurrencyParams {
    /// Symbol ID (USR) or name.
    pub symbol: String,
}

/// Result of concurrency analysis for a symbol.
#[derive(Debug, Serialize)]
pub struct ConcurrencyResult {
    /// The analyzed symbol.
    pub symbol: String,
    /// Detected isolation context.
    pub isolation: IsolationInfo,
    /// Whether the type conforms to Sendable.
    pub sendable: bool,
    /// Cross-actor calls made by this symbol.
    pub cross_actor_calls: Vec<String>,
    /// Mutable state accessed by this symbol.
    pub mutable_state: Vec<MutableStateAccess>,
    /// Concurrency warnings/suggestions.
    pub warnings: Vec<String>,
}

/// Isolation context for a symbol.
#[derive(Debug, Serialize)]
pub struct IsolationInfo {
    /// "MainActor", "custom_actor", "nonisolated", "unknown"
    pub context: String,
    /// The actor type if isolated.
    pub actor: Option<String>,
    /// Whether explicitly marked nonisolated.
    pub is_nonisolated: bool,
}

/// A mutable state access detected in the symbol.
#[derive(Debug, Serialize)]
pub struct MutableStateAccess {
    /// Property name.
    pub property: String,
    /// Whether it's @Published.
    pub is_published: bool,
    /// Whether the access is protected (inside actor or synchronized).
    pub is_protected: bool,
}

/// Analyze concurrency annotations and patterns for a symbol.
pub fn analyze_concurrency(db_path: &Path, params: ConcurrencyParams) -> Result<ConcurrencyResult> {
    let conn = storage::open_db(db_path)?;

    // Resolve symbol
    let node = queries::get_node(&conn, &params.symbol)?
        .or_else(|| {
            queries::search_nodes(&conn, &params.symbol, 1)
                .ok()
                .and_then(|v| v.into_iter().next())
        })
        .or_else(|| {
            queries::find_nodes_by_name(&conn, &params.symbol, None, 1)
                .ok()
                .and_then(|v| v.into_iter().next())
        })
        .ok_or_else(|| anyhow::anyhow!("symbol not found: {}", params.symbol))?;

    let isolation = detect_isolation(&node);

    // Check Sendable conformance
    let sendable = node.attributes.iter().any(|a| a.contains("Sendable"))
        || queries::get_conformances(&conn, &node.id, "conforms", 50)
            .unwrap_or_default()
            .iter()
            .any(|e| e.target.contains("Sendable"));

    // Detect cross-actor calls
    let mut cross_actor_calls = Vec::new();
    if let Ok(callees) = queries::get_callees(&conn, &node.id, 100) {
        for edge in callees {
            if let Ok(Some(target)) = queries::get_node(&conn, &edge.target) {
                let target_isolation = detect_isolation(&target);
                if target_isolation.actor.is_some() && target_isolation.actor != isolation.actor {
                    cross_actor_calls.push(format!(
                        "{} (isolated to {})",
                        target.name, target_isolation.context
                    ));
                }
            }
        }
    }

    // Detect mutable state access
    let mut mutable_state = Vec::new();
    if let Ok(edges) = queries::get_all_outgoing(&conn, &node.id, 100) {
        for edge in edges {
            if edge.kind.as_str() == "mutates" || edge.kind.as_str() == "references" {
                if let Ok(Some(target)) = queries::get_node(&conn, &edge.target) {
                    if target.kind.as_str() == "property" {
                        let is_published =
                            target.attributes.iter().any(|a| a.contains("Published"));
                        let target_isolation = detect_isolation(&target);
                        mutable_state.push(MutableStateAccess {
                            property: target.name.clone(),
                            is_published,
                            is_protected: target_isolation.actor.is_some(),
                        });
                    }
                }
            }
        }
    }

    let warnings = generate_warnings(&isolation, sendable, &cross_actor_calls, &mutable_state);

    Ok(ConcurrencyResult {
        symbol: node.name.clone(),
        isolation,
        sendable,
        cross_actor_calls,
        mutable_state,
        warnings,
    })
}

/// Detect isolation context from attributes.
fn detect_isolation(node: &GraphNode) -> IsolationInfo {
    let attrs = &node.attributes;

    // Check for @MainActor
    if attrs.iter().any(|a| a.contains("MainActor")) {
        return IsolationInfo {
            context: "MainActor".into(),
            actor: Some("MainActor".into()),
            is_nonisolated: false,
        };
    }

    // Check for custom actor isolation (@SomeActor)
    for attr in attrs {
        if attr.contains("Actor") && !attr.contains("nonisolated") {
            return IsolationInfo {
                context: attr.trim_start_matches('@').to_string(),
                actor: Some(attr.trim_start_matches('@').to_string()),
                is_nonisolated: false,
            };
        }
    }

    // Check if nonisolated
    if attrs.iter().any(|a| a.contains("nonisolated")) {
        return IsolationInfo {
            context: "nonisolated".into(),
            actor: None,
            is_nonisolated: true,
        };
    }

    // Check if the symbol is an actor type
    if let Some(ref sig) = node.signature {
        if sig.contains("actor ") {
            return IsolationInfo {
                context: format!("self (actor {})", node.name),
                actor: Some(node.name.clone()),
                is_nonisolated: false,
            };
        }
    }

    IsolationInfo {
        context: "unknown".into(),
        actor: None,
        is_nonisolated: false,
    }
}

/// Generate concurrency warnings based on analysis.
fn generate_warnings(
    isolation: &IsolationInfo,
    sendable: bool,
    cross_actor_calls: &[String],
    mutable_state: &[MutableStateAccess],
) -> Vec<String> {
    let mut warnings = Vec::new();

    if isolation.context == "unknown" {
        warnings.push(
            "Symbol has no explicit isolation annotation — may cause data races in Swift 6 strict concurrency"
                .into(),
        );
    }

    if !cross_actor_calls.is_empty() && !sendable {
        warnings.push(format!(
            "Makes {} cross-actor call(s) but type is not Sendable — parameters must be Sendable",
            cross_actor_calls.len()
        ));
    }

    let unprotected: Vec<_> = mutable_state.iter().filter(|s| !s.is_protected).collect();
    if !unprotected.is_empty() {
        let names: Vec<_> = unprotected.iter().map(|s| s.property.as_str()).collect();
        warnings.push(format!(
            "Accesses mutable state outside actor isolation: {}",
            names.join(", ")
        ));
    }

    let published_unprotected: Vec<_> = mutable_state
        .iter()
        .filter(|s| s.is_published && !s.is_protected)
        .collect();
    if !published_unprotected.is_empty() {
        warnings
            .push("@Published properties accessed outside MainActor — UI updates may crash".into());
    }

    warnings
}
