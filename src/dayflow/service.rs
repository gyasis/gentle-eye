//! The one engine behind all three surfaces (US6).
//!
//! MCP, CLI and HTTP are adapters over this type. That is the whole of T044's
//! parity requirement, and it is deliberately STRUCTURAL rather than a
//! convention the three surfaces agree to follow: three implementations that
//! must be kept in step drift the moment one is changed alone, and the drift
//! shows up as "the CLI says running and the dashboard says stopped" — a
//! contradiction the user has no way to resolve.
//!
//! So there is one state, one set of transitions, and each surface only
//! translates between its own wire format and these calls.

use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::DayflowConfig;
use crate::dayflow::engine::DayflowRun;
use crate::dayflow::errors::DayflowError;
use crate::dayflow::models::{DayflowLiveness, TimelineEntry};
use crate::dayflow::timeline::TimelineStore;

/// What every surface reports for `status`.
///
/// One shape, so a discrepancy between surfaces is impossible by construction
/// rather than by review.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DayflowStatus {
    /// Whether a session is running at all.
    pub running: bool,
    /// The session, when one is running.
    pub session_id: Option<Uuid>,
    /// When it started.
    pub started_at: Option<DateTime<Utc>>,
    /// Displays being captured.
    pub displays: Vec<u32>,
    /// Liveness — whether it is actually producing, not merely alive.
    ///
    /// `None` when nothing is running, which is NOT a fault: a stopped session
    /// that reported "degraded" would send an operator hunting for a failure
    /// that is simply an absence.
    pub liveness: Option<DayflowLiveness>,
}

impl DayflowStatus {
    /// The status of a machine with no session running.
    pub fn stopped() -> Self {
        Self {
            running: false,
            session_id: None,
            started_at: None,
            displays: Vec::new(),
            liveness: None,
        }
    }

    /// Whether this status should worry an operator.
    ///
    /// A degraded session is still a RUNNING one: every surface reports it with
    /// a success code and the degradation in the payload, because exiting
    /// non-zero for "recording but not producing" makes every script treat a
    /// recoverable state as a crash.
    ///
    /// Uses [`DayflowHealth::is_fault`], which already draws the distinction
    /// that matters — a pause and an off switch are quiet on purpose, and only
    /// Degraded means something is wrong. Re-deriving "unhealthy" here would
    /// have made a deliberate pause read as a fault on every surface.
    pub fn is_degraded(&self) -> bool {
        self.liveness.as_ref().is_some_and(|l| l.health.is_fault())
    }
}

/// Resolve an optional RFC-3339 range to "today so far".
///
/// **One implementation, called by all three surfaces.** It was written three
/// times first — once per surface — and R36 claimed that had been avoided while
/// the duplicates sat in the tree. Two of the three copies were then provably
/// undefended: mutating the MCP default and the CLI default to produce an EMPTY
/// range (so every question would be answered about nothing) survived the whole
/// suite, because only the HTTP copy had a test.
///
/// Duplication is how "the same question returns different answers depending on
/// how you asked it" gets in, and three copies means three chances. There is
/// one now, and the surfaces call it.
pub fn resolve_range(
    from: Option<&str>,
    to: Option<&str>,
    now: DateTime<Utc>,
) -> Result<(DateTime<Utc>, DateTime<Utc>), String> {
    let parse = |s: &str| {
        DateTime::parse_from_rfc3339(s)
            .map(|d| d.with_timezone(&Utc))
            .map_err(|e| format!("bad timestamp '{s}': {e}"))
    };
    let to = match to {
        Some(s) => parse(s)?,
        None => now,
    };
    let from = match from {
        Some(s) => parse(s)?,
        // Midnight of the day the range ENDS on, so "today so far" stays one
        // day even when `to` was supplied explicitly.
        None => to.date_naive().and_hms_opt(0, 0, 0).map(|d| d.and_utc()).unwrap_or(to),
    };
    if from > to {
        return Err(format!("range starts after it ends: {from} > {to}"));
    }
    Ok((from, to))
}

/// The single Dayflow engine, shared by every surface.
pub struct DayflowService {
    run: Mutex<Option<DayflowRun>>,
    store: Arc<dyn TimelineStore + Send + Sync>,
    config: DayflowConfig,
}

impl DayflowService {
    /// Build the service over a timeline store.
    pub fn new(store: Arc<dyn TimelineStore + Send + Sync>, config: DayflowConfig) -> Self {
        Self { run: Mutex::new(None), store, config }
    }

