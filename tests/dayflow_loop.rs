//! The capture driver: sequencing and timing.
//!
//! These tests deliberately do NOT re-assert policy. Windowing, gating, budget
//! refusal and retry order each have their own tests; asserting them again here
//! would pass while the loop bypassed the component entirely (013/R29). What is
//! tested here is what only the loop can get wrong: the ORDER it does things
//! in, and what it does when a source fails.

use chrono::{DateTime, Duration, TimeZone, Utc};
use gentle_eye::dayflow::capture_loop::CaptureLoop;
use gentle_eye::config::{DayflowConfig, DeltaConfig};
use gentle_eye::dayflow::engine::DayflowRun;
use gentle_eye::dayflow::models::DayflowMode;
use gentle_eye::dayflow::sampler::{DropReason, Sampler};
use gentle_eye::dayflow::source::{
    Availability, CaptureSource, SourceError, SourceFrame, SourceIdentity,
};
use gentle_eye::regions::Region;

/// A source under the test's control: it yields the frames it is given, in
/// order, and reports the availability the test sets.
struct FakeSource {
    ordinal: u32,
    /// Each entry is one tick's answer: Some(distinct pixel value) or a failure.
    script: Vec<Option<u8>>,
    cursor: usize,
    availability: Availability,
    regions: Option<Vec<Region>>,
    /// Every frame this source was actually asked for, in order.
    asked: Vec<usize>,
}

impl FakeSource {
    fn new(ordinal: u32, script: Vec<Option<u8>>) -> Self {
        Self {
            ordinal,
            script,
            cursor: 0,
            availability: Availability::Available,
            regions: Some(Vec::new()),
            asked: Vec::new(),
        }
    }

    fn ends_when_exhausted(mut self) -> Self {
        self.availability = Availability::Ended;
        self
    }

    fn with_no_cascade(mut self) -> Self {
        self.regions = None;
        self
    }
}

impl CaptureSource for FakeSource {
    fn next_frame(&mut self) -> Result<SourceFrame, SourceError> {
        let i = self.cursor;
        self.cursor += 1;
        self.asked.push(i);
        match self.script.get(i).copied().flatten() {
            // TEXTURED, not a uniform fill: the content gate rejects a
            // featureless frame as Blank, so a flat colour would be skipped and
            // never reach disk — a fixture that cannot produce the condition
            // any path assertion names.
            Some(v) => {
                const W: usize = 32;
                const H: usize = 32;
                let mut bgra = vec![0u8; W * H * 4];
                for (i, px) in bgra.chunks_mut(4).enumerate() {
                    let x = (i % W) as u8;
                    let y = (i / W) as u8;
                    px[0] = x.wrapping_mul(7).wrapping_add(v);
                    px[1] = y.wrapping_mul(11).wrapping_add(v);
                    px[2] = (x ^ y).wrapping_mul(3).wrapping_add(v);
                    px[3] = 255;
                }
                Ok(SourceFrame { bgra, width: W as u32, height: H as u32 })
            }
            None => Err(SourceError::new("scripted failure")),
        }
    }
    fn regions_for(&self, _f: &SourceFrame) -> Option<Vec<Region>> {
        self.regions.clone()
    }
    fn availability(&self) -> Availability {
        // Available while frames remain; the scripted state once exhausted.
        if self.cursor < self.script.len() && self.script[self.cursor.min(self.script.len() - 1)].is_some() {
            Availability::Available
        } else {
            self.availability
        }
    }
    fn identity(&self) -> SourceIdentity {
        SourceIdentity::new("fake", self.ordinal.to_string())
    }
    fn ordinal(&self) -> u32 {
        self.ordinal
    }
}

fn at(secs: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_800_000_000 + secs, 0).unwrap()
}

fn run_for(displays: Vec<u32>) -> (DayflowRun, DayflowConfig) {
    let cfg = DayflowConfig::default();
    let run = DayflowRun::start(&cfg, DayflowMode::Session, displays, at(0)).expect("run starts");
    (run, cfg)
}

/// Every live source is asked exactly once per tick, in source order.
#[test]
fn one_tick_asks_every_live_source_once() {
    let (mut run, _cfg) = run_for(vec![0, 1]);
    let dir = tempfile::tempdir().unwrap();
    let sources: Vec<Box<dyn CaptureSource>> = vec![
        Box::new(FakeSource::new(0, vec![Some(10), Some(20)])),
        Box::new(FakeSource::new(1, vec![Some(30), Some(40)])),
    ];
    let mut lp = CaptureLoop::new(sources, Sampler::new(DeltaConfig::default()), dir.path());

    let t1 = lp.tick(&mut run, at(60));
    assert_eq!(t1.sources.len(), 2, "both sources asked");
    assert_eq!(
        t1.sources.iter().map(|s| s.ordinal).collect::<Vec<_>>(),
        vec![0, 1],
        "asked in source order"
    );
    assert_eq!(t1.frames_taken(), 2);

    let t2 = lp.tick(&mut run, at(120));
    assert_eq!(t2.frames_taken(), 2, "the second tick asks again");
}

