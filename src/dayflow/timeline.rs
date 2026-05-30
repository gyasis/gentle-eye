//! SQLite-backed activity timeline store (Wave 4 · PRD dayflow P3).
//!
//! Persists [`TimelineEntry`] rows (the `timeline_entries` table, see
//! `storage::database`) and answers range queries — the durable, queryable
//! artifact of dayflow (the raw video is scaffolding; this is what survives).
//! Injection-safe (parameterized queries only). Sync over a shared connection;
//! the async pipeline wraps it.

use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use uuid::Uuid;

use crate::contracts::errors::StorageError;
use crate::dayflow::models::{ActivityCategory, TimelineEntry};

/// Persist + query the activity timeline.
pub trait TimelineStore {
    /// Insert (or replace by id) one timeline entry.
    fn insert_entry(&self, entry: &TimelineEntry) -> Result<(), StorageError>;
    /// Entries whose `start_time` is in `[from, to)`, ordered ascending.
    fn query_range(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<TimelineEntry>, StorageError>;
    /// Total number of entries.
    fn count(&self) -> Result<usize, StorageError>;
}

/// SQLite timeline store over a shared connection (shares the `StorageManager`'s
/// connection in production; takes its own `init_in_memory` connection in tests).
pub struct SqliteTimelineStore {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteTimelineStore {
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>, StorageError> {
        self.conn
            .lock()
            .map_err(|_| StorageError::DatabaseError("timeline connection mutex poisoned".into()))
    }
}

fn category_to_db(c: &ActivityCategory) -> String {
    // serde gives the snake_case token (e.g. "coding"); store the bare token.
    serde_json::to_string(c)
        .unwrap_or_else(|_| "\"other\"".to_string())
        .trim_matches('"')
        .to_string()
}

fn category_from_db(s: &str) -> ActivityCategory {
    serde_json::from_str(&format!("\"{s}\"")).unwrap_or(ActivityCategory::Other)
}

fn db(e: impl std::fmt::Display) -> StorageError {
    StorageError::DatabaseError(e.to_string())
}

impl TimelineStore for SqliteTimelineStore {
    fn insert_entry(&self, e: &TimelineEntry) -> Result<(), StorageError> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT OR REPLACE INTO timeline_entries \
             (id, recording_id, start_time, end_time, category, app, activity, summary) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                e.id.to_string(),
                e.recording_id.to_string(),
                e.start_time.to_rfc3339(),
                e.end_time.to_rfc3339(),
                category_to_db(&e.category),
                e.app,
                e.activity,
                e.summary,
            ],
        )
        .map_err(db)?;
        Ok(())
    }

    fn query_range(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<TimelineEntry>, StorageError> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, recording_id, start_time, end_time, category, app, activity, summary \
                 FROM timeline_entries WHERE start_time >= ?1 AND start_time < ?2 \
                 ORDER BY start_time ASC",
            )
            .map_err(db)?;
        // Collect raw columns first, then parse outside the closure for clean errors.
        type Raw = (String, String, String, String, String, String, String, String);
        let raw: Vec<Raw> = stmt
            .query_map(params![from.to_rfc3339(), to.to_rfc3339()], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                ))
            })
            .map_err(db)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db)?;
        drop(stmt);
        drop(conn);

        let parse_ts = |s: &str| -> Result<DateTime<Utc>, StorageError> {
            DateTime::parse_from_rfc3339(s)
                .map(|d| d.with_timezone(&Utc))
                .map_err(|e| StorageError::DatabaseError(format!("bad timestamp '{s}': {e}")))
        };
        let mut out = Vec::with_capacity(raw.len());
        for (id, rid, st, et, cat, app, activity, summary) in raw {
            out.push(TimelineEntry {
                id: Uuid::parse_str(&id).map_err(|e| db(format!("bad id '{id}': {e}")))?,
                recording_id: Uuid::parse_str(&rid)
                    .map_err(|e| db(format!("bad recording_id '{rid}': {e}")))?,
                start_time: parse_ts(&st)?,
                end_time: parse_ts(&et)?,
                category: category_from_db(&cat),
                app,
                activity,
                summary,
            });
        }
        Ok(out)
    }

    fn count(&self) -> Result<usize, StorageError> {
        let conn = self.lock()?;
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM timeline_entries", [], |r| r.get(0))
            .map_err(db)?;
        Ok(n as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::database::init_in_memory;
    use chrono::TimeZone;

    fn store() -> SqliteTimelineStore {
        let conn = init_in_memory().expect("in-memory db");
        SqliteTimelineStore::new(Arc::new(Mutex::new(conn)))
    }

    fn entry(min: i64, category: ActivityCategory, activity: &str) -> TimelineEntry {
        let base = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        TimelineEntry {
            id: Uuid::new_v4(),
            recording_id: Uuid::new_v4(),
            start_time: base + chrono::Duration::minutes(min),
            end_time: base + chrono::Duration::minutes(min + 15),
            category,
            app: "editor".into(),
            activity: activity.into(),
            summary: format!("did {activity}"),
        }
    }

    #[test]
    fn round_trip_insert_query_count() {
        let s = store();
        assert_eq!(s.count().unwrap(), 0);

        s.insert_entry(&entry(0, ActivityCategory::Coding, "a")).unwrap();
        s.insert_entry(&entry(15, ActivityCategory::Docs, "b")).unwrap();
        s.insert_entry(&entry(30, ActivityCategory::Browsing, "c")).unwrap();
        assert_eq!(s.count().unwrap(), 3);

        let base = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        // Full window → all 3, ordered ascending by start_time.
        let all = s.query_range(base, base + chrono::Duration::hours(1)).unwrap();
        assert_eq!(all.len(), 3);
        assert!(all[0].start_time <= all[1].start_time && all[1].start_time <= all[2].start_time);
        // Category + text survive the round-trip.
        assert_eq!(all[0].category, ActivityCategory::Coding);
        assert_eq!(all[1].category, ActivityCategory::Docs);
        assert_eq!(all[0].activity, "a");

        // Narrow window [10min, 25min) → only the 15-min entry.
        let mid = s
            .query_range(
                base + chrono::Duration::minutes(10),
                base + chrono::Duration::minutes(25),
            )
            .unwrap();
        assert_eq!(mid.len(), 1);
        assert_eq!(mid[0].activity, "b");
    }
}
