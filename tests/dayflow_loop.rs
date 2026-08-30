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

// ── T012/T013: watch one specific thing ──────────────────────────────────────

use gentle_eye::dayflow::source::window::{WindowLocator, WindowSource, WindowState};
use gentle_eye::dayflow::source::NamedTargetSource;
use gentle_eye::target::model::{NormRect, Target, TargetSource};
use std::sync::Mutex as StdMutex;

/// A locator the test drives: it answers whatever state the test set.
struct ScriptedLocator {
    state: Arc<StdMutex<WindowState>>,
}

impl WindowLocator for ScriptedLocator {
    fn locate(&self, _label: &str) -> WindowState {
        self.state.lock().unwrap().clone()
    }
}

/// A source that yields caller-supplied frames verbatim, so a test can control
/// exactly which PIXELS change between ticks.
struct ScriptedFrames {
    frames: Vec<SourceFrame>,
    cursor: usize,
    ordinal: u32,
}

impl CaptureSource for ScriptedFrames {
    fn next_frame(&mut self) -> Result<SourceFrame, SourceError> {
        let f = self
            .frames
            .get(self.cursor)
            .cloned()
            .ok_or_else(|| SourceError::new("script exhausted"))?;
        self.cursor += 1;
        Ok(f)
    }
    fn regions_for(&self, _f: &SourceFrame) -> Option<Vec<Region>> {
        Some(Vec::new())
    }
    fn availability(&self) -> Availability {
        if self.cursor < self.frames.len() { Availability::Available } else { Availability::Ended }
    }
    fn identity(&self) -> SourceIdentity {
        SourceIdentity::new("scripted", self.ordinal.to_string())
    }
    fn ordinal(&self) -> u32 {
        self.ordinal
    }
}

/// A 64x64 frame: textured everywhere, with the top-left 32x32 quadrant filled
/// from `inside` and the rest from `outside`.
fn quadrant_frame(inside: u8, outside: u8) -> SourceFrame {
    const W: usize = 64;
    const H: usize = 64;
    let mut bgra = vec![0u8; W * H * 4];
    for (i, px) in bgra.chunks_mut(4).enumerate() {
        let (x, y) = ((i % W) as u8, (i / W) as u8);
        let seed = if (x as usize) < 32 && (y as usize) < 32 { inside } else { outside };
        px[0] = x.wrapping_mul(7).wrapping_add(seed);
        px[1] = y.wrapping_mul(11).wrapping_add(seed);
        px[2] = (x ^ y).wrapping_mul(3).wrapping_add(seed);
        px[3] = 255;
    }
    SourceFrame { bgra, width: W as u32, height: H as u32 }
}

/// A window session records ONLY that window. A change elsewhere on the desktop
/// produces no sample — asserted on what reached disk, not on intent.
#[test]
fn a_change_outside_the_window_produces_no_sample() {
    let (mut run, _cfg) = run_for(vec![0]);
    let dir = tempfile::tempdir().unwrap();

    // Frame 1 establishes the baseline. Frames 2 and 3 change ONLY the outside.
    let frames = vec![
        quadrant_frame(10, 10),
        quadrant_frame(10, 200),
        quadrant_frame(10, 90),
    ];
    let inner = Box::new(ScriptedFrames { frames, cursor: 0, ordinal: 0 });
    let state = Arc::new(StdMutex::new(WindowState::Visible(PixelRect {
        x: 0,
        y: 0,
        w: 32,
        h: 32,
    })));
    let win = WindowSource::new(
        inner,
        Box::new(ScriptedLocator { state: Arc::clone(&state) }),
        "terminal",
        0,
    );
    let mut lp = CaptureLoop::new(vec![Box::new(win)], Sampler::new(DeltaConfig::default()), dir.path());

    let mut kept = Vec::new();
    for k in 1..=3 {
        let t = lp.tick(&mut run, at(k * 30));
        for s in &t.sources {
            if let Some(p) = s.record.as_ref().and_then(|r| r.path.clone()) {
                kept.push(p);
            }
        }
    }

    assert_eq!(
        kept.len(),
        1,
        "the desktop changed twice outside the window and produced {} samples — \
         the session is recording more than the window it was pointed at",
        kept.len()
    );

    // And the reason is the CONTENT, not luck: the crop really is 32x32.
    let img = image::open(&kept[0]).expect("the kept sample is a readable image");
    assert_eq!((img.width(), img.height()), (32, 32), "the sample is not the window's crop");
}

/// The converse: a change INSIDE the window is recorded. Without this, the test
/// above passes for a source that records nothing at all.
#[test]
fn a_change_inside_the_window_is_recorded() {
    let (mut run, _cfg) = run_for(vec![0]);
    let dir = tempfile::tempdir().unwrap();
    let frames = vec![
        quadrant_frame(10, 10),
        quadrant_frame(200, 10),
        quadrant_frame(90, 10),
    ];
    let inner = Box::new(ScriptedFrames { frames, cursor: 0, ordinal: 0 });
    let state = Arc::new(StdMutex::new(WindowState::Visible(PixelRect { x: 0, y: 0, w: 32, h: 32 })));
    let win = WindowSource::new(
        inner,
        Box::new(ScriptedLocator { state }),
        "terminal",
        0,
    );
    let mut lp = CaptureLoop::new(vec![Box::new(win)], Sampler::new(DeltaConfig::default()), dir.path());

    let mut kept = 0;
    for k in 1..=3 {
        for s in &lp.tick(&mut run, at(k * 30)).sources {
            if s.record.as_ref().and_then(|r| r.path.as_ref()).is_some() {
                kept += 1;
            }
        }
    }
    assert_eq!(kept, 3, "changes inside the window must be recorded, got {kept}");
}

/// A named target drives a session, and its identity is the target's NAME —
/// editing the rectangle must not split the day's record in two.
#[test]
fn a_named_target_drives_a_session_and_keeps_its_identity() {
    let (mut run, _cfg) = run_for(vec![0]);
    let dir = tempfile::tempdir().unwrap();
    let frames = vec![
        quadrant_frame(10, 10),
        quadrant_frame(200, 10),
    ];
    let inner = Box::new(ScriptedFrames { frames, cursor: 0, ordinal: 0 });
    // Top-left quarter, normalised.
    let target = Target::new(
        "qa-panel",
        TargetSource::Display { index: 0 },
        NormRect::new(0.0, 0.0, 0.5, 0.5),
    );
    let src = NamedTargetSource::new(inner, &target, 0);
    let before = src.identity();
    let mut lp = CaptureLoop::new(vec![Box::new(src)], Sampler::new(DeltaConfig::default()), dir.path());

    let mut kept = Vec::new();
    for k in 1..=2 {
        for s in &lp.tick(&mut run, at(k * 30)).sources {
            if let Some(p) = s.record.as_ref().and_then(|r| r.path.clone()) {
                kept.push(p);
            }
        }
    }
    assert!(!kept.is_empty(), "the target produced no samples");
    let img = image::open(&kept[0]).expect("readable");
    assert_eq!((img.width(), img.height()), (32, 32), "the target crop is not half the frame");

    // The same target with a MOVED rectangle is the same source.
    let moved = Target::new(
        "qa-panel",
        TargetSource::Display { index: 0 },
        NormRect::new(0.25, 0.25, 0.5, 0.5),
    );
    let after = NamedTargetSource::new(
        Box::new(ScriptedFrames { frames: vec![quadrant_frame(1, 1)], cursor: 0, ordinal: 0 }),
        &moved,
        0,
    )
    .identity();
    assert_eq!(before.hash(), after.hash(), "moving a target must not start a new identity");
}

