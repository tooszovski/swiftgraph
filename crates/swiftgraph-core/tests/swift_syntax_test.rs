//! swift-syntax protocol and enrichment, driven by a fake parser script so the
//! tests run without a Swift toolchain. An optional test at the end exercises
//! the real `swiftgraph-parser` when it has been built.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use swiftgraph_core::pipeline::{self, SwiftSyntaxMode};
use swiftgraph_core::storage::{self, queries};
use swiftgraph_core::swift_syntax::{SwiftSyntaxError, SwiftSyntaxParser, PROTOCOL_VERSION};

/// Write an executable fake parser. `version_json` answers `--version`;
/// `per_file` is a shell snippet printing one JSON line for "$f".
fn fake_parser(dir: &Path, version_json: &str, per_file: &str) -> PathBuf {
    let path = dir.join("fake-parser");
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo '{version_json}'; exit 0; fi\n\
         if [ \"$1\" = \"--stdin\" ]; then while IFS= read -r f; do {per_file}; done; exit 0; fi\n\
         exit 2\n"
    );
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn version(protocol: u32) -> String {
    format!(r#"{{"name":"swiftgraph-parser","version":"test","protocol":{protocol}}}"#)
}

#[test]
fn handshake_accepts_matching_protocol() {
    let dir = tempfile::tempdir().unwrap();
    let p = fake_parser(dir.path(), &version(PROTOCOL_VERSION), "true");
    let parser = SwiftSyntaxParser::probe(&p).unwrap();
    assert_eq!(parser.info.protocol, PROTOCOL_VERSION);
    assert!(parser.describe().contains("protocol 2"));
}

#[test]
fn handshake_rejects_other_protocol_and_old_parsers() {
    let dir = tempfile::tempdir().unwrap();
    let p = fake_parser(dir.path(), &version(1), "true");
    assert!(matches!(
        SwiftSyntaxParser::probe(&p),
        Err(SwiftSyntaxError::ProtocolMismatch { found: 1, .. })
    ));

    // Phase A parsers treat --version as a file name and fail.
    let old = dir.path().join("old-parser");
    std::fs::write(&old, "#!/bin/sh\necho 'Error reading file' >&2\nexit 1\n").unwrap();
    std::fs::set_permissions(&old, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(SwiftSyntaxParser::probe(&old).is_err());
}

#[test]
fn batch_returns_results_in_input_order_with_failures() {
    let dir = tempfile::tempdir().unwrap();
    let per_file = r#"case "$f" in *bad*) echo "{\"file\":\"$f\",\"error\":\"unreadable\"}";; *) echo "{\"version\":2,\"file\":\"$f\",\"declarations\":[],\"imports\":[]}";; esac"#;
    let p = fake_parser(dir.path(), &version(PROTOCOL_VERSION), per_file);
    let parser = SwiftSyntaxParser::probe(&p).unwrap();
    let files: Vec<PathBuf> = ["/x/a.swift", "/x/bad.swift", "/x/c.swift"]
        .iter()
        .map(PathBuf::from)
        .collect();
    let out = parser.parse_batch(&files).unwrap();
    assert_eq!(out.len(), 3);
    assert_eq!(out[0].as_ref().unwrap().file, "/x/a.swift");
    assert!(out[1].as_ref().unwrap_err().contains("unreadable"));
    assert_eq!(out[2].as_ref().unwrap().file, "/x/c.swift");
}

#[test]
fn enrichment_updates_members_and_imports_in_one_run() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("A.swift"),
        "import Foundation\n\nstruct Foo {\n    func bar() {}\n}\n",
    )
    .unwrap();

    let json = r#"{"version":2,"file":"F","imports":[{"name":"Foundation","line":1,"attributes":["@testable"]},{"name":"Combine","line":2,"attributes":[]}],"declarations":[{"name":"Foo","kind":"struct","line":3,"endLine":5,"attributes":["@MainActor"],"accessLevel":"public","signature":"struct Foo","docComment":"/// A foo.","members":[{"name":"bar","kind":"method","line":4,"endLine":4,"attributes":["@discardableResult"],"accessLevel":"private","signature":"func bar()","docComment":"/// Bars.","members":null}]}]}"#;
    let per_file = format!("echo '{json}'");
    let p = fake_parser(dir.path(), &version(PROTOCOL_VERSION), &per_file);
    let parser = SwiftSyntaxParser::probe(&p).unwrap();

    let db = dir.path().join("db.sqlite");
    let result = pipeline::index_directory_with_options(
        &db,
        &root,
        false,
        None,
        &SwiftSyntaxMode::Parser(parser),
    )
    .unwrap();
    assert!(result.nodes_enriched >= 3, "{}", result.nodes_enriched);

    let conn = storage::open_db(&db).unwrap();
    let find = |name: &str| {
        queries::find_nodes_by_name(&conn, name, None, 10)
            .unwrap()
            .into_iter()
            .find(|n| n.name == name)
            .unwrap_or_else(|| panic!("{name} missing"))
    };
    let bar = find("bar");
    assert_eq!(bar.doc_comment.as_deref(), Some("/// Bars."));
    assert_eq!(bar.attributes, vec!["@discardableResult".to_string()]);
    assert_eq!(
        bar.access_level,
        swiftgraph_core::graph::AccessLevel::Private
    );
    let foo = find("Foo");
    assert_eq!(foo.attributes, vec!["@MainActor".to_string()]);
    assert_eq!(foo.doc_comment.as_deref(), Some("/// A foo."));
    assert_eq!(find("Foundation").attributes, vec!["@testable".to_string()]);
    assert_eq!(
        find("Combine").kind,
        swiftgraph_core::graph::SymbolKind::Import
    );
    conn.execute_batch("INSERT INTO node_fts(node_fts, rank) VALUES('integrity-check', 1);")
        .unwrap();
}

#[test]
fn broken_parser_never_fails_indexing() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("A.swift"), "struct Foo {}\n").unwrap();
    let p = fake_parser(dir.path(), &version(PROTOCOL_VERSION), "echo garbage");
    let parser = SwiftSyntaxParser::probe(&p).unwrap();
    let result = pipeline::index_directory_with_options(
        &dir.path().join("db.sqlite"),
        &root,
        false,
        None,
        &SwiftSyntaxMode::Parser(parser),
    )
    .unwrap();
    assert!(result.nodes_added > 0);
    assert_eq!(result.nodes_enriched, 0);
}

/// Uses the real parser when `SWIFTGRAPH_PARSER_PATH` is set or the workspace
/// release build exists; skipped otherwise.
#[test]
fn real_parser_speaks_the_protocol() {
    let candidate = std::env::var_os("SWIFTGRAPH_PARSER_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../swiftgraph-parser/.build/release/swiftgraph-parser")
        });
    if !candidate.is_file() {
        eprintln!("skipped: swiftgraph-parser not built");
        return;
    }
    let parser = SwiftSyntaxParser::probe(&candidate).expect("handshake");
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/spm/Sources/Fixture/Store.swift");
    let out = parser.parse_batch(&[fixture]).unwrap();
    let result = out[0].as_ref().unwrap();
    let store = result
        .declarations
        .iter()
        .find(|d| d.name == "MemoryStore")
        .unwrap();
    let members: Vec<&str> = store
        .members
        .as_ref()
        .unwrap()
        .iter()
        .map(|m| m.kind.as_str())
        .collect();
    assert!(members.contains(&"initializer"), "{members:?}");
    assert!(members.contains(&"method"));
    assert!(members.contains(&"property"));
}
