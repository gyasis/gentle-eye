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

CREATE INDEX IF NOT EXISTS idx_recordings_status ON recordings(status);
CREATE INDEX IF NOT EXISTS idx_recordings_start_time ON recordings(start_time);
CREATE INDEX IF NOT EXISTS idx_results_request ON analysis_results(request_id);
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
}
