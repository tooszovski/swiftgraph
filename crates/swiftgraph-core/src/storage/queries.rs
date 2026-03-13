use rusqlite::{params, Connection, Result as SqlResult};

use crate::graph::{
    AccessLevel, EdgeKind, GraphEdge, GraphNode, Location, NodeMetrics, SymbolKind, SymbolSubKind,
};

/// Insert or replace a node in the database.
pub fn upsert_node(conn: &Connection, node: &GraphNode) -> SqlResult<()> {
    conn.execute(
        r#"INSERT OR REPLACE INTO nodes
           (id, name, qualified_name, kind, sub_kind, file, line, col, end_line, end_col,
            signature, attributes, access_level, container_usr, doc_comment,
            lines, complexity, parameter_count)
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)"#,
        params![
            node.id,
            node.name,
            node.qualified_name,
            node.kind.as_str(),
            node.sub_kind.map(|sk| format!("{sk:?}")),
            node.location.file,
            node.location.line,
            node.location.column,
            node.location.end_line,
            node.location.end_column,
            node.signature,
            serde_json::to_string(&node.attributes).unwrap_or_default(),
            format!("{:?}", node.access_level),
            node.container_usr,
            node.doc_comment,
            node.metrics.as_ref().and_then(|m| m.lines),
            node.metrics.as_ref().and_then(|m| m.complexity),
            node.metrics.as_ref().and_then(|m| m.parameter_count),
        ],
    )?;
    Ok(())
}

/// Insert an edge into the database.
pub fn insert_edge(conn: &Connection, edge: &GraphEdge) -> SqlResult<()> {
    conn.execute(
        r#"INSERT OR IGNORE INTO edges (source, target, kind, file, line, col, is_implicit)
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"#,
        params![
            edge.source,
            edge.target,
            edge.kind.as_str(),
            edge.location.as_ref().map(|l| &l.file),
            edge.location.as_ref().map(|l| l.line),
            edge.location.as_ref().map(|l| l.column),
            edge.is_implicit as i32,
        ],
    )?;
    Ok(())
}

/// Upsert a file record.
pub fn upsert_file(conn: &Connection, path: &str, hash: &str, symbol_count: u32) -> SqlResult<()> {
    conn.execute(
        r#"INSERT OR REPLACE INTO files (path, hash, last_indexed, symbol_count)
           VALUES (?1, ?2, datetime('now'), ?3)"#,
        params![path, hash, symbol_count],
    )?;
    Ok(())
}

/// Search nodes by name using FTS5 with BM25 ranking.
/// Name matches are weighted 10x higher than qualified_name (5x) and signature (1x).
pub fn search_nodes(conn: &Connection, query: &str, limit: u32) -> SqlResult<Vec<GraphNode>> {
    let mut stmt = conn.prepare(
        r#"SELECT n.id, n.name, n.qualified_name, n.kind, n.sub_kind,
                  n.file, n.line, n.col, n.end_line, n.end_col,
                  n.signature, n.attributes, n.access_level, n.container_usr,
                  n.doc_comment, n.lines, n.complexity, n.parameter_count
           FROM node_fts f
           JOIN nodes n ON n.rowid = f.rowid
           WHERE node_fts MATCH ?1
           ORDER BY bm25(node_fts, 10.0, 5.0, 1.0)
           LIMIT ?2"#,
    )?;

    let rows = stmt.query_map(params![query, limit], row_to_node)?;
    rows.collect()
}

/// Search nodes by substring using trigram FTS (supports "Delegate" matching "AppDelegate").
pub fn search_nodes_trigram(
    conn: &Connection,
    query: &str,
    limit: u32,
) -> SqlResult<Vec<GraphNode>> {
    let mut stmt = conn.prepare(
        r#"SELECT n.id, n.name, n.qualified_name, n.kind, n.sub_kind,
                  n.file, n.line, n.col, n.end_line, n.end_col,
                  n.signature, n.attributes, n.access_level, n.container_usr,
                  n.doc_comment, n.lines, n.complexity, n.parameter_count
           FROM node_trigram t
           JOIN nodes n ON n.rowid = t.rowid
           WHERE node_trigram MATCH ?1
           LIMIT ?2"#,
    )?;

    let rows = stmt.query_map(params![query, limit], row_to_node)?;
    rows.collect()
}

