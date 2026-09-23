use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Errors from project detection.
#[derive(Debug, Error)]
pub enum ProjectError {
    /// No project markers found in the directory or below it.
    #[error("no Swift project found in {0}")]
    NotFound(PathBuf),
    /// Index Store could not be located.
    #[error("could not locate Index Store: {0}")]
    IndexStoreNotFound(String),
    /// Filesystem error.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Detected project type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProjectType {
    Spm,
    Xcode,
    XcodeWorkspace,
    XcodeGen,
    Tuist,
}

impl ProjectType {
    /// Stable string form used in tool output.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Spm => "spm",
            Self::Xcode => "xcode",
            Self::XcodeWorkspace => "xcode-workspace",
            Self::XcodeGen => "xcodegen",
            Self::Tuist => "tuist",
        }
    }
}

/// Detected project info.
#[derive(Debug, Clone)]
pub struct ProjectInfo {
    /// Directory holding the project markers (may be below the requested root).
    pub root: PathBuf,
    /// Detected project type.
    pub project_type: ProjectType,
    /// Project name (workspace/project file stem, else directory name).
    pub name: String,
    /// Index Store location, if one was found.
    pub index_store_path: Option<PathBuf>,
}

/// How many directory levels below the root [`detect_project`] searches for
/// project markers when the root itself has none (e.g. code living in `./ios`).
pub const MAX_DETECT_DEPTH: usize = 3;

/// Directories never descended into while looking for project markers.
const SKIPPED_DIRS: &[&str] = &[
    ".build",
    ".git",
    ".swiftgraph",
    "Pods",
    "Carthage",
    "DerivedData",
    "node_modules",
    "build",
    "vendor",
];

/// Detect the Swift project type and metadata from a directory.
///
/// Resolution order:
/// 1. `project_dir` from `.swiftgraph/config.json`, if set (relative to `root`);
/// 2. project markers directly in `root`;
/// 3. breadth-first search up to [`MAX_DETECT_DEPTH`] levels below `root`,
///    skipping build output and vendored dependencies.
///
/// The returned [`ProjectInfo::root`] is the directory that holds the markers.
pub fn detect_project(root: &Path) -> Result<ProjectInfo, ProjectError> {
    let root = root.canonicalize().map_err(ProjectError::Io)?;

    let config = crate::config::Config::load(&root);
    if let Some(dir) = config.project_dir.as_deref().filter(|d| !d.is_empty()) {
        let project_dir = root.join(dir);
        let project_dir = project_dir.canonicalize().map_err(ProjectError::Io)?;
        return detect_in_dir(&project_dir).ok_or(ProjectError::NotFound(project_dir));
    }

    if let Some(info) = detect_in_dir(&root) {
        return Ok(info);
    }

    let mut level = vec![root.clone()];
    for _ in 0..MAX_DETECT_DEPTH {
        let mut next = Vec::new();
        for dir in &level {
            for child in sorted_subdirs(dir) {
                if let Some(info) = detect_in_dir(&child) {
                    return Ok(info);
                }
                next.push(child);
            }
        }
        level = next;
    }

    Err(ProjectError::NotFound(root))
}

/// Resolve the Index Store to use for `root`, honouring `index_store_path`
/// from `.swiftgraph/config.json` (`"auto"`, `"none"`, or a path).
///
/// Returns `None` when the store is disabled, the configured path does not
/// exist, or auto-detection finds nothing. Never fails.
pub fn resolve_index_store(root: &Path) -> Option<PathBuf> {
    use crate::config::{Config, IndexStoreSetting};
    match Config::load(root).index_store_setting(root) {
        IndexStoreSetting::Disabled => None,
        IndexStoreSetting::Path(p) => {
            if p.is_dir() {
                Some(p)
            } else {
                tracing::warn!("configured index_store_path {} not found", p.display());
                None
            }
        }
        IndexStoreSetting::Auto => detect_project(root).ok()?.index_store_path,
    }
}

