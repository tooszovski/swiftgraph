/// SQL statements for creating the SwiftGraph database schema.
pub const CREATE_TABLES: &str = r#"
CREATE TABLE IF NOT EXISTS files (
    path        TEXT PRIMARY KEY,
    language    TEXT NOT NULL DEFAULT 'swift',
    hash        TEXT NOT NULL,
    last_indexed TEXT NOT NULL,
    symbol_count INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS nodes (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL,
    qualified_name  TEXT NOT NULL,
    kind            TEXT NOT NULL,
    sub_kind        TEXT,
    file            TEXT NOT NULL,
    line            INTEGER NOT NULL,
    col             INTEGER NOT NULL,
    end_line        INTEGER,
    end_col         INTEGER,
    signature       TEXT,
    attributes      TEXT,  -- JSON array
    access_level    TEXT NOT NULL DEFAULT 'internal',
    container_usr   TEXT,
    doc_comment     TEXT,
    lines           INTEGER,
    complexity      INTEGER,
    parameter_count INTEGER,
    FOREIGN KEY (file) REFERENCES files(path)
);

CREATE TABLE IF NOT EXISTS edges (
    source      TEXT NOT NULL,
    target      TEXT NOT NULL,
    kind        TEXT NOT NULL,
    file        TEXT,
    line        INTEGER,
    col         INTEGER,
    is_implicit INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (source, target, kind, line)
    -- No FK on source/target: targets may reference SDK symbols not in our index
);

CREATE TABLE IF NOT EXISTS diagnostics (
    id          TEXT NOT NULL,
    category    TEXT NOT NULL,
    severity    TEXT NOT NULL,
    rule        TEXT NOT NULL,
    message     TEXT NOT NULL,
    file        TEXT NOT NULL,
    line        INTEGER NOT NULL,
    symbol      TEXT,
    fix         TEXT,
    PRIMARY KEY (file, id, line)
);

-- Indexes for common query patterns
CREATE INDEX IF NOT EXISTS idx_nodes_name ON nodes(name);
CREATE INDEX IF NOT EXISTS idx_nodes_kind ON nodes(kind);
CREATE INDEX IF NOT EXISTS idx_nodes_file ON nodes(file);
CREATE INDEX IF NOT EXISTS idx_nodes_container ON nodes(container_usr);
CREATE INDEX IF NOT EXISTS idx_edges_source ON edges(source);
CREATE INDEX IF NOT EXISTS idx_edges_target ON edges(target);
CREATE INDEX IF NOT EXISTS idx_edges_kind ON edges(kind);
CREATE INDEX IF NOT EXISTS idx_diagnostics_file ON diagnostics(file);
CREATE INDEX IF NOT EXISTS idx_diagnostics_category ON diagnostics(category);
"#;

pub const CREATE_FTS: &str = r#"
CREATE VIRTUAL TABLE IF NOT EXISTS node_fts USING fts5(
    name,
    qualified_name,
    signature,
    content=nodes,
    content_rowid=rowid
);

-- Triggers to keep FTS in sync
CREATE TRIGGER IF NOT EXISTS nodes_ai AFTER INSERT ON nodes BEGIN
    INSERT INTO node_fts(rowid, name, qualified_name, signature)
    VALUES (new.rowid, new.name, new.qualified_name, new.signature);
END;

CREATE TRIGGER IF NOT EXISTS nodes_ad AFTER DELETE ON nodes BEGIN
    INSERT INTO node_fts(node_fts, rowid, name, qualified_name, signature)
    VALUES ('delete', old.rowid, old.name, old.qualified_name, old.signature);
END;

CREATE TRIGGER IF NOT EXISTS nodes_au AFTER UPDATE ON nodes BEGIN
    INSERT INTO node_fts(node_fts, rowid, name, qualified_name, signature)
    VALUES ('delete', old.rowid, old.name, old.qualified_name, old.signature);
    INSERT INTO node_fts(rowid, name, qualified_name, signature)
    VALUES (new.rowid, new.name, new.qualified_name, new.signature);
END;
"#;

/// Trigram FTS table for substring matching (e.g., "Delegate" matches "AppDelegate").
pub const CREATE_FTS_TRIGRAM: &str = r#"
CREATE VIRTUAL TABLE IF NOT EXISTS node_trigram USING fts5(
    name,
    content=nodes,
    content_rowid=rowid,
    tokenize='trigram'
);

CREATE TRIGGER IF NOT EXISTS nodes_tri_ai AFTER INSERT ON nodes BEGIN
    INSERT INTO node_trigram(rowid, name)
    VALUES (new.rowid, new.name);
END;

CREATE TRIGGER IF NOT EXISTS nodes_tri_ad AFTER DELETE ON nodes BEGIN
    INSERT INTO node_trigram(node_trigram, rowid, name)
    VALUES ('delete', old.rowid, old.name);
END;

CREATE TRIGGER IF NOT EXISTS nodes_tri_au AFTER UPDATE ON nodes BEGIN
    INSERT INTO node_trigram(node_trigram, rowid, name)
    VALUES ('delete', old.rowid, old.name);
    INSERT INTO node_trigram(rowid, name)
    VALUES (new.rowid, new.name);
END;
"#;