// ── T014: three failure modes, three different outcomes ──────────────────────

/// Minimised, a transient inner failure, and quit must NOT collapse into one
/// another. Collapsing them makes a minimised window read as a fault, or a dead
/// source read as quiet-on-purpose (FR-113).
///
/// SCOPE NOTE: the DONE line said "three DIFFERENT gap causes". Per D014-9 a
/// per-source failure is a DROP, not a gap — a gap claims capture stopped,
/// which is false while other sources produce. So this asserts three different
/// OUTCOMES (availability, retry, and what is recorded), which is the same
/// distinction the DONE line was reaching for and is actually implementable.
#[test]
fn minimised_transient_and_quit_are_three_different_outcomes() {
    use gentle_eye::dayflow::window::PauseCause;

    let dir = tempfile::tempdir().unwrap();
    let rect = gentle_eye::target::model::PixelRect { x: 0, y: 0, w: 32, h: 32 };

    // ── minimised: occluded, retried, no gap-ending ──
    let (mut run, _c) = run_for(vec![0]);
    let state = Arc::new(StdMutex::new(WindowState::Minimised));
    let win = WindowSource::new(
        Box::new(ScriptedFrames { frames: vec![quadrant_frame(5, 5)], cursor: 0, ordinal: 0 }),
        Box::new(ScriptedLocator { state: Arc::clone(&state) }),
        "term",
        0,
    );
    let mut lp = CaptureLoop::new(vec![Box::new(win)], Sampler::new(DeltaConfig::default()), dir.path());
    let t = lp.tick(&mut run, at(30));
    let minimised = t.sources[0].failure.expect("minimised must fail the frame");
    assert_eq!(minimised, Availability::Occluded);
    assert!(t.retired.is_empty(), "a minimised window must NOT be retired — it will come back");
    assert_eq!(minimised.gap_cause(), Some(PauseCause::SourceOccluded));
    assert!(minimised.retryable());

    // It is still asked on the next tick, and RECOVERS when restored.
    *state.lock().unwrap() = WindowState::Visible(rect);
    let t2 = lp.tick(&mut run, at(60));
    assert!(t2.sources[0].failure.is_none(), "a restored window must produce again");

    // ── quit: ended, retired, never asked again ──
    let (mut run2, _c) = run_for(vec![0]);
    let gone = Arc::new(StdMutex::new(WindowState::Gone));
    let win2 = WindowSource::new(
        Box::new(ScriptedFrames { frames: vec![quadrant_frame(5, 5)], cursor: 0, ordinal: 0 }),
        Box::new(ScriptedLocator { state: gone }),
        "term",
        0,
    );
    let mut lp2 = CaptureLoop::new(vec![Box::new(win2)], Sampler::new(DeltaConfig::default()), dir.path());
    let q = lp2.tick(&mut run2, at(30));
    let quit = q.sources[0].failure.expect("a quit window must fail the frame");
    assert_eq!(quit, Availability::Ended);
    assert_eq!(q.retired, vec![0], "a quit window must be retired");
    assert_eq!(quit.gap_cause(), Some(PauseCause::SourceEnded));
    assert!(!quit.retryable());

    // ── transient inner failure: the window is fine, the capture hiccuped ──
    let (mut run3, _c) = run_for(vec![0]);
    let visible = Arc::new(StdMutex::new(WindowState::Visible(rect)));
    let win3 = WindowSource::new(
        // An exhausted script fails the frame while the window is still there.
        Box::new(ScriptedFrames { frames: Vec::new(), cursor: 0, ordinal: 0 }),
        Box::new(ScriptedLocator { state: visible }),
        "term",
        0,
    );
    let mut lp3 = CaptureLoop::new(vec![Box::new(win3)], Sampler::new(DeltaConfig::default()), dir.path());
    let tr = lp3.tick(&mut run3, at(30));
    let transient = tr.sources[0].failure.expect("a failed inner capture must fail the frame");
    assert_eq!(
        transient,
        Availability::Occluded,
        "an inner capture failure must not retire a window that is still on screen"
    );
    assert!(tr.retired.is_empty());

    // The three are genuinely distinguishable, not three names for one thing.
    assert_ne!(minimised.gap_cause(), quit.gap_cause());
    assert_ne!(minimised.retryable(), quit.retryable());
    // And a per-source failure is recorded as a DROP, never as a session gap.
    assert_eq!(lp3.drops().len(), 1);
    assert_eq!(lp3.drops()[0].reason, DropReason::SourceUnavailable);
}

// ── T015: status says WHAT the record is a record of ─────────────────────────

/// `status` names the session's source and its live availability, on every
/// surface. Without it a timeline cannot say whether it recorded the whole
/// desktop, one window, or a stream — and `displays` cannot answer, because
/// under D014-2 the ordinal occupies that field for every source kind.
#[test]
fn status_names_the_source_and_tracks_its_availability() {
    use gentle_eye::dayflow::http;
    use gentle_eye::dayflow::service::DayflowService;
    use gentle_eye::dayflow::timeline::SqliteTimelineStore;

    let store = Arc::new(SqliteTimelineStore::new(Arc::new(std::sync::Mutex::new(
        gentle_eye::storage::database::init_in_memory().expect("db"),
    ))));
    let svc = DayflowService::new(store, DayflowConfig::default());
    svc.start(DayflowMode::Session, vec![0], Utc::now()).expect("starts");
    assert!(svc.status(Utc::now()).unwrap().sources.is_empty(), "nothing captured yet");

    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(StdMutex::new(WindowState::Visible(
        gentle_eye::target::model::PixelRect { x: 0, y: 0, w: 32, h: 32 },
    )));
    let shared = Arc::clone(&state);
    svc.start_capture(
        Box::new(move || {
            let frames: Vec<SourceFrame> =
                (0..400).map(|i| quadrant_frame((i as u8).wrapping_mul(90), 7)).collect();
            vec![Box::new(WindowSource::new(
                Box::new(ScriptedFrames { frames, cursor: 0, ordinal: 0 }),
                Box::new(ScriptedLocator { state: shared }),
                "my-terminal",
                0,
            )) as Box<dyn CaptureSource>]
        }),
        Arc::new(FlakySummarizer { calls: AtomicUsize::new(0), fail_first: 0 }),
        dir.path().to_path_buf(),
        std::time::Duration::from_millis(5),
    )
    .expect("capture starts");

    // Wait until the thread has published what it is capturing.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline && svc.status(Utc::now()).unwrap().sources.is_empty() {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let named = svc.status(Utc::now()).unwrap();
    assert_eq!(named.sources.len(), 1, "status did not name the source");
    assert_eq!(named.sources[0].kind, "window", "status says the wrong KIND of record");
    assert_eq!(named.sources[0].name, "my-terminal", "status does not say WHICH window");
    assert_eq!(named.sources[0].availability, Some(Availability::Available));

    // Minimise it: availability must FOLLOW, not stay frozen at start.
    *state.lock().unwrap() = WindowState::Minimised;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut seen = Some(Availability::Available);
    while std::time::Instant::now() < deadline {
        seen = svc.status(Utc::now()).unwrap().sources[0].availability;
        if seen != Some(Availability::Available) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    svc.stop_capture().expect("stops");
    assert_eq!(
        seen,
        Some(Availability::Occluded),
        "status still reports the window Available after it was minimised — \
         availability is published once at start, not tracked"
    );

    // On every surface: HTTP and the serialised struct the MCP/CLI emit.
    let (code, body) = http::route("GET", "/dayflow/status", "", &svc);
    assert_eq!(code, "200 OK");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["sources"][0]["kind"], "window", "HTTP omitted the source: {body}");
    assert_eq!(v["sources"][0]["name"], "my-terminal");
    let mcp = serde_json::to_value(svc.status(Utc::now()).unwrap()).unwrap();
    assert_eq!(mcp["sources"][0]["name"], "my-terminal");
}

