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
    /// WHAT the session is capturing, so a restart resumes the same subject.
    ///
    /// Stored RESOLVED — displays already enumerated, never `Displays { [] }`.
    /// A spec re-resolved on restart can name a different set than the session's
    /// own ordinals (the W7 gate's single-enumeration invariant), and samples
    /// filed under an ordinal the run does not know are silently dropped by
    /// `on_sample`.
    ///
    /// `None` for records written before sources existed: those sessions
    /// captured displays, but WHICH displays is not recoverable, so guessing
    /// would resume the wrong subject. A restart on such a record starts fresh.
    #[serde(default)]
    pub spec: Option<crate::dayflow::source::SourceSpec>,
    /// The port the daemon's HTTP surface is listening on.
    ///
    /// Published here so a CLI or MCP invocation can ATTACH to the running
    /// session instead of constructing its own engine (D014-15). Without it,
    /// every process builds a private in-memory service and `start` in one and
    /// `status` in the next are different sessions talking to nobody.
    ///
    /// `None` means no HTTP surface — the session is local to one process.
    #[serde(default)]
    pub port: Option<u16>,
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
            spec: None,
            port: None,
        }
    }

    /// Note the highest sequence reached on a display.
    ///
    /// Monotonic: a lower value never lowers what is recorded, so an
    /// out-of-order update cannot make a restart reuse sequences.
    /// Record WHAT this session captures. Resolved, never as typed.
    pub fn with_spec(mut self, spec: crate::dayflow::source::SourceSpec) -> Self {
        self.spec = Some(spec);
        self
    }

    /// Publish the port a surface can attach on.
    pub fn with_port(mut self, port: u16) -> Self {
        self.port = Some(port);
        self
    }

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