/// Get a node by its ID (USR).
pub fn get_node(conn: &Connection, id: &str) -> SqlResult<Option<GraphNode>> {
    let mut stmt = conn.prepare(
        r#"SELECT id, name, qualified_name, kind, sub_kind,
                  file, line, col, end_line, end_col,
                  signature, attributes, access_level, container_usr,
                  doc_comment, lines, complexity, parameter_count
           FROM nodes WHERE id = ?1"#,
    )?;

    let mut rows = stmt.query_map(params![id], row_to_node)?;
    match rows.next() {
        Some(row) => Ok(Some(row?)),
        None => Ok(None),
    }
}

/// Find nodes by name (exact or prefix match).
pub fn find_nodes_by_name(
    conn: &Connection,
    name: &str,
    kind: Option<&str>,
    limit: u32,
) -> SqlResult<Vec<GraphNode>> {
    let sql = if kind.is_some() {
        r#"SELECT id, name, qualified_name, kind, sub_kind,
                  file, line, col, end_line, end_col,
                  signature, attributes, access_level, container_usr,
                  doc_comment, lines, complexity, parameter_count
           FROM nodes WHERE name LIKE ?1 AND kind = ?2 LIMIT ?3"#
    } else {
        r#"SELECT id, name, qualified_name, kind, sub_kind,
                  file, line, col, end_line, end_col,
                  signature, attributes, access_level, container_usr,
                  doc_comment, lines, complexity, parameter_count
           FROM nodes WHERE name LIKE ?1 LIMIT ?3"#
    };

    let mut stmt = conn.prepare(sql)?;
    let pattern = format!("%{name}%");

    let rows = if let Some(k) = kind {
        stmt.query_map(params![pattern, k, limit], row_to_node)?
    } else {
        stmt.query_map(params![pattern, "", limit], row_to_node)?
    };
    rows.collect()
}

/// Get edges where source matches (outgoing edges).
pub fn get_callees(conn: &Connection, symbol_id: &str, limit: u32) -> SqlResult<Vec<GraphEdge>> {
    get_edges_by(conn, "source", symbol_id, Some("calls"), limit)
}

/// Get edges where target matches (incoming edges).
pub fn get_callers(conn: &Connection, symbol_id: &str, limit: u32) -> SqlResult<Vec<GraphEdge>> {
    get_edges_by(conn, "target", symbol_id, Some("calls"), limit)
}

/// Get all references to a symbol.
pub fn get_references(conn: &Connection, symbol_id: &str, limit: u32) -> SqlResult<Vec<GraphEdge>> {
    get_edges_by(conn, "target", symbol_id, None, limit)
}

/// Get type hierarchy edges (conformsTo, inheritsFrom).
pub fn get_subtypes(conn: &Connection, symbol_id: &str, limit: u32) -> SqlResult<Vec<GraphEdge>> {
    let mut stmt = conn.prepare(
        r#"SELECT source, target, kind, file, line, col, is_implicit
           FROM edges
           WHERE target = ?1 AND kind IN ('conformsTo', 'inheritsFrom')
           LIMIT ?2"#,
    )?;
    let rows = stmt.query_map(params![symbol_id, limit], row_to_edge)?;
    rows.collect()
}

/// Get supertypes of a symbol.
pub fn get_supertypes(conn: &Connection, symbol_id: &str, limit: u32) -> SqlResult<Vec<GraphEdge>> {
    let mut stmt = conn.prepare(
        r#"SELECT source, target, kind, file, line, col, is_implicit
           FROM edges
           WHERE source = ?1 AND kind IN ('conformsTo', 'inheritsFrom')
           LIMIT ?2"#,
    )?;
    let rows = stmt.query_map(params![symbol_id, limit], row_to_edge)?;
    rows.collect()
}