    /// Start a session.
    ///
    /// Refuses when one is already running rather than silently replacing it:
    /// a second start that discarded the first would drop the running session's
    /// unwritten windows, and the caller would see success.
    pub fn start(
        &self,
        mode: crate::dayflow::models::DayflowMode,
        displays: Vec<u32>,
        now: DateTime<Utc>,
    ) -> Result<Uuid, DayflowError> {
        let mut guard = self.lock()?;
        if guard.is_some() {
            return Err(DayflowError::AlreadyRunning);
        }
        let run = DayflowRun::start(&self.config, mode, displays, now)?;
        let id = run.session_id();
        *guard = Some(run);
        Ok(id)
    }

    /// Stop the running session, returning the windows it closed.
    pub fn stop(&self, now: DateTime<Utc>) -> Result<Vec<crate::dayflow::window::ClosedWindow>, DayflowError> {
        let mut guard = self.lock()?;
        let mut run = guard.take().ok_or(DayflowError::NoActiveSession)?;
        Ok(run.stop(now))
    }

    /// The current status, from whichever surface asks.
    pub fn status(&self, now: DateTime<Utc>) -> Result<DayflowStatus, DayflowError> {
        let guard = self.lock()?;
        Ok(match guard.as_ref() {
            None => DayflowStatus::stopped(),
            Some(run) => DayflowStatus {
                running: true,
                session_id: Some(run.session_id()),
                started_at: Some(run.started_at()),
                displays: run.displays().to_vec(),
                liveness: Some(run.liveness(now)),
            },
        })
    }

