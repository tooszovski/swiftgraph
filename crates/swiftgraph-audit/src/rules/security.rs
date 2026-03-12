//! Security audit rules (SEC-001 through SEC-004).

use regex::Regex;

use crate::engine::{AuditIssue, Category, Severity};
use crate::rules::{AuditRule, FileContext};

/// SEC-001: Hardcoded secrets — API keys, tokens, passwords in string literals.
pub struct HardcodedSecrets;

impl AuditRule for HardcodedSecrets {
    fn id(&self) -> &str {
        "SEC-001"
    }
    fn name(&self) -> &str {
        "hardcoded-secrets"
    }
    fn category(&self) -> Category {
        Category::Security
    }
    fn severity(&self) -> Severity {
        Severity::Critical
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let mut issues = Vec::new();

        let patterns = [
            (
                r#"(?i)(api[_\-]?key|apikey)\s*[:=]\s*"[^"]{8,}""#,
                "API key",
            ),
            (
                r#"(?i)(secret|token|password|passwd|pwd)\s*[:=]\s*"[^"]{8,}""#,
                "secret/token/password",
            ),
            (r#"(?i)bearer\s+[a-zA-Z0-9\-._~+/]+=*"#, "Bearer token"),
            (r#"sk-[a-zA-Z0-9]{20,}"#, "OpenAI API key"),
            (r#"ghp_[a-zA-Z0-9]{36}"#, "GitHub PAT"),
            (r#"xox[bprs]-[a-zA-Z0-9\-]+"#, "Slack token"),
        ];

        for (pattern, label) in &patterns {
            if let Ok(re) = Regex::new(pattern) {
                for (i, line) in ctx.source.lines().enumerate() {
                    // Skip comments
                    let trimmed = line.trim();
                    if trimmed.starts_with("//")
                        || trimmed.starts_with("/*")
                        || trimmed.starts_with("*")
                    {
                        continue;
                    }
                    if re.is_match(line) {
                        issues.push(AuditIssue {
                            id: format!("{}:{}", self.id(), ctx.file_path),
                            category: self.category(),
                            severity: self.severity(),
                            rule: self.id().to_string(),
                            message: format!("Possible hardcoded {label} detected"),
                            file: ctx.file_path.to_string(),
                            line: i as u32 + 1,
                            symbol: None,
                            fix: Some("Move secrets to Keychain, environment variables, or a secure config file".into()),
                        });
                    }
                }
            }
        }

        issues
    }
}

/// SEC-002: Insecure data storage — UserDefaults for sensitive data.
pub struct InsecureStorage;

impl AuditRule for InsecureStorage {
    fn id(&self) -> &str {
        "SEC-002"
    }
    fn name(&self) -> &str {
        "insecure-storage"
    }
    fn category(&self) -> Category {
        Category::Security
    }
    fn severity(&self) -> Severity {
        Severity::High
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let mut issues = Vec::new();

        let sensitive_patterns = [
            "token",
            "password",
            "secret",
            "credential",
            "apiKey",
            "api_key",
            "accessToken",
            "access_token",
            "refreshToken",
            "refresh_token",
            "authToken",
            "auth_token",
            "sessionToken",
            "session_token",
        ];

        for (i, line) in ctx.source.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.starts_with("//") {
                continue;
            }

            // Check for UserDefaults storing sensitive data
            if line.contains("UserDefaults") || line.contains("@AppStorage") {
                for pattern in &sensitive_patterns {
                    if line.to_lowercase().contains(&pattern.to_lowercase()) {
                        issues.push(AuditIssue {
                            id: format!("{}:{}", self.id(), ctx.file_path),
                            category: self.category(),
                            severity: self.severity(),
                            rule: self.id().to_string(),
                            message: format!(
                                "Sensitive data (`{pattern}`) stored in UserDefaults/@AppStorage — not encrypted"
                            ),
                            file: ctx.file_path.to_string(),
                            line: i as u32 + 1,
                            symbol: None,
                            fix: Some("Use Keychain Services for sensitive data storage".into()),
                        });
                        break;
                    }
                }
            }
        }

        issues
    }
}

/// SEC-003: Logging sensitive data — print/NSLog/os_log with credentials.
pub struct SensitiveLogging;

impl AuditRule for SensitiveLogging {
    fn id(&self) -> &str {
        "SEC-003"
    }
    fn name(&self) -> &str {
        "sensitive-logging"
    }
    fn category(&self) -> Category {
        Category::Security
    }
    fn severity(&self) -> Severity {
        Severity::Medium
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let mut issues = Vec::new();

        let log_functions = ["print(", "NSLog(", "os_log(", "Logger.", "debugPrint("];
        let sensitive = [
            "password",
            "token",
            "secret",
            "credential",
            "apiKey",
            "api_key",
        ];

        for (i, line) in ctx.source.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.starts_with("//") {
                continue;
            }

            let has_log = log_functions.iter().any(|f| line.contains(f));
            if !has_log {
                continue;
            }

            let line_lower = line.to_lowercase();
            for s in &sensitive {
                if line_lower.contains(&s.to_lowercase()) {
                    issues.push(AuditIssue {
                        id: format!("{}:{}", self.id(), ctx.file_path),
                        category: self.category(),
                        severity: self.severity(),
                        rule: self.id().to_string(),
                        message: format!("Potentially logging sensitive data (`{s}`)"),
                        file: ctx.file_path.to_string(),
                        line: i as u32 + 1,
                        symbol: None,
                        fix: Some(
                            "Redact sensitive values in log output or use `.private` privacy level"
                                .into(),
                        ),
                    });
                    break;
                }
            }
        }

        issues
    }
}

/// SEC-004: ATS bypass — App Transport Security exceptions.
pub struct AtsBypass;

impl AuditRule for AtsBypass {
    fn id(&self) -> &str {
        "SEC-004"
    }
    fn name(&self) -> &str {
        "ats-bypass"
    }
    fn category(&self) -> Category {
        Category::Security
    }
    fn severity(&self) -> Severity {
        Severity::High
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let mut issues = Vec::new();

        // Check for http:// URLs (non-https)
        for (i, line) in ctx.source.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.starts_with("//") {
                continue;
            }

            if line.contains("http://")
                && !line.contains("http://localhost")
                && !line.contains("http://127.0.0.1")
            {
                issues.push(AuditIssue {
                    id: format!("{}:{}", self.id(), ctx.file_path),
                    category: self.category(),
                    severity: self.severity(),
                    rule: self.id().to_string(),
                    message: "Non-HTTPS URL detected — may require ATS exception".into(),
                    file: ctx.file_path.to_string(),
                    line: i as u32 + 1,
                    symbol: None,
                    fix: Some(
                        "Use HTTPS URLs. If HTTP is required, document the ATS exception".into(),
                    ),
                });
            }
        }

        issues
    }
}

/// All security rules.
pub fn all_rules() -> Vec<Box<dyn AuditRule>> {
    vec![
        Box::new(HardcodedSecrets),
        Box::new(InsecureStorage),
        Box::new(SensitiveLogging),
        Box::new(AtsBypass),
    ]
}