/// Regions come back in WINDOW-LOCAL coordinates. The frame handed downstream
/// is the crop, so a region left in screen coordinates addresses the wrong
/// pixels of it — and the crop still succeeds, so the wrong text is extracted
/// with nothing erroring anywhere.
#[test]
fn window_regions_are_translated_into_the_crops_coordinates() {
    /// An inner source reporting one region at a known SCREEN position.
    struct WithRegion;
    impl CaptureSource for WithRegion {
        fn next_frame(&mut self) -> Result<SourceFrame, SourceError> {
            Ok(quadrant_frame(3, 9))
        }
        fn regions_for(&self, _f: &SourceFrame) -> Option<Vec<Region>> {
            Some(vec![
                // Inside the window (window is 16,16 .. 48,48).
                Region::new(
                    gentle_eye::target::model::PixelRect { x: 20, y: 24, w: 8, h: 8 },
                    RegionSource::Wm,
                    Granularity::Pane,
                    1.0,
                ),
                // ENTIRELY outside (before the window's origin, no overlap):
                // nothing of it exists in the crop, so it must be dropped.
                Region::new(
                    gentle_eye::target::model::PixelRect { x: 0, y: 0, w: 4, h: 4 },
                    RegionSource::Wm,
                    Granularity::Pane,
                    1.0,
                ),
                // PARTIALLY overlapping: starts inside but overflows the far
                // edge (window is 16,16..48,48; this ends at 52). It must be
                // CLIPPED to the intersection, not dropped — WM frame geometry
                // and detected pane boxes routinely disagree by a border's
                // width, and containment-only filtering silently discarded the
                // whole pane for a one-pixel overhang.
                Region::new(
                    gentle_eye::target::model::PixelRect { x: 44, y: 44, w: 8, h: 8 },
                    RegionSource::Wm,
                    Granularity::Pane,
                    1.0,
                ),
            ])
        }
        fn availability(&self) -> Availability { Availability::Available }
        fn identity(&self) -> SourceIdentity { SourceIdentity::new("inner", "x") }
        fn ordinal(&self) -> u32 { 0 }
    }

    let rect = gentle_eye::target::model::PixelRect { x: 16, y: 16, w: 32, h: 32 };
    let win = WindowSource::new(
        Box::new(WithRegion),
        Box::new(ScriptedLocator { state: Arc::new(StdMutex::new(WindowState::Visible(rect))) }),
        "term",
        0,
    );
    let frame = quadrant_frame(3, 9);
    let regions = win.regions_for(&frame).expect("the inner source has a cascade");

    assert_eq!(
        regions.len(),
        2,
        "the inside region survives whole and the overflowing one survives \
         CLIPPED; only the region with no overlap at all is dropped"
    );
    assert_eq!(
        (regions[0].bbox.x, regions[0].bbox.y),
        (4, 8),
        "the region is still in SCREEN coordinates — cropping it out of the \
         window's frame would read the wrong pixels, silently"
    );
    assert_eq!((regions[0].bbox.w, regions[0].bbox.h), (8, 8), "a contained region keeps its size");
    // The overflowing region: intersection is 44,44..48,48 → window-local
    // 28,28 with the overhang cut off.
    assert_eq!(
        (regions[1].bbox.x, regions[1].bbox.y, regions[1].bbox.w, regions[1].bbox.h),
        (28, 28, 4, 4),
        "a partially overlapping region is clipped to the crop, in local coordinates"
    );
}

/// When the cascade answered but NOTHING it said intersects the crop, the
/// source must answer `None` — "I cannot answer for this crop" — never
/// `Some(vec![])`, which claims the cascade looked here and found nothing and
/// hides the whole-frame read from `samples_read_whole` (D014-3, FR-103).
/// This is the exact defect the W5 gate fixed in
/// `DisplaySource::select_regions`; the two cropping sources reintroduced it.
#[test]
fn a_crop_that_can_keep_no_region_answers_none_not_found_nothing() {
    struct FarRegions;
    impl CaptureSource for FarRegions {
        fn next_frame(&mut self) -> Result<SourceFrame, SourceError> {
            Ok(quadrant_frame(3, 9))
        }
        fn regions_for(&self, _f: &SourceFrame) -> Option<Vec<Region>> {
            // A real answer, entirely outside the 0,0..32,32 crop below.
            Some(vec![Region::new(
                gentle_eye::target::model::PixelRect { x: 40, y: 40, w: 8, h: 8 },
                RegionSource::Wm,
                Granularity::Pane,
                1.0,
            )])
        }
        fn availability(&self) -> Availability { Availability::Available }
        fn identity(&self) -> SourceIdentity { SourceIdentity::new("inner", "x") }
        fn ordinal(&self) -> u32 { 0 }
    }
    struct NoRegions;
    impl CaptureSource for NoRegions {
        fn next_frame(&mut self) -> Result<SourceFrame, SourceError> {
            Ok(quadrant_frame(3, 9))
        }
        fn regions_for(&self, _f: &SourceFrame) -> Option<Vec<Region>> {
            Some(Vec::new()) // the cascade ran and found nothing anywhere
        }
        fn availability(&self) -> Availability { Availability::Available }
        fn identity(&self) -> SourceIdentity { SourceIdentity::new("inner", "x") }
        fn ordinal(&self) -> u32 { 0 }
    }

    let rect = gentle_eye::target::model::PixelRect { x: 0, y: 0, w: 32, h: 32 };
    let frame = quadrant_frame(3, 9);

    // WindowSource: regions exist, none in the window → cannot answer.
    let win = WindowSource::new(
        Box::new(FarRegions),
        Box::new(ScriptedLocator { state: Arc::new(StdMutex::new(WindowState::Visible(rect))) }),
        "term",
        0,
    );
    assert_eq!(
        win.regions_for(&frame),
        None,
        "WindowSource turned 'nothing attributable to this crop' into \
         Some(vec![]) — the whole-frame read is hidden from the counter"
    );

    // NamedTargetSource: same rule, same seam.
    let target = Target::new(
        "panel",
        TargetSource::Display { index: 0 },
        NormRect::new(0.0, 0.0, 0.5, 0.5), // 0,0..32,32 of the 64x64 frame
    );
    let tgt = NamedTargetSource::new(Box::new(FarRegions), &target, 0);
    assert_eq!(
        tgt.regions_for(&frame),
        None,
        "NamedTargetSource turned 'nothing attributable to this crop' into \
         Some(vec![]) — the whole-frame read is hidden from the counter"
    );

    // And the OTHER empty stays an answer: an empty cascade result is
    // Some(vec![]) — the cascade ran, there was nothing to find, and that is
    // not a whole-frame read (D014-3 draws exactly this line).
    let win2 = WindowSource::new(
        Box::new(NoRegions),
        Box::new(ScriptedLocator { state: Arc::new(StdMutex::new(WindowState::Visible(rect))) }),
        "term",
        0,
    );
    assert_eq!(win2.regions_for(&frame), Some(Vec::new()));
    let tgt2 = NamedTargetSource::new(Box::new(NoRegions), &target, 0);
    assert_eq!(tgt2.regions_for(&frame), Some(Vec::new()));
}

