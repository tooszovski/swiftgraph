//! Accessibility rules match the modifier or element itself (not text of
//! enclosing containers) and report its own line and column.

use swiftgraph_audit::engine::AuditIssue;
use swiftgraph_audit::rules::{self, AuditRule, FileContext, ProjectFacts};

fn check(rule: &dyn AuditRule, source: &str) -> Vec<AuditIssue> {
    let mut parser = rules::swift_parser().unwrap();
    let tree = parser.parse(source, None).unwrap();
    let project = ProjectFacts::default();
    let ctx = FileContext {
        file_path: "View.swift",
        source,
        tree: &tree,
        project: &project,
    };
    rule.check(&ctx)
}

fn at(issues: &[AuditIssue]) -> Vec<(u32, Option<u32>)> {
    issues.iter().map(|i| (i.line, i.column)).collect()
}

#[test]
fn a11y001_flags_icon_only_controls_and_state_icons() {
    let rule = rules::accessibility::MissingAccessibilityLabel;
    let source = r#"
struct Row: View {
    let isSelected: Bool
    var body: some View {
        VStack {
            Button(action: close) {
                Image(systemName: "xmark")
                    .font(.system(size: 17))
            }
            HStack {
                Image(systemName: "cloud.fill")
                Text("Digital card")
            }
            if isSelected {
                Image(systemName: "checkmark.circle")
            }
            Button(action: share) {
                Image(systemName: "square.and.arrow.up")
            }
            .accessibilityLabel("Share")
            Image(systemName: "star").onTapGesture { favorite() }
            SwiftUI.Button(action: help, label: { Image("help") })
        }
    }
}

struct Loader {
    func load() async -> Image { let image = Image(uiImage: ui); return image }
    init() { icon = Image(systemName: "a") }
}

#Preview {
    Button(action: {}) { Image("qr_code_example") }
}
"#;
    let lines: Vec<u32> = check(&rule, source).iter().map(|i| i.line).collect();
    assert_eq!(lines, vec![7, 15, 21, 22]);
}

#[test]
fn a11y002_reports_each_fixed_font_once_at_the_modifier() {
    let rule = rules::accessibility::FixedFontSize;
    let source = r#"
struct Row: View {
    @ScaledMetric private var iconSize: CGFloat = 20
    var body: some View {
        HStack {
            Button(action: close) {
                Image(systemName: "xmark")
                    .font(.system(size: 17, weight: .semibold))
                    .foregroundColor(.white)
            }
            Text("A").font(.system(size: 12, relativeTo: .body))
            Image(systemName: "b").font(.system(size: iconSize))
            Text("C").font(.system(size: 14)).font(.system(size: 15))
        }
    }
}
"#;
    assert_eq!(
        at(&check(&rule, source)),
        vec![(8, Some(22)), (13, Some(23)), (13, Some(47))]
    );
}

#[test]
fn a11y003_flags_color_only_status_not_visibility_toggles() {
    let rule = rules::accessibility::ColorOnlyInfo;
    let source = r#"
struct Status: View {
    var body: some View {
        VStack {
            Circle()
                .fill(isOnline ? Color.green : Color.red)
                .frame(width: 8, height: 8)
            if isLoading {
                Image(systemName: "arrow").foregroundColor(.gray)
            }
            Text(status).foregroundColor(isError ? .red : .primary)
        }
    }
}
"#;
    let lines: Vec<u32> = check(&rule, source).iter().map(|i| i.line).collect();
    assert_eq!(lines, vec![6]);
}

#[test]
fn a11y004_checks_the_frame_of_tappable_elements_only() {
    let rule = rules::accessibility::SmallTouchTarget;
    let source = r#"
struct Bar: View {
    var body: some View {
        HStack(alignment: .bottom) {
            if isLoading {
                ProgressView().frame(width: 16, height: 16)
            }
            Button { clear() } label: {
                Assets.clear.image.resizable().frame(width: 24, height: 24)
            }
            Button { next() } label: {
                Image("next").frame(width: 20, height: 20)
            }
            .padding(12)
            Image("dot").frame(width: 10, height: 10).onTapGesture { tap() }
            MainButton(title: "Go").frame(height: 56)
        }
    }
}
"#;
    let lines: Vec<u32> = check(&rule, source).iter().map(|i| i.line).collect();
    assert_eq!(lines, vec![9, 15]);
}
