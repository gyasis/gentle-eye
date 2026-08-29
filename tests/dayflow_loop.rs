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

/// `start_capture` must RUN the loop, not merely spawn a thread that compiles.
/// The proof is observable state changing in the shared run.
#[test]
fn the_service_capture_thread_actually_drives_the_loop() {
    use gentle_eye::dayflow::service::DayflowService;
    use gentle_eye::dayflow::timeline::SqliteTimelineStore;

    let store = Arc::new(SqliteTimelineStore::new(Arc::new(std::sync::Mutex::new(
        gentle_eye::storage::database::init_in_memory().expect("db"),
    ))));
    let svc = DayflowService::new(store, DayflowConfig::default());
    svc.start(DayflowMode::Session, vec![0], Utc::now()).expect("session starts");
    assert!(!svc.capture_running(), "no capture thread before start_capture");

    let dir = tempfile::tempdir().unwrap();
    svc.start_capture(
        Box::new(|| {
            let script: Vec<Option<u8>> = (0..200)
                .map(|i| Some((i as u8).wrapping_mul(90)))
                .collect();
            vec![Box::new(FakeSource::new(0, script)) as Box<dyn CaptureSource>]
        }),
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