/// A target's availability must follow its INNER source between frames.
/// `state` is only written inside `next_frame`, so without the passthrough an
/// inner source that went Occluded after the last frame kept reporting a stale
/// `Available` — while `WindowSource` re-asks its locator live. Two sources
/// answering the same trait question with different freshness is a defect, not
/// a style choice.
#[test]
fn a_targets_availability_follows_its_inner_source_between_frames() {
    struct MoodySource {
        mood: Availability,
    }
    impl CaptureSource for MoodySource {
        fn next_frame(&mut self) -> Result<SourceFrame, SourceError> {
            Err(SourceError::new("not the point of this test"))
        }
        fn regions_for(&self, _f: &SourceFrame) -> Option<Vec<Region>> {
            None
        }
        fn availability(&self) -> Availability {
            self.mood
        }
        fn identity(&self) -> SourceIdentity { SourceIdentity::new("inner", "x") }
        fn ordinal(&self) -> u32 { 0 }
    }

    let target = Target::new(
        "panel",
        TargetSource::Display { index: 0 },
        NormRect::new(0.0, 0.0, 0.5, 0.5),
    );
    // NO next_frame call happens in this test: the inner state must reach the
    // trait surface on its own.
    let occluded = NamedTargetSource::new(Box::new(MoodySource { mood: Availability::Occluded }), &target, 0);
    assert_eq!(
        occluded.availability(),
        Availability::Occluded,
        "the inner source is Occluded and the target reports a stale Available"
    );
    let ended = NamedTargetSource::new(Box::new(MoodySource { mood: Availability::Ended }), &target, 0);
    assert_eq!(ended.availability(), Availability::Ended);
    let fine = NamedTargetSource::new(Box::new(MoodySource { mood: Availability::Available }), &target, 0);
    assert_eq!(fine.availability(), Availability::Available);
}

/// The mirror of the target test above, for the window: `availability()` must
/// re-ask the LOCATOR, so a window that closed between frames is seen without
/// waiting for the next failed capture. Every in-loop caller happens to run
/// right after `next_frame` refreshed `state`, so only a direct between-frames
/// call can tell a live re-ask from a stale field (mutation M14 survived until
/// this test existed).
#[test]
fn a_windows_availability_follows_its_locator_between_frames() {
    let rect = gentle_eye::target::model::PixelRect { x: 0, y: 0, w: 32, h: 32 };
    let state = Arc::new(StdMutex::new(WindowState::Visible(rect)));
    let win = WindowSource::new(
        Box::new(ScriptedFrames { frames: vec![quadrant_frame(5, 5)], cursor: 0, ordinal: 0 }),
        Box::new(ScriptedLocator { state: Arc::clone(&state) }),
        "term",
        0,
    );
    // NO next_frame call in this test: the change must reach the trait surface
    // through the locator alone.
    assert_eq!(win.availability(), Availability::Available);
    *state.lock().unwrap() = WindowState::Minimised;
    assert_eq!(
        win.availability(),
        Availability::Occluded,
        "the window minimised between frames and availability still reports the stale state"
    );
    *state.lock().unwrap() = WindowState::Gone;
    assert_eq!(
        win.availability(),
        Availability::Ended,
        "the window closed between frames and availability still reports the stale state"
    );
}

/// The SESSION-WIDE arm of FR-113 (D014-9's second row): a watched window that
/// QUITS must leave a gap in the session's ledger saying why — not silence.
/// Before this wire existed, `Availability::gap_cause()` was called by nothing
/// outside tests, and the health mapping for `SourceEnded` (models.rs →
/// Degraded) was unreachable in production.
#[test]
fn a_watched_window_that_quits_leaves_a_gap_with_its_cause_not_silence() {
    use gentle_eye::dayflow::window::PauseCause;

    let (mut run, _cfg) = run_for(vec![0]);
    let dir = tempfile::tempdir().unwrap();
    let win = WindowSource::new(
        Box::new(ScriptedFrames { frames: vec![quadrant_frame(5, 5)], cursor: 0, ordinal: 0 }),
        Box::new(ScriptedLocator { state: Arc::new(StdMutex::new(WindowState::Gone)) }),
        "term",
        0,
    );
    let mut lp = CaptureLoop::new(vec![Box::new(win)], Sampler::new(DeltaConfig::default()), dir.path());

    lp.tick(&mut run, at(30));
    let pauses = run.pauses_seen();
    assert_eq!(pauses.len(), 1, "the session recorded no gap for its only source dying");
    assert_eq!(pauses[0].cause, PauseCause::SourceEnded, "the gap must carry the CAUSE");
    assert!(pauses[0].to.is_none(), "SourceEnded never lifts on its own — the source is retired");

    // Health reads the difference: quiet-on-purpose vs dead. This is the
    // models.rs SourceEnded→Degraded arm, reachable for the first time.
    assert_eq!(
        run.liveness(at(60)).health,
        gentle_eye::dayflow::models::DayflowHealth::Degraded,
        "a session whose only source is dead must not read as healthy or as paused-on-purpose"
    );

    // Idempotent: later ticks over the retired source add no second interval.
    lp.tick(&mut run, at(60));
    assert_eq!(run.pauses_seen().len(), 1, "re-ticking a dead session must not stack gaps");
}

