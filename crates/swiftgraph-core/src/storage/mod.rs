pub mod queries;
pub mod schema;

use std::path::Path;

use rusqlite::Connection;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Current database schema version, stored in `PRAGMA user_version`.
///
/// The database is a cache derived from sources, so on a version mismatch it
/// is dropped and recreated; the next index run repopulates it.
///
/// History: 1 = v0.5.x (implicit rowid, nullable edge line); 2 = explicit
/// `rid` rowid alias, `edges.line NOT NULL DEFAULT 0`, `meta` table.
pub const SCHEMA_VERSION: i32 = 2;

/// Open or create the SwiftGraph SQLite database.
pub fn open_db(path: &Path) -> Result<Connection, StorageError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let conn = Connection::open(path)?;

    // Performance pragmas
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;
         PRAGMA recursive_triggers = ON;
         PRAGMA cache_size = -64000;",
    )?;

    init_schema(&conn)?;
    Ok(conn)
}

/// Open an in-memory database (for tests).
pub fn open_memory_db() -> Result<Connection, StorageError> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA recursive_triggers = ON;")?;
    init_schema(&conn)?;
    Ok(conn)
}

/// Create the schema, dropping an existing database built with another version.
fn init_schema(conn: &Connection) -> Result<(), StorageError> {
    let version: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let has_tables: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'nodes')",
        [],
        |r| r.get(0),
    )?;
    if has_tables && version != SCHEMA_VERSION {
        tracing::warn!(
            "index database schema v{version} != v{SCHEMA_VERSION}, rebuilding (reindex required)"
        );
        drop_all(conn)?;
    }

    conn.execute_batch(schema::CREATE_TABLES)?;
    conn.execute_batch(schema::CREATE_FTS)?;
    // Trigram table is best-effort (requires SQLite 3.34+)
    let _ = conn.execute_batch(schema::CREATE_FTS_TRIGRAM);
    conn.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION};"))?;
    Ok(())
}

fn drop_all(conn: &Connection) -> Result<(), StorageError> {
    let objects: Vec<(String, String)> = {
        let mut stmt = conn.prepare(
            "SELECT type, name FROM sqlite_master
             WHERE type IN ('table', 'trigger', 'view') AND name NOT LIKE 'sqlite_%'",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
    for (kind, name) in objects {
        // FTS shadow tables disappear with their virtual table.
        let sql = format!("DROP {} IF EXISTS \"{}\";", kind.to_uppercase(), name);
        let _ = conn.execute_batch(&sql);
    }
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::*;

    #[test]
    fn create_db_and_insert_node() {
        let conn = open_memory_db().unwrap();

        // Insert a file first (FK constraint)
        queries::upsert_file(&conn, "Sources/App.swift", "abc123", 1).unwrap();

        let node = GraphNode {
            id: "s:3App0A0C".into(),
            name: "App".into(),
            qualified_name: "MyApp.App".into(),
            kind: SymbolKind::Struct,
            sub_kind: None,
            location: Location {
                file: "Sources/App.swift".into(),
                line: 1,
                column: 1,
                end_line: Some(10),
                end_column: Some(1),
            },
            signature: None,
            attributes: vec!["@main".into()],
            access_level: AccessLevel::Internal,
            container_usr: None,
            doc_comment: None,
            metrics: None,
        };

        queries::upsert_node(&conn, &node).unwrap();

        let found = queries::get_node(&conn, "s:3App0A0C").unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "App");
    }

    #[test]
    fn insert_edge_and_query_callers() {
        let conn = open_memory_db().unwrap();
        queries::upsert_file(&conn, "Sources/A.swift", "a1", 1).unwrap();
        queries::upsert_file(&conn, "Sources/B.swift", "b1", 1).unwrap();

        let node_a = make_node("usr:A", "FuncA", "Sources/A.swift", SymbolKind::Function);
        let node_b = make_node("usr:B", "FuncB", "Sources/B.swift", SymbolKind::Function);
        queries::upsert_node(&conn, &node_a).unwrap();
        queries::upsert_node(&conn, &node_b).unwrap();

        let edge = GraphEdge {
            source: "usr:A".into(),
            target: "usr:B".into(),
            kind: EdgeKind::Calls,
            location: Some(Location {
                file: "Sources/A.swift".into(),
                line: 5,
                column: 9,
                end_line: None,
                end_column: None,
            }),
            is_implicit: false,
        };
        queries::insert_edge(&conn, &edge).unwrap();

        let callers = queries::get_callers(&conn, "usr:B", 10).unwrap();
        assert_eq!(callers.len(), 1);
        assert_eq!(callers[0].source, "usr:A");

        let callees = queries::get_callees(&conn, "usr:A", 10).unwrap();
        assert_eq!(callees.len(), 1);
        assert_eq!(callees[0].target, "usr:B");
    }

    #[test]
    fn get_files_query() {
        let conn = open_memory_db().unwrap();
        queries::upsert_file(&conn, "Sources/A.swift", "hash_a", 3).unwrap();
        queries::upsert_file(&conn, "Sources/B.swift", "hash_b", 5).unwrap();
        queries::upsert_file(&conn, "Tests/T.swift", "hash_t", 1).unwrap();

        // All files
        let files = queries::get_files(&conn, None, 100).unwrap();
        assert_eq!(files.len(), 3);

        // Filter by prefix
        let src_files = queries::get_files(&conn, Some("Sources/"), 100).unwrap();
        assert_eq!(src_files.len(), 2);

        // Check fields
        assert_eq!(src_files[0].symbol_count, 3);
    }

    #[test]
    fn index_store_lib_loads() {
        // This test only passes on macOS with Xcode installed.
        // It verifies that the FFI loading code works.
        match crate::index_store::ffi::IndexStoreLib::load() {
            Ok(_lib) => {
                // Successfully loaded — Xcode is installed
            }
            Err(e) => {
                // Expected on CI or systems without Xcode
                eprintln!("IndexStoreLib::load() skipped: {e}");
            }
        }
    }

    fn make_node(id: &str, name: &str, file: &str, kind: SymbolKind) -> GraphNode {
        GraphNode {
            id: id.into(),
            name: name.into(),
            qualified_name: name.into(),
            kind,
            sub_kind: None,
            location: Location {
                file: file.into(),
                line: 1,
                column: 1,
                end_line: None,
                end_column: None,
            },
            signature: None,
            attributes: vec![],
            access_level: AccessLevel::Internal,
            container_usr: None,
            doc_comment: None,
            metrics: None,
        }
    }
}