/// A source failing does not end the tick — the others are still asked. This is
/// the property that makes a multi-source session survive one bad screen.
#[test]
fn a_failing_source_does_not_stop_the_others() {
    let (mut run, _cfg) = run_for(vec![0, 1, 2]);
    let dir = tempfile::tempdir().unwrap();
    let sources: Vec<Box<dyn CaptureSource>> = vec![
        Box::new(FakeSource::new(0, vec![Some(10)])),
        Box::new(FakeSource::new(1, vec![None])), // fails
        Box::new(FakeSource::new(2, vec![Some(30)])),
    ];
    let mut lp = CaptureLoop::new(sources, Sampler::new(DeltaConfig::default()), dir.path());

    let t = lp.tick(&mut run, at(60));
    assert_eq!(t.sources.len(), 3, "the failure did not abort the tick");
    assert_eq!(t.frames_taken(), 2, "the two healthy sources produced");
    assert!(t.sources[1].failure.is_some(), "source 1 recorded a failure");
    assert!(t.sources[1].record.is_none());

    // A per-source failure is a DROP, not a gap (D014-9).
    let drops = lp.drops();
    assert_eq!(drops.len(), 1, "exactly one drop");
    assert_eq!(drops[0].display_id, 1);
    assert_eq!(drops[0].reason, DropReason::SourceUnavailable);

    // And the RUN must know about the hole: the sampler's ledger has to be
    // synced into the run each tick, or `status` liveness reports zero
    // frames_dropped forever while sources fail every interval — the
    // false-green `sync_drops_from`'s own doc names, which is what happened
    // until the W4 gate connected it.
    assert_eq!(
        run.frames_dropped(),
        1,
        "the drop was recorded in the sampler but never synced into the run"
    );
}

/// An ENDED source is retired and never asked again; an OCCLUDED one is retried.
/// Conflating them spins forever on a window that is gone.
#[test]
fn an_ended_source_is_retired_but_an_occluded_one_is_retried() {
    let (mut run, _cfg) = run_for(vec![0, 1]);
    let dir = tempfile::tempdir().unwrap();
    let sources: Vec<Box<dyn CaptureSource>> = vec![
        Box::new(FakeSource::new(0, vec![None]).ends_when_exhausted()),
        Box::new(FakeSource::new(1, vec![None])), // stays Occluded
    ];
    let mut lp = CaptureLoop::new(sources, Sampler::new(DeltaConfig::default()), dir.path());

    let t1 = lp.tick(&mut run, at(60));
    assert_eq!(t1.retired, vec![0], "the ended source was retired");
    assert_eq!(lp.active_ordinals(), vec![1], "only the occluded one remains live");

    let t2 = lp.tick(&mut run, at(120));
    assert_eq!(
        t2.sources.iter().map(|s| s.ordinal).collect::<Vec<_>>(),
        vec![1],
        "the retired source is not asked again"
    );
    assert!(t2.retired.is_empty(), "an occluded source is not retired");
}

/// A source with no cascade must be COUNTED as a whole-frame read, not merely
/// survived. If this returned 0 the degradation would be invisible (FR-103).
#[test]
fn a_source_with_no_cascade_is_counted_as_a_whole_frame_read() {
    let (mut run, _cfg) = run_for(vec![0, 1]);
    let dir = tempfile::tempdir().unwrap();
    let sources: Vec<Box<dyn CaptureSource>> = vec![
        Box::new(FakeSource::new(0, vec![Some(10)])), // has a cascade
        Box::new(FakeSource::new(1, vec![Some(30)]).with_no_cascade()),
    ];
    let mut lp = CaptureLoop::new(sources, Sampler::new(DeltaConfig::default()), dir.path());

    let t = lp.tick(&mut run, at(60));
    assert_eq!(t.frames_taken(), 2);
    assert_eq!(
        t.whole_frame_reads(),
        1,
        "exactly the cascade-less source counts as a whole-frame read"
    );
    assert!(t.sources[0].regions.is_some(), "a cascade that ran reports Some");
    assert!(t.sources[1].regions.is_none(), "no cascade reports None, not an empty vec");
}

/// TIMING: the loop drives time from its parameter, and a window closes because
/// simulated time advanced past the segment length — not because wall-clock did.
/// A rule with the clock inside the function is undefended by construction
/// (013/R36); this test is only reachable because `tick` takes `now`.
#[test]
fn windows_close_on_injected_time_not_wall_clock() {
    let (mut run, cfg) = run_for(vec![0]);
    let dir = tempfile::tempdir().unwrap();
    let script: Vec<Option<u8>> = (0..40).map(|i| Some((i as u8).wrapping_mul(90))).collect();
    let sources: Vec<Box<dyn CaptureSource>> = vec![Box::new(FakeSource::new(0, script))];
    let mut lp = CaptureLoop::new(sources, Sampler::new(DeltaConfig::default()), dir.path());

    let segment = cfg.segment_seconds as i64;
    assert!(segment > 0);

    let started = std::time::Instant::now();
    let mut closed = Vec::new();
    // Advance well past two segments in simulated time.
    let step = segment / 4;
    for k in 1..=12 {
        let t = lp.tick(&mut run, at(step * k));
        closed.extend(t.closed);
    }
    assert!(
        !closed.is_empty(),
        "no window closed after {} simulated seconds (segment = {segment}s)",
        step * 12
    );
    // The real clock barely moved: the closes came from the injected time.
    assert!(
        started.elapsed() < std::time::Duration::from_secs(segment as u64),
        "the test consumed real wall-clock comparable to a segment — time is not injected"
    );

    // Sequences advance monotonically across closes.
    let seqs: Vec<u64> = closed.iter().map(|w| w.sequence).collect();
    let mut sorted = seqs.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(seqs, sorted, "sequences must be strictly increasing: {seqs:?}");
}

