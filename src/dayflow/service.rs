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
    /// What this session is a record OF: each source's kind, name and current
    /// availability.
    ///
    /// A timeline is worthless if you cannot tell whether it recorded the whole
    /// desktop, one window, or a stream. `displays` alone cannot say: under
    /// D014-2 the ordinal occupies that field for EVERY source kind, so a
    /// window session and a display session are indistinguishable there.
    #[serde(default)]
    pub sources: Vec<SourceStatus>,
    /// Samples that will be read as WHOLE FRAMES because no usable region
    /// sidecar reached disk beside them.
    ///
    /// Surfaced because the path FAILS OPEN by design: a missing sidecar is
    /// read as "no regions" and the segment quietly summarises whole frames.
    /// Nothing errors, every test passes, and crop-before-extract is simply
    /// absent — so the only way this degradation is visible at all is a number
    /// someone can look at (FR-103, 013/R29).
    #[serde(default)]
    pub samples_read_whole: u64,
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
            sources: Vec::new(),
            samples_read_whole: 0,
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
    run: Arc<Mutex<Option<DayflowRun>>>,
    store: Arc<dyn TimelineStore + Send + Sync>,
    config: DayflowConfig,
    /// The running capture thread, when a session is driving itself.
    capture: Mutex<Option<CaptureHandle>>,
    /// Whole-frame reads, shared with the capture loop on its own thread.
    read_whole: Arc<std::sync::atomic::AtomicU64>,
    /// What the running session is capturing, republished by the capture thread
    /// each tick so availability is live rather than whatever it was at start.
    sources: Arc<Mutex<Vec<SourceStatus>>>,
}

/// One source in a running session, as a surface reports it.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct SourceStatus {
    /// Which kind — `display`, `window`, `target`, `input`.
    pub kind: String,
    /// The kind's own name for it: a display index, a window label, a target
    /// name, a stream URL.
    pub name: String,
    /// Its position in the session's source list (the `display_id` field).
    pub ordinal: u32,
    /// Whether it was producing AS OF THE LAST CAPTURE TICK.
    ///
    /// Republished each tick, so it is live while capture runs — but when the
    /// capture thread has halted (stop_capture, a panicked tick, the session
    /// ending under it) this is the last observed value, not a current claim.
    /// `liveness` is the staleness authority for that window: silence past the
    /// threshold reports the session Degraded whatever this field says.
    ///
    /// `None` means NO capture tick has reported yet — the session named this
    /// source but nothing has read from it. A distinct state on purpose:
    /// reporting `Available` there would claim a source is producing on the
    /// strength of the user having asked for it.
    pub availability: Option<crate::dayflow::source::Availability>,
}

/// A capture thread and its stop signal.
///
/// The loop is deliberately NOT `Send` (D014-10): platform capture handles are
/// thread-affine, so the sources are BUILT on the capture thread and never
/// cross it. The service therefore holds a control handle, not the loop.
///
/// The stop signal is a channel, not a flag: the thread WAITS on it between
/// ticks, so `stop_capture` interrupts the wait instead of blocking its caller
/// for a whole interval. With a flag + `sleep` a coarse 3-minute cadence made
/// `dayflow stop` hang for up to 3 minutes.
struct CaptureHandle {
    stop: std::sync::mpsc::Sender<()>,
    join: std::thread::JoinHandle<()>,
}

impl DayflowService {
    /// Build the service over a timeline store.
    pub fn new(store: Arc<dyn TimelineStore + Send + Sync>, config: DayflowConfig) -> Self {
        Self {
            run: Arc::new(Mutex::new(None)),
            store,
            config,
            capture: Mutex::new(None),
            read_whole: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            sources: Arc::new(Mutex::new(Vec::new())),
        }
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
        // A NEW session starts with a clean degradation ledger. The counter is
        // service-lifetime state while everything else in `DayflowStatus` is
        // session-scoped (session_id, started_at, displays, liveness) — without
        // this reset, session B's status reports session A's whole-frame reads
        // as its own, and an operator debugs a degradation that is not there.
        self.read_whole.store(0, std::sync::atomic::Ordering::SeqCst);
        // Same reason the counter resets: every other status field is
        // session-scoped, and a stale source list would name the PREVIOUS
        // session's window as this one's subject.
        self.sources.lock().unwrap_or_else(|p| p.into_inner()).clear();
        *guard = Some(run);
        Ok(id)
    }