/// Check a single directory for project markers (no recursion).
fn detect_in_dir(dir: &Path) -> Option<ProjectInfo> {
    if dir.join("Tuist").is_dir() {
        return Some(make_info(dir, ProjectType::Tuist));
    }
    if dir.join("project.yml").is_file() {
        return Some(make_info(dir, ProjectType::XcodeGen));
    }
    if find_with_extension(dir, "xcworkspace").is_some() {
        return Some(make_info(dir, ProjectType::XcodeWorkspace));
    }
    if find_with_extension(dir, "xcodeproj").is_some() {
        return Some(make_info(dir, ProjectType::Xcode));
    }
    if dir.join("Package.swift").is_file() {
        return Some(make_info(dir, ProjectType::Spm));
    }
    None
}

/// Locate a SwiftPM Index Store under `root/.build`.
///
/// Checks the explicit `-index-store-path .build/index/store` location first,
/// then SwiftPM's own `.build/<triple>/{debug,release}/index/store`, picking
/// the most recently modified one.
fn find_spm_index_store(root: &Path) -> Option<PathBuf> {
    let build = root.join(".build");
    let explicit = build.join("index/store");
    if explicit.is_dir() {
        return Some(explicit);
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    for config in ["debug", "release"] {
        candidates.push(build.join(config).join("index/store"));
    }
    if let Ok(entries) = std::fs::read_dir(&build) {
        for entry in entries.flatten() {
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                for config in ["debug", "release"] {
                    candidates.push(entry.path().join(config).join("index/store"));
                }
            }
        }
    }
    candidates
        .into_iter()
        .filter(|p| p.is_dir())
        .filter_map(|p| {
            let modified = std::fs::metadata(&p).and_then(|m| m.modified()).ok()?;
            Some((modified, p.canonicalize().unwrap_or(p)))
        })
        .max_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.cmp(&a.1)))
        .map(|(_, p)| p)
}

/// Subdirectories of `dir` worth searching, in a stable (sorted) order.
fn sorted_subdirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            !name.starts_with('.')
                && !SKIPPED_DIRS.contains(&name.as_ref())
                && !name.ends_with(".xcodeproj")
                && !name.ends_with(".xcworkspace")
        })
        .map(|e| e.path())
        .collect();
    dirs.sort();
    dirs
}

/// Find Index Store path in DerivedData for Xcode-based projects.
pub fn find_xcode_index_store(project_name: &str) -> Option<PathBuf> {
    let home = dirs_hint();
    let derived_data = home.join("Library/Developer/Xcode/DerivedData");

    if !derived_data.is_dir() {
        return None;
    }

    // DerivedData dirs are named like "ProjectName-abcdef123456"
    let entries = std::fs::read_dir(&derived_data).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with(project_name) || name_str.starts_with(&format!("{project_name}-")) {
            let index_path = entry.path().join("Index.noindex/DataStore");
            if index_path.is_dir() {
                return Some(index_path);
            }
            // Also check older path format
            let index_path_v2 = entry.path().join("Index/DataStore");
            if index_path_v2.is_dir() {
                return Some(index_path_v2);
            }
        }
    }

    None
}

fn make_info(root: &Path, project_type: ProjectType) -> ProjectInfo {
    let dir_name = root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "Unknown".into());

    // DerivedData folders are named after the workspace/project, not the directory.
    let name = find_with_extension(root, "xcworkspace")
        .or_else(|| find_with_extension(root, "xcodeproj"))
        .unwrap_or(dir_name);

    let index_store_path = match project_type {
        ProjectType::Spm => find_spm_index_store(root),
        ProjectType::Xcode
        | ProjectType::XcodeWorkspace
        | ProjectType::XcodeGen
        | ProjectType::Tuist => find_xcode_index_store(&name),
    };

    ProjectInfo {
        root: root.to_path_buf(),
        project_type,
        name,
        index_store_path,
    }
}

/// File stem of the first (alphabetically) entry in `dir` with the given extension.
fn find_with_extension(dir: &Path, ext: &str) -> Option<String> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut stems: Vec<String> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == ext))
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
        .collect();
    stems.sort();
    stems.into_iter().next()
}

