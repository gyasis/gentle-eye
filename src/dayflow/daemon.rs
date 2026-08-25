//! Daemon lifecycle: durable state that survives a restart.
//!
//! A [`DayflowRun`](crate::dayflow::engine::DayflowRun) lives in memory and dies
//! with the process. The daemon's distinguishing requirement is that it does
//! NOT: a crash at 11am must resume onto the same day's timeline at 11:05, not
//! open a second session and file the afternoon separately.
//!
//! This module owns the small piece of durable state that makes that possible,
//! and nothing else — no threads, no capture. It is deliberately a plain
//! serialisable record plus atomic read/write, because the failure it guards
//! against is a process dying at an arbitrary moment.

use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::dayflow::errors::DayflowError;

/// What a daemon needs on disk to resume itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonState {
    /// The session being continued.
    pub session_id: Uuid,
    /// The calendar day it belongs to. A restart on a DIFFERENT day must start a
    /// new session, not continue this one.
    pub day: NaiveDate,
    /// When the session started.
    pub started_at: DateTime<Utc>,
    /// Process id of the daemon that wrote this, for liveness checks.
    pub pid: u32,
    /// Highest window sequence recorded per display, so sequences keep climbing
    /// across a restart instead of colliding with what is already on disk.
    pub last_sequence: std::collections::BTreeMap<u32, u64>,
    /// When this record was last written.
    pub updated_at: DateTime<Utc>,
}

impl DaemonState {
    /// A fresh record for a new session.
    pub fn new(session_id: Uuid, started_at: DateTime<Utc>, pid: u32) -> Self {
        Self {
            session_id,
            day: started_at.date_naive(),
            started_at,
            pid,
            last_sequence: std::collections::BTreeMap::new(),
            updated_at: started_at,
        }
    }

    /// Note the highest sequence reached on a display.
    ///
    /// Monotonic: a lower value never lowers what is recorded, so an
    /// out-of-order update cannot make a restart reuse sequences.
    pub fn note_sequence(&mut self, display_id: u32, sequence: u64, now: DateTime<Utc>) {
        let e = self.last_sequence.entry(display_id).or_insert(sequence);
        if sequence > *e {
            *e = sequence;
        }
        self.updated_at = now;
    }

    /// The sequence a restart should continue from for `display_id`.
    pub fn next_sequence(&self, display_id: u32) -> u64 {
        self.last_sequence.get(&display_id).map_or(0, |s| s + 1)
    }

    /// Whether a restart at `now` may continue this session.
    pub fn resumable_at(&self, now: DateTime<Utc>) -> bool {
        now.date_naive() == self.day
    }
}

/// Something wrong with the state on disk, worth surfacing rather than swallowing.
///
/// Both variants mean the daemon starts fresh. The distinction is diagnostic:
/// they say the last run did not stop cleanly, so its final windows may be
/// incomplete — the same class of evidence as a dropped frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StateAnomaly {
    /// The file existed but did not parse — a half-written or truncated save.
    Corrupt,
    /// The file existed but could not be read at all.
    Unreadable,
}

impl StateAnomaly {
    /// Short stable label for status payloads.
    pub fn label(self) -> &'static str {
        match self {
            Self::Corrupt => "corrupt_state",
            Self::Unreadable => "unreadable_state",
        }
    }
}

/// Where the daemon keeps its state, and how it reads and writes it.
#[derive(Debug, Clone)]
pub struct DaemonStateStore {
    path: PathBuf,
}

impl DaemonStateStore {
    /// A store backed by `path`.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The file this store reads and writes.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Load the state, if any.
    ///
    /// A corrupt or unreadable file yields no state rather than an error: the
    /// daemon must still start, because a recorder that refuses to run loses the
    /// whole day, while starting fresh loses only continuity.
    ///
    /// But it is NOT silent. Use [`DaemonStateStore::load_reporting`] to see
    /// WHY there is no state — a half-written file is a symptom worth
    /// investigating, and the previous session's tail may itself be incomplete.
    pub fn load(&self) -> Result<Option<DaemonState>, DayflowError> {
        Ok(self.load_reporting()?.0)
    }