/// A sample belongs to the window that was OPEN when it was taken, not to the
/// one that opens because it closed. Reading the sequence after `on_sample`
/// would silently file the boundary sample under the wrong window.
#[test]
fn the_boundary_sample_belongs_to_the_window_it_closed() {
    let (mut run, cfg) = run_for(vec![0]);
    let dir = tempfile::tempdir().unwrap();
    // Alternating black/white: every consecutive pair is maximally different, so
    // the delta gate keeps each one and the boundary sample reaches disk.
    let script: Vec<Option<u8>> = (0..10).map(|i| Some((i as u8).wrapping_mul(90))).collect();
    let sources: Vec<Box<dyn CaptureSource>> = vec![Box::new(FakeSource::new(0, script))];
    let mut lp = CaptureLoop::new(sources, Sampler::new(DeltaConfig::default()), dir.path());

    let segment = cfg.segment_seconds as i64;
    lp.tick(&mut run, at(1));
    assert_eq!(run.current_sequence(0), 0, "the first window is sequence 0");

    // A tick past the segment boundary closes window 0. The assertion is on the
    // SAMPLE's sequence, not the ClosedWindow's: the window's sequence is
    // assigned by the engine and is already covered by the engine's own tests,
    // so asserting it here would pass no matter when the loop read the sequence
    // — it would be testing the wrong layer (013/R29).
    let t = lp.tick(&mut run, at(segment + Duration::seconds(1).num_seconds()));
    let closed = t.closed.first().expect("the boundary tick closed a window");
    assert_eq!(closed.sequence, 0, "the engine closed window 0");

    let record = t.sources[0].record.as_ref().expect("the boundary tick took a sample");
    assert_eq!(
        record.sequence, closed.sequence,
        "the boundary sample must be filed under the window it CLOSED ({}), not its \
         successor — reading the sequence after on_sample files it one window late",
        closed.sequence
    );
    let path = record.path.as_ref().expect("the boundary sample was kept");
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    assert!(
        name.starts_with(&gentle_eye::dayflow::sampler::sample_prefix(0, 0)),
        "the sample on disk is named for the wrong window: {name}"
    );
    assert_eq!(run.current_sequence(0), 1, "the next window is sequence 1");
}

// ── T008: summarisation runs DURING the session ───────────────────────────────

use gentle_eye::contracts::errors::DayflowError;
use gentle_eye::dayflow::models::{ChunkRef, ChunkSummary, RollingContext};
use gentle_eye::dayflow::summarizer::ChunkSummarizer;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Fails its first `fail_first` calls, then succeeds. Proves retry-never-drop.
struct FlakySummarizer {
    calls: AtomicUsize,
    fail_first: usize,
}

#[async_trait::async_trait]
impl ChunkSummarizer for FlakySummarizer {
    async fn summarize_chunk(
        &self,
        chunk: &ChunkRef,
        _prior: &RollingContext,
    ) -> Result<ChunkSummary, DayflowError> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        if n < self.fail_first {
            return Err(DayflowError::Invalid("model unavailable".into()));
        }
        Ok(ChunkSummary {
            chunk_index: chunk.index,
            start_time: chunk.start_wall,
            end_time: chunk.end_wall,
            category: gentle_eye::dayflow::models::ActivityCategory::Other,
            app: "fake".into(),
            activity: format!("window {}", chunk.sequence),
            detail: String::new(),
        })
    }
}

/// Entries appear WHILE the session is still running, not only after stop.
#[tokio::test]
async fn entries_appear_during_the_run_not_only_after_stop() {
    let (mut run, cfg) = run_for(vec![0]);
    let dir = tempfile::tempdir().unwrap();
    let script: Vec<Option<u8>> = (0..40).map(|i| Some((i as u8).wrapping_mul(90))).collect();
    let sources: Vec<Box<dyn CaptureSource>> = vec![Box::new(FakeSource::new(0, script))];
    let mut lp = CaptureLoop::new(sources, Sampler::new(DeltaConfig::default()), dir.path());

    let summarizer = FlakySummarizer { calls: AtomicUsize::new(0), fail_first: 0 };
    let rec = uuid::Uuid::new_v4();
    let segment = cfg.segment_seconds as i64;

    // Tick past one segment boundary so a window closes...
    let mut closed_any = false;
    let mut t = 0i64;
    for _ in 0..8 {
        t += segment / 3;
        if !lp.tick(&mut run, at(t)).closed.is_empty() {
            closed_any = true;
            break;
        }
    }
    assert!(closed_any, "no window closed within the simulated span");

    // ...and settle it WITHOUT stopping the session.
    let entries = lp.settle_due(&summarizer, rec, at(t)).await;
    assert!(
        !entries.is_empty(),
        "a closed window produced no entry while the session was still running"
    );
    // The session is still live: more ticks still work.
    let after = lp.tick(&mut run, at(t + segment / 3));
    assert_eq!(after.frames_taken(), 1, "the session kept capturing after summarising");
}

