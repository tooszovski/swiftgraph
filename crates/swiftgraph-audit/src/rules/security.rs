//! Security audit rules (SEC-001 through SEC-004).

use regex::Regex;

use crate::engine::{AuditIssue, Category, Severity};
use crate::rules::{decl_name, find_descendants, node_text, AuditRule, FileContext};

/// SEC-001: Hardcoded secrets — API keys, tokens, passwords in string literals.
///
/// Literals that name something rather than hold a secret are skipped:
/// enum raw values (`case removeToken = "..."`), HTTP header names
/// (`"X-API-KEY"`), public addresses (`0x` + 40 hex digits) and text with
/// spaces or brackets (analytics events, UI strings).
pub struct HardcodedSecrets;

/// Whether a matched literal is a name or label rather than a secret.
fn is_benign_literal(line: &str, value: &str) -> bool {
    static HEADER: std::sync::OnceLock<Option<Regex>> = std::sync::OnceLock::new();
    static ADDRESS: std::sync::OnceLock<Option<Regex>> = std::sync::OnceLock::new();
    let matches = |cell: &'static std::sync::OnceLock<Option<Regex>>, pattern: &str| {
        cell.get_or_init(|| Regex::new(pattern).ok())
            .as_ref()
            .is_some_and(|re| re.is_match(value))
    };
    line.trim_start().starts_with("case ")
        || value
            .chars()
            .any(|c| c.is_whitespace() || "[]()".contains(c))
        || matches(&HEADER, r"^[A-Za-z]+(-[A-Za-z]+)+$")
        || matches(&ADDRESS, r"^0x[0-9a-fA-F]{40}$")
}

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

        // (pattern, label, whether group 2 is an assigned literal to vet)
        let patterns = [
            (
                r#"(?i)(api[_\-]?key|apikey)\s*[:=]\s*"([^"]{8,})""#,
                "API key",
                true,
            ),
            (
                r#"(?i)(secret|token|password|passwd|pwd)\s*[:=]\s*"([^"]{8,})""#,
                "secret/token/password",
                true,
            ),
            (
                r#"(?i)bearer\s+[a-zA-Z0-9\-._~+/]+=*"#,
                "Bearer token",
                false,
            ),
            (r#"sk-[a-zA-Z0-9]{20,}"#, "OpenAI API key", false),
            (r#"ghp_[a-zA-Z0-9]{36}"#, "GitHub PAT", false),
            (r#"xox[bprs]-[a-zA-Z0-9\-]+"#, "Slack token", false),
        ];

        for (pattern, label, literal) in &patterns {
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
                    let Some(caps) = re.captures(line) else {
                        continue;
                    };
                    let benign = *literal
                        && caps
                            .get(2)
                            .is_some_and(|value| is_benign_literal(line, value.as_str()));
                    if !benign {
                        issues.push(AuditIssue {
                            id: format!("{}:{}", self.id(), ctx.file_path),
                            category: self.category(),
                            severity: self.severity(),
                            rule: self.id().to_string(),
                            message: format!("Possible hardcoded {label} detected"),
                            file: ctx.file_path.to_string(),
                            line: i as u32 + 1,
                            column: None,
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
                            column: None,
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
///
/// Identifiers on the log line are split into words; `token` alone (an
/// asset in wallet apps, `token.name`, `tokenItem`) is not sensitive, while
/// `password`, `secret`, `mnemonic`, `seed`, `privateKey`, `accessToken`,
/// `refreshToken`, `apiKey` and similar are.
pub struct SensitiveLogging;

/// Single words that mark a credential.
const SENSITIVE_WORDS: &[&str] = &[
    "password",
    "passwd",
    "passphrase",
    "secret",
    "mnemonic",
    "seed",
    "credential",
    "credentials",
    "apikey",
    "privatekey",
    "accesstoken",
    "refreshtoken",
];

/// Adjacent word pairs that mark a credential.
const SENSITIVE_PAIRS: &[(&str, &str)] = &[
    ("private", "key"),
    ("access", "token"),
    ("refresh", "token"),
    ("auth", "token"),
    ("session", "token"),
    ("id", "token"),
    ("api", "key"),
    ("secret", "key"),
];

/// Lowercase words of an identifier: `userPassword` → `user`, `password`.
fn identifier_words(ident: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut prev_lower = false;
    for c in ident.chars() {
        if c == '_' || !c.is_alphanumeric() {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            prev_lower = false;
            continue;
        }
        if c.is_uppercase() && prev_lower && !current.is_empty() {
            words.push(std::mem::take(&mut current));
        }
        prev_lower = c.is_lowercase() || c.is_ascii_digit();
        current.extend(c.to_lowercase());
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

/// What a log line actually writes out: the code inside string
/// interpolations (`"pwd \\(password)"`) and unlabeled arguments of the call
/// (`NSLog("%@", privateKey)`). Message text and labeled arguments such as
/// `error: Error.missingAccessToken` are left out.
fn logged_code(line: &str) -> String {
    let mut outer = String::with_capacity(line.len());
    let mut interpolated = String::new();
    let mut in_string = false;
    let mut depth = 0usize; // parentheses inside an interpolation
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if in_string && depth == 0 {
            match c {
                '\\' if chars.peek() == Some(&'(') => {
                    chars.next();
                    depth = 1;
                    interpolated.push(' ');
                }
                '\\' => {
                    chars.next();
                }
                '"' => {
                    in_string = false;
                    outer.push('_');
                }
                _ => {}
            }
            continue;
        }
        if in_string {
            match c {
                '(' => depth += 1,
                ')' => depth -= 1,
                _ => {}
            }
            if depth > 0 {
                interpolated.push(c);
            }
            continue;
        }
        if c == '"' {
            in_string = true;
            continue;
        }
        outer.push(c);
    }

    // Unlabeled arguments of the first call on the line
    if let Some(open) = outer.find('(') {
        let mut level = 0usize;
        let mut arg = String::new();
        let mut args = Vec::new();
        for c in outer[open + 1..].chars() {
            match c {
                '(' | '[' => level += 1,
                ')' | ']' if level == 0 => break,
                ')' | ']' => level -= 1,
                ',' if level == 0 => {
                    args.push(std::mem::take(&mut arg));
                    continue;
                }
                _ => {}
            }
            arg.push(c);
        }
        args.push(arg);
        for arg in args {
            let trimmed = arg.trim_start();
            let label_len = trimmed
                .find(|c: char| !(c.is_alphanumeric() || c == '_'))
                .unwrap_or(trimmed.len());
            let labeled = label_len > 0 && trimmed[label_len..].trim_start().starts_with(':');
            if !labeled {
                interpolated.push(' ');
                interpolated.push_str(&arg);
            }
        }
    }
    interpolated
}

/// The credential term written by the log call on `line`, if any.
fn sensitive_term(line: &str) -> Option<String> {
    let code = logged_code(line);
    for ident in code
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
    {
        let words = identifier_words(ident);
        if let Some(w) = words.iter().find(|w| SENSITIVE_WORDS.contains(&w.as_str())) {
            return Some(w.clone());
        }
        if let Some((a, b)) = words.windows(2).find_map(|pair| {
            SENSITIVE_PAIRS
                .iter()
                .find(|(a, b)| pair[0] == *a && pair[1] == *b)
        }) {
            return Some(format!("{a} {b}"));
        }
    }
    None
}

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

        for (i, line) in ctx.source.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.starts_with("//") {
                continue;
            }

            let has_log = log_functions.iter().any(|f| line.contains(f));
            if !has_log {
                continue;
            }

            if let Some(s) = sensitive_term(line) {
                issues.push(AuditIssue {
                    id: format!("{}:{}", self.id(), ctx.file_path),
                    category: self.category(),
                    severity: self.severity(),
                    rule: self.id().to_string(),
                    message: format!("Potentially logging sensitive data (`{s}`)"),
                    file: ctx.file_path.to_string(),
                    line: i as u32 + 1,
                    column: None,
                    symbol: None,
                    fix: Some(
                        "Redact sensitive values in log output or use `.private` privacy level"
                            .into(),
                    ),
                });
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
                    column: None,
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

/// SEC-005: Injectable format strings (String(format:) with user input).
pub struct InjectableFormatString;

impl AuditRule for InjectableFormatString {
    fn id(&self) -> &str {
        "SEC-005"
    }
    fn name(&self) -> &str {
        "injectable-format-string"
    }
    fn category(&self) -> Category {
        Category::Security
    }
    fn severity(&self) -> Severity {
        Severity::Medium
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        let calls = find_descendants(root, ctx.source, &|node, src| {
            if node.kind() != "call_expression" {
                return false;
            }
            let text = node_text(node, src);
            // Detect String(format:) or NSString(format:) with non-literal first argument
            (text.contains("String(format:") || text.contains("NSString(format:"))
                && !text.contains("format: \"")
        });

        for call in calls {
            issues.push(AuditIssue {
                id: format!("{}:{}", self.id(), ctx.file_path),
                category: self.category(),
                severity: self.severity(),
                rule: self.id().to_string(),
                message: "String(format:) with non-literal format argument — potential format string injection".into(),
                file: ctx.file_path.to_string(),
                line: call.start_position().row as u32 + 1,
                column: None,
                symbol: None,
                fix: Some("Use string interpolation instead, or ensure the format string is a compile-time literal".into()),
            });
        }

        issues
    }
}

/// SEC-006: Missing certificate pinning for sensitive endpoints.
pub struct MissingCertPinning;

impl AuditRule for MissingCertPinning {
    fn id(&self) -> &str {
        "SEC-006"
    }
    fn name(&self) -> &str {
        "missing-cert-pinning"
    }
    fn category(&self) -> Category {
        Category::Security
    }
    fn severity(&self) -> Severity {
        Severity::High
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        // Detect URLSessionDelegate implementations without certificate validation
        let class_decls = find_descendants(root, ctx.source, &|node, src| {
            if node.kind() != "class_declaration" {
                return false;
            }
            let text = node_text(node, src);
            text.contains("URLSessionDelegate") || text.contains("URLSessionTaskDelegate")
        });

        for decl in class_decls {
            let text = node_text(decl, ctx.source);
            let name = decl_name(decl, ctx.source).unwrap_or_default();

            // Check for didReceive challenge handler
            if text.contains("urlSession") && text.contains("didReceive") {
                // Check if it accepts all certificates (completionHandler(.useCredential, ...))
                if text.contains(".useCredential") && !text.contains("SecTrust") {
                    issues.push(AuditIssue {
                        id: format!("{}:{}", self.id(), ctx.file_path),
                        category: self.category(),
                        severity: self.severity(),
                        rule: self.id().to_string(),
                        message: format!(
                            "`{name}` accepts credentials without certificate validation — bypasses TLS"
                        ),
                        file: ctx.file_path.to_string(),
                        line: decl.start_position().row as u32 + 1,
                        column: None,
                        symbol: Some(name),
                        fix: Some("Validate the server certificate against known pins using SecTrust".into()),
                    });
                }
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
        Box::new(InjectableFormatString),
        Box::new(MissingCertPinning),
    ]
}