    /// Load the state, reporting any anomaly alongside it.
    ///
    /// The anomaly is the point: "no state" from a clean stop and "no state
    /// because the file was half-written" mean very different things, and only
    /// the second says something went wrong last time.
    pub fn load_reporting(
        &self,
    ) -> Result<(Option<DaemonState>, Option<StateAnomaly>), DayflowError> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => match serde_json::from_str::<DaemonState>(&text) {
                Ok(state) => Ok((Some(state), None)),
                Err(e) => {
                    tracing::warn!(
                        path = %self.path.display(),
                        bytes = text.len(),
                        error = %e,
                        "dayflow: daemon state is unreadable — treating it as a crashed \
                         session and starting fresh; the previous session's tail may be \
                         incomplete and is worth investigating"
                    );
                    Ok((None, Some(StateAnomaly::Corrupt)))
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok((None, None)),
            Err(e) => {
                tracing::warn!(
                    path = %self.path.display(),
                    error = %e,
                    "dayflow: daemon state could not be read — starting fresh"
                );
                Ok((None, Some(StateAnomaly::Unreadable)))
            }
        }
    }

    /// Write the state ATOMICALLY: to a temporary file, then rename.
    ///
    /// A daemon can die mid-write. A half-written state file that still parses
    /// is worse than none — it would resume onto a session that never existed —
    /// so the rename makes the swap all-or-nothing.
    pub fn save(&self, state: &DaemonState) -> Result<(), DayflowError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| DayflowError::Internal(format!("create {}: {e}", parent.display())))?;
        }
        let tmp = self.path.with_extension("json.tmp");
        let text = serde_json::to_string_pretty(state)
            .map_err(|e| DayflowError::Internal(format!("serialize daemon state: {e}")))?;
        std::fs::write(&tmp, text)
            .map_err(|e| DayflowError::Internal(format!("write {}: {e}", tmp.display())))?;
        std::fs::rename(&tmp, &self.path)
            .map_err(|e| DayflowError::Internal(format!("rename into {}: {e}", self.path.display())))?;
        Ok(())
    }

    /// Remove the state file — the daemon has stopped cleanly.
    ///
    /// Leaving it behind would make the next start look like a crash recovery.
    pub fn clear(&self) -> Result<(), DayflowError> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(DayflowError::Internal(format!(
                "remove {}: {e}",
                self.path.display()
            ))),
        }
    }
}

/// What a daemon start decided to do about any previous state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeDecision {
    /// No prior state — a genuinely new session.
    Fresh,
    /// Prior state from today — continue it.
    Resumed,
    /// Prior state from another day — start a new session and leave that day's
    /// timeline alone.
    NewDay,
}