    /// Timeline entries overlapping `[from, to)`.
    pub fn timeline(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<TimelineEntry>, DayflowError> {
        Ok(self.store.query_range(from, to)?)
    }

    /// Answer a question about a range, grounded strictly on stored entries.
    pub fn ask<F>(
        &self,
        question: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        answerer: F,
    ) -> Result<crate::dayflow::timeline::DayAnswer, DayflowError>
    where
        F: FnOnce(&str) -> String,
    {
        crate::dayflow::timeline::ask_day(self.store.as_ref(), question, from, to, answerer)
            .map_err(DayflowError::from)
    }

    /// Write an entry to the timeline.
    ///
    /// The surfaces do not write entries — the summarisation path does — but
    /// they must all READ the same store, so it is exposed here rather than
    /// letting a surface construct its own connection. Two stores is how a
    /// timeline that is visible on one surface goes missing on another.
    pub fn insert_entry(&self, entry: &TimelineEntry) -> Result<(), DayflowError> {
        Ok(self.store.insert_entry(entry)?)
    }

    /// Run something against the live session, if there is one.
    ///
    /// The seam the capture loop will use. Exposed so a surface never has to
    /// reach into the mutex itself — the lock is this type's business, and a
    /// second holder is how the two-locks deadlock starts.
    pub fn with_run<T>(&self, f: impl FnOnce(&mut DayflowRun) -> T) -> Result<T, DayflowError> {
        let mut guard = self.lock()?;
        let run = guard.as_mut().ok_or(DayflowError::NoActiveSession)?;
        Ok(f(run))
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Option<DayflowRun>>, DayflowError> {
        // A poisoned lock means another thread panicked while holding it. The
        // state behind it is a plain Option, so recovering is safe — and
        // refusing every subsequent call would turn one panic into a dead
        // daemon that reports nothing.
        Ok(self.run.lock().unwrap_or_else(|p| p.into_inner()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dayflow::models::DayflowMode;
    use crate::dayflow::timeline::SqliteTimelineStore;
    use crate::storage::database::init_in_memory;

    fn service() -> DayflowService {
        let store = Arc::new(SqliteTimelineStore::new(Arc::new(Mutex::new(
            init_in_memory().unwrap(),
        ))));
        DayflowService::new(store, DayflowConfig::default())
    }

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_787_500_000 + secs, 0).unwrap()
    }

    #[test]
    fn a_session_started_once_is_visible_to_every_later_caller() {
        // US6's independent test, expressed where it can actually be enforced:
        // the surfaces do not each hold state, so "started on one, visible from
        // the others" reduces to "the service has one state" — which is a
        // property of the type rather than an agreement between three
        // implementations that must be kept in step.
        let s = service();
        assert!(!s.status(at(0)).unwrap().running, "nothing running to begin with");

        let id = s.start(DayflowMode::Session, vec![0], at(0)).unwrap();

        let seen = s.status(at(10)).unwrap();
        assert!(seen.running);
        assert_eq!(seen.session_id, Some(id), "the same session, whoever asks");
        assert_eq!(seen.started_at, Some(at(0)));
        assert_eq!(seen.displays, vec![0]);
    }

    #[test]
    fn starting_twice_is_refused_rather_than_silently_replacing_the_first() {
        // A second start that discarded the first would drop the running
        // session's unwritten windows, and the caller would see success.
        let s = service();
        let first = s.start(DayflowMode::Session, vec![0], at(0)).unwrap();
        let again = s.start(DayflowMode::Session, vec![1], at(10));
        assert!(matches!(again, Err(DayflowError::AlreadyRunning)));

        let after = s.status(at(20)).unwrap();
        assert_eq!(after.session_id, Some(first), "the original session is untouched");
        assert_eq!(after.displays, vec![0], "and its displays are not replaced");
    }

    #[test]
    fn stopping_when_nothing_runs_is_an_error_not_a_silent_success() {
        let s = service();
        assert!(matches!(s.stop(at(0)), Err(DayflowError::NoActiveSession)));
    }

    #[test]
    fn a_stopped_service_reports_no_liveness_rather_than_a_fault() {
        // A stopped session reporting "degraded" would send an operator hunting
        // for a failure that is simply an absence.
        let s = service();
        let stopped = s.status(at(0)).unwrap();
        assert!(stopped.liveness.is_none());
        assert!(!stopped.is_degraded(), "not running is not a fault");

        s.start(DayflowMode::Session, vec![0], at(0)).unwrap();
        s.stop(at(100)).unwrap();
        let after = s.status(at(200)).unwrap();
        assert!(!after.running);
        assert!(after.liveness.is_none());
        assert!(!after.is_degraded());
    }

    #[test]
    fn a_deliberate_pause_is_not_reported_as_a_fault() {
        // FR-032. Deriving "unhealthy" as "not Healthy" would make every idle
        // lunch break look like a broken recorder on all three surfaces.
        let s = service();
        s.start(DayflowMode::Session, vec![0], at(0)).unwrap();
        s.with_run(|r| r.turn_off(at(60))).unwrap();

        let paused = s.status(at(120)).unwrap();
        assert!(paused.running, "still a session");
        assert!(!paused.is_degraded(), "off is quiet on purpose, not a fault");
    }

    #[test]
    fn the_timeline_is_readable_whether_or_not_a_session_is_running() {
        // Asking what happened yesterday must not require a recorder to be
        // running today.
        let s = service();
        let e = TimelineEntry {
            id: Uuid::new_v4(),
            recording_id: Uuid::new_v4(),
            start_time: at(0),
            end_time: at(600),
            category: crate::dayflow::models::ActivityCategory::Coding,
            app: "editor".into(),
            activity: "refactor".into(),
            summary: "worked on the ladder".into(),
            provenance: None,
        };
        s.store.insert_entry(&e).unwrap();

        let without_session = s.timeline(at(0), at(1_000)).unwrap();
        assert_eq!(without_session.len(), 1);

        s.start(DayflowMode::Session, vec![0], at(2_000)).unwrap();
        let with_session = s.timeline(at(0), at(1_000)).unwrap();
        assert_eq!(with_session.len(), 1, "and the answer does not change");
    }

    #[test]
    fn asking_about_an_empty_range_never_reaches_the_model() {
        let s = service();
        let mut called = false;
        let a = s
            .ask("what was I doing?", at(0), at(600), |_| {
                called = true;
                "invented".into()
            })
            .unwrap();
        assert!(!called, "no grounding, no question");
        assert_eq!(a.answer, crate::dayflow::timeline::NO_RECORD);
    }

    #[test]
    fn a_panic_while_holding_the_lock_does_not_kill_the_service() {
        // Refusing every subsequent call on a poisoned lock turns one panic
        // into a daemon that reports nothing at all — worse than the panic.
        let s = Arc::new(service());
        s.start(DayflowMode::Session, vec![0], at(0)).unwrap();

        let poisoner = Arc::clone(&s);
        let _ = std::thread::spawn(move || {
            poisoner.with_run(|_| panic!("boom")).ok();
        })
        .join();

        let after = s.status(at(10)).expect("the service still answers");
        assert!(after.running, "and the session is intact");
    }
}
