//! Shared helpers for integration tests.

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

/// Copy `tests/fixtures/spm` into the cargo test tmp dir and build it with
/// SwiftPM so that `.build/<triple>/debug/index/store` exists.
///
/// Returns `None` (and the caller should skip) when `swift` is unavailable or
/// the build fails, e.g. on a machine without Xcode.
#[allow(dead_code)]
pub fn built_spm_fixture() -> Option<PathBuf> {
    static ROOT: OnceLock<Option<PathBuf>> = OnceLock::new();
    ROOT.get_or_init(|| {
        let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/spm");
        let dst = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("spm-fixture");
        copy_dir(&src, &dst).ok()?;
        let status = Command::new("swift")
            .arg("build")
            .current_dir(&dst)
            .output()
            .ok()?;
        if !status.status.success() {
            eprintln!(
                "swift build failed, skipping Index Store tests:\n{}",
                String::from_utf8_lossy(&status.stderr)
            );
            return None;
        }
        dst.canonicalize().ok()
    })
    .clone()
}

fn copy_dir(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let target = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}