/// The occluded half of the same arm: a minimised window pauses the session
/// with `SourceOccluded` (auto-lifting), and the FIRST frame after restore
/// both lifts the pause and is COUNTED — the resume must run before
/// `on_sample`, or the run is still paused when the recovered frame arrives
/// and silently uncounts it.
#[test]
fn a_minimised_window_pauses_the_session_and_recovery_counts_its_first_frame() {
    use gentle_eye::dayflow::window::PauseCause;

    let (mut run, _cfg) = run_for(vec![0]);
    let dir = tempfile::tempdir().unwrap();
    let rect = gentle_eye::target::model::PixelRect { x: 0, y: 0, w: 32, h: 32 };
    let state = Arc::new(StdMutex::new(WindowState::Minimised));
    let win = WindowSource::new(
        Box::new(ScriptedFrames {
            frames: vec![quadrant_frame(5, 5), quadrant_frame(80, 80)],
            cursor: 0,
            ordinal: 0,
        }),
        Box::new(ScriptedLocator { state: Arc::clone(&state) }),
        "term",
        0,
    );
    let mut lp = CaptureLoop::new(vec![Box::new(win)], Sampler::new(DeltaConfig::default()), dir.path());

    lp.tick(&mut run, at(30));
    assert_eq!(run.pauses_seen().len(), 1, "a minimised only-source must pause the session");
    assert_eq!(run.pauses_seen()[0].cause, PauseCause::SourceOccluded);
    assert!(run.pauses_seen()[0].to.is_none(), "still minimised — the pause is open");

    // Restore the window: the next tick produces, lifts the pause, and the
    // recovered frame itself is recorded.
    *state.lock().unwrap() = WindowState::Visible(rect);
    let t = lp.tick(&mut run, at(60));
    assert!(t.sources[0].failure.is_none(), "the restored window must produce");
    assert_eq!(
        run.pauses_seen()[0].to,
        Some(at(60)),
        "the pause must lift when the source recovers"
    );

    // The window that closes at stop STARTED at the recovery tick with a real
    // sample in it. If resume ran after `on_sample`, the run was paused when
    // the frame arrived, `on_sample` no-opped, and this window starts later
    // (or holds no sample).
    let closed = run.stop(at(90));
    let w = closed
        .iter()
        .find(|w| w.sample_count > 0)
        .expect("the recovered frame must land in a window — it was silently uncounted");
    assert_eq!(w.start_wall, at(60), "the window must open AT the recovery frame, not after it");
}

/// A frame arriving says NOTHING about the user's state: source recovery must
/// lift only a `SourceOccluded` pause, never an `Idle` one. Frames keep
/// changing while the user is away — that is the whole reason `Idle` exists —
/// so a loop that resumed any automatic pause on any frame would un-pause an
/// idle session on its very next tick, silently deleting idleness from the
/// record.
#[test]
fn a_frame_does_not_lift_an_idle_pause() {
    use gentle_eye::dayflow::window::PauseCause;

    let (mut run, _cfg) = run_for(vec![0]);
    let dir = tempfile::tempdir().unwrap();

    // Drive the idle detector over its threshold (300s) + dwell (30s): first
    // sighting arms the transition, the second fires it.
    let idle = Some(std::time::Duration::from_secs(400));
    let step = std::time::Duration::from_secs(30);
    run.tick_idle(idle, step, at(30));
    run.tick_idle(idle, step, at(60));
    assert_eq!(
        run.pauses_seen().last().map(|p| p.cause),
        Some(PauseCause::Idle),
        "fixture failure: the detector did not pause — the assertions below prove nothing"
    );

    // A producing source ticks while the user is idle.
    let src = ScriptedFrames { frames: vec![quadrant_frame(5, 5)], cursor: 0, ordinal: 0 };
    let mut lp = CaptureLoop::new(vec![Box::new(src)], Sampler::new(DeltaConfig::default()), dir.path());
    lp.tick(&mut run, at(90));

    assert!(
        run.pauses_seen().last().is_some_and(|p| p.to.is_none()),
        "a frame lifted the IDLE pause — recovery must be scoped to SourceOccluded"
    );
}

/// A retired source stays `Ended` in `describe()`, even when the source's OWN
/// availability flips back (a stream that reconnects, a window recreated under
/// the same label): the contract says an ended source is never restarted, so
/// the status surface must not advertise Available for a source the loop will
/// never ask again — that is a liveness lie in the other direction.
///
/// FIXTURE NOTE: a `WindowSource` cannot drive this — after a Gone failure its
/// internal state stays `Ended`, so its `availability()` reports Ended with or
/// without the retired override, and the first version of this test passed
/// while proving nothing (the class-1 defect, caught when mutation M10
/// survived it). The source here reports its availability from a shared cell,
/// so the "came back" condition genuinely reaches `describe()`.
#[test]
fn a_retired_source_is_described_as_ended_even_if_its_availability_recovers() {
    struct FlippableSource {
        avail: Arc<StdMutex<Availability>>,
    }
    impl CaptureSource for FlippableSource {
        fn next_frame(&mut self) -> Result<SourceFrame, SourceError> {
            Err(SourceError::new("down"))
        }
        fn regions_for(&self, _f: &SourceFrame) -> Option<Vec<Region>> {
            None
        }
        fn availability(&self) -> Availability {
            *self.avail.lock().unwrap()
        }
        fn identity(&self) -> SourceIdentity { SourceIdentity::new("flippable", "x") }
        fn ordinal(&self) -> u32 { 0 }
    }

    let (mut run, _cfg) = run_for(vec![0]);
    let dir = tempfile::tempdir().unwrap();
    let avail = Arc::new(StdMutex::new(Availability::Ended));
    let src = FlippableSource { avail: Arc::clone(&avail) };
    let mut lp = CaptureLoop::new(vec![Box::new(src)], Sampler::new(DeltaConfig::default()), dir.path());

    let t = lp.tick(&mut run, at(30));
    assert_eq!(t.retired, vec![0], "fixture failure: the ended source was not retired");

    // The source itself now claims to be fine again. The loop will still never
    // ask it — describe must say so.
    *avail.lock().unwrap() = Availability::Available;
    let described = lp.describe();
    assert_eq!(described[0].0.kind, "flippable");
    assert_eq!(
        described[0].2,
        Availability::Ended,
        "describe() reports a retired source by its live self-report — \
         status would say Available for a source the loop never asks"
    );
}

/// One occluded source AMONG PRODUCERS is a per-source drop, never a session
/// gap — a gap claims capture stopped, which is false while the other source
/// records (D014-9's first row, guarding the any/all boundary in the loop).
#[test]
fn one_occluded_source_among_producers_is_a_drop_not_a_session_gap() {
    let (mut run, _cfg) = run_for(vec![0, 1]);
    let dir = tempfile::tempdir().unwrap();
    let producing = ScriptedFrames { frames: vec![quadrant_frame(5, 5)], cursor: 0, ordinal: 0 };
    let minimised = WindowSource::new(
        Box::new(ScriptedFrames { frames: vec![quadrant_frame(5, 5)], cursor: 0, ordinal: 1 }),
        Box::new(ScriptedLocator { state: Arc::new(StdMutex::new(WindowState::Minimised)) }),
        "term",
        1,
    );
    let mut lp = CaptureLoop::new(
        vec![Box::new(producing), Box::new(minimised)],
        Sampler::new(DeltaConfig::default()),
        dir.path(),
    );

    let t = lp.tick(&mut run, at(30));
    assert!(t.sources[0].failure.is_none(), "source 0 produced");
    assert_eq!(t.sources[1].failure, Some(Availability::Occluded), "source 1 failed");
    assert!(
        run.pauses_seen().is_empty(),
        "one occluded source among producers must NOT pause the session — \
         that gap would claim the whole session stopped while source 0 records"
    );
    assert_eq!(lp.drops().len(), 1, "the failure is recorded per-source, as a drop");
}