/// A failed summary is requeued and never marked summarised.
#[tokio::test]
async fn a_failed_summary_is_requeued_and_not_marked_summarised() {
    let (mut run, cfg) = run_for(vec![0]);
    let dir = tempfile::tempdir().unwrap();
    let script: Vec<Option<u8>> = (0..40).map(|i| Some((i as u8).wrapping_mul(90))).collect();
    let sources: Vec<Box<dyn CaptureSource>> = vec![Box::new(FakeSource::new(0, script))];
    let mut lp = CaptureLoop::new(sources, Sampler::new(DeltaConfig::default()), dir.path());

    let segment = cfg.segment_seconds as i64;
    let mut t = 0i64;
    for _ in 0..8 {
        t += segment / 3;
        if !lp.tick(&mut run, at(t)).closed.is_empty() {
            break;
        }
    }

    // First attempt fails.
    let flaky = FlakySummarizer { calls: AtomicUsize::new(0), fail_first: 1 };
    let entries = lp.settle_due(&flaky, uuid::Uuid::new_v4(), at(t)).await;
    assert!(entries.is_empty(), "a failed summary must not produce an entry");
    assert_eq!(lp.scheduler().pending(), 1, "the window was requeued, not dropped");
    assert_eq!(lp.scheduler().settled_count(), 0, "it must NOT be marked summarised");

    // After the backoff, the retry succeeds and the entry appears.
    let later = at(t + 3600);
    let entries = lp.settle_due(&flaky, uuid::Uuid::new_v4(), later).await;
    assert_eq!(entries.len(), 1, "the retry produced the entry");
    assert_eq!(lp.scheduler().pending(), 0);
    assert_eq!(lp.scheduler().settled_count(), 1);
}

// ── T009: the service actually runs the loop ─────────────────────────────────

use gentle_eye::dayflow::service::DayflowService;
use gentle_eye::dayflow::timeline::SqliteTimelineStore;

fn live_service() -> DayflowService {
    let store = Arc::new(SqliteTimelineStore::new(Arc::new(std::sync::Mutex::new(
        gentle_eye::storage::database::init_in_memory().expect("db"),
    ))));
    DayflowService::new(store, DayflowConfig::default())
}

fn scripted_sources() -> Box<dyn FnOnce() -> Vec<Box<dyn CaptureSource>> + Send> {
    Box::new(|| {
        let script: Vec<Option<u8>> =
            (0..500).map(|i| Some((i as u8).wrapping_mul(90))).collect();
        vec![Box::new(FakeSource::new(0, script)) as Box<dyn CaptureSource>]
    })
}

fn ok_summarizer() -> Arc<FlakySummarizer> {
    Arc::new(FlakySummarizer { calls: AtomicUsize::new(0), fail_first: 0 })
}