/// Get graph statistics.
pub fn get_stats(conn: &Connection) -> SqlResult<GraphStats> {
    let file_count: u32 = conn.query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))?;
    let node_count: u32 = conn.query_row("SELECT COUNT(*) FROM nodes", [], |r| r.get(0))?;
    let edge_count: u32 = conn.query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))?;

    Ok(GraphStats {
        file_count,
        node_count,
        edge_count,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphStats {
    pub file_count: u32,
    pub node_count: u32,
    pub edge_count: u32,
}

use serde::{Deserialize, Serialize};

// --- internal helpers ---

fn get_edges_by(
    conn: &Connection,
    field: &str,
    symbol_id: &str,
    kind_filter: Option<&str>,
    limit: u32,
) -> SqlResult<Vec<GraphEdge>> {
    let sql = if let Some(kind) = kind_filter {
        format!(
            "SELECT source, target, kind, file, line, col, is_implicit FROM edges WHERE {field} = ?1 AND kind = '{kind}' LIMIT ?2"
        )
    } else {
        format!(
            "SELECT source, target, kind, file, line, col, is_implicit FROM edges WHERE {field} = ?1 LIMIT ?2"
        )
    };

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![symbol_id, limit], row_to_edge)?;
    rows.collect()
}

fn row_to_node(row: &rusqlite::Row) -> SqlResult<GraphNode> {
    let kind_str: String = row.get(3)?;
    let attributes_json: String = row.get::<_, Option<String>>(11)?.unwrap_or_default();

    Ok(GraphNode {
        id: row.get(0)?,
        name: row.get(1)?,
        qualified_name: row.get(2)?,
        kind: parse_symbol_kind(&kind_str),
        sub_kind: row
            .get::<_, Option<String>>(4)?
            .as_deref()
            .and_then(parse_sub_kind),
        location: Location {
            file: row.get(5)?,
            line: row.get(6)?,
            column: row.get(7)?,
            end_line: row.get(8)?,
            end_column: row.get(9)?,
        },
        signature: row.get(10)?,
        attributes: serde_json::from_str(&attributes_json).unwrap_or_default(),
        access_level: row
            .get::<_, Option<String>>(12)?
            .as_deref()
            .map(parse_access_level)
            .unwrap_or_default(),
        container_usr: row.get(13)?,
        doc_comment: row.get(14)?,
        metrics: Some(NodeMetrics {
            lines: row.get(15)?,
            complexity: row.get(16)?,
            parameter_count: row.get(17)?,
        }),
    })
}

fn row_to_edge(row: &rusqlite::Row) -> SqlResult<GraphEdge> {
    let kind_str: String = row.get(2)?;
    let file: Option<String> = row.get(3)?;
    let line: Option<u32> = row.get(4)?;
    let col: Option<u32> = row.get(5)?;

    Ok(GraphEdge {
        source: row.get(0)?,
        target: row.get(1)?,
        kind: parse_edge_kind(&kind_str),
        location: file.map(|f| Location {
            file: f,
            line: line.unwrap_or(0),
            column: col.unwrap_or(0),
            end_line: None,
            end_column: None,
        }),
        is_implicit: row.get::<_, i32>(6)? != 0,
    })
}

fn parse_symbol_kind(s: &str) -> SymbolKind {
    match s {
        "class" => SymbolKind::Class,
        "struct" => SymbolKind::Struct,
        "enum" => SymbolKind::Enum,
        "protocol" => SymbolKind::Protocol,
        "method" => SymbolKind::Method,
        "property" => SymbolKind::Property,
        "function" => SymbolKind::Function,
        "typeAlias" => SymbolKind::TypeAlias,
        "extension" => SymbolKind::Extension,
        "enumCase" => SymbolKind::EnumCase,
        "macro" => SymbolKind::Macro,
        "associatedType" => SymbolKind::AssociatedType,
        "module" => SymbolKind::Module,
        "import" => SymbolKind::Import,
        "file" => SymbolKind::File,
        _ => SymbolKind::Function, // fallback
    }
}

/// File info from the files table.
#[derive(Debug, serde::Serialize)]
pub struct FileInfo {
    pub path: String,
    pub hash: String,
    pub last_indexed: String,
    pub symbol_count: u32,
}

/// List indexed files with optional path filter.
pub fn get_files(
    conn: &Connection,
    path_prefix: Option<&str>,
    limit: u32,
) -> SqlResult<Vec<FileInfo>> {
    let (sql, params_vec): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(prefix) =
        path_prefix
    {
        (
            "SELECT path, hash, last_indexed, symbol_count FROM files WHERE path LIKE ?1 ORDER BY path LIMIT ?2",
            vec![Box::new(format!("{prefix}%")), Box::new(limit)],
        )
    } else {
        (
            "SELECT path, hash, last_indexed, symbol_count FROM files ORDER BY path LIMIT ?1",
            vec![Box::new(limit)],
        )
    };

    let mut stmt = conn.prepare(sql)?;
    let params_refs: Vec<&dyn rusqlite::types::ToSql> =
        params_vec.iter().map(|p| p.as_ref()).collect();
    let rows = stmt.query_map(params_refs.as_slice(), |row| {
        Ok(FileInfo {
            path: row.get(0)?,
            hash: row.get(1)?,
            last_indexed: row.get(2)?,
            symbol_count: row.get(3)?,
        })
    })?;

    let mut files = Vec::new();
    for row in rows {
        files.push(row?);
    }
    Ok(files)
}

/// Get all extensions of a type (edges with kind = 'extendsType' targeting the symbol).
pub fn get_extensions(conn: &Connection, symbol_id: &str, limit: u32) -> SqlResult<Vec<GraphEdge>> {
    let mut stmt = conn.prepare(
        r#"SELECT source, target, kind, file, line, col, is_implicit
           FROM edges WHERE target = ?1 AND kind = 'extendsType'
           LIMIT ?2"#,
    )?;
    let rows = stmt.query_map(params![symbol_id, limit], row_to_edge)?;
    rows.collect()
}

/// Get conformances for a symbol (edges with kind = 'conformsTo').
/// If `direction` is "conformedBy", find types conforming to the symbol (as protocol).
/// If "conforms", find protocols the symbol conforms to.
pub fn get_conformances(
    conn: &Connection,
    symbol_id: &str,
    direction: &str,
    limit: u32,
) -> SqlResult<Vec<GraphEdge>> {
    let sql = if direction == "conformedBy" {
        "SELECT source, target, kind, file, line, col, is_implicit FROM edges WHERE target = ?1 AND kind = 'conformsTo' LIMIT ?2"
    } else {
        "SELECT source, target, kind, file, line, col, is_implicit FROM edges WHERE source = ?1 AND kind = 'conformsTo' LIMIT ?2"
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params![symbol_id, limit], row_to_edge)?;
    rows.collect()
}

/// Get all nodes in a given file.
pub fn get_nodes_in_file(conn: &Connection, file_path: &str) -> SqlResult<Vec<GraphNode>> {
    let mut stmt = conn.prepare(
        r#"SELECT id, name, qualified_name, kind, sub_kind,
                  file, line, col, end_line, end_col,
                  signature, attributes, access_level, container_usr,
                  doc_comment, lines, complexity, parameter_count
           FROM nodes WHERE file = ?1
           ORDER BY line"#,
    )?;
    let rows = stmt.query_map(params![file_path], row_to_node)?;
    rows.collect()
}

/// Get all incoming edges to a symbol (any kind).
pub fn get_all_incoming(
    conn: &Connection,
    symbol_id: &str,
    limit: u32,
) -> SqlResult<Vec<GraphEdge>> {
    get_edges_by(conn, "target", symbol_id, None, limit)
}

/// Get all outgoing edges from a symbol (any kind).
pub fn get_all_outgoing(
    conn: &Connection,
    symbol_id: &str,
    limit: u32,
) -> SqlResult<Vec<GraphEdge>> {
    get_edges_by(conn, "source", symbol_id, None, limit)
}

/// Count incoming edges to a symbol.
pub fn count_incoming(conn: &Connection, symbol_id: &str) -> SqlResult<u32> {
    conn.query_row(
        "SELECT COUNT(*) FROM edges WHERE target = ?1",
        params![symbol_id],
        |r| r.get(0),
    )
}

/// Count outgoing edges from a symbol.
pub fn count_outgoing(conn: &Connection, symbol_id: &str) -> SqlResult<u32> {
    conn.query_row(
        "SELECT COUNT(*) FROM edges WHERE source = ?1",
        params![symbol_id],
        |r| r.get(0),
    )
}

/// Find nodes by name pattern (LIKE).
pub fn find_nodes_by_name_pattern(
    conn: &Connection,
    pattern: &str,
    limit: u32,
) -> SqlResult<Vec<GraphNode>> {
    let mut stmt = conn.prepare(
        r#"SELECT id, name, qualified_name, kind, sub_kind,
                  file, line, col, end_line, end_col,
                  signature, attributes, access_level, container_usr,
                  doc_comment, lines, complexity, parameter_count
           FROM nodes WHERE name LIKE ?1
           LIMIT ?2"#,
    )?;
    let rows = stmt.query_map(params![format!("%{pattern}%"), limit], row_to_node)?;
    rows.collect()
}

/// Get distinct files referenced by edges of given nodes.
pub fn get_affected_files(conn: &Connection, node_ids: &[String]) -> SqlResult<Vec<String>> {
    if node_ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders: String = node_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT DISTINCT n.file FROM nodes n \
         INNER JOIN edges e ON (e.source = n.id OR e.target = n.id) \
         WHERE e.source IN ({placeholders}) OR e.target IN ({placeholders})"
    );
    let mut stmt = conn.prepare(&sql)?;
    let params: Vec<&dyn rusqlite::types::ToSql> = node_ids
        .iter()
        .map(|s| s as &dyn rusqlite::types::ToSql)
        .collect();
    // Duplicate params for both IN clauses
    let mut all_params = params.clone();
    all_params.extend(params);
    let rows = stmt.query_map(all_params.as_slice(), |row| row.get::<_, String>(0))?;
    rows.collect()
}

