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

// ─── T024: grounded day-level Q&A ──────────────────────────────────────────

/// An answer about a day, and the entries it was built from.
///
/// The entries are returned, not just cited, so a caller can SHOW its working.
/// An answer whose `grounding` is empty but whose `answer` is confident prose is
/// the failure this type exists to make visible.
#[derive(Debug, Clone)]
pub struct DayAnswer {
    /// The answer text.
    pub answer: String,
    /// The entries it was grounded on, in time order.
    pub grounding: Vec<TimelineEntry>,
}

impl DayAnswer {
    /// Whether this answer rests on any recorded evidence.
    pub fn is_grounded(&self) -> bool {
        !self.grounding.is_empty()
    }
}

/// The exact text returned when a range holds no entries.
///
/// A fixed string rather than a model call: with nothing to ground on, asking a
/// model produces plausible invention, and for a record of someone's day an
/// invented answer is worse than no answer (FR-018).
pub const NO_RECORD: &str = "No activity was recorded for that period.";

/// Build the grounding prompt for a day-level question.
///
/// The entries are rendered with their REAL time ranges. Never summarise the
/// ranges or round them: a user asking "what was I doing at 2pm" is asking about
/// a specific minute, and the entry boundaries are the only thing that answers it.
pub fn build_day_prompt(question: &str, entries: &[TimelineEntry]) -> String {
    let mut p = String::from(
        "Answer the question using ONLY the recorded activity below. \
         If the record does not contain the answer, say so plainly. \
         Do not speculate about what the user might have been doing.\n\n\
         RECORDED ACTIVITY:\n",
    );
    for e in entries {
        p.push_str(&format!(
            "- {} to {} | {:?} | {} | {} | {}\n",
            e.start_time.format("%H:%M"),
            e.end_time.format("%H:%M"),
            e.category,
            e.app,
            e.activity,
            e.summary
        ));
    }
    p.push_str(&format!("\nQUESTION: {question}\n"));
    p
}

/// Ask a question about a time range, grounded strictly on stored entries.
///
/// `answerer` receives the prompt and returns the model's reply. It is a
/// parameter so the grounding rules — which are the part that must not be got
/// wrong — are testable without a model.
///
/// Returns [`NO_RECORD`] without calling `answerer` when the range is empty.
pub fn ask_day<S, F>(
    store: &S,
    question: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    answerer: F,
) -> Result<DayAnswer, StorageError>
where
    S: TimelineStore + ?Sized,
    F: FnOnce(&str) -> String,
{
    let entries = store.query_range(from, to)?;
    if entries.is_empty() {
        // Nothing to ground on. Do NOT ask the model: with no evidence it
        // produces plausible invention, and a fabricated account of someone's
        // day is worse than an admission of ignorance.
        return Ok(DayAnswer { answer: NO_RECORD.to_string(), grounding: Vec::new() });
    }
    let answer = answerer(&build_day_prompt(question, &entries));
    Ok(DayAnswer { answer, grounding: entries })
}

