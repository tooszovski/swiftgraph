//! Storage audit rules (STR-001 through STR-004).

use crate::engine::{AuditIssue, Category, Severity};
use crate::rules::{find_descendants, node_text, AuditRule, FileContext};

/// STR-001: Storing files in wrong directory (Documents vs Caches vs tmp).
pub struct WrongStorageDirectory;

impl AuditRule for WrongStorageDirectory {
    fn id(&self) -> &str {
        "STR-001"
    }
    fn name(&self) -> &str {
        "wrong-storage-directory"
    }
    fn category(&self) -> Category {
        Category::Storage
    }
    fn severity(&self) -> Severity {
        Severity::Medium
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        // Find .documentDirectory used for non-user-created content
        let dir_refs = find_descendants(root, ctx.source, &|node, src| {
            let text = node_text(node, src);
            text.contains(".documentDirectory")
                && (text.contains("cache")
                    || text.contains("Cache")
                    || text.contains("temp")
                    || text.contains("Temp")
                    || text.contains("thumbnail")
                    || text.contains("Thumbnail"))
        });

        for dir_ref in dir_refs {
            issues.push(AuditIssue {
                id: format!("{}:{}", self.id(), ctx.file_path),
                category: self.category(),
                severity: self.severity(),
                rule: self.id().to_string(),
                message:
                    "Cache/temp files stored in Documents directory — use Caches or tmp instead"
                        .into(),
                file: ctx.file_path.to_string(),
                line: dir_ref.start_position().row as u32 + 1,
                symbol: None,
                fix: Some(
                    "Use .cachesDirectory for cache files or NSTemporaryDirectory() for temp files"
                        .into(),
                ),
            });
        }

        issues
    }
}

/// STR-002: Missing backup exclusion for regenerable data.
pub struct MissingBackupExclusion;

impl AuditRule for MissingBackupExclusion {
    fn id(&self) -> &str {
        "STR-002"
    }
    fn name(&self) -> &str {
        "missing-backup-exclusion"
    }
    fn category(&self) -> Category {
        Category::Storage
    }
    fn severity(&self) -> Severity {
        Severity::Medium
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        // Find files written to Application Support without backup exclusion
        let writes = find_descendants(root, ctx.source, &|node, src| {
            let text = node_text(node, src);
            text.contains(".applicationSupportDirectory")
                && (text.contains("write(") || text.contains("createFile("))
        });

        for write in writes {
            if !ctx.source.contains("isExcludedFromBackup")
                && !ctx.source.contains("excludedFromBackup")
            {
                issues.push(AuditIssue {
                    id: format!("{}:{}", self.id(), ctx.file_path),
                    category: self.category(),
                    severity: self.severity(),
                    rule: self.id().to_string(),
                    message: "Files in Application Support without backup exclusion — may bloat iCloud backup".into(),
                    file: ctx.file_path.to_string(),
                    line: write.start_position().row as u32 + 1,
                    symbol: None,
                    fix: Some("Set isExcludedFromBackup = true on regenerable data".into()),
                });
            }
        }

        issues
    }
}

/// STR-003: Missing file protection for sensitive data.
pub struct MissingFileProtection;

impl AuditRule for MissingFileProtection {
    fn id(&self) -> &str {
        "STR-003"
    }
    fn name(&self) -> &str {
        "missing-file-protection"
    }
    fn category(&self) -> Category {
        Category::Storage
    }
    fn severity(&self) -> Severity {
        Severity::High
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        // Find file writes with sensitive-looking names without protection attributes
        let writes = find_descendants(root, ctx.source, &|node, src| {
            if node.kind() != "call_expression" {
                return false;
            }
            let text = node_text(node, src);
            text.contains(".write(")
                && (text.contains("token")
                    || text.contains("Token")
                    || text.contains("secret")
                    || text.contains("Secret")
                    || text.contains("credential")
                    || text.contains("password")
                    || text.contains("private"))
        });

        for write in writes {
            if !ctx.source.contains("FileProtectionType")
                && !ctx.source.contains(".completeFileProtection")
                && !ctx.source.contains(".protectionComplete")
            {
                issues.push(AuditIssue {
                    id: format!("{}:{}", self.id(), ctx.file_path),
                    category: self.category(),
                    severity: self.severity(),
                    rule: self.id().to_string(),
                    message: "Sensitive data written to file without file protection — readable when device is locked".into(),
                    file: ctx.file_path.to_string(),
                    line: write.start_position().row as u32 + 1,
                    symbol: None,
                    fix: Some("Use .completeFileProtection or store in Keychain instead".into()),
                });
            }
        }

        issues
    }
}

/// STR-004: Large data stored in UserDefaults.
pub struct LargeUserDefaults;

impl AuditRule for LargeUserDefaults {
    fn id(&self) -> &str {
        "STR-004"
    }
    fn name(&self) -> &str {
        "large-user-defaults"
    }
    fn category(&self) -> Category {
        Category::Storage
    }
    fn severity(&self) -> Severity {
        Severity::Medium
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        // Find UserDefaults storing potentially large data
        let ud_calls = find_descendants(root, ctx.source, &|node, src| {
            if node.kind() != "call_expression" && node.kind() != "navigation_expression" {
                return false;
            }
            let text = node_text(node, src);
            text.contains("UserDefaults")
                && text.contains(".set(")
                && (text.contains("Data")
                    || text.contains("data")
                    || text.contains("[")
                    || text.contains("Array"))
        });

        for call in ud_calls {
            issues.push(AuditIssue {
                id: format!("{}:{}", self.id(), ctx.file_path),
                category: self.category(),
                severity: self.severity(),
                rule: self.id().to_string(),
                message:
                    "Storing potentially large data in UserDefaults — use file storage or database"
                        .into(),
                file: ctx.file_path.to_string(),
                line: call.start_position().row as u32 + 1,
                symbol: None,
                fix: Some("Use FileManager, SwiftData, or Core Data for large data".into()),
            });
        }

        issues
    }
}

/// All storage rules.
pub fn all_rules() -> Vec<Box<dyn AuditRule>> {
    vec![
        Box::new(WrongStorageDirectory),
        Box::new(MissingBackupExclusion),
        Box::new(MissingFileProtection),
        Box::new(LargeUserDefaults),
    ]
}