fn dirs_hint() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_type_as_str() {
        assert_eq!(ProjectType::Spm.as_str(), "spm");
        assert_eq!(ProjectType::Xcode.as_str(), "xcode");
    }

    #[test]
    fn detects_xcode_project_in_subdirectory() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("ios/MyApp.xcodeproj")).unwrap();
        std::fs::create_dir_all(dir.path().join("android")).unwrap();

        let info = detect_project(dir.path()).unwrap();
        assert_eq!(info.project_type, ProjectType::Xcode);
        assert_eq!(info.name, "MyApp");
        assert_eq!(info.root, dir.path().canonicalize().unwrap().join("ios"));
    }

    #[test]
    fn prefers_workspace_name_for_xcode_projects() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("App.xcworkspace")).unwrap();
        std::fs::create_dir_all(dir.path().join("AppCore.xcodeproj")).unwrap();

        let info = detect_project(dir.path()).unwrap();
        assert_eq!(info.project_type, ProjectType::XcodeWorkspace);
        assert_eq!(info.name, "App");
    }

    #[test]
    fn skips_build_and_vendor_directories() {
        let dir = tempfile::tempdir().unwrap();
        for skipped in [".build/checkouts/dep", "Pods/Dep", "node_modules/x"] {
            let d = dir.path().join(skipped);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("Package.swift"), "").unwrap();
        }
        assert!(matches!(
            detect_project(dir.path()),
            Err(ProjectError::NotFound(_))
        ));
    }

    #[test]
    fn project_dir_from_config_wins() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("a/First.xcodeproj")).unwrap();
        std::fs::create_dir_all(dir.path().join("b")).unwrap();
        std::fs::write(dir.path().join("b/Package.swift"), "").unwrap();
        std::fs::create_dir_all(dir.path().join(".swiftgraph")).unwrap();
        std::fs::write(
            dir.path().join(".swiftgraph/config.json"),
            r#"{"project_dir": "b"}"#,
        )
        .unwrap();

        let info = detect_project(dir.path()).unwrap();
        assert_eq!(info.project_type, ProjectType::Spm);
        assert_eq!(info.root, dir.path().canonicalize().unwrap().join("b"));
    }

    #[test]
    fn root_markers_take_priority_over_subdirectories() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Package.swift"), "").unwrap();
        std::fs::create_dir_all(dir.path().join("Example/Demo.xcodeproj")).unwrap();

        let info = detect_project(dir.path()).unwrap();
        assert_eq!(info.project_type, ProjectType::Spm);
    }
}

#[cfg(test)]
mod index_store_resolution_tests {
    use super::*;

    fn write_config(root: &Path, json: &str) {
        std::fs::create_dir_all(root.join(".swiftgraph")).unwrap();
        std::fs::write(root.join(".swiftgraph/config.json"), json).unwrap();
    }

    #[test]
    fn explicit_index_store_path_from_config_is_used() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Package.swift"), "").unwrap();
        std::fs::create_dir_all(dir.path().join("custom/store")).unwrap();
        write_config(dir.path(), r#"{"index_store_path": "custom/store"}"#);

        let store = resolve_index_store(dir.path()).unwrap();
        assert!(store.ends_with("custom/store"));
    }

    #[test]
    fn index_store_can_be_disabled() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Package.swift"), "").unwrap();
        std::fs::create_dir_all(dir.path().join(".build/index/store")).unwrap();
        assert!(resolve_index_store(dir.path()).is_some());

        write_config(dir.path(), r#"{"index_store_path": "none"}"#);
        assert!(resolve_index_store(dir.path()).is_none());
    }

    #[test]
    fn missing_explicit_path_degrades_to_none() {
        let dir = tempfile::tempdir().unwrap();
        write_config(dir.path(), r#"{"index_store_path": "nope"}"#);
        assert!(resolve_index_store(dir.path()).is_none());
    }
}

/// Environment variable overriding the database location.
pub const DB_ENV: &str = "SWIFTGRAPH_DB";

/// Database path for a project: `$SWIFTGRAPH_DB` if set, otherwise
/// `<root>/.swiftgraph/db.sqlite`. The override lets several indexes of one
/// source tree coexist (e.g. a read-only checkout or a running server).
pub fn db_path(root: &Path) -> PathBuf {
    match std::env::var_os(DB_ENV).filter(|v| !v.is_empty()) {
        Some(p) => PathBuf::from(p),
        None => root.join(".swiftgraph/db.sqlite"),
    }
}