// ── T016: an input taken, not a display consumed ─────────────────────────────

use gentle_eye::dayflow::source::input::{FrameGrabber, InputSource};

/// A grabber the test drives: yields frames, or fails on command.
struct ScriptedGrabber {
    script: Vec<Option<u8>>,
    cursor: usize,
}

impl FrameGrabber for ScriptedGrabber {
    fn grab(&mut self, _url: &str) -> Result<SourceFrame, SourceError> {
        let v = self.script.get(self.cursor).copied().flatten();
        self.cursor += 1;
        match v {
            Some(seed) => Ok(quadrant_frame(seed, seed.wrapping_add(40))),
            None => Err(SourceError::new("stream unavailable")),
        }
    }
}

/// A session records frames from an input — content that was never rendered on
/// this machine's screen — and the loop needs no knowledge that it is a stream.
#[test]
fn a_session_records_frames_from_an_input() {
    let (mut run, _cfg) = run_for(vec![0]);
    let dir = tempfile::tempdir().unwrap();
    let src = InputSource::new(
        "rtsp://camera.local/live",
        Box::new(ScriptedGrabber {
            script: (0..5).map(|i| Some((i as u8).wrapping_mul(90))).collect(),
            cursor: 0,
        }),
        0,
    );
    let mut lp = CaptureLoop::new(vec![Box::new(src)], Sampler::new(DeltaConfig::default()), dir.path());

    let mut kept = 0;
    for k in 1..=3 {
        for s in &lp.tick(&mut run, at(k * 30)).sources {
            if s.record.as_ref().and_then(|r| r.path.as_ref()).is_some() {
                kept += 1;
            }
        }
    }
    assert!(kept >= 2, "an input session recorded {kept} samples");
}

/// `regions_for` returns None HONESTLY. Synthesising a whole-frame region would
/// be indistinguishable from a real detection and would hide the whole-frame
/// read — on the one source kind where it is guaranteed to happen.
#[test]
fn an_input_reports_no_regions_and_the_read_is_counted() {
    let (mut run, _cfg) = run_for(vec![0]);
    let dir = tempfile::tempdir().unwrap();
    let src = InputSource::new(
        "rtsp://camera.local/live",
        Box::new(ScriptedGrabber {
            script: (0..5).map(|i| Some((i as u8).wrapping_mul(90))).collect(),
            cursor: 0,
        }),
        0,
    );
    let mut lp = CaptureLoop::new(vec![Box::new(src)], Sampler::new(DeltaConfig::default()), dir.path());

    let mut samples = Vec::new();
    for k in 1..=3 {
        let t = lp.tick(&mut run, at(k * 30));
        for s in &t.sources {
            assert!(s.regions.is_none(), "an input must not synthesise regions");
            if let Some(p) = s.record.as_ref().and_then(|r| r.path.clone()) {
                samples.push(p);
            }
        }
    }
    assert!(!samples.is_empty());
    for p in &samples {
        assert!(
            !gentle_eye::dayflow::perception::regions_path(p).exists(),
            "no sidecar may be written for an input — an empty one would claim a cascade ran"
        );
    }
    assert_eq!(
        lp.samples_read_whole() as usize,
        samples.len(),
        "every input sample is a whole-frame read and must be COUNTED"
    );
}

/// A stream hiccup is Occluded and retried; only a sustained outage ends it.
/// One failed grab is not proof a stream is finished — an encoder restart, a
/// flapping network and a waking camera all look identical.
#[test]
fn a_stream_hiccup_is_retried_but_a_sustained_outage_ends_it() {
    let (mut run, _cfg) = run_for(vec![0]);
    let dir = tempfile::tempdir().unwrap();
    // Two failures, a RECOVERY, then two more. With the counter resetting on
    // success this never reaches the threshold of 3; without the reset it hits
    // 3 on the last tick and retires. The recovery must be in the MIDDLE and
    // the failures either side must be fewer than the threshold, or both
    // behaviours retire at the same tick and the test proves nothing (the
    // fixture that cannot produce the condition it names — 013).
    let script: Vec<Option<u8>> = vec![None, None, Some(200), None, None];
    let src = InputSource::new(
        "rtsp://camera.local/live",
        Box::new(ScriptedGrabber { script, cursor: 0 }),
        0,
    )
    .with_give_up_after(3);
    let mut lp = CaptureLoop::new(vec![Box::new(src)], Sampler::new(DeltaConfig::default()), dir.path());

    let t1 = lp.tick(&mut run, at(30));
    assert_eq!(t1.sources[0].failure, Some(Availability::Occluded), "one hiccup is not the end");
    assert!(t1.retired.is_empty(), "a hiccup must not retire the input");
    let t2 = lp.tick(&mut run, at(60));
    assert_eq!(t2.sources[0].failure, Some(Availability::Occluded), "two is still not the end");
    assert!(t2.retired.is_empty(), "two failures are below the threshold of 3");

    let t3 = lp.tick(&mut run, at(90));
    assert!(t3.sources[0].failure.is_none(), "the input recovered");

    // Two more failures. The run of failures either side is BELOW the
    // threshold, so reaching it here proves the recovery did NOT clear the
    // count — which is the whole property.
    let mut retired = Vec::new();
    for k in 4..=5 {
        retired.extend(lp.tick(&mut run, at(k * 30)).retired);
    }
    assert!(
        retired.is_empty(),
        "a recovery must reset the failure count — the input was retired after two \
         separate short outages that never reached the threshold of 3"
    );
}

/// A sustained outage DOES end the input. Retrying a permanently dead URL
/// forever spends an ffmpeg invocation every tick for the rest of the day and
/// reports a source as "temporarily" unavailable until midnight.
#[test]
fn a_sustained_outage_ends_the_input() {
    let (mut run, _cfg) = run_for(vec![0]);
    let dir = tempfile::tempdir().unwrap();
    let src = InputSource::new(
        "rtsp://gone.local/live",
        Box::new(ScriptedGrabber { script: vec![None, None, None, None], cursor: 0 }),
        0,
    )
    .with_give_up_after(3);
    let mut lp = CaptureLoop::new(vec![Box::new(src)], Sampler::new(DeltaConfig::default()), dir.path());

    // Tick BY tick, pinning the threshold from both sides: retirement on the
    // 2nd failure means the knob lies low, retirement only on a 4th means it
    // lies high ("give up after 3" that actually takes 4 survives a loop that
    // runs one tick past the boundary — so the loop stops AT the boundary).
    assert!(lp.tick(&mut run, at(30)).retired.is_empty(), "one failure must not end it");
    assert!(lp.tick(&mut run, at(60)).retired.is_empty(), "two failures must not end it");
    let third = lp.tick(&mut run, at(90));
    assert_eq!(
        third.retired,
        vec![0],
        "the THIRD consecutive failure must end the input — exactly at give_up_after, \
         not one past it"
    );
    assert!(lp.active_ordinals().is_empty(), "an ended input is not asked again");
}