/// `start_capture` must RUN the loop, not merely spawn a thread that compiles.
/// The proof is observable state changing in the shared run.
#[test]
fn the_service_capture_thread_actually_drives_the_loop() {
    let svc = live_service();
    svc.start(DayflowMode::Session, vec![0], Utc::now()).expect("session starts");
    assert!(!svc.capture_running(), "no capture thread before start_capture");

    let dir = tempfile::tempdir().unwrap();
    svc.start_capture(
        scripted_sources(),
        ok_summarizer(),
        dir.path().to_path_buf(),
        std::time::Duration::from_millis(5),
    )
    .expect("capture starts");
    assert!(svc.capture_running(), "the handle is held while running");

    // Wait for observable evidence the thread ticked: samples on disk.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut seen = 0;
    while std::time::Instant::now() < deadline {
        seen = std::fs::read_dir(dir.path()).map(|d| d.count()).unwrap_or(0);
        if seen >= 2 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    svc.stop_capture().expect("capture stops");
    assert!(!svc.capture_running(), "the handle is cleared after stop");
    assert!(
        seen >= 2,
        "the capture thread wrote {seen} samples — it spawned but did not drive the loop"
    );
}

/// The capture thread must DRAIN the summary queue into the store, not merely
/// fill it. Without this a started session takes samples all day and no entry
/// ever appears (FR-014) — the queue is an orphan with a producer and no
/// consumer, which is precisely the wired-but-inert class (013/R29).
#[test]
fn the_capture_thread_summarises_closed_windows_into_the_store() {
    let svc = live_service();
    let t0 = Utc::now();
    svc.start(DayflowMode::Session, vec![0], t0).expect("session starts");
    // Windows must close within test time: shrink them to one second. This is
    // the engine's own FR-035 seam, not a test backdoor.
    svc.with_run(|r| r.set_interval(std::time::Duration::from_secs(1), t0))
        .expect("interval set");

    let dir = tempfile::tempdir().unwrap();
    svc.start_capture(
        scripted_sources(),
        ok_summarizer(),
        dir.path().to_path_buf(),
        std::time::Duration::from_millis(50),
    )
    .expect("capture starts");

    // An entry must appear WHILE the session is still running.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let mut entries = Vec::new();
    while std::time::Instant::now() < deadline {
        entries = svc
            .timeline(t0 - Duration::hours(1), Utc::now() + Duration::hours(1))
            .expect("timeline readable")
            .entries;
        if !entries.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let still = svc.status(Utc::now()).expect("status");
    assert!(still.running, "the session must still be running when the entry lands");
    svc.stop_capture().expect("capture stops");
    assert!(
        !entries.is_empty(),
        "windows closed but no timeline entry appeared — the capture thread \
         fills the summary queue and nothing drains it"
    );
}

/// A source that panics on its first frame — the panic fires INSIDE the tick,
/// while the capture thread holds the run mutex.
struct PanickingSource;
impl CaptureSource for PanickingSource {
    fn next_frame(&mut self) -> Result<SourceFrame, SourceError> {
        panic!("frame acquisition exploded");
    }
    fn regions_for(&self, _f: &SourceFrame) -> Option<Vec<Region>> {
        None
    }
    fn availability(&self) -> Availability {
        Availability::Available
    }
    fn identity(&self) -> SourceIdentity {
        SourceIdentity::new("panic", "0")
    }
    fn ordinal(&self) -> u32 {
        0
    }
}

/// D014-11: a panic mid-tick is a MULTI-STEP mutation dying under the run
/// mutex. It must not poison the lock (the service's poison recovery is sound
/// only for single-assignment mutations), must not kill the service, and must
/// not leave a zombie handle that blocks every later `start_capture`.
#[test]
fn a_panicking_tick_halts_capture_without_poisoning_the_service() {
    let svc = live_service();
    svc.start(DayflowMode::Session, vec![0], Utc::now()).expect("session starts");

    let dir = tempfile::tempdir().unwrap();
    svc.start_capture(
        Box::new(|| vec![Box::new(PanickingSource) as Box<dyn CaptureSource>]),
        ok_summarizer(),
        dir.path().to_path_buf(),
        std::time::Duration::from_millis(5),
    )
    .expect("capture starts");

    // The thread must halt itself after the panic.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while svc.capture_running() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(!svc.capture_running(), "a panicking tick must halt capture, not spin");

    // The run mutex must NOT be poisoned. This must be asserted through the
    // probe, not through `with_run` succeeding: `lock()` RECOVERS poison, so
    // every public call succeeds either way and would pass with the
    // catch_unwind deleted — a fixture unable to fail on the condition it
    // names (013's dominant defect class).
    assert!(
        !svc.run_lock_poisoned(),
        "the tick's panic unwound through the run guard — catch_unwind is not \
         protecting the multi-step mutation"
    );
    let status = svc.status(Utc::now()).expect("the service still answers");
    assert!(status.running, "the session itself survives the capture panic");
    let sid = svc.with_run(|r| r.session_id()).expect("the run is still serveable");
    assert_eq!(status.session_id, Some(sid));

    // The dead thread's handle must not block a restart with a healthy source.
    svc.start_capture(
        scripted_sources(),
        ok_summarizer(),
        dir.path().to_path_buf(),
        std::time::Duration::from_millis(5),
    )
    .expect("a finished thread's handle is reaped, not reported AlreadyRunning");
    svc.stop_capture().expect("capture stops");
}

/// `stop_capture` must interrupt the inter-tick wait, not ride it out. With a
/// flag checked only between sleeps, a coarse cadence (3-minute day interval)
/// makes `dayflow stop` block its caller for the whole interval.
#[test]
fn stop_capture_returns_promptly_despite_a_coarse_interval() {
    let svc = live_service();
    svc.start(DayflowMode::Session, vec![0], Utc::now()).expect("session starts");

    let dir = tempfile::tempdir().unwrap();
    svc.start_capture(
        scripted_sources(),
        ok_summarizer(),
        dir.path().to_path_buf(),
        std::time::Duration::from_secs(120),
    )
    .expect("capture starts");
    // Let the thread finish its first tick and enter the inter-tick wait.
    std::thread::sleep(std::time::Duration::from_millis(300));

    let asked = std::time::Instant::now();
    svc.stop_capture().expect("capture stops");
    let took = asked.elapsed();
    assert!(
        took < std::time::Duration::from_secs(10),
        "stop_capture took {took:?} against a 120s interval — the stop signal \
         does not interrupt the wait"
    );
    assert!(!svc.capture_running());
}

// ── T010: the region sidecar producer ────────────────────────────────────────

use gentle_eye::config::DayflowIntent;
use gentle_eye::contracts::traits::{AnalysisResult, TimeRange, VisionProvider};
use gentle_eye::contracts::errors::VisionError;
use gentle_eye::dayflow::perception::{
    dayflow_budget, regions_path, summarize_segment_via_ladder, PerceptionRouter, Tier,
};
use gentle_eye::regions::{Granularity, Source as RegionSource};
use gentle_eye::target::model::PixelRect;
use std::path::Path;

/// Counts calls per tier so "did it crop?" is answerable by arithmetic.
struct Counting {
    tier: Tier,
    calls: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl VisionProvider for Counting {
    async fn analyze_video(&self, _: &Path, _: &str, _: Option<TimeRange>) -> Result<AnalysisResult, VisionError> {
        panic!("dayflow samples frames; it must never analyse video")
    }
    async fn analyze_image(&self, _image: &Path, _prompt: &str) -> Result<AnalysisResult, VisionError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let text = match self.tier {
            Tier::Text => "screen text".to_string(),
            Tier::Reason => {
                r#"{"category":"coding","app":"editor","activity":"refactor","detail":"d"}"#.into()
            }
        };
        Ok(AnalysisResult {
            request_id: uuid::Uuid::new_v4(),
            analysis_text: text,
            provider: "counting".into(),
            model_used: "counting".into(),
            processing_time_ms: 1,
            token_count: None,
            completed_at: Utc::now(),
        })
    }
    async fn health_check(&self) -> Result<(), VisionError> { Ok(()) }
    fn name(&self) -> &'static str { "counting" }
    fn max_video_size(&self) -> u64 { 0 }
    fn supports_native_video(&self) -> bool { false }
    fn model(&self) -> &str { "counting" }
}

fn boxes(n: u32) -> Vec<Region> {
    // Distinct, non-overlapping strips inside the 32x32 fake frame.
    (0..n)
        .map(|i| {
            Region::new(
                PixelRect { x: 0, y: i * 8, w: 32, h: 8 },
                RegionSource::Wm,
                Granularity::Pane,
                1.0,
            )
            .on_display(0)
        })
        .collect()
}

/// The producer exists: a sidecar is written beside every KEPT sample, and the
/// ladder consequently takes CROPS — the text tier is called once per region
/// per sample, not once per sample. The consumer has existed since 013 with no
/// producer, and because that read fails open the absence was invisible.
#[tokio::test]
async fn the_loop_writes_sidecars_and_the_ladder_crops() {
    const REGIONS: u32 = 3;
    let (mut run, _cfg) = run_for(vec![0]);
    let dir = tempfile::tempdir().unwrap();
    let script: Vec<Option<u8>> = (0..6).map(|i| Some((i as u8).wrapping_mul(90))).collect();
    let mut src = FakeSource::new(0, script);
    src.regions = Some(boxes(REGIONS));
    let sources: Vec<Box<dyn CaptureSource>> = vec![Box::new(src)];
    let mut lp = CaptureLoop::new(sources, Sampler::new(DeltaConfig::default()), dir.path());

    // Drive a few ticks and collect the samples the gate actually kept.
    let mut samples = Vec::new();
    for k in 1..=3 {
        let t = lp.tick(&mut run, at(k * 30));
        for s in &t.sources {
            if let Some(p) = s.record.as_ref().and_then(|r| r.path.clone()) {
                samples.push(p);
            }
        }
    }
    assert!(!samples.is_empty(), "the gate kept no samples — nothing to crop");
    // Fixture honesty: with one sample the samples x regions arithmetic still
    // discriminates (3 vs 1), but only because REGIONS > 1 — pin BOTH factors
    // above 1 so a future edit to either cannot quietly make the crop
    // assertion unable to fail.
    assert!(
        samples.len() >= 2 && REGIONS >= 2,
        "fixture degraded: {} samples x {REGIONS} regions cannot prove cropping",
        samples.len()
    );

    // Every kept sample has its sidecar, and it round-trips.
    for p in &samples {
        let side = regions_path(p);
        assert!(side.exists(), "no sidecar beside {}", p.display());
        let parsed: Vec<Region> =
            serde_json::from_str(&std::fs::read_to_string(&side).unwrap()).expect("sidecar parses");
        assert_eq!(parsed.len(), REGIONS as usize, "sidecar lost regions");
    }

    // The ladder must now CROP: text calls = samples x regions, plus one reason call.
    let text = Arc::new(AtomicUsize::new(0));
    let reason = Arc::new(AtomicUsize::new(0));
    let router = PerceptionRouter::new(
        Arc::new(Counting { tier: Tier::Text, calls: Arc::clone(&text) }),
        Arc::new(Counting { tier: Tier::Reason, calls: Arc::clone(&reason) }),
        dayflow_budget(60, 1, 64),
    );
    let (_summary, latency) =
        summarize_segment_via_ladder(&router, &samples, "what happened?", 0, REGIONS as usize)
            .await
            .expect("ladder runs");

    assert_eq!(
        text.load(Ordering::SeqCst),
        samples.len() * REGIONS as usize,
        "the text tier was called once per SAMPLE, not once per region — the ladder \
         read whole frames, so the sidecar never reached it"
    );
    assert_eq!(reason.load(Ordering::SeqCst), 1, "the reasoning tier runs once per segment");
    assert_eq!(
        latency.samples_read_whole, 0,
        "no sample should have been read whole: every one had a sidecar"
    );
    let _ = DayflowIntent::Activity;
}

/// The converse, and the one that matters for honesty: a source with NO cascade
/// writes NO sidecar, and the ladder COUNTS the whole-frame read rather than
/// merely surviving it.
#[tokio::test]
async fn a_cascade_less_source_writes_no_sidecar_and_is_counted() {
    let (mut run, _cfg) = run_for(vec![0]);
    let dir = tempfile::tempdir().unwrap();
    let script: Vec<Option<u8>> = (0..6).map(|i| Some((i as u8).wrapping_mul(90))).collect();
    let sources: Vec<Box<dyn CaptureSource>> =
        vec![Box::new(FakeSource::new(0, script).with_no_cascade())];
    let mut lp = CaptureLoop::new(sources, Sampler::new(DeltaConfig::default()), dir.path());

    let mut samples = Vec::new();
    for k in 1..=3 {
        let t = lp.tick(&mut run, at(k * 30));
        for s in &t.sources {
            if let Some(p) = s.record.as_ref().and_then(|r| r.path.clone()) {
                samples.push(p);
            }
        }
    }
    assert!(!samples.is_empty());
    for p in &samples {
        assert!(
            !regions_path(p).exists(),
            "a source with no cascade must write NO sidecar — an empty one would claim \
             the cascade ran and found nothing, hiding the whole-frame read"
        );
    }

    let text = Arc::new(AtomicUsize::new(0));
    let reason = Arc::new(AtomicUsize::new(0));
    let router = PerceptionRouter::new(
        Arc::new(Counting { tier: Tier::Text, calls: Arc::clone(&text) }),
        Arc::new(Counting { tier: Tier::Reason, calls: Arc::clone(&reason) }),
        dayflow_budget(60, 1, 64),
    );
    let (_s, latency) =
        summarize_segment_via_ladder(&router, &samples, "what happened?", 0, 3)
            .await
            .expect("ladder runs");

    assert_eq!(text.load(Ordering::SeqCst), samples.len(), "one whole-frame read per sample");
    assert_eq!(
        latency.samples_read_whole,
        samples.len(),
        "the degradation must be COUNTED, not merely survived"
    );
}

// ── T011: the degradation is VISIBLE, not merely survived ────────────────────

/// A session whose source has no cascade reports a NON-ZERO whole-frame count
/// in `status`, through every surface.
///
/// The sidecar read fails open by design: a missing file is read as "no
/// regions" and the segment quietly summarises whole frames. Nothing errors and
/// every test passes — so a number someone can look at is the ONLY thing that
/// makes crop-before-extract's absence detectable (FR-103, 013/R29).
#[test]
fn a_cascade_less_session_reports_whole_frame_reads_on_every_surface() {
    use gentle_eye::dayflow::http;
    use gentle_eye::dayflow::service::DayflowService;
    use gentle_eye::dayflow::timeline::SqliteTimelineStore;

    let store = Arc::new(SqliteTimelineStore::new(Arc::new(std::sync::Mutex::new(
        gentle_eye::storage::database::init_in_memory().expect("db"),
    ))));
    let svc = DayflowService::new(store, DayflowConfig::default());
    svc.start(DayflowMode::Session, vec![0], Utc::now()).expect("session starts");

    // Baseline: nothing degraded yet.
    assert_eq!(svc.status(Utc::now()).unwrap().samples_read_whole, 0);

    let dir = tempfile::tempdir().unwrap();
    svc.start_capture(
        Box::new(|| {
            let script: Vec<Option<u8>> = (0..200)
                .map(|i| Some((i as u8).wrapping_mul(90)))
                .collect();
            vec![Box::new(FakeSource::new(0, script).with_no_cascade()) as Box<dyn CaptureSource>]
        }),
        Arc::new(FlakySummarizer { calls: AtomicUsize::new(0), fail_first: 0 }),
        dir.path().to_path_buf(),
        std::time::Duration::from_millis(5),
    )
    .expect("capture starts");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline
        && svc.status(Utc::now()).unwrap().samples_read_whole == 0
    {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    svc.stop_capture().expect("capture stops");

    let n = svc.status(Utc::now()).unwrap().samples_read_whole;
    assert!(n > 0, "a cascade-less session reported {n} whole-frame reads — the degradation is invisible");

    // Surface 1 — the service call the CLI makes.
    assert_eq!(svc.status(Utc::now()).unwrap().samples_read_whole, n);
    // Surface 2 — HTTP.
    let (code, body) = http::route("GET", "/dayflow/status", "", &svc);
    assert_eq!(code, "200 OK");
    let v: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
    assert_eq!(
        v["samples_read_whole"].as_u64(),
        Some(n),
        "HTTP status omitted the whole-frame count: {body}"
    );
    // Surface 3 — MCP serialises the same struct; assert the shape it emits.
    let mcp = serde_json::to_value(svc.status(Utc::now()).unwrap()).expect("serialises");
    assert_eq!(mcp["samples_read_whole"].as_u64(), Some(n));
}

// ── Gate W5: the sidecar must reach the PRODUCTION summariser, not only the
// test's hand-assembled path ──────────────────────────────────────────────────

/// The T010 test above collects sample paths itself and calls the ladder
/// directly. In production the summariser is `RoutedChunkSummarizer`, which
/// resolves a window's PNGs ITSELF by `sample_prefix` under its OWN
/// `sample_dir` — a second, independent directory parameter. This test drives
/// the REAL chain — loop writes samples + sidecars, `settle_due` hands due
/// windows to a real `RoutedChunkSummarizer` over the SAME dir — and proves by
/// arithmetic that the sidecars were found and the ladder CROPPED. If the loop
/// wrote sidecars anywhere the production summariser does not look, the
/// latencies report whole-frame reads and the call count collapses to
/// one-per-sample, and this fails.
#[tokio::test]
async fn the_production_summarizer_finds_the_loops_sidecars_and_crops() {
    use gentle_eye::dayflow::summarizer::RoutedChunkSummarizer;

    const REGIONS: u32 = 3;
    let (mut run, _cfg) = run_for(vec![0]);
    // Windows must close within the driven ticks (the engine's FR-035 seam).
    run.set_interval(std::time::Duration::from_secs(1), at(0));
    let dir = tempfile::tempdir().unwrap();
    let script: Vec<Option<u8>> = (0..6).map(|i| Some((i as u8).wrapping_mul(90))).collect();
    let mut src = FakeSource::new(0, script);
    src.regions = Some(boxes(REGIONS));
    let sources: Vec<Box<dyn CaptureSource>> = vec![Box::new(src)];
    let mut lp = CaptureLoop::new(sources, Sampler::new(DeltaConfig::default()), dir.path());
    let sid = run.session_id();

    for k in 1..=3 {
        lp.tick(&mut run, at(k * 30));
    }

    let text = Arc::new(AtomicUsize::new(0));
    let reason = Arc::new(AtomicUsize::new(0));
    let router = Arc::new(PerceptionRouter::new(
        Arc::new(Counting { tier: Tier::Text, calls: Arc::clone(&text) }),
        Arc::new(Counting { tier: Tier::Reason, calls: Arc::clone(&reason) }),
        dayflow_budget(60, 1, 64),
    ));
    // The SAME directory the loop wrote into — the invariant production wiring
    // must uphold, asserted here because nothing ties the two parameters
    // together structurally.
    let summarizer = RoutedChunkSummarizer::new(Arc::clone(&router), lp.sample_dir())
        .with_max_regions(REGIONS as usize);

    let entries = lp.settle_due(&summarizer, sid, at(100_000)).await;
    assert!(
        !entries.is_empty(),
        "no window settled through the production summariser — the real path was never exercised"
    );

    let latencies = summarizer.latencies();
    let settled: usize = latencies.iter().map(|l| l.samples).sum();
    assert!(settled >= 1, "the settled windows resolved no samples");
    assert_eq!(
        text.load(Ordering::SeqCst),
        settled * REGIONS as usize,
        "the production summariser read whole frames — the loop's sidecars never reached it"
    );
    assert_eq!(
        latencies.iter().map(|l| l.samples_read_whole).sum::<usize>(),
        0,
        "the ladder counted whole-frame reads despite the loop having written every sidecar"
    );
    assert_eq!(reason.load(Ordering::SeqCst), entries.len(), "one reasoning call per settled window");
}

/// An EMPTY cascade answer (`Some(vec![])` — "the cascade ran and found
/// nothing", D014-3) is NOT the degradation: the loop writes an honest empty
/// sidecar and counts nothing, and the ladder's per-segment counter agrees.
/// Before this gate the ladder counted every empty sidecar as a whole-frame
/// read while the status counter did not — two same-named numbers disagreeing
/// on every frame of an empty desktop.
#[tokio::test]
async fn an_empty_cascade_answer_is_counted_as_degradation_by_neither_counter() {
    let (mut run, _cfg) = run_for(vec![0]);
    let dir = tempfile::tempdir().unwrap();
    let script: Vec<Option<u8>> = (0..6).map(|i| Some((i as u8).wrapping_mul(90))).collect();
    // Default FakeSource: regions = Some(vec![]) — the cascade answers "nothing".
    let sources: Vec<Box<dyn CaptureSource>> = vec![Box::new(FakeSource::new(0, script))];
    let mut lp = CaptureLoop::new(sources, Sampler::new(DeltaConfig::default()), dir.path());

    let mut samples = Vec::new();
    for k in 1..=3 {
        let t = lp.tick(&mut run, at(k * 30));
        for s in &t.sources {
            if let Some(p) = s.record.as_ref().and_then(|r| r.path.clone()) {
                samples.push(p);
            }
        }
    }
    assert!(!samples.is_empty());

    // Capture side: an answered cascade is not a degradation.
    assert_eq!(lp.samples_read_whole(), 0, "an EMPTY answer must not count as a whole-frame read");
    // And the sidecar says so honestly: present, and empty.
    for p in &samples {
        let side = regions_path(p);
        assert!(side.exists(), "an answered cascade writes its (empty) answer");
        let parsed: Vec<Region> =
            serde_json::from_str(&std::fs::read_to_string(&side).unwrap()).unwrap();
        assert!(parsed.is_empty());
    }

    // Read side: the ladder reads the frames whole — correctly — and its
    // same-named counter must agree with the capture-side one.
    let text = Arc::new(AtomicUsize::new(0));
    let reason = Arc::new(AtomicUsize::new(0));
    let router = PerceptionRouter::new(
        Arc::new(Counting { tier: Tier::Text, calls: Arc::clone(&text) }),
        Arc::new(Counting { tier: Tier::Reason, calls: Arc::clone(&reason) }),
        dayflow_budget(60, 1, 64),
    );
    let (_s, latency) = summarize_segment_via_ladder(&router, &samples, "what happened?", 0, 3)
        .await
        .expect("ladder runs");
    assert_eq!(text.load(Ordering::SeqCst), samples.len(), "whole-frame reads, one per sample");
    assert_eq!(
        latency.samples_read_whole, 0,
        "the ladder counted an ANSWERED-empty cascade as degradation — it now disagrees \
         with the capture-side counter under the same name"
    );
}

/// The degradation counter is SESSION-scoped, like everything else in
/// `DayflowStatus`. Without a reset at `start`, session B's status reports
/// session A's whole-frame reads as its own, and an operator debugs a
/// degradation that is not there.
#[test]
fn a_new_session_does_not_inherit_the_old_sessions_whole_frame_count() {
    use gentle_eye::dayflow::service::DayflowService;
    use gentle_eye::dayflow::timeline::SqliteTimelineStore;

    let store = Arc::new(SqliteTimelineStore::new(Arc::new(std::sync::Mutex::new(
        gentle_eye::storage::database::init_in_memory().expect("db"),
    ))));
    let svc = DayflowService::new(store, DayflowConfig::default());
    svc.start(DayflowMode::Session, vec![0], Utc::now()).expect("session A starts");

    let dir = tempfile::tempdir().unwrap();
    svc.start_capture(
        Box::new(|| {
            let script: Vec<Option<u8>> =
                (0..200).map(|i| Some((i as u8).wrapping_mul(90))).collect();
            vec![Box::new(FakeSource::new(0, script).with_no_cascade()) as Box<dyn CaptureSource>]
        }),
        Arc::new(FlakySummarizer { calls: AtomicUsize::new(0), fail_first: 0 }),
        dir.path().to_path_buf(),
        std::time::Duration::from_millis(5),
    )
    .expect("capture starts");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline
        && svc.status(Utc::now()).unwrap().samples_read_whole == 0
    {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    svc.stop_capture().expect("capture stops");
    let n = svc.status(Utc::now()).unwrap().samples_read_whole;
    assert!(n > 0, "session A never degraded — the fixture proves nothing about inheritance");

    svc.stop(Utc::now()).expect("session A stops");
    svc.start(DayflowMode::Session, vec![0], Utc::now()).expect("session B starts");
    assert_eq!(
        svc.status(Utc::now()).unwrap().samples_read_whole,
        0,
        "session B inherited session A's degradation count ({n}) — status attributes \
         one session's whole-frame reads to another"
    );
}
