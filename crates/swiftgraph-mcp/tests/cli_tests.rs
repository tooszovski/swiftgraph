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