    /// Start the capture loop for the running session.
    ///
    /// `build_sources` runs ON the capture thread: platform capture handles are
    /// thread-affine (D014-10), so a source built here and sent over would be
    /// unusable — and for scrap's X11 capturer, would not compile.
    ///
    /// The thread ticks on `interval`, driving the same pipeline the tests
    /// drive, and between ticks it settles due windows through `summarizer`,
    /// writing the resulting entries to the timeline store. That second half is
    /// not optional plumbing: without it a started session takes samples and
    /// closes windows into a queue nothing ever drains, and no entry appears
    /// all day (FR-014) — the exact "wired but inert" shape 013/R29 names.
    ///
    /// It stops when `stop_capture` is called, when the session ends, or when a
    /// tick panics (see the `catch_unwind` note in the body).
    ///
    /// `sample_dir` MUST be the directory the summariser resolves samples
    /// under. Nothing ties the two structurally: `RoutedChunkSummarizer` is
    /// constructed with its own directory and finds a window's PNGs (and their
    /// region sidecars) by prefix there, so a caller that passes different
    /// directories gets a loop that writes samples and sidecars where the
    /// summariser never looks — every segment then reads whole frames and
    /// `samples_read_whole` stays 0, because the capture side did its half
    /// correctly. The wiring test in `tests/dayflow_loop.rs`
    /// (`the_production_summarizer_finds_the_loops_sidecars_and_crops`) is the
    /// executable form of this sentence.
    pub fn start_capture(
        &self,
        build_sources: Box<dyn FnOnce() -> Vec<Box<dyn crate::dayflow::source::CaptureSource>> + Send>,
        summarizer: Arc<dyn crate::dayflow::summarizer::ChunkSummarizer + Send + Sync>,
        sample_dir: std::path::PathBuf,
        interval: std::time::Duration,
    ) -> Result<(), DayflowError> {
        let mut guard = self
            .capture
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        // Reap a thread that exited on its own (session ended, or a tick
        // panicked). Leaving it here would refuse every later start with
        // AlreadyRunning for a thread that is not running at all.
        if guard.as_ref().is_some_and(|h| h.join.is_finished()) {
            if let Some(h) = guard.take() {
                let _ = h.join.join();
            }
        }
        if guard.is_some() {
            return Err(DayflowError::AlreadyRunning);
        }
        let (stop, stopped) = std::sync::mpsc::channel::<()>();
        let delta = self.config.delta.clone();
        let run = Arc::clone(&self.run);
        let read_whole = Arc::clone(&self.read_whole);
        let sources_pub = Arc::clone(&self.sources);
        let store = Arc::clone(&self.store);
        let join = std::thread::spawn(move || {
            let sources = build_sources();
            let mut lp = crate::dayflow::capture_loop::CaptureLoop::new(
                sources,
                crate::dayflow::sampler::Sampler::new(delta),
                sample_dir,
            )
            // Share the SERVICE's counter: the loop lives on this thread and
            // `status()` is called from another, so a loop-private counter
            // would always read zero to every surface.
            .with_read_whole_counter(read_whole);
            // The summarizer is async (the perception ladder); this thread is
            // not. A current-thread runtime keeps the capture thread the sole
            // owner of the loop (D014-10) while still able to await it.
            let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(rt) => rt,
                Err(e) => {
                    tracing::error!(error = %e, "capture thread could not build a runtime; capture never starts");
                    return;
                }
            };
            loop {
                // `Utc::now()` appears HERE and nowhere below it. This is the
                // edge that supplies the clock; every decision downstream takes
                // `now` as a parameter, which is what makes the loop testable
                // at all (T007, 013/R36).
                let now = Utc::now();
                let session_id = {
                    let mut guard = match run.lock() {
                        Ok(g) => g,
                        Err(p) => p.into_inner(),
                    };
                    match guard.as_mut() {
                        Some(active) => {
                            let sid = active.session_id();
                            // D014-11: `on_sample` inside `tick` is a MULTI-STEP
                            // mutation, and `lock()`'s poison recovery is sound
                            // only while every mutation under the guard is a
                            // single atomic assignment. Rather than weaken that
                            // premise, restore it: a panic inside the tick is
                            // caught HERE, before it can unwind through the
                            // guard and poison the lock. Every interruption
                            // point in `on_sample` leaves the run valid-but-
                            // undercounting (each field write is atomic; the
                            // window map is consistent between statements), so
                            // continuing to serve the run is sound — the only
                            // loss is the in-flight ClosedWindow, which dies
                            // with the panicking frame regardless of locking.
                            let tick = std::panic::catch_unwind(
                                std::panic::AssertUnwindSafe(|| {
                                    lp.tick(active, now);
                                }),
                            );
                            // Republish what we are capturing, WITH live
                            // availability. Published every tick rather than at
                            // start: a window that minimised at 10am must not
                            // still read Available at 4pm.
                            {
                                let mut published =
                                    sources_pub.lock().unwrap_or_else(|p| p.into_inner());
                                *published = lp
                                    .describe()
                                    .into_iter()
                                    .map(|(id, ordinal, availability)| SourceStatus {
                                        kind: id.kind.to_string(),
                                        name: id.key,
                                        ordinal,
                                        availability: Some(availability),
                                    })
                                    .collect();
                            }
                            if tick.is_err() {
                                // Deliberate halt, not a retry: whatever made
                                // this tick panic will make the next one panic.
                                // Silence past the staleness threshold then
                                // reports the session Degraded.
                                tracing::error!(session = %sid,
                                    "capture tick panicked; capture halts, the session keeps answering");
                                return;
                            }
                            Some(sid)
                        }
                        // The session ended underneath us; stop rather than
                        // tick a dead run.
                        None => None,
                    }
                };
                let Some(sid) = session_id else { break };

                // Summarise due windows WITHOUT the run lock held: perception
                // is seconds-slow, and holding the lock through it would stall
                // every status/stop call for the duration.
                let entries = rt.block_on(lp.settle_due(summarizer.as_ref(), sid, now));
                if !entries.is_empty() {
                    for e in &entries {
                        // Best-effort like persist_pauses: a failed write is
                        // loud, and must not kill the recorder.
                        if let Err(err) = store.insert_entry(e) {
                            tracing::warn!(error = %err, session = %sid,
                                "could not persist a timeline entry; that slice will read as missing");
                        }
                    }
                    let mut guard = match run.lock() {
                        Ok(g) => g,
                        Err(p) => p.into_inner(),
                    };
                    if let Some(active) = guard.as_mut() {
                        active.note_summarized(now);
                    }
                }

                // Wait for the next tick OR a stop signal, whichever first.
                match stopped.recv_timeout(interval) {
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    // A signal, or the service dropped the sender: stop.
                    Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
        });
        *guard = Some(CaptureHandle { stop, join });
        Ok(())
    }

    /// Start a session for `spec`, naming its source in `status`.
    ///
    /// This is what the CLI, MCP and HTTP call. It does NOT start capture:
    /// driving the loop needs a summariser and a sample directory, which the
    /// daemon owns (T022). What it does guarantee is that `status` says WHAT
    /// the session is a record of, with availability `None` until something
    /// actually reads from the source — because reporting `Available` on the
    /// strength of the user having asked for it is a claim, not an observation.
    pub fn start_session(
        &self,
        mode: crate::dayflow::models::DayflowMode,
        spec: crate::dayflow::source::SourceSpec,
        now: DateTime<Utc>,
    ) -> Result<Uuid, DayflowError> {
        let ordinals = spec.ordinals();
        let ordinals = if ordinals.is_empty() {
            crate::capture::display::DisplayManager::list_available()
                .map_err(|e| DayflowError::Invalid(format!("enumerating displays: {e}")))?
                .iter()
                .map(|d| d.index as u32)
                .collect()
        } else {
            ordinals
        };
        let id = self.start(mode, ordinals.clone(), now)?;
        let (kind, name) = match &spec {
            crate::dayflow::source::SourceSpec::Displays { .. } => ("display", String::new()),
            crate::dayflow::source::SourceSpec::Window { label } => ("window", label.clone()),
            crate::dayflow::source::SourceSpec::Target { name } => ("target", name.clone()),
            crate::dayflow::source::SourceSpec::Input { url } => ("input", url.clone()),
        };
        let mut published = self.sources.lock().unwrap_or_else(|p| p.into_inner());
        *published = ordinals
            .iter()
            .map(|&ordinal| SourceStatus {
                kind: kind.to_string(),
                name: if name.is_empty() { ordinal.to_string() } else { name.clone() },
                ordinal,
                availability: None,
            })
            .collect();
        Ok(id)
    }

    /// Start a session AND its capture loop from one source spec.
    ///
    /// NOT yet called by any surface: the surfaces call `start_session`, which
    /// names the source but starts no capture, because driving the loop needs a
    /// summariser and a sample directory and those belong to the daemon (T022).
    /// This is the entry the daemon will call; it exists now because the two
    /// halves it joins (`start_session`, `start_capture`) are both live and the
    /// join has an invariant worth pinning early — see below. The W7 gate
    /// proves it end to end in `tests/dayflow_loop.rs`
    /// (`start_with_source_records_a_real_input_end_to_end`, `#[ignore]`d:
    /// it needs the ffmpeg binary).
    ///
    /// The spec crosses to the capture thread and the sources are built THERE:
    /// platform capture handles are thread-affine (D014-10), and this is the
    /// type system enforcing it rather than a comment asking.
    ///
    /// The invariant: "every display" is resolved to concrete indices ONCE,
    /// here, and the RESOLVED spec is what crosses to the thread. Resolving
    /// twice — once for the run's ordinals, once on the thread — lets the two
    /// enumerations disagree (a monitor unplugged in between), and then the
    /// run says displays [0,1,2] while the thread built a different set:
    /// samples file under ordinals no run window owns and `on_sample` drops
    /// them silently. `build_sources` refuses an unresolved spec outright.
    pub fn start_with_source(
        &self,
        mode: crate::dayflow::models::DayflowMode,
        spec: crate::dayflow::source::SourceSpec,
        summarizer: Arc<dyn crate::dayflow::summarizer::ChunkSummarizer + Send + Sync>,
        sample_dir: std::path::PathBuf,
        now: DateTime<Utc>,
    ) -> Result<Uuid, DayflowError> {
        let resolved = match &spec {
            crate::dayflow::source::SourceSpec::Displays { indices } if indices.is_empty() => {
                let indices = crate::capture::display::DisplayManager::list_available()
                    .map_err(|e| DayflowError::Invalid(format!("enumerating displays: {e}")))?
                    .iter()
                    .map(|d| d.index as u32)
                    .collect();
                crate::dayflow::source::SourceSpec::Displays { indices }
            }
            other => other.clone(),
        };
        // `start_session` derives the run's ordinals from the SAME resolved
        // spec the thread will build from, and names the sources in `status`
        // (availability `None` until the first tick republishes what the loop
        // actually observes).
        let id = self.start_session(mode, resolved.clone(), now)?;
        let interval = self
            .with_run(|r| r.sampling_interval(&self.config))
            .unwrap_or_else(|_| std::time::Duration::from_secs(60));
        let scratch = sample_dir.clone();
        if let Err(e) = self.start_capture(
            Box::new(move || {
                crate::dayflow::source::build_sources(&resolved, &scratch).unwrap_or_else(|err| {
                    // A source that cannot be built is a session that records
                    // nothing, and this closure has no error channel. What
                    // actually happens: the error is logged HERE, the loop
                    // ticks an empty source list, and silence past the
                    // staleness threshold reports the session Degraded — loud
                    // in the log and visible in health, but NOT an immediate
                    // failure of `start_with_source` itself.
                    tracing::error!(error = %err, "could not build the capture source");
                    Vec::new()
                })
            }),
            summarizer,
            sample_dir,
            interval,
        ) {
            // Do not leave a session claiming to run with no capture behind it.
            let _ = self.stop(now);
            return Err(e);
        }
        Ok(id)
    }

    /// Signal the capture thread to stop and wait for it.
    ///
    /// Returns promptly: the thread waits on the stop channel between ticks,
    /// so the signal interrupts the wait rather than expiring it. The join
    /// then only waits out an in-flight tick, never a full interval.
    pub fn stop_capture(&self) -> Result<(), DayflowError> {
        let handle = {
            let mut guard = self.capture.lock().unwrap_or_else(|p| p.into_inner());
            guard.take()
        };
        if let Some(h) = handle {
            // Send fails only when the thread already exited; join either way.
            let _ = h.stop.send(());
            let _ = h.join.join();
        }
        Ok(())
    }

    /// Whether a capture thread is currently running.
    ///
    /// Checks the thread, not the handle: a thread that exited on its own
    /// (session ended, tick panicked) leaves its handle behind until reaped,
    /// and reporting that as "running" is a false green.
    pub fn capture_running(&self) -> bool {
        self.capture
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .is_some_and(|h| !h.join.is_finished())
    }

    /// Stop the running session, returning the windows it closed.
    pub fn stop(&self, now: DateTime<Utc>) -> Result<Vec<crate::dayflow::window::ClosedWindow>, DayflowError> {
        let mut guard = self.lock()?;
        let mut run = guard.take().ok_or(DayflowError::NoActiveSession)?;
        let closed = run.stop(now);
        // The final durable record of the session's pauses — `stop` just closed
        // any open one, and the run is about to be dropped. Best-effort like
        // the mid-run flush: the closed windows still belong to the caller.
        Self::persist_pauses(self.store.as_ref(), &run);
        Ok(closed)
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
                sources: self
                    .sources
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .clone(),
                samples_read_whole: self.read_whole.load(std::sync::atomic::Ordering::SeqCst),
                liveness: Some(run.liveness(now)),
            },
        })
    }

    /// Timeline entries overlapping `[from, to)`, WITH the recorded gaps in it.
    ///
    /// One return type on purpose (T023): handing a surface the entries alone
    /// lets it render a paused hour as missing data, so the entries and the
    /// gaps travel together and every surface serialises both.
    pub fn timeline(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<crate::dayflow::timeline::TimelineSlice, DayflowError> {
        Ok(crate::dayflow::timeline::TimelineSlice {
            entries: self.store.query_range(from, to)?,
            gaps: self.store.query_gaps(from, to)?,
        })
    }

    /// The day, categorised (US7).
    ///
    /// Lives here rather than in each surface for the same reason
    /// [`resolve_range`] does: a digest computed differently per surface is a
    /// day that looks different depending on how you asked about it.
    pub fn standup(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<crate::dayflow::standup::Standup, DayflowError> {
        let entries = self.timeline(from, to)?.entries;
        Ok(crate::dayflow::standup::digest(&entries, from, to))
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
        let out = f(run);
        // Pause transitions happen inside `f` (tick_idle, turn_off/on), and the
        // engine is deliberately storage-free — so THIS seam is where they
        // become durable. Upsert-keyed on (session, start), so re-recording an
        // unchanged pause is idempotent, and an open pause is on disk the
        // moment it opens: a crash mid-pause leaves a recorded fact, not an
        // unexplained silence — the distinction the table exists for.
        Self::persist_pauses(self.store.as_ref(), run);
        Ok(out)
    }

    /// Best-effort durable record of a run's pauses.
    ///
    /// Best-effort BY DESIGN: `f`'s result (or `stop`'s closed windows) still
    /// belong to the caller, and failing capture because a bookkeeping write
    /// failed would trade a recoverable gap-in-the-gaps for a dead recorder.
    /// The failure is loud in the log, not silent.
    fn persist_pauses(store: &(dyn TimelineStore + Send + Sync), run: &DayflowRun) {
        let sid = run.session_id();
        for pause in run.pauses_seen() {
            if let Err(e) = store.record_pause(sid, pause) {
                tracing::warn!(error = %e, session = %sid,
                    "could not persist a pause interval; the gap will read as missing data");
            }
        }
    }

    /// Whether the run mutex has ever been poisoned.
    ///
    /// Observability for the D014-11 guarantee: the capture thread's multi-step
    /// tick must never poison this lock (its panics are caught at the tick
    /// boundary), so poison here can only mean a single-assignment mutation
    /// site panicked — the case `lock()`'s recovery is documented sound for.
    /// Without this probe the guarantee is untestable: the recovery itself
    /// masks poison from every public call path.
    pub fn run_lock_poisoned(&self) -> bool {
        self.run.is_poisoned()
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Option<DayflowRun>>, DayflowError> {
        // A poisoned lock means another thread panicked while holding it.
        // Recovering is safe HERE FOR A SPECIFIC REASON, not in general: every
        // mutation under this guard is a single atomic `Option` assignment
        // (`take` or assign), so a panic can only leave a fully-old or
        // fully-new value — never a torn invariant. Refusing every subsequent
        // call would turn one panic into a dead daemon that reports nothing.
        //
        // ⚠ THAT PROPERTY IS LOAD-BEARING AND NOTHING ENFORCES IT. If a future
        // change puts a MULTI-STEP mutation under this guard — set field A,
        // then field B — a panic between them leaves a half-updated run that
        // this recovery silently papers over, and the HTTP surface's
        // `catch_unwind` then turns it into a wrong-but-live answer. Keep
        // mutations here atomic, or make this return the error.
        //
        // The ONE multi-step mutation that exists — the capture thread's
        // `tick`, which calls `run.on_sample` (D014-11) — upholds the premise
        // from the other side: the thread wraps the tick in `catch_unwind`, so
        // a panic inside it is caught BEFORE it can unwind through the guard
        // and poison this lock. A poison observed here therefore still means a
        // single-assignment mutation site panicked, and the recovery argument
        // above holds unchanged.
        //
        // The timeline store deliberately does NOT recover: a panic during a
        // query fails loud with a 500 rather than serving stale data.
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
        assert_eq!(without_session.entries.len(), 1);

        s.start(DayflowMode::Session, vec![0], at(2_000)).unwrap();
        let with_session = s.timeline(at(0), at(1_000)).unwrap();
        assert_eq!(with_session.entries.len(), 1, "and the answer does not change");
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
