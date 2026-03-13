//! Testing audit rules (TST-001 through TST-005).

use crate::engine::{AuditIssue, Category, Severity};
use crate::rules::{find_descendants, node_text, AuditRule, FileContext};

/// TST-001: sleep() in tests causing flakiness.
pub struct SleepInTest;

impl AuditRule for SleepInTest {
    fn id(&self) -> &str {
        "TST-001"
    }
    fn name(&self) -> &str {
        "sleep-in-test"
    }
    fn category(&self) -> Category {
        Category::Testing
    }
    fn severity(&self) -> Severity {
        Severity::High
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        // Only check test files
        if !ctx.file_path.contains("Test") && !ctx.file_path.contains("test") {
            return Vec::new();
        }

        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        let sleeps = find_descendants(root, ctx.source, &|node, src| {
            if node.kind() != "call_expression" {
                return false;
            }
            let text = node_text(node, src);
            text.contains("Thread.sleep") || text.contains("sleep(") || text.contains("usleep(")
        });

        for sleep in sleeps {
            issues.push(AuditIssue {
                id: format!("{}:{}", self.id(), ctx.file_path),
                category: self.category(),
                severity: self.severity(),
                rule: self.id().to_string(),
                message: "sleep() in test — causes flakiness and slow test suite".into(),
                file: ctx.file_path.to_string(),
                line: sleep.start_position().row as u32 + 1,
                symbol: None,
                fix: Some(
                    "Use XCTestExpectation, async/await, or Clock.sleep for Swift Testing".into(),
                ),
            });
        }

        issues
    }
}

/// TST-002: Test without any assertion.
pub struct MissingAssertion;

impl AuditRule for MissingAssertion {
    fn id(&self) -> &str {
        "TST-002"
    }
    fn name(&self) -> &str {
        "missing-assertion"
    }
    fn category(&self) -> Category {
        Category::Testing
    }
    fn severity(&self) -> Severity {
        Severity::High
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        if !ctx.file_path.contains("Test") && !ctx.file_path.contains("test") {
            return Vec::new();
        }

        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        // Find test functions
        let test_funcs = find_descendants(root, ctx.source, &|node, src| {
            if node.kind() != "function_declaration" {
                return false;
            }
            let text = node_text(node, src);
            text.contains("func test") || crate::rules::has_attribute(node, src, "Test")
        });

        for func in test_funcs {
            let text = node_text(func, ctx.source);
            let has_assertion = text.contains("XCTAssert")
                || text.contains("#expect(")
                || text.contains("#require(")
                || text.contains("XCTFail")
                || text.contains("XCTUnwrap");

            if !has_assertion {
                let name = crate::rules::decl_name(func, ctx.source).unwrap_or_default();
                issues.push(AuditIssue {
                    id: format!("{}:{}", self.id(), ctx.file_path),
                    category: self.category(),
                    severity: self.severity(),
                    rule: self.id().to_string(),
                    message: format!("Test `{name}` has no assertions — test always passes"),
                    file: ctx.file_path.to_string(),
                    line: func.start_position().row as u32 + 1,
                    symbol: Some(name),
                    fix: Some("Add XCTAssert/expect/require assertions".into()),
                });
            }
        }

        issues
    }
}

/// TST-003: Shared mutable state between tests.
pub struct SharedMutableState;

impl AuditRule for SharedMutableState {
    fn id(&self) -> &str {
        "TST-003"
    }
    fn name(&self) -> &str {
        "shared-mutable-state"
    }
    fn category(&self) -> Category {
        Category::Testing
    }
    fn severity(&self) -> Severity {
        Severity::Medium
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        if !ctx.file_path.contains("Test") && !ctx.file_path.contains("test") {
            return Vec::new();
        }

        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        // Find static var in test classes
        let static_vars = find_descendants(root, ctx.source, &|node, src| {
            if node.kind() != "property_declaration" {
                return false;
            }
            let text = node_text(node, src);
            text.contains("static var ") || text.contains("static let ")
        });

        // Only flag if we're in a test class
        let is_test_class = ctx.source.contains("XCTestCase") || ctx.source.contains("@Suite");

        if is_test_class {
            for var in static_vars {
                let text = node_text(var, ctx.source);
                if text.contains("static var ") {
                    let name = crate::rules::decl_name(var, ctx.source).unwrap_or_default();
                    issues.push(AuditIssue {
                        id: format!("{}:{}", self.id(), ctx.file_path),
                        category: self.category(),
                        severity: self.severity(),
                        rule: self.id().to_string(),
                        message: format!(
                            "Static mutable var `{name}` shared between tests — causes ordering-dependent failures"
                        ),
                        file: ctx.file_path.to_string(),
                        line: var.start_position().row as u32 + 1,
                        symbol: Some(name),
                        fix: Some("Use instance properties reset in setUp() instead".into()),
                    });
                }
            }
        }

        issues
    }
}

