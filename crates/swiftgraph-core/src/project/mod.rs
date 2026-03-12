use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProjectError {
    #[error("no Swift project found in {0}")]
    NotFound(PathBuf),
    #[error("could not locate Index Store: {0}")]
    IndexStoreNotFound(String),
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
    pub root: PathBuf,
    pub project_type: ProjectType,
    pub name: String,
    pub index_store_path: Option<PathBuf>,
}

/// Detect the Swift project type and metadata from a directory.
pub fn detect_project(root: &Path) -> Result<ProjectInfo, ProjectError> {
    let root = root.canonicalize().map_err(ProjectError::Io)?;

    // Check markers in priority order
    if root.join("Tuist").is_dir() {
        return Ok(make_info(&root, ProjectType::Tuist));
    }
    if root.join("project.yml").is_file() {
        return Ok(make_info(&root, ProjectType::XcodeGen));
    }
    if has_extension(&root, "xcworkspace") {
        return Ok(make_info(&root, ProjectType::XcodeWorkspace));
    }
    if has_extension(&root, "xcodeproj") {
        return Ok(make_info(&root, ProjectType::Xcode));
    }
    if root.join("Package.swift").is_file() {
        let mut info = make_info(&root, ProjectType::Spm);
        // SPM Index Store is at .build/index/store/
        let spm_index = root.join(".build/index/store");
        if spm_index.is_dir() {
            info.index_store_path = Some(spm_index);
        }
        return Ok(info);
    }

    Err(ProjectError::NotFound(root))
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
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "Unknown".into());

    let index_store_path = match project_type {
        ProjectType::Spm => {
            let p = root.join(".build/index/store");
            p.is_dir().then_some(p)
        }
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

fn has_extension(dir: &Path, ext: &str) -> bool {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .any(|e| e.path().extension().is_some_and(|e| e == ext))
        })
        .unwrap_or(false)
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
}