/// The ONE place a daemon's state lives under a storage root.
///
/// Every surface — the serving daemon, the attaching CLI, the attaching MCP —
/// must derive the path from HERE. A second derivation is a second state store
/// in waiting: two surfaces that compute the path differently are two views of
/// one session that silently diverge (013/R29, T023).
pub fn state_path_under(base_dir: &Path) -> PathBuf {
    base_dir.join("dayflow-daemon.json")
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

/// The daemon: the one owner of a session's cross-process state.
///
/// # Why this exists
///
/// Before it, `DaemonState`, `DaemonStateStore` and `decide_resume` had no
/// caller — the machinery for surviving a restart was complete and nothing used
/// it. Each CLI invocation built its own in-memory service, so `dayflow start`
/// in one process and `dayflow status` in the next were different sessions
/// talking to nobody.
///
/// # Exactly one store
///
/// The daemon owns THE state store. A second store is how two views of one
/// session diverge, and the divergence is silent — both report a running
/// session, with different sequences (013/R29).
pub struct Daemon {
    store: DaemonStateStore,
    service: std::sync::Arc<crate::dayflow::service::DayflowService>,
    /// The summariser + sample directory that make a session actually RECORD.
    ///
    /// `None` means the daemon owns session STATE only — starting a session
    /// that names its source but captures nothing (the state tests use this).
    /// A serving daemon must be built `with_capture`: T022's whole purpose is
    /// a daemon that owns a RUNNING session, and `start_session` alone starts
    /// no capture — the process would serve HTTP all day and record nothing.
    capture: Option<CaptureWiring>,
    /// Serialises every load-mutate-save of the state file WITHIN this
    /// process. `publish_port` and the state keeper both read-modify-write;
    /// unserialised, one overwrites the other's fields and the loss is silent.
    write: std::sync::Mutex<()>,
    /// The OS-level singleton lock, held for the daemon's lifetime once
    /// [`Daemon::lock_exclusive`] succeeds. Advisory `flock`: released
    /// automatically when the process dies, so a crash never wedges the next
    /// start.
    flock: std::sync::Mutex<Option<std::fs::File>>,
}

/// What a capturing daemon supplies beyond state: the pieces
/// `DayflowService::start_with_source` needs to drive the loop.
struct CaptureWiring {
    summarizer: std::sync::Arc<dyn crate::dayflow::summarizer::ChunkSummarizer + Send + Sync>,
    sample_dir: PathBuf,
}

impl Daemon {
    /// A daemon persisting to `state_path`, over `service`.
    pub fn new(
        state_path: impl Into<PathBuf>,
        service: std::sync::Arc<crate::dayflow::service::DayflowService>,
    ) -> Self {
        Self {
            store: DaemonStateStore::new(state_path),
            service,
            capture: None,
            write: std::sync::Mutex::new(()),
            flock: std::sync::Mutex::new(None),
        }
    }

    /// Wire the summariser and sample directory, so `start_or_resume` starts a
    /// session that actually CAPTURES.
    ///
    /// The summariser must already carry its residency policy
    /// (`RoutedChunkSummarizer::with_residency`) — applied at construction so a
    /// configured `Resident` is expressible by the running system (T020's W8
    /// note), and must resolve samples under this same `sample_dir`, or every
    /// segment reads whole frames while capture works perfectly.
    pub fn with_capture(
        mut self,
        summarizer: std::sync::Arc<dyn crate::dayflow::summarizer::ChunkSummarizer + Send + Sync>,
        sample_dir: impl Into<PathBuf>,
    ) -> Self {
        self.capture = Some(CaptureWiring { summarizer, sample_dir: sample_dir.into() });
        self
    }

    /// Become THE daemon for this state file, or refuse.
    ///
    /// [`Daemon::live_peer_port`] answers "is a daemon ALREADY serving?", but
    /// it is a probe, not a lock: two daemons started simultaneously both see
    /// no published port (neither has published yet), both pass, and one
    /// silently overwrites the other's record while both keep capturing —
    /// interleaved sequences in one timeline with nothing saying so. This is
    /// the OS closing that window: an advisory exclusive lock on a sibling
    /// `.lock` file, held until the process dies. A crashed daemon's lock is
    /// released by the kernel, so a crash never requires deleting anything by
    /// hand — which is exactly why the lock is a separate file and never the
    /// state file itself.
    pub fn lock_exclusive(&self) -> Result<(), DayflowError> {
        let path = self.store.path().with_extension("json.lock");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| DayflowError::Internal(format!("create {}: {e}", parent.display())))?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            // Never truncate: the file's CONTENT is irrelevant (the lock is
            // the kernel's), and truncating a file another process holds
            // locked would be a pointless write to contested state.
            .truncate(false)
            .open(&path)
            .map_err(|e| DayflowError::Internal(format!("open {}: {e}", path.display())))?;
        file.try_lock().map_err(|e| {
            DayflowError::Invalid(format!(
                "another dayflow daemon holds {} ({e}) — refusing to become a second: \
                 two daemons would interleave their windows into one timeline",
                path.display()
            ))
        })?;
        *self.flock.lock().unwrap_or_else(|p| p.into_inner()) = Some(file);
        Ok(())
    }

    /// The state store. There is exactly one, and it is this.
    pub fn store(&self) -> &DaemonStateStore {
        &self.store
    }

    /// The service every surface reads through.
    pub fn service(&self) -> &std::sync::Arc<crate::dayflow::service::DayflowService> {
        &self.service
    }

    /// Start, or RESUME, a session for `spec`.
    ///
    /// Returns the decision alongside the session id so the caller can say which
    /// happened. A resume that silently looked like a fresh start would make the
    /// interruption invisible — and the interruption is the fact worth keeping.
    pub fn start_or_resume(
        &self,
        mode: crate::dayflow::models::DayflowMode,
        spec: crate::dayflow::source::SourceSpec,
        now: DateTime<Utc>,
    ) -> Result<(Uuid, ResumeDecision), DayflowError> {
        let _w = self.write.lock().unwrap_or_else(|p| p.into_inner());
        let (prior, anomaly) = self.store.load_reporting()?;
        if let Some(a) = anomaly {
            // Not swallowed: it says the last run did not stop cleanly, so its
            // final windows may be incomplete.
            tracing::warn!(anomaly = a.label(), "daemon state was unusable; starting fresh");
        }
        let decision = decide_resume(prior.as_ref(), now);

        // A resume keeps the PERSISTED spec, not the one just typed. The
        // session is a record of what it has been capturing all along; adopting
        // a new subject mid-session would make one timeline describe two
        // different things with nothing marking the seam.
        //
        // A FRESH spec is resolved here, ONCE — the persisted record must never
        // hold an unresolved "every display": re-resolving on restart can name
        // different displays than the session's own ordinals (the W7 gate's
        // single-enumeration invariant, D014-15's persistence consequence).
        let effective = match (&decision, prior.as_ref().and_then(|p| p.spec.clone())) {
            (ResumeDecision::Resumed, Some(persisted)) => persisted,
            _ => crate::dayflow::service::resolve_source_spec(&spec)?,
        };

        // Read the interruption's start from the PRIOR record, BEFORE the new
        // state overwrites it. Reading it back after the save would date the
        // gap from `now` to `now` — a zero-width interval no timeline shows,
        // which makes the interruption exactly the invisible hole FR-032
        // exists to prevent.
        let interrupted_from = match (&decision, prior.as_ref()) {
            (ResumeDecision::Resumed, Some(p)) => Some(p.updated_at),
            _ => None,
        };
        // The sequence each display should CONTINUE from (T022: sequences keep
        // climbing across a restart instead of colliding with what is already
        // on disk).
        let resume_sequences: std::collections::BTreeMap<u32, u64> = match (&decision, prior.as_ref())
        {
            (ResumeDecision::Resumed, Some(p)) => {
                p.last_sequence.keys().map(|&d| (d, p.next_sequence(d))).collect()
            }
            _ => std::collections::BTreeMap::new(),
        };

        // Everything a resume carries into the run, applied BEFORE the capture
        // thread can produce anything under the wrong identity or sequence.
        let carry = crate::dayflow::service::ResumeCarry {
            next_sequences: resume_sequences,
            adopt_orphaned_samples: decision == ResumeDecision::Resumed,
            identity: match (&decision, prior.as_ref()) {
                (ResumeDecision::Resumed, Some(p)) => Some((p.session_id, p.started_at)),
                _ => None,
            },
        };
        let id = match &self.capture {
            // The daemon's real path: a session that CAPTURES. Adoption of the
            // dead process's samples is gated on this being a resume — adopted
            // summaries are filed under the running session, and only a resume
            // is the same session.
            Some(w) => self.service.start_with_source(
                mode,
                effective.clone(),
                std::sync::Arc::clone(&w.summarizer),
                w.sample_dir.clone(),
                &carry,
                now,
            )?,
            // State-only: names the source, records nothing. For tests of the
            // daemon's persistence; a SERVING daemon is built `with_capture`.
            None => {
                let mut id = self.service.start_session(mode, effective.clone(), now)?;
                if let Some((session_id, started_at)) = carry.identity {
                    self.service.with_run(|r| r.adopt_identity(session_id, started_at))?;
                    id = session_id;
                }
                if !carry.next_sequences.is_empty() {
                    self.service
                        .with_run(|r| r.seed_next_sequences(carry.next_sequences.clone()))?;
                }
                id
            }
        };

        let mut state = match (&decision, prior) {
            // Continue the SAME session id, and keep the sequence high-water
            // marks so windows do not collide with what is already on disk.
            (ResumeDecision::Resumed, Some(p)) => DaemonState {
                session_id: p.session_id,
                day: p.day,
                started_at: p.started_at,
                pid: std::process::id(),
                last_sequence: p.last_sequence,
                updated_at: now,
                spec: Some(effective),
                // The NEW process's port, set by whoever serves; a resumed
                // record must not advertise the dead process's listener.
                port: None,
            },
            _ => DaemonState::new(id, now, std::process::id()).with_spec(effective),
        };
        state.updated_at = now;
        self.store.save(&state)?;

        // The interruption between the dead process and this one is a RECORDED
        // fact — a gap with a cause, not an absence (FR-032). Best-effort: a
        // resume whose gap could not be written is still a resume.
        if let Some(from) = interrupted_from {
            if from < now {
                if let Err(e) = self.record_interruption(state.session_id, from, now) {
                    tracing::warn!(error = %e, "could not record the restart gap");
                }
            }
        }
        Ok((state.session_id, decision))
    }

    /// Persist the run's CURRENT sequence high-water marks.
    ///
    /// This is what feeds `DaemonState::last_sequence` — without a writer, the
    /// field is persisted machinery no restart can use, and every resume
    /// restarts sequences at 0 (the disk-scan seed in `start_with_source` is
    /// the backstop; this is the primary record). Returns `false` when there is
    /// no session to read, so a keeper loop knows to stop.
    pub fn persist_sequences(&self, now: DateTime<Utc>) -> bool {
        let seqs = match self.service.with_run(|r| {
            r.displays()
                .iter()
                .map(|&d| (d, r.current_sequence(d)))
                .collect::<Vec<_>>()
        }) {
            Ok(s) => s,
            // No active session (stopped, or not yet started): nothing to keep.
            Err(_) => return false,
        };
        let _w = self.write.lock().unwrap_or_else(|p| p.into_inner());
        match self.store.load() {
            Ok(Some(mut state)) => {
                for (d, s) in seqs {
                    state.note_sequence(d, s, now);
                }
                if let Err(e) = self.store.save(&state) {
                    tracing::warn!(error = %e, "could not persist sequence high-water marks");
                }
                true
            }
            // The state file is gone (a stop raced us) or unreadable: stop
            // keeping rather than resurrect a record the stop just cleared.
            _ => false,
        }
    }

    /// Keep the persisted sequence marks fresh while the session runs.
    ///
    /// A restart can only continue from what was WRITTEN: state saved once at
    /// start records sequence 0 forever, and the whole day's windows would
    /// collide on resume. The keeper exits on its own when the session ends.
    pub fn spawn_state_keeper(
        self: &std::sync::Arc<Self>,
        period: std::time::Duration,
    ) -> std::thread::JoinHandle<()> {
        let daemon = std::sync::Arc::clone(self);
        std::thread::spawn(move || loop {
            std::thread::sleep(period);
            if !daemon.persist_sequences(Utc::now()) {
                break;
            }
        })
    }

    /// Whether another daemon is already serving this state file.
    ///
    /// The T023 requirement is exactly ONE state store, and the failure it
    /// guards against is silent: a second daemon overwrites the first's record,
    /// both keep capturing, and the two sessions interleave sequences into one
    /// timeline with nothing saying so. Checking the published port ANSWERS is
    /// what distinguishes a live daemon from a stale record left by a crash —
    /// a crashed daemon's file still names a port.
    pub fn live_peer_port(&self) -> Option<u16> {
        let port = self.store.load().ok()??.port?;
        crate::dayflow::client::DaemonClient::new(port)
            .get("/dayflow/status")
            .ok()
            .map(|_| port)
    }

    /// Publish the port this daemon serves on, so surfaces can attach.
    pub fn publish_port(&self, port: u16) -> Result<(), DayflowError> {
        let _w = self.write.lock().unwrap_or_else(|p| p.into_inner());
        let Some(mut state) = self.store.load()? else {
            return Err(DayflowError::NoActiveSession);
        };
        state.port = Some(port);
        self.store.save(&state)
    }

    /// Record the interruption a restart implies.
    ///
    /// A resumed session that simply carried on would show an unexplained hole:
    /// no entries, no gap, indistinguishable from a quiet afternoon. The gap
    /// says capture STOPPED and why — the same distinction 013 drew between an
    /// absence and a recorded pause (FR-032).
    pub fn record_interruption(
        &self,
        session_id: Uuid,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<(), DayflowError> {
        // Through the store's EXISTING pause seam, not a second gap path: a
        // gap IS a recorded pause read back, and a parallel writer is how the
        // two representations drift.
        self.service.record_pause(
            session_id,
            &crate::dayflow::window::PauseInterval {
                from,
                to: Some(to),
                cause: crate::dayflow::window::PauseCause::DaemonRestart,
            },
        )
    }

    /// Stop the session and clear the state, so the next start is fresh.
    pub fn stop(&self, now: DateTime<Utc>) -> Result<usize, DayflowError> {
        self.service.stop_capture()?;
        let closed = self.service.stop(now)?;
        let _w = self.write.lock().unwrap_or_else(|p| p.into_inner());
        self.store.clear()?;
        Ok(closed.len())
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
