/// Architecture pattern detection and validation.
pub mod architecture;
/// Complexity analysis — fan-in/fan-out and structural complexity.
pub mod complexity;
/// Task-based context builder — collects relevant symbols for a given task description.
pub mod context;
/// Module coupling analysis — Ca, Ce, instability, abstractness.
pub mod coupling;
/// Dependency cycle detection at file level.
pub mod cycles;
/// Dead code detection — symbols with no incoming edges.
pub mod dead_code;
/// Git diff-based impact analysis.
pub mod diff_impact;
/// Blast radius analysis — impact of changing a symbol.
pub mod impact;
/// Module dependency graph from import declarations.
pub mod imports;
