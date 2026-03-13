//! Configuration file loading for `.swiftgraph/config.json`.

use std::path::Path;

use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::Deserialize;

/// SwiftGraph project configuration.
#[derive(Debug, Clone, Deserialize)]
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
    /// Index Store path ("auto" or explicit path).
    #[serde(default = "default_index_store")]
    pub index_store_path: String,
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
        }
    }
}

impl Config {
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
        builder
            .build()
            .unwrap_or_else(|_| GlobSetBuilder::new().build().unwrap())
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
