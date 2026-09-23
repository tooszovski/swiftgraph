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
    -- Explicit rowid alias: FTS5 external content keys on it and VACUUM must not renumber it
    rid             INTEGER PRIMARY KEY,
    id              TEXT NOT NULL UNIQUE,
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

-- Interned strings (symbol IDs, file paths): edges and call sites refer
-- to them by integer key.
CREATE TABLE IF NOT EXISTS ids (
    key INTEGER PRIMARY KEY,
    id  TEXT NOT NULL UNIQUE
);

CREATE TABLE IF NOT EXISTS edge_rows (
    source      INTEGER NOT NULL,              -- ids.key
    target      INTEGER NOT NULL,              -- ids.key
    kind        TEXT NOT NULL,
    file        INTEGER,                       -- ids.key of the path
    line        INTEGER NOT NULL DEFAULT 0,    -- 0 = no location (e.g. tree-sitter containment)
    col         INTEGER,
    is_implicit INTEGER NOT NULL DEFAULT 0,
    ambiguous   INTEGER NOT NULL DEFAULT 0,    -- 1 = one of several plausible call targets
    PRIMARY KEY (source, target, kind, line)
    -- No FK on source/target: targets may reference SDK symbols not in our index
) WITHOUT ROWID;

-- The API view: string IDs as before, integer keys underneath.
CREATE VIEW IF NOT EXISTS edges AS
    SELECT s.id AS source, t.id AS target, e.kind, f.id AS file, e.line, e.col,
           e.is_implicit, e.ambiguous, e.source AS source_key, e.target AS target_key
    FROM edge_rows e
    JOIN ids s ON s.key = e.source
    JOIN ids t ON t.key = e.target
    LEFT JOIN ids f ON f.key = e.file;

CREATE TRIGGER IF NOT EXISTS edges_insert INSTEAD OF INSERT ON edges BEGIN
    INSERT OR IGNORE INTO ids (id) VALUES (NEW.source);
    INSERT OR IGNORE INTO ids (id) VALUES (NEW.target);
    INSERT OR IGNORE INTO ids (id) SELECT NEW.file WHERE NEW.file IS NOT NULL;
    INSERT OR IGNORE INTO edge_rows (source, target, kind, file, line, col, is_implicit, ambiguous)
    VALUES ((SELECT key FROM ids WHERE id = NEW.source),
            (SELECT key FROM ids WHERE id = NEW.target),
            NEW.kind,
            (SELECT key FROM ids WHERE id = NEW.file),
            COALESCE(NEW.line, 0), NEW.col, COALESCE(NEW.is_implicit, 0), COALESCE(NEW.ambiguous, 0));
END;

CREATE TRIGGER IF NOT EXISTS edges_delete INSTEAD OF DELETE ON edges BEGIN
    DELETE FROM edge_rows
    WHERE source = OLD.source_key AND target = OLD.target_key AND kind = OLD.kind AND line = OLD.line;
END;


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

-- Project names referenced in a file without a confident edge: calls with an
-- unknown receiver or too many candidates, member reads, type references.
-- Dead-code treats project symbols with such names as possibly used.
CREATE TABLE IF NOT EXISTS name_refs (
    file TEXT NOT NULL,
    name TEXT NOT NULL,
    PRIMARY KEY (file, name)
) WITHOUT ROWID;

-- Declared property types per type, for typing receivers across files.
CREATE TABLE IF NOT EXISTS member_types (
    file      TEXT NOT NULL,
    owner     TEXT NOT NULL,
    member    TEXT NOT NULL,
    type_name TEXT NOT NULL,
    PRIMARY KEY (file, owner, member)
) WITHOUT ROWID;

-- Tree-sitter call sites (JSON), resolved again on incremental runs when a
-- declaration with their name is added, changed or removed.
CREATE TABLE IF NOT EXISTS call_sites (
    file   INTEGER NOT NULL,   -- ids.key of the path
    caller INTEGER NOT NULL,   -- ids.key of the calling declaration
    name   TEXT NOT NULL,
    line   INTEGER NOT NULL,
    site   TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_call_sites_name ON call_sites(name);
CREATE INDEX IF NOT EXISTS idx_call_sites_file ON call_sites(file);

CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Indexes for common query patterns
CREATE INDEX IF NOT EXISTS idx_nodes_name ON nodes(name);
CREATE INDEX IF NOT EXISTS idx_nodes_kind ON nodes(kind);
CREATE INDEX IF NOT EXISTS idx_nodes_file ON nodes(file);
CREATE INDEX IF NOT EXISTS idx_nodes_container ON nodes(container_usr);
CREATE INDEX IF NOT EXISTS idx_edge_rows_target ON edge_rows(target);
CREATE INDEX IF NOT EXISTS idx_edge_rows_file ON edge_rows(file);
CREATE INDEX IF NOT EXISTS idx_edge_rows_kind ON edge_rows(kind);
CREATE INDEX IF NOT EXISTS idx_diagnostics_file ON diagnostics(file);
CREATE INDEX IF NOT EXISTS idx_diagnostics_category ON diagnostics(category);
"#;

pub const CREATE_FTS: &str = r#"
CREATE VIRTUAL TABLE IF NOT EXISTS node_fts USING fts5(
    name,
    qualified_name,
    signature,
    content=nodes,
    content_rowid=rid
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
    content_rowid=rid,
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
