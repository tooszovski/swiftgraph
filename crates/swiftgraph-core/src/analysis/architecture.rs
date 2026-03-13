//! Architecture pattern detection and validation.
//!
//! Auto-detects architectural patterns (MVVM, VIPER, TCA, MVC) and validates
//! whether the codebase conforms to the detected or specified pattern.

use std::collections::HashMap;
use std::path::Path;

use serde::Serialize;
use thiserror::Error;

use crate::storage::{self, queries};

#[derive(Debug, Error)]
pub enum ArchitectureError {
    #[error("storage error: {0}")]
    Storage(#[from] crate::storage::StorageError),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

/// Detected architecture pattern.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub enum ArchPattern {
    MVVM,
    MVVMCoordinator,
    VIPER,
    TCA,
    MVC,
    Unknown,
}

impl ArchPattern {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::MVVM => "MVVM",
            Self::MVVMCoordinator => "MVVM+Coordinator",
            Self::VIPER => "VIPER",
            Self::TCA => "TCA/ComposableArchitecture",
            Self::MVC => "MVC",
            Self::Unknown => "Unknown",
        }
    }
}

/// Architecture analysis result.
#[derive(Debug, Serialize)]
pub struct ArchitectureResult {
    pub detected_pattern: String,
    pub confidence: f64,
    pub evidence: Vec<ArchEvidence>,
    pub violations: Vec<ArchViolation>,
    pub layer_stats: Vec<LayerStats>,
}

/// Evidence supporting pattern detection.
#[derive(Debug, Serialize)]
pub struct ArchEvidence {
    pub pattern: String,
    pub signal: String,
    pub count: u32,
}

/// Architecture violation.
#[derive(Debug, Serialize)]
pub struct ArchViolation {
    pub rule: String,
    pub file: String,
    pub symbol: String,
    pub description: String,
}

/// Stats for an architectural layer.
#[derive(Debug, Serialize)]
pub struct LayerStats {
    pub layer: String,
    pub file_count: u32,
    pub symbol_count: u32,
    pub examples: Vec<String>,
}

