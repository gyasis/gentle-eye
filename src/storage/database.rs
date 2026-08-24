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

-- ── dayflow ledgers ────────────────────────────────────────────────────────
-- A dayflow "segment" is a WINDOW of sampled frames, not a continuously encoded
-- video chunk (D9). The window ledger is what liveness and eviction read: both
-- must be answerable from rows another process wrote, never from a flag the
-- daemon keeps about itself.
CREATE TABLE IF NOT EXISTS dayflow_segments (
    session_id   TEXT    NOT NULL,
    display_id   INTEGER NOT NULL,
    sequence     INTEGER NOT NULL,
    start_wall   TEXT    NOT NULL,
    end_wall     TEXT    NOT NULL,
    sample_count INTEGER NOT NULL DEFAULT 0,
    summarized   INTEGER NOT NULL DEFAULT 0,
    tier         TEXT    NOT NULL DEFAULT 'hot',
    bytes        INTEGER,
    intent       TEXT    NOT NULL DEFAULT 'activity',
    PRIMARY KEY (session_id, display_id, sequence)
);

-- One row per sampled frame, INCLUDING the ones the delta gate skipped. Skips
-- are recorded rather than omitted: "nothing changed for an hour" and "the
-- sampler died an hour ago" must be distinguishable, and an absent row cannot
-- tell them apart.
CREATE TABLE IF NOT EXISTS dayflow_samples (
    session_id  TEXT    NOT NULL,
    display_id  INTEGER NOT NULL,
    sequence    INTEGER NOT NULL,
    taken_at    TEXT    NOT NULL,
    path        TEXT,
    skipped     INTEGER NOT NULL DEFAULT 0,
    skip_reason TEXT,
    perceived   INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (session_id, display_id, sequence, taken_at)
);

-- Paused intervals, so a gap is a RECORDED FACT with a cause rather than an
-- absence of rows. Without this, an idle pause and a crash look identical.
CREATE TABLE IF NOT EXISTS dayflow_pauses (
    id         TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    from_ts    TEXT NOT NULL,
    to_ts      TEXT,
    cause      TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_recordings_status ON recordings(status);
CREATE INDEX IF NOT EXISTS idx_recordings_start_time ON recordings(start_time);
CREATE INDEX IF NOT EXISTS idx_results_request ON analysis_results(request_id);
CREATE INDEX IF NOT EXISTS idx_timeline_start ON timeline_entries(start_time);
CREATE INDEX IF NOT EXISTS idx_timeline_range ON timeline_entries(start_time, end_time);
-- Eviction order is (tier, summarized, age): oldest summarized raw first, then
-- oldest warm, and NEVER an unsummarized segment.
CREATE INDEX IF NOT EXISTS idx_segments_evict ON dayflow_segments(tier, summarized, end_wall);
CREATE INDEX IF NOT EXISTS idx_segments_live ON dayflow_segments(session_id, end_wall);
CREATE INDEX IF NOT EXISTS idx_samples_window ON dayflow_samples(session_id, display_id, sequence);
CREATE INDEX IF NOT EXISTS idx_pauses_session ON dayflow_pauses(session_id, from_ts);
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
    fn dayflow_ledger_tables_exist() {
        let conn = init_in_memory().unwrap();
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table'")
            .unwrap();
        let tables: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        for want in ["dayflow_segments", "dayflow_samples", "dayflow_pauses"] {
            assert!(tables.contains(&want.to_string()), "missing {want} in {tables:?}");
        }
    }

    #[test]
    fn dayflow_migration_preserves_existing_timeline_rows() {
        // The additive requirement (FR-021): a database written by the earlier
        // schema must survive this migration with its entries intact. The
        // timeline is the permanent artifact — a migration that drops a row is
        // the one unrecoverable bug in this feature.
        let conn = init_in_memory().unwrap();
        conn.execute(
            "INSERT INTO timeline_entries (id, recording_id, start_time, end_time, category, app, activity, summary)
             VALUES ('e1','r1','2026-08-24T09:00:00Z','2026-08-24T09:15:00Z','coding','vscode','edited config','wrote the sampler')",
            [],
        )
        .unwrap();

        // re-run the whole migration, as happens on every open
        apply_migrations(&conn).unwrap();

        let summary: String = conn
            .query_row("SELECT summary FROM timeline_entries WHERE id='e1'", [], |r| r.get(0))
            .expect("the pre-existing entry must still be there");
        assert_eq!(summary, "wrote the sampler");
    }

    #[test]
    fn a_skipped_sample_is_recorded_not_omitted() {
        // "Nothing changed for an hour" and "the sampler died an hour ago" must
        // be distinguishable. An absent row cannot tell them apart, so the delta
        // gate records the skip rather than writing nothing.
        let conn = init_in_memory().unwrap();
        conn.execute(
            "INSERT INTO dayflow_samples (session_id, display_id, sequence, taken_at, path, skipped, skip_reason)
             VALUES ('s1', 0, 1, '2026-08-24T09:01:00Z', NULL, 1, 'unchanged')",
            [],
        )
        .unwrap();
        let (skipped, reason): (i64, String) = conn
            .query_row(
                "SELECT skipped, skip_reason FROM dayflow_samples WHERE session_id='s1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(skipped, 1);
        assert_eq!(reason, "unchanged");
    }

    #[test]
    fn segment_identity_is_session_display_sequence() {
        // NOT the filename and NOT ffmpeg's per-run counter, which restarts on
        // every pause, interval change and display change.
        let conn = init_in_memory().unwrap();
        let ins = "INSERT INTO dayflow_segments (session_id, display_id, sequence, start_wall, end_wall)
                   VALUES (?1, ?2, ?3, '2026-08-24T09:00:00Z', '2026-08-24T09:15:00Z')";
        conn.execute(ins, rusqlite::params!["s1", 0, 1]).unwrap();
        // same sequence on a DIFFERENT display is a different segment
        conn.execute(ins, rusqlite::params!["s1", 1, 1]).unwrap();
        // ...but the same triple collides
        assert!(
            conn.execute(ins, rusqlite::params!["s1", 0, 1]).is_err(),
            "(session, display, sequence) must be unique"
        );
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM dayflow_segments", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 2);
    }

    #[test]
    fn an_old_database_without_dayflow_tables_is_upgraded() {
        // A db file created before dayflow existed: CREATE TABLE IF NOT EXISTS
        // must add the ledgers without disturbing what is already there.
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE recordings (id TEXT PRIMARY KEY, status TEXT NOT NULL, start_time TEXT NOT NULL);
             INSERT INTO recordings (id, status, start_time) VALUES ('r9','completed','2026-01-01T00:00:00Z');",
        )
        .unwrap();
        apply_migrations(&conn).unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM dayflow_segments", [], |r| r.get(0))
            .expect("ledger must exist after upgrade");
        assert_eq!(n, 0);
        let status: String = conn
            .query_row("SELECT status FROM recordings WHERE id='r9'", [], |r| r.get(0))
            .expect("the pre-existing recording must survive");
        assert_eq!(status, "completed");
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
