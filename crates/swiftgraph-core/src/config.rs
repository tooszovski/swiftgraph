//! Configuration file loading for `.swiftgraph/config.json`.

use std::path::{Path, PathBuf};

use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};

/// Where the Index Store comes from, as configured by `index_store_path`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexStoreSetting {
    /// `"auto"` (default): detect from `.build/index/store` or DerivedData.
    Auto,
    /// `"none"` / `"off"`: never use the Index Store, tree-sitter only.
    Disabled,
    /// Explicit path (relative paths resolve against the project root).
    Path(PathBuf),
}

/// SwiftGraph project configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    /// Config version.
    #[serde(default = "default_version")]
    pub version: u32,
    /// Include globs (e.g., `["Sources/**/*.swift", "Tests/**/*.swift"]`).
    #[serde(default)]
    pub include: Vec<String>,
    /// Exclude globs (e.g., `["**/Generated/**", "**/Pods/**"]`).
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Index Store path: `"auto"`, `"none"`, or an explicit path.
    #[serde(default = "default_index_store")]
    pub index_store_path: String,
    /// Directory (relative to the project root) holding the Xcode project or
    /// `Package.swift`, e.g. `"ios"`. When unset it is auto-detected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_dir: Option<String>,
    /// Call resolution settings for tree-sitter mode.
    #[serde(default)]
    pub resolution: ResolutionConfig,
}

/// How tree-sitter call sites are resolved to project symbols.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ResolutionConfig {
    /// Maximum number of equally plausible targets for which a call still
    /// gets (ambiguous) edges. Calls with more candidates get no edges; their
    /// names are only recorded as possibly used.
    #[serde(default = "default_max_candidates")]
    pub max_candidates: usize,
}

fn default_max_candidates() -> usize {
    3
}

impl Default for ResolutionConfig {
    fn default() -> Self {
        Self {
            max_candidates: default_max_candidates(),
        }
    }
}

fn default_version() -> u32 {
    1
}

fn default_index_store() -> String {
    "auto".into()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            include: vec![],
            exclude: vec![
                "**/Generated/**".into(),
                "**/Pods/**".into(),
                "**/.build/**".into(),
                "**/DerivedData/**".into(),
            ],
            index_store_path: "auto".into(),
            project_dir: None,
            resolution: ResolutionConfig::default(),
        }
    }
}

impl Config {
    /// Write the default config to `.swiftgraph/config.json` unless the file
    /// already exists. Returns the config file path.
    pub fn write_default(project_root: &Path) -> std::io::Result<PathBuf> {
        let config_dir = project_root.join(".swiftgraph");
        std::fs::create_dir_all(&config_dir)?;
        let config_path = config_dir.join("config.json");
        if !config_path.exists() {
            let json = serde_json::to_string_pretty(&Self::default())
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            std::fs::write(&config_path, json + "\n")?;
        }
        Ok(config_path)
    }

    /// Interpret `index_store_path` relative to `project_root`.
    pub fn index_store_setting(&self, project_root: &Path) -> IndexStoreSetting {
        match self.index_store_path.trim() {
            "" | "auto" => IndexStoreSetting::Auto,
            "none" | "off" | "disabled" => IndexStoreSetting::Disabled,
            p => IndexStoreSetting::Path(project_root.join(p)),
        }
    }

    /// Load config from `.swiftgraph/config.json` in the project root.
    /// Returns default config if the file doesn't exist.
    pub fn load(project_root: &Path) -> Self {
        let config_path = project_root.join(".swiftgraph/config.json");
        if config_path.exists() {
            match std::fs::read_to_string(&config_path) {
                Ok(content) => match serde_json::from_str::<Config>(&content) {
                    Ok(config) => return config,
                    Err(e) => {
                        tracing::warn!("Failed to parse config.json: {e}");
                    }
                },
                Err(e) => {
                    tracing::warn!("Failed to read config.json: {e}");
                }
            }
        }
        Self::default()
    }

    /// Build a GlobSet from the include patterns.
    /// Returns None if no include patterns are specified (= include everything).
    pub fn include_globset(&self) -> Option<GlobSet> {
        if self.include.is_empty() {
            return None;
        }
        let mut builder = GlobSetBuilder::new();
        for pattern in &self.include {
            if let Ok(glob) = Glob::new(pattern) {
                builder.add(glob);
            }
        }
        builder.build().ok()
    }

    /// Build a GlobSet from the exclude patterns.
    pub fn exclude_globset(&self) -> GlobSet {
        let mut builder = GlobSetBuilder::new();
        for pattern in &self.exclude {
            if let Ok(glob) = Glob::new(pattern) {
                builder.add(glob);
            }
        }
        builder.build().unwrap_or_else(|e| {
            tracing::warn!("invalid exclude globs, excluding nothing: {e}");
            GlobSet::empty()
        })
    }

    /// Check if a path should be included based on include/exclude globs.
    pub fn should_include(
        &self,
        path: &Path,
        include_set: &Option<GlobSet>,
        exclude_set: &GlobSet,
    ) -> bool {
        // Check exclude first
        if exclude_set.is_match(path) {
            return false;
        }
        // If include patterns exist, path must match at least one
        if let Some(ref incl) = include_set {
            return incl.is_match(path);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn written_default_config_includes_everything_outside_excludes() {
        let dir = tempfile::tempdir().unwrap();
        let path = Config::write_default(dir.path()).unwrap();
        assert!(path.ends_with(".swiftgraph/config.json"));

        let raw: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(raw.get("swift_syntax_path").is_none(), "dead key written");
        assert!(raw.get("audit").is_none(), "dead key written");

        let config = Config::load(dir.path());
        assert!(config.include.is_empty());
        let incl = config.include_globset();
        let excl = config.exclude_globset();
        assert!(config.should_include(Path::new("ios/App/Feature.swift"), &incl, &excl));
        assert!(config.should_include(Path::new("App/AppDelegate.swift"), &incl, &excl));
        assert!(!config.should_include(Path::new("ios/Pods/X/Y.swift"), &incl, &excl));
        assert!(!config.should_include(
            Path::new("DerivedData/App/SourcePackages/Z.swift"),
            &incl,
            &excl
        ));
    }

    #[test]
    fn write_default_keeps_existing_config() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".swiftgraph")).unwrap();
        let path = dir.path().join(".swiftgraph/config.json");
        std::fs::write(&path, r#"{"include": ["App/**"]}"#).unwrap();
        Config::write_default(dir.path()).unwrap();
        assert_eq!(Config::load(dir.path()).include, vec!["App/**".to_string()]);
    }

    #[test]
    fn index_store_setting_parsing() {
        let root = Path::new("/p");
        let mut c = Config::default();
        assert_eq!(c.index_store_setting(root), IndexStoreSetting::Auto);
        c.index_store_path = "none".into();
        assert_eq!(c.index_store_setting(root), IndexStoreSetting::Disabled);
        c.index_store_path = "build/index".into();
        assert_eq!(
            c.index_store_setting(root),
            IndexStoreSetting::Path("/p/build/index".into())
        );
        c.index_store_path = "/abs/store".into();
        assert_eq!(
            c.index_store_setting(root),
            IndexStoreSetting::Path("/abs/store".into())
        );
    }
}