// ── T017: source selection on all three surfaces ─────────────────────────────

use gentle_eye::dayflow::source::SourceSpec;

/// The parse refuses two kinds rather than picking a winner. A caller who
/// passed both `--window` and `--input` has two intentions, and silently
/// honouring one records the wrong thing all day.
#[test]
fn asking_for_two_sources_is_refused_not_resolved() {
    let both = SourceSpec::parse(None, Some("term".into()), None, Some("rtsp://x".into()));
    let msg = both.expect_err("two kinds must be refused");
    assert!(msg.contains("window"), "the error must name what was asked: {msg}");
    assert!(msg.contains("input"), "the error must name BOTH: {msg}");

    // One kind each is fine, and empty strings do not count as a choice.
    assert_eq!(
        SourceSpec::parse(None, Some("term".into()), None, Some("  ".into())).unwrap(),
        SourceSpec::Window { label: "term".into() }
    );
    // Nothing named is a display session, which is what Dayflow always did.
    assert_eq!(
        SourceSpec::parse(None, None, None, None).unwrap(),
        SourceSpec::Displays { indices: Vec::new() }
    );
}

/// Every surface starts every source kind, and the three AGREE — a session
/// started on one surface reads identically from the others (FR-115).
#[test]
fn all_three_surfaces_start_every_source_kind_and_agree() {
    use gentle_eye::dayflow::http;
    use gentle_eye::dayflow::service::DayflowService;
    use gentle_eye::dayflow::timeline::SqliteTimelineStore;

    fn service() -> DayflowService {
        let store = Arc::new(SqliteTimelineStore::new(Arc::new(std::sync::Mutex::new(
            gentle_eye::storage::database::init_in_memory().expect("db"),
        ))));
        DayflowService::new(store, DayflowConfig::default())
    }

    // (query fragment, expected kind, expected name)
    let cases = [
        ("window=my-term", "window", "my-term"),
        ("target=qa-panel", "target", "qa-panel"),
        ("input=rtsp%3A%2F%2Fcam.local%2Flive", "input", "rtsp://cam.local/live"),
        ("displays=2", "display", "2"),
    ];

    for (query, kind, name) in cases {
        // ── HTTP ──
        let svc = service();
        let (code, body) = http::route("POST", "/dayflow/start", query, &svc);
        assert_eq!(code, "200 OK", "HTTP could not start {kind}: {body}");
        let via_http = svc.status(Utc::now()).unwrap();
        assert_eq!(via_http.sources.len(), 1, "{kind}: expected one source");
        assert_eq!(via_http.sources[0].kind, kind, "HTTP named the wrong kind");
        assert_eq!(via_http.sources[0].name, name, "HTTP named the wrong source");
        // D014-2: a single non-display source occupies ordinal 0 — the
        // `display_id` position on every sample it will produce. A display
        // session keeps its real index (2 here), NOT a renumbered 0: renaming
        // display 2's samples to ordinal 0 would file them under a source the
        // run does not have.
        let expected_ordinal = if kind == "display" { 2 } else { 0 };
        assert_eq!(via_http.sources[0].ordinal, expected_ordinal, "{kind}: wrong ordinal");
        assert_eq!(
            via_http.sources[0].availability, None,
            "nothing has read from {kind} yet — reporting Available would be a claim, \
             not an observation"
        );

        // ── the service call the CLI and MCP both make, same spec ──
        let svc2 = service();
        let spec = match kind {
            "window" => SourceSpec::Window { label: name.into() },
            "target" => SourceSpec::Target { name: name.into() },
            "input" => SourceSpec::Input { url: name.into() },
            _ => SourceSpec::Displays { indices: vec![2] },
        };
        svc2.start_session(DayflowMode::Session, spec, Utc::now()).expect("starts");
        let direct = svc2.status(Utc::now()).unwrap();

        // The three must AGREE on what the session is a record of.
        assert_eq!(direct.sources, via_http.sources, "{kind}: the surfaces disagree");
        let as_json = serde_json::to_value(&direct).unwrap();
        assert_eq!(as_json["sources"][0]["kind"], kind, "the serialised payload lost the kind");
        assert_eq!(as_json["sources"][0]["name"], name);
    }
}