#[cfg(test)]
mod ask_tests {
    use super::*;
    use crate::storage::database::init_in_memory;
    use std::sync::{Arc, Mutex};
    use uuid::Uuid;

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_787_500_000 + secs, 0).unwrap()
    }

    fn store() -> SqliteTimelineStore {
        SqliteTimelineStore::new(Arc::new(Mutex::new(init_in_memory().unwrap())))
    }

    fn entry(from: i64, to: i64, activity: &str) -> TimelineEntry {
        TimelineEntry {
            id: Uuid::new_v4(),
            recording_id: Uuid::new_v4(),
            start_time: at(from),
            end_time: at(to),
            category: ActivityCategory::Coding,
            app: "editor".into(),
            activity: activity.into(),
            summary: format!("worked on {activity}"),
        }
    }

    #[test]
    fn an_empty_range_says_so_and_never_asks_the_model() {
        // FR-018. With no evidence a model invents, and a fabricated account of
        // someone's day is worse than admitting there is no record.
        let s = store();
        let mut called = false;
        let a = ask_day(&s, "what was I doing at 2pm?", at(0), at(10_000), |_| {
            called = true;
            "You were deep in a refactor.".into()
        })
        .unwrap();

        assert!(!called, "the model must NOT be consulted with nothing to ground on");
        assert_eq!(a.answer, NO_RECORD);
        assert!(!a.is_grounded());
        assert!(a.grounding.is_empty());
    }

    #[test]
    fn an_answer_is_grounded_on_the_entries_that_cover_the_range() {
        let s = store();
        s.insert_entry(&entry(0, 600, "the sampler")).unwrap();
        s.insert_entry(&entry(600, 1200, "the gate")).unwrap();
        s.insert_entry(&entry(50_000, 50_600, "something else entirely")).unwrap();

        let a = ask_day(&s, "what was I doing?", at(0), at(1_200), |_| "answer".into()).unwrap();
        assert!(a.is_grounded());
        assert_eq!(a.grounding.len(), 2, "only entries inside the range");
        assert!(a.grounding.iter().all(|e| e.start_time < at(1_200)));
    }

    #[test]
    fn the_prompt_carries_the_real_time_ranges() {
        // "What was I doing at 2pm" is a question about a specific minute; the
        // entry boundaries are the only thing that answers it.
        let entries = vec![entry(0, 600, "the sampler")];
        let p = build_day_prompt("what was I doing?", &entries);
        assert!(p.contains("the sampler"), "the activity must reach the model");
        assert!(p.contains(" to "), "and its time range: {p}");
        assert!(
            p.contains("ONLY the recorded activity"),
            "the prompt must constrain the model to the record"
        );
        assert!(
            p.to_lowercase().contains("do not speculate"),
            "and forbid invention explicitly"
        );
    }

    #[test]
    fn grounding_is_returned_so_a_caller_can_show_its_working() {
        // An answer with confident prose and an EMPTY grounding list is the
        // failure this type makes visible.
        let s = store();
        s.insert_entry(&entry(0, 600, "the scheduler")).unwrap();
        let a = ask_day(&s, "?", at(0), at(600), |_| "you wrote the scheduler".into()).unwrap();
        assert!(a.is_grounded());
        assert_eq!(a.grounding[0].activity, "the scheduler");
    }

    #[test]
    fn entries_reach_the_model_in_time_order() {
        let s = store();
        // inserted out of order on purpose
        s.insert_entry(&entry(1_200, 1_800, "third")).unwrap();
        s.insert_entry(&entry(0, 600, "first")).unwrap();
        s.insert_entry(&entry(600, 1_200, "second")).unwrap();

        let mut seen = String::new();
        ask_day(&s, "?", at(0), at(2_000), |p| {
            seen = p.to_string();
            "ok".into()
        })
        .unwrap();
        let (f, sec, t) = (
            seen.find("first").unwrap(),
            seen.find("second").unwrap(),
            seen.find("third").unwrap(),
        );
        assert!(f < sec && sec < t, "a day must be narrated in time order");
    }

    #[test]
    fn a_range_covering_only_a_gap_is_not_answered_from_neighbours() {
        // The subtle one: entries exist on either side, but nothing covers the
        // asked-about window. Answering from the neighbours would describe a
        // period the user was demonstrably not recorded doing anything in.
        let s = store();
        s.insert_entry(&entry(0, 600, "morning")).unwrap();
        s.insert_entry(&entry(50_000, 50_600, "evening")).unwrap();

        let mut called = false;
        let a = ask_day(&s, "what about mid-afternoon?", at(10_000), at(20_000), |_| {
            called = true;
            "invented".into()
        })
        .unwrap();
        assert!(!called);
        assert_eq!(a.answer, NO_RECORD);
    }
}