/// TST-004: Force unwrap in tests without XCTUnwrap.
pub struct ForceUnwrapInTest;

impl AuditRule for ForceUnwrapInTest {
    fn id(&self) -> &str {
        "TST-004"
    }
    fn name(&self) -> &str {
        "force-unwrap-in-test"
    }
    fn category(&self) -> Category {
        Category::Testing
    }
    fn severity(&self) -> Severity {
        Severity::Low
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        if !ctx.file_path.contains("Test") && !ctx.file_path.contains("test") {
            return Vec::new();
        }

        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        let force_unwraps = find_descendants(root, ctx.source, &|node, src| {
            if node.kind() != "force_unwrap_expression" {
                return false;
            }
            // Exclude common safe patterns
            let text = node_text(node, src);
            !text.contains("Bundle.main") && !text.contains("URL(string: \"http")
        });

        if force_unwraps.len() > 5 {
            issues.push(AuditIssue {
                id: format!("{}:{}", self.id(), ctx.file_path),
                category: self.category(),
                severity: self.severity(),
                rule: self.id().to_string(),
                message: format!(
                    "{} force unwraps in test file — use XCTUnwrap or #require for better failure messages",
                    force_unwraps.len()
                ),
                file: ctx.file_path.to_string(),
                line: force_unwraps[0].start_position().row as u32 + 1,
                symbol: None,
                fix: Some("Replace `!` with try XCTUnwrap() or #require()".into()),
            });
        }

        issues
    }
}

/// TST-005: XCTest that should migrate to Swift Testing.
pub struct MigrationOpportunity;

impl AuditRule for MigrationOpportunity {
    fn id(&self) -> &str {
        "TST-005"
    }
    fn name(&self) -> &str {
        "swift-testing-migration"
    }
    fn category(&self) -> Category {
        Category::Testing
    }
    fn severity(&self) -> Severity {
        Severity::Low
    }

    fn check(&self, ctx: &FileContext) -> Vec<AuditIssue> {
        if !ctx.file_path.contains("Test") && !ctx.file_path.contains("test") {
            return Vec::new();
        }

        let root = ctx.tree.root_node();
        let mut issues = Vec::new();

        // Check if file uses XCTest
        let has_xctest = ctx.source.contains("import XCTest") || ctx.source.contains("XCTestCase");
        let has_swift_testing =
            ctx.source.contains("import Testing") || ctx.source.contains("@Test");

        if has_xctest && !has_swift_testing {
            // Count test methods
            let test_funcs = find_descendants(root, ctx.source, &|node, src| {
                if node.kind() != "function_declaration" {
                    return false;
                }
                let text = node_text(node, src);
                text.contains("func test")
            });

            if !test_funcs.is_empty() {
                issues.push(AuditIssue {
                    id: format!("{}:{}", self.id(), ctx.file_path),
                    category: self.category(),
                    severity: self.severity(),
                    rule: self.id().to_string(),
                    message: format!(
                        "{} XCTest methods — consider migrating to Swift Testing (@Test, #expect)",
                        test_funcs.len()
                    ),
                    file: ctx.file_path.to_string(),
                    line: 1,
                    symbol: None,
                    fix: Some("Migrate to Swift Testing: @Test instead of func test*, #expect instead of XCTAssert".into()),
                });
            }
        }

        issues
    }
}

/// All testing rules.
pub fn all_rules() -> Vec<Box<dyn AuditRule>> {
    vec![
        Box::new(SleepInTest),
        Box::new(MissingAssertion),
        Box::new(SharedMutableState),
        Box::new(ForceUnwrapInTest),
        Box::new(MigrationOpportunity),
    ]
}