/// End-to-end proof that `start_with_source` — the T022 entry, called by no
/// surface yet — actually joins its two halves: the spec crosses to the capture
/// thread, `build_sources` constructs a REAL `InputSource` with the REAL
/// `FfmpegGrabber`, and a sample from content never rendered on this screen
/// lands on disk with `status` naming the input.
///
/// The "stream" is a plain image file: ffmpeg reads a file path through the
/// same `-i <url>` it reads rtsp://, so this exercises the whole grab+decode
/// path with no network. `#[ignore]`d because it needs the ffmpeg binary
/// (house rule — see tests/dayflow_segmentation.rs).
#[test]
#[ignore = "live: requires the ffmpeg binary"]
fn start_with_source_records_a_real_input_end_to_end() {
    let svc = live_service();
    let dir = tempfile::tempdir().unwrap();

    // A frame with real variation, so the sampler's content gate keeps it.
    let src_png = dir.path().join("fake-camera.png");
    let img = image::RgbaImage::from_fn(320, 240, |x, y| {
        image::Rgba([(x % 256) as u8, (y % 256) as u8, ((x * y) % 256) as u8, 255])
    });
    img.save(&src_png).unwrap();

    let sample_dir = dir.path().join("samples");
    let id = svc
        .start_with_source(
            DayflowMode::Session,
            SourceSpec::Input { url: src_png.to_string_lossy().into_owned() },
            ok_summarizer(),
            sample_dir.clone(),
            Utc::now(),
        )
        .expect("start_with_source starts");

    // The first tick is immediate; wait for its evidence.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let mut samples = 0;
    let mut status = svc.status(Utc::now()).unwrap();
    while std::time::Instant::now() < deadline {
        samples = std::fs::read_dir(&sample_dir)
            .map(|d| d.filter_map(Result::ok).filter(|e| e.path().extension().is_some_and(|x| x == "png")).count())
            .unwrap_or(0);
        status = svc.status(Utc::now()).unwrap();
        // Wait for BOTH pieces of evidence: the sample on disk AND the tick's
        // republish of observed availability (they land one after the other).
        if samples >= 1 && status.sources.first().is_some_and(|s| s.availability.is_some()) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    svc.stop_capture().expect("stops");
    let _ = svc.stop(Utc::now());

    assert!(samples >= 1, "no sample landed — the capture thread never grabbed the input");
    assert_eq!(status.session_id, Some(id));
    assert_eq!(status.sources.len(), 1, "status must name the one input");
    assert_eq!(status.sources[0].kind, "input");
    assert_eq!(
        status.sources[0].availability,
        Some(Availability::Available),
        "after a real grab the tick republishes what it OBSERVED"
    );
}

// ── T018: retention runs on a schedule, and the rule still holds ─────────────

use gentle_eye::dayflow::retention::{Action, RetentionConfig};

/// Disk falls, and NO unsummarised segment is reclaimed. The rule is unchanged
/// from 013 — this re-proves it where the loop actually drives it, because a
/// rule that holds in a unit test and not in the loop is a rule the product
/// does not have.
#[tokio::test]
async fn a_retention_sweep_frees_disk_but_never_touches_an_unsummarised_segment() {
    let (mut run, cfg) = run_for(vec![0]);
    let dir = tempfile::tempdir().unwrap();
    let script: Vec<Option<u8>> = (0..200).map(|i| Some((i as u8).wrapping_mul(90))).collect();
    let sources: Vec<Box<dyn CaptureSource>> = vec![Box::new(FakeSource::new(0, script))];
    let mut lp = CaptureLoop::new(sources, Sampler::new(DeltaConfig::default()), dir.path());

    // Close several windows.
    let segment = cfg.segment_seconds as i64;
    let mut t = 0i64;
    for _ in 0..30 {
        t += segment / 3;
        lp.tick(&mut run, at(t));
    }
    let closed = lp.segments();
    assert!(closed.len() >= 2, "expected several closed windows, got {}", closed.len());

    // Summarise only SOME of them. `settle_due` drains every DUE window, so a
    // summariser that always succeeds leaves nothing unsummarised and the
    // safety assertion below becomes vacuous — it would pass against a
    // retention pass that reclaimed everything indiscriminately.
    let summarizer = FlakySummarizer {
        calls: AtomicUsize::new(0),
        fail_first: 1,
    };
    let entries = lp.settle_due(&summarizer, uuid::Uuid::new_v4(), at(t)).await;
    assert!(!entries.is_empty(), "nothing settled — the fixture cannot discriminate");

    let before: u64 = lp.segments().iter().map(|s| s.raw_bytes).sum();
    assert!(before > 0, "no bytes on disk to reclaim");
    let unsummarised: Vec<(u32, u64)> = lp
        .segments()
        .iter()
        .filter(|s| !s.summarized)
        .map(|s| s.key())
        .collect();

    // Sweep far enough in the future that the hot tier has expired.
    let retention = RetentionConfig::default();
    let later = at(t) + chrono::Duration::from_std(retention.hot).unwrap() + Duration::hours(1);
    let decisions = lp.sweep_retention(&retention, later);

    assert!(!decisions.is_empty(), "the sweep decided nothing");
    let shrunk: Vec<(u32, u64)> = decisions
        .iter()
        .filter(|d| d.action == Action::Shrink)
        .map(|d| (d.display_id, d.sequence))
        .collect();
    assert!(!shrunk.is_empty(), "the sweep freed nothing — disk did not fall");

    // Keep this: without at least one unsummarised segment the rule assertion
    // below passes against a sweep that reclaims everything.
    assert!(
        !unsummarised.is_empty(),
        "the fixture produced no unsummarised segment — the safety assertion is vacuous"
    );
    for key in &unsummarised {
        assert!(
            !shrunk.contains(key),
            "an UNSUMMARISED segment {key:?} was reclaimed — eviction is gated on a summary \
             existing, never on age or budget pressure"
        );
    }

    let after: u64 = lp.segments().iter().map(|s| s.raw_bytes).sum();
    assert!(after < before, "disk did not fall: {before} -> {after}");
}

// ── T019: entries carry where their text came from ───────────────────────────

/// Entries written by the loop have NON-NULL provenance whose region id matches
/// the sidecar the loop wrote. Before this, every entry stored NULL: the
/// cascade ran, the crops were read, and where the text came from was discarded
/// at the last step.
#[tokio::test]
async fn entries_carry_provenance_matching_the_sidecar() {
    let (mut run, cfg) = run_for(vec![0]);
    let dir = tempfile::tempdir().unwrap();
    let script: Vec<Option<u8>> = (0..200).map(|i| Some((i as u8).wrapping_mul(90))).collect();
    let mut src = FakeSource::new(0, script);
    src.regions = Some(boxes(3));
    let mut lp = CaptureLoop::new(vec![Box::new(src)], Sampler::new(DeltaConfig::default()), dir.path());

    let segment = cfg.segment_seconds as i64;
    let mut t = 0i64;
    for _ in 0..12 {
        t += segment / 3;
        if !lp.tick(&mut run, at(t)).closed.is_empty() {
            break;
        }
    }

    let summarizer = FlakySummarizer { calls: AtomicUsize::new(0), fail_first: 0 };
    let entries = lp.settle_due(&summarizer, uuid::Uuid::new_v4(), at(t)).await;
    assert!(!entries.is_empty(), "no entry to check");

    let p = entries[0]
        .provenance
        .as_ref()
        .expect("the entry has NULL provenance — the regions were discarded at the last step");

    // The id must match a region actually in the sidecar, not a plausible one.
    let key_dir = std::fs::read_dir(dir.path()).unwrap();
    let sidecar = key_dir
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.to_string_lossy().ends_with(".regions.json"))
        .expect("no sidecar on disk");
    let stored: Vec<Region> =
        serde_json::from_str(&std::fs::read_to_string(&sidecar).unwrap()).unwrap();
    let ids: Vec<u64> = stored.iter().map(|r| r.identity()).collect();
    assert!(
        ids.contains(&p.region_id),
        "provenance region_id {} is in no sidecar region {ids:?}",
        p.region_id
    );

    // It is the region a reader reaches FIRST, not an arbitrary one.
    let order = gentle_eye::regions::reading_order(&stored);
    let first = stored[order[0]].identity();
    assert_eq!(p.region_id, first, "provenance must name the region at reading-order rank 0");
    assert_eq!(p.reading_order, 0);
    assert_eq!(p.display_id, 0);
    assert_eq!((p.bbox_w, p.bbox_h), (32, 8), "the stored box is not the region's box");
}

/// A window read WHOLE has no region to attribute to, and must store NULL —
/// inventing one would make a whole-frame read indistinguishable from a
/// measured layout.
#[tokio::test]
async fn a_whole_frame_window_stores_null_provenance() {
    let (mut run, cfg) = run_for(vec![0]);
    let dir = tempfile::tempdir().unwrap();
    let script: Vec<Option<u8>> = (0..200).map(|i| Some((i as u8).wrapping_mul(90))).collect();
    let sources: Vec<Box<dyn CaptureSource>> =
        vec![Box::new(FakeSource::new(0, script).with_no_cascade())];
    let mut lp = CaptureLoop::new(sources, Sampler::new(DeltaConfig::default()), dir.path());

    let segment = cfg.segment_seconds as i64;
    let mut t = 0i64;
    for _ in 0..12 {
        t += segment / 3;
        if !lp.tick(&mut run, at(t)).closed.is_empty() {
            break;
        }
    }
    let summarizer = FlakySummarizer { calls: AtomicUsize::new(0), fail_first: 0 };
    let entries = lp.settle_due(&summarizer, uuid::Uuid::new_v4(), at(t)).await;
    assert!(!entries.is_empty());
    assert!(
        entries[0].provenance.is_none(),
        "a whole-frame read must store NULL provenance, not an invented region"
    );
}