/// Decide what a daemon starting at `now` should do with `prior` state.
pub fn decide_resume(prior: Option<&DaemonState>, now: DateTime<Utc>) -> ResumeDecision {
    match prior {
        None => ResumeDecision::Fresh,
        Some(s) if s.resumable_at(now) => ResumeDecision::Resumed,
        Some(_) => ResumeDecision::NewDay,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_787_500_000 + secs, 0).unwrap()
    }

    fn state() -> DaemonState {
        DaemonState::new(Uuid::new_v4(), at(0), 4242)
    }

    #[test]
    fn state_round_trips_through_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = DaemonStateStore::new(dir.path().join("daemon.json"));
        assert_eq!(store.load().unwrap(), None, "nothing yet");

        let mut s = state();
        s.note_sequence(0, 7, at(100));
        s.note_sequence(1, 3, at(100));
        store.save(&s).unwrap();

        let back = store.load().unwrap().expect("state must load");
        assert_eq!(back, s);
        assert_eq!(back.next_sequence(0), 8, "sequences continue, not restart");
        assert_eq!(back.next_sequence(1), 4);
        assert_eq!(back.next_sequence(9), 0, "an unseen display starts at 0");
    }

    #[test]
    fn a_restart_the_same_day_resumes_the_same_session() {
        let s = state();
        assert_eq!(decide_resume(Some(&s), at(3_600)), ResumeDecision::Resumed);
    }

    #[test]
    fn a_restart_on_a_different_day_starts_a_new_session() {
        // Continuing yesterday's session would file today's windows under the
        // wrong date, and the timeline is the permanent artifact.
        let s = state();
        let tomorrow = at(0) + chrono::Duration::days(1);
        assert_eq!(decide_resume(Some(&s), tomorrow), ResumeDecision::NewDay);
        assert!(!s.resumable_at(tomorrow));
    }

    #[test]
    fn no_prior_state_is_a_fresh_session() {
        assert_eq!(decide_resume(None, at(0)), ResumeDecision::Fresh);
    }

    #[test]
    fn sequences_never_go_backwards() {
        // A restart must not reuse a sequence already on disk, or two different
        // windows collide on the (session, display, sequence) key.
        let mut s = state();
        s.note_sequence(0, 9, at(10));
        s.note_sequence(0, 4, at(20)); // a stale/out-of-order update
        assert_eq!(s.last_sequence[&0], 9, "a lower value must not lower the record");
        assert_eq!(s.next_sequence(0), 10);
    }

    #[test]
    fn a_corrupt_state_file_starts_fresh_but_says_so() {
        // A recorder that will not start loses the whole day; starting fresh
        // loses only continuity. But a half-written file is a SYMPTOM — the last
        // run did not stop cleanly and its final windows may be incomplete — so
        // it must be reported, not swallowed.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.json");
        std::fs::write(&path, "{ this is not json").unwrap();
        let store = DaemonStateStore::new(&path);

        let (state, anomaly) = store.load_reporting().unwrap();
        assert_eq!(state, None, "it still starts");
        assert_eq!(
            anomaly,
            Some(StateAnomaly::Corrupt),
            "and it reports WHY there is no state"
        );
        assert_eq!(decide_resume(state.as_ref(), at(0)), ResumeDecision::Fresh);
    }

    #[test]
    fn a_clean_start_is_distinguishable_from_a_crashed_one() {
        // The distinction that makes the anomaly useful: "no state because we
        // stopped cleanly" and "no state because the file was half-written" look
        // identical through `load()` alone.
        let dir = tempfile::tempdir().unwrap();
        let clean = DaemonStateStore::new(dir.path().join("clean.json"));
        assert_eq!(clean.load_reporting().unwrap(), (None, None), "never written ⇒ no anomaly");

        let crashed = DaemonStateStore::new(dir.path().join("crashed.json"));
        std::fs::write(crashed.path(), "{\"session_id\": \"trunc").unwrap();
        let (_, anomaly) = crashed.load_reporting().unwrap();
        assert!(anomaly.is_some(), "a truncated save must be distinguishable from a clean stop");
        assert_eq!(anomaly.unwrap().label(), "corrupt_state");
    }

    #[test]
    fn saving_is_atomic_and_leaves_no_temporary_behind() {
        // A daemon can die mid-write; a half-written file that still parses
        // would resume onto a session that never existed.
        let dir = tempfile::tempdir().unwrap();
        let store = DaemonStateStore::new(dir.path().join("daemon.json"));
        store.save(&state()).unwrap();
        let leftovers: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains("tmp"))
            .collect();
        assert!(leftovers.is_empty(), "no .tmp must survive a save: {leftovers:?}");
    }

    #[test]
    fn overwriting_state_never_leaves_a_partial_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = DaemonStateStore::new(dir.path().join("daemon.json"));
        let mut s = state();
        for i in 0..20 {
            s.note_sequence(0, i, at(i as i64));
            store.save(&s).unwrap();
            let back = store.load().unwrap().expect("every save must remain readable");
            assert_eq!(back.last_sequence[&0], i);
        }
    }

    #[test]
    fn a_clean_stop_removes_the_state_so_the_next_start_is_not_a_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let store = DaemonStateStore::new(dir.path().join("daemon.json"));
        store.save(&state()).unwrap();
        assert!(store.load().unwrap().is_some());
        store.clear().unwrap();
        assert_eq!(store.load().unwrap(), None, "a clean stop leaves nothing behind");
        store.clear().unwrap(); // idempotent
    }
}
