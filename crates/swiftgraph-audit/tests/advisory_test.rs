//! Advisory rules (style suggestions with low measured precision) are hidden
//! by default, and iOS 17 migrations respect the deployment target.

use swiftgraph_audit::engine::Severity;
use swiftgraph_audit::rules::ios_deployment_target;
use swiftgraph_audit::runner::{run_audit, AuditOptions};

const MODEL: &str = "struct Wallet {\n    var a = 1\n    var b = 2\n    var c = 3\n    var d = 4\n    var e = 5\n    var f: [Int] = []\n}\n\nfinal class SettingsModel: ObservableObject {\n    @Published var title = \"\"\n}\n";

fn project(extra: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Model.swift"), MODEL).unwrap();
    for (path, body) in extra {
        let p = dir.path().join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }
    dir
}

fn rules(dir: &tempfile::TempDir, min: Severity) -> Vec<(String, Severity)> {
    let options = AuditOptions {
        min_severity: min,
        ..AuditOptions::default()
    };
    let result = run_audit(dir.path(), &options).unwrap();
    result
        .issues
        .into_iter()
        .map(|i| (i.rule, i.severity))
        .collect()
}

#[test]
fn advisory_rules_are_hidden_by_default() {
    let dir = project(&[]);
    let default = rules(&dir, Severity::Low);
    for rule in ["PERF-001", "PERF-006", "MOD-001"] {
        assert!(
            default.iter().all(|(r, _)| r != rule),
            "{rule}: {default:?}"
        );
    }
    let all = rules(&dir, Severity::Advisory);
    for rule in ["PERF-001", "PERF-006", "MOD-001"] {
        assert!(
            all.contains(&(rule.to_string(), Severity::Advisory)),
            "{rule}: {all:?}"
        );
    }
}

#[test]
fn config_can_promote_an_advisory_rule() {
    let dir = project(&[(
        ".swiftgraph/config.json",
        r#"{"audit": {"severity": {"PERF-006": "low"}}}"#,
    )]);
    assert!(rules(&dir, Severity::Low).contains(&("PERF-006".to_string(), Severity::Low)));
}

#[test]
fn observable_migration_needs_ios_17() {
    let pkg = |platform: &str| {
        format!("// swift-tools-version:5.9\nlet package = Package(name: \"A\", platforms: [{platform}])\n")
    };
    let old = project(&[("Package.swift", &pkg(".iOS(\"16.4\")"))]);
    assert!(rules(&old, Severity::Advisory)
        .iter()
        .all(|(r, _)| r != "MOD-001"));
    let new = project(&[("Package.swift", &pkg(".iOS(.v17)"))]);
    assert!(rules(&new, Severity::Advisory)
        .iter()
        .any(|(r, _)| r == "MOD-001"));
}

#[test]
fn deployment_target_is_read_from_build_settings() {
    let dir = project(&[
        (
            "App.xcodeproj/project.pbxproj",
            "IPHONEOS_DEPLOYMENT_TARGET = 17.0;\nIPHONEOS_DEPLOYMENT_TARGET = 16.4;\n",
        ),
        (
            "Kit/Package.swift",
            "platforms: [.iOS(.v17), .macOS(.v14)]\n",
        ),
        (
            "project.yml",
            "options:\n  deploymentTarget:\n    iOS: \"18.0\"\n",
        ),
    ]);
    assert_eq!(ios_deployment_target(dir.path()), Some((16, 4)));
    let none = tempfile::tempdir().unwrap();
    assert_eq!(ios_deployment_target(none.path()), None);
}
