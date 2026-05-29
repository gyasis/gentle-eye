//! SQLite database initialization + schema migrations.
//!
//! Implements the idempotent schema migration runner (T030). The schema matches
//! the columns read by [`crate::storage::metadata`] `FromRow` impls exactly, so
//! the row types round-trip without drift.
//!
//! Authored 2026-05-28 from the metadata-row contract — the recovered source was
//! an empty stub (only `init_database` opening a bare connection).

use crate::contracts::errors::StorageError;
use rusqlite::Connection;
use std::path::Path;

/// DDL applied on every open. `IF NOT EXISTS` makes it idempotent.
const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS recordings (
    id              TEXT PRIMARY KEY,
    status          TEXT NOT NULL,
    start_time      TEXT NOT NULL,
    end_time        TEXT,
    duration_ms     INTEGER,
    file_path       TEXT,
    fps             INTEGER NOT NULL DEFAULT 1,
    width           INTEGER NOT NULL DEFAULT 0,
    height          INTEGER NOT NULL DEFAULT 0,
    file_size_bytes INTEGER,
    error_message   TEXT,
    display_name    TEXT,
    encoder_mode    TEXT NOT NULL DEFAULT 'in_memory_pipe',
    max_duration_seconds INTEGER,
    output_dir      TEXT,
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);

CREATE TABLE IF NOT EXISTS analysis_requests (
    id              TEXT PRIMARY KEY,
    recording_id    TEXT,
    video_path      TEXT NOT NULL,
    prompt          TEXT NOT NULL,
    provider        TEXT NOT NULL,
    timestamp       TEXT NOT NULL,
    timeframe_start REAL,
    timeframe_end   REAL,
    FOREIGN KEY (recording_id) REFERENCES recordings(id) ON DELETE SET NULL
);

CREATE TABLE IF NOT EXISTS analysis_results (
    id                 TEXT PRIMARY KEY,
    request_id         TEXT NOT NULL,
    analysis_text      TEXT NOT NULL,
    model_used         TEXT NOT NULL,
    token_count        INTEGER,
    processing_time_ms INTEGER NOT NULL,
    timestamp          TEXT NOT NULL,
    success            INTEGER NOT NULL,
    error_message      TEXT,
    FOREIGN KEY (request_id) REFERENCES analysis_requests(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS timeline_entries (
    id              TEXT PRIMARY KEY,
    recording_id    TEXT NOT NULL,
    start_time      TEXT NOT NULL,
    end_time        TEXT NOT NULL,
    category        TEXT NOT NULL,
    app             TEXT NOT NULL,
    activity        TEXT NOT NULL,
    summary         TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_recordings_status ON recordings(status);
CREATE INDEX IF NOT EXISTS idx_recordings_start_time ON recordings(start_time);
CREATE INDEX IF NOT EXISTS idx_results_request ON analysis_results(request_id);
CREATE INDEX IF NOT EXISTS idx_timeline_start ON timeline_entries(start_time);
"#;

/// Open (or create) the database at `db_path` and apply the schema.
pub fn init_database(db_path: &Path) -> Result<Connection, StorageError> {
    let conn = Connection::open(db_path)?;
    apply_migrations(&conn)?;
    Ok(conn)
}

/// Open an in-memory database with the schema applied (used by tests).
pub fn init_in_memory() -> Result<Connection, StorageError> {
    let conn = Connection::open_in_memory()?;
    apply_migrations(&conn)?;
    Ok(conn)
}

/// Apply pragmas + the schema DDL. Idempotent; safe to call on every open.
fn apply_migrations(conn: &Connection) -> Result<(), StorageError> {
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(|e| StorageError::MigrationError(e.to_string()))?;
    conn.execute_batch(SCHEMA_SQL)
        .map_err(|e| StorageError::MigrationError(e.to_string()))?;
    // Column-add migrations for databases created before these columns existed.
    // `CREATE TABLE IF NOT EXISTS` never ALTERs an existing table, so a db file
    // from an older schema is missing newer columns and inserts fail. Add them
    // idempotently: a "duplicate column name" error means it's already present
    // (fresh db from the DDL above) and is safely ignored.
    const COLUMN_ADDS: &[&str] = &[
        "ALTER TABLE recordings ADD COLUMN max_duration_seconds INTEGER",
        "ALTER TABLE recordings ADD COLUMN output_dir TEXT",
        "ALTER TABLE recordings ADD COLUMN display_name TEXT",
        "ALTER TABLE recordings ADD COLUMN encoder_mode TEXT NOT NULL DEFAULT 'in_memory_pipe'",
    ];
    for stmt in COLUMN_ADDS {
        if let Err(e) = conn.execute(stmt, []) {
            let msg = e.to_string();
            if !msg.contains("duplicate column name") {
                return Err(StorageError::MigrationError(msg));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_creates_expected_tables() {
        let conn = init_in_memory().unwrap();
        let names: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(names.contains(&"recordings".to_string()));
        assert!(names.contains(&"analysis_requests".to_string()));
        assert!(names.contains(&"analysis_results".to_string()));
    }

    #[test]
    fn migrations_are_idempotent() {
        let conn = init_in_memory().unwrap();
        // Re-applying must not error.
        assert!(apply_migrations(&conn).is_ok());
    }

    #[test]
    fn upgrades_old_schema_missing_columns() {
        // Simulate a db created before the newer columns existed: a bare
        // `recordings` table. CREATE TABLE IF NOT EXISTS won't touch it, so the
        // column-add migration must backfill the missing columns (the bug that
        // caused "table recordings has no column named max_duration_seconds").
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE recordings (id TEXT PRIMARY KEY, status TEXT NOT NULL, start_time TEXT NOT NULL);",
        )
        .unwrap();
        apply_migrations(&conn).unwrap();
        let cols: Vec<String> = conn
            .prepare("PRAGMA table_info(recordings)")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(cols.contains(&"max_duration_seconds".to_string()));
        assert!(cols.contains(&"output_dir".to_string()));
        // Re-applying over the upgraded db must still be idempotent.
        assert!(apply_migrations(&conn).is_ok());
    }
}
