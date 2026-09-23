/// Architecture pattern detection and validation.
pub mod architecture;
/// Architecture boundary enforcement.
pub mod boundaries;
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

/// Whether `path` belongs to a test target or is a SwiftPM manifest:
/// a `Package.swift` file, a `*Tests.swift` file, or a directory whose name
/// ends in `Tests` (also `UITests`) or `TestKit`.
///
/// Dead-code and complexity skip these by default: manifests declare a
/// single unused `package` constant, and test helpers dominate fan-in.
pub fn is_test_or_manifest(path: &str) -> bool {
    let mut components = path.split('/').filter(|c| !c.is_empty()).peekable();
    while let Some(component) = components.next() {
        if components.peek().is_none() {
            return component == "Package.swift" || component.ends_with("Tests.swift");
        }
        if component.ends_with("Tests") || component.ends_with("TestKit") {
            return true;
        }
    }
    false
}
