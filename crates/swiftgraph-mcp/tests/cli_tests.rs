//! Black-box tests of the `swiftgraph` binary.

use std::process::Command;

fn swiftgraph() -> Command {
    Command::new(env!("CARGO_BIN_EXE_swiftgraph"))
}

#[test]
fn missing_boundaries_config_is_an_error_not_a_panic() {
    let dir = tempfile::tempdir().unwrap();
    let out = swiftgraph()
        .args(["boundaries", "--config", "does-not-exist.json"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success());
    assert!(!stderr.contains("panicked"), "binary panicked: {stderr}");
    assert!(stderr.contains("does-not-exist.json"), "{stderr}");
}

fn project_with_findings(n: usize) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..n {
        std::fs::write(
            dir.path().join(format!("Holder{i}.swift")),
            "final class Holder: NSObject {\n    var delegate: HolderDelegate?\n}\n",
        )
        .unwrap();
    }
    dir
}

#[test]
fn audit_json_is_not_capped_by_default() {
    let dir = project_with_findings(120);
    let out = swiftgraph()
        .args(["audit", "--format", "json"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["total_issues"], 120);
    assert_eq!(json["truncated"], false);
}

#[test]
fn audit_text_says_how_many_findings_were_cut() {
    let dir = project_with_findings(12);
    let out = swiftgraph()
        .args(["audit", "--max-issues", "5"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("truncated, 7 more"), "{text}");
}