/// Analyze architecture of the project.
pub fn analyze_architecture(
    db_path: &Path,
    expected_pattern: Option<&str>,
) -> Result<ArchitectureResult, ArchitectureError> {
    let conn = storage::open_db(db_path)?;
    let all_nodes = queries::get_all_nodes(&conn, 50000)?;

    // Count naming convention signals
    let mut signals: HashMap<&str, Vec<(&str, u32)>> = HashMap::new();

    let mut view_model_count = 0u32;
    let mut coordinator_count = 0u32;
    let mut router_count = 0u32;
    let mut presenter_count = 0u32;
    let mut interactor_count = 0u32;
    let mut wireframe_count = 0u32;
    let mut reducer_count = 0u32;
    let mut store_count = 0u32;
    let mut view_count = 0u32;
    let mut controller_count = 0u32;
    let mut view_model_files: Vec<String> = Vec::new();
    let mut coordinator_files: Vec<String> = Vec::new();
    let mut presenter_files: Vec<String> = Vec::new();
    let mut interactor_files: Vec<String> = Vec::new();
    let mut reducer_files: Vec<String> = Vec::new();
    let mut view_files: Vec<String> = Vec::new();
    let mut controller_files: Vec<String> = Vec::new();

    for node in &all_nodes {
        let name_lower = node.name.to_lowercase();
        let file = &node.location.file;

        if name_lower.contains("viewmodel") || name_lower.ends_with("vm") {
            view_model_count += 1;
            if view_model_files.len() < 5 {
                view_model_files.push(file.clone());
            }
        }
        if name_lower.contains("coordinator") {
            coordinator_count += 1;
            if coordinator_files.len() < 5 {
                coordinator_files.push(file.clone());
            }
        }
        if name_lower.contains("router") && !name_lower.contains("nsurlrouter") {
            router_count += 1;
        }
        if name_lower.contains("presenter") {
            presenter_count += 1;
            if presenter_files.len() < 5 {
                presenter_files.push(file.clone());
            }
        }
        if name_lower.contains("interactor") {
            interactor_count += 1;
            if interactor_files.len() < 5 {
                interactor_files.push(file.clone());
            }
        }
        if name_lower.contains("wireframe") || name_lower.contains("assembly") {
            wireframe_count += 1;
        }
        if name_lower.contains("reducer") || name_lower.contains("composable") {
            reducer_count += 1;
            if reducer_files.len() < 5 {
                reducer_files.push(file.clone());
            }
        }
        if name_lower.contains("store") && !name_lower.contains("restore") {
            store_count += 1;
        }
        if name_lower.ends_with("view") || name_lower.ends_with("screen") {
            view_count += 1;
            if view_files.len() < 5 {
                view_files.push(file.clone());
            }
        }
        if name_lower.contains("viewcontroller") || name_lower.contains("controller") {
            controller_count += 1;
            if controller_files.len() < 5 {
                controller_files.push(file.clone());
            }
        }
    }

    signals.insert(
        "MVVM",
        vec![
            ("ViewModel/VM suffix", view_model_count),
            ("View/Screen suffix", view_count),
        ],
    );
    signals.insert(
        "MVVM+Coordinator",
        vec![
            ("ViewModel/VM suffix", view_model_count),
            ("Coordinator", coordinator_count),
            ("Router", router_count),
        ],
    );
    signals.insert(
        "VIPER",
        vec![
            ("Presenter", presenter_count),
            ("Interactor", interactor_count),
            ("Wireframe/Assembly", wireframe_count),
            ("Router", router_count),
        ],
    );
    signals.insert(
        "TCA",
        vec![
            ("Reducer", reducer_count),
            ("Store", store_count),
            ("View", view_count),
        ],
    );
    signals.insert(
        "MVC",
        vec![
            ("ViewController/Controller", controller_count),
            ("View", view_count),
        ],
    );

    // Score each pattern
    let mut scores: Vec<(&str, f64)> = signals
        .iter()
        .map(|(pattern, sigs)| {
            let score: f64 = sigs
                .iter()
                .map(|(_, count)| {
                    if *count > 0 {
                        (*count as f64).ln() + 1.0
                    } else {
                        0.0
                    }
                })
                .sum::<f64>()
                * (sigs.iter().filter(|(_, c)| *c > 0).count() as f64 / sigs.len() as f64);
            (*pattern, score)
        })
        .collect();
    scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let (detected_name, best_score) = scores.first().copied().unwrap_or(("Unknown", 0.0));
    let max_possible = 30.0; // rough upper bound
    let confidence = (best_score / max_possible).min(1.0);

    let detected = match detected_name {
        "MVVM" => ArchPattern::MVVM,
        "MVVM+Coordinator" => ArchPattern::MVVMCoordinator,
        "VIPER" => ArchPattern::VIPER,
        "TCA" => ArchPattern::TCA,
        "MVC" => ArchPattern::MVC,
        _ => ArchPattern::Unknown,
    };

    // Build evidence
    let mut evidence: Vec<ArchEvidence> = Vec::new();
    if let Some(sigs) = signals.get(detected_name) {
        for (signal, count) in sigs {
            if *count > 0 {
                evidence.push(ArchEvidence {
                    pattern: detected_name.to_string(),
                    signal: signal.to_string(),
                    count: *count,
                });
            }
        }
    }

    // Build layer stats
    let mut layer_stats = Vec::new();
    let add_layer = |stats: &mut Vec<LayerStats>, name: &str, count: u32, files: &[String]| {
        if count > 0 {
            stats.push(LayerStats {
                layer: name.to_string(),
                file_count: count,
                symbol_count: count,
                examples: files.to_vec(),
            });
        }
    };
    add_layer(&mut layer_stats, "Views", view_count, &view_files);
    add_layer(
        &mut layer_stats,
        "ViewModels",
        view_model_count,
        &view_model_files,
    );
    add_layer(
        &mut layer_stats,
        "Coordinators",
        coordinator_count,
        &coordinator_files,
    );
    add_layer(
        &mut layer_stats,
        "Presenters",
        presenter_count,
        &presenter_files,
    );
    add_layer(
        &mut layer_stats,
        "Interactors",
        interactor_count,
        &interactor_files,
    );
    add_layer(&mut layer_stats, "Reducers", reducer_count, &reducer_files);
    add_layer(
        &mut layer_stats,
        "Controllers",
        controller_count,
        &controller_files,
    );

    // Validate against expected pattern
    let validate_pattern = expected_pattern
        .map(|p| match p.to_lowercase().as_str() {
            "mvvm" => ArchPattern::MVVM,
            "mvvm+coordinator" | "mvvmc" => ArchPattern::MVVMCoordinator,
            "viper" => ArchPattern::VIPER,
            "tca" | "composable" => ArchPattern::TCA,
            "mvc" => ArchPattern::MVC,
            _ => ArchPattern::Unknown,
        })
        .unwrap_or(detected.clone());

    let mut violations = Vec::new();

    // Check for pattern violations based on expected architecture
    match validate_pattern {
        ArchPattern::MVVM | ArchPattern::MVVMCoordinator => {
            // Views should not directly access services/repositories
            // ViewModels expected for views
            if view_count > 0 && view_model_count == 0 {
                violations.push(ArchViolation {
                    rule: "MVVM: Missing ViewModels".to_string(),
                    file: String::new(),
                    symbol: String::new(),
                    description: format!(
                        "{view_count} views found but no ViewModels. Views should delegate logic to ViewModels."
                    ),
                });
            }
        }
        ArchPattern::VIPER => {
            if presenter_count > 0 && interactor_count == 0 {
                violations.push(ArchViolation {
                    rule: "VIPER: Missing Interactors".to_string(),
                    file: String::new(),
                    symbol: String::new(),
                    description: format!(
                        "{presenter_count} presenters but no interactors. Business logic should be in Interactors."
                    ),
                });
            }
        }
        _ => {}
    }

    Ok(ArchitectureResult {
        detected_pattern: detected.as_str().to_string(),
        confidence,
        evidence,
        violations,
        layer_stats,
    })
}