/// Get all nodes (up to limit).
pub fn get_all_nodes(conn: &Connection, limit: u32) -> SqlResult<Vec<GraphNode>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, qualified_name, kind, sub_kind, file, line, col, end_line, end_col, \
         signature, attributes, access_level, container_usr, doc_comment, \
         lines, complexity, parameter_count \
         FROM nodes LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit], row_to_node)?;
    rows.collect()
}

/// Get nodes filtered by file path prefix.
pub fn get_nodes_by_path_prefix(
    conn: &Connection,
    prefix: &str,
    limit: u32,
) -> SqlResult<Vec<GraphNode>> {
    let pattern = format!("{prefix}%");
    let mut stmt = conn.prepare(
        "SELECT id, name, qualified_name, kind, sub_kind, file, line, col, end_line, end_col, \
         signature, attributes, access_level, container_usr, doc_comment, \
         lines, complexity, parameter_count \
         FROM nodes WHERE file LIKE ?1 LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![pattern, limit], row_to_node)?;
    rows.collect()
}

/// Get cross-file edges: returns (source_file, target_file) pairs.
pub fn get_cross_file_edges(
    conn: &Connection,
    path_filter: Option<&str>,
    limit: u32,
) -> SqlResult<Vec<(String, String)>> {
    let (sql, params_vec): (String, Vec<Box<dyn rusqlite::types::ToSql>>) =
        if let Some(prefix) = path_filter {
            let pattern = format!("{prefix}%");
            (
                "SELECT DISTINCT n1.file, n2.file \
             FROM edges e \
             JOIN nodes n1 ON e.source = n1.id \
             JOIN nodes n2 ON e.target = n2.id \
             WHERE n1.file LIKE ?1 AND n2.file LIKE ?1 AND n1.file != n2.file \
             LIMIT ?2"
                    .to_string(),
                vec![
                    Box::new(pattern) as Box<dyn rusqlite::types::ToSql>,
                    Box::new(limit),
                ],
            )
        } else {
            (
                "SELECT DISTINCT n1.file, n2.file \
             FROM edges e \
             JOIN nodes n1 ON e.source = n1.id \
             JOIN nodes n2 ON e.target = n2.id \
             WHERE n1.file != n2.file \
             LIMIT ?1"
                    .to_string(),
                vec![Box::new(limit) as Box<dyn rusqlite::types::ToSql>],
            )
        };
    let mut stmt = conn.prepare(&sql)?;
    let params_refs: Vec<&dyn rusqlite::types::ToSql> =
        params_vec.iter().map(|p| p.as_ref()).collect();
    let rows = stmt.query_map(params_refs.as_slice(), |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    rows.collect()
}

/// Parse a `SymbolSubKind` from its `Debug` string representation.
fn parse_sub_kind(s: &str) -> Option<SymbolSubKind> {
    match s {
        "Getter" => Some(SymbolSubKind::Getter),
        "Setter" => Some(SymbolSubKind::Setter),
        "Subscript" => Some(SymbolSubKind::Subscript),
        "Initializer" => Some(SymbolSubKind::Initializer),
        "Deinitializer" => Some(SymbolSubKind::Deinitializer),
        _ => None,
    }
}

/// Parse an `AccessLevel` from its `Debug` string representation.
fn parse_access_level(s: &str) -> AccessLevel {
    match s {
        "Open" => AccessLevel::Open,
        "Public" => AccessLevel::Public,
        "Package" => AccessLevel::Package,
        "Internal" => AccessLevel::Internal,
        "FilePrivate" => AccessLevel::FilePrivate,
        "Private" => AccessLevel::Private,
        _ => AccessLevel::Internal, // fallback
    }
}

fn parse_edge_kind(s: &str) -> EdgeKind {
    match s {
        "calls" => EdgeKind::Calls,
        "conformsTo" => EdgeKind::ConformsTo,
        "inheritsFrom" => EdgeKind::InheritsFrom,
        "extendsType" => EdgeKind::ExtendsType,
        "overrides" => EdgeKind::Overrides,
        "implementsRequirement" => EdgeKind::ImplementsRequirement,
        "references" => EdgeKind::References,
        "mutates" => EdgeKind::Mutates,
        "imports" => EdgeKind::Imports,
        "dependsOn" => EdgeKind::DependsOn,
        "contains" => EdgeKind::Contains,
        "returns" => EdgeKind::Returns,
        "parameterOf" => EdgeKind::ParameterOf,
        "propertyType" => EdgeKind::PropertyType,
        _ => EdgeKind::References, // fallback
    }
}
