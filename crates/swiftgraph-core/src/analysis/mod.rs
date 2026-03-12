/// Complexity analysis — fan-in/fan-out and structural complexity.
pub mod complexity;
/// Task-based context builder — collects relevant symbols for a given task description.
pub mod context;
/// Dependency cycle detection at file level.
pub mod cycles;
/// Dead code detection — symbols with no incoming edges.
pub mod dead_code;
/// Git diff-based impact analysis.
pub mod diff_impact;
/// Blast radius analysis — impact of changing a symbol.
pub mod impact;
