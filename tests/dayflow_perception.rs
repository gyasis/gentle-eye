//! US3 acceptance coverage — the two-tier perception ladder.
//!
//! These assert the acceptance scenarios from `spec.md` across module
//! boundaries, where the unit tests each see only one side. The costliest
//! defect this wave produced was a module that nothing called while its task
//! was marked done, so several of these exist specifically to fail if the
//! ladder stops being REACHABLE, not merely correct.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use gentle_eye::config::{DayflowIntent, ResidencyPolicy};
use gentle_eye::contracts::errors::VisionError;
use gentle_eye::contracts::traits::{AnalysisResult, TimeRange, VisionProvider};
use gentle_eye::dayflow::models::{ChunkRef, RollingContext};
use gentle_eye::dayflow::perception::{
    aggregator_for, dayflow_budget, PerceptionKind, PerceptionRouter, Tier,
};
use gentle_eye::dayflow::summarizer::{ChunkSummarizer, RoutedChunkSummarizer};

/// Records which tier answered and what it was asked.
struct Recorder {
    tier: Tier,
    calls: Arc<AtomicUsize>,
    prompts: Arc<Mutex<Vec<String>>>,
    /// Which image each call was given — without this, nothing pins WHICH
    /// frame the reasoning tier sees, and a `last`→`first` mutation survives.
    paths: Arc<Mutex<Vec<std::path::PathBuf>>>,
}

#[async_trait]
impl VisionProvider for Recorder {
    async fn analyze_video(
        &self,
        _: &Path,
        _: &str,
        _: Option<TimeRange>,
    ) -> Result<AnalysisResult, VisionError> {
        panic!("dayflow perception must never analyse video: it samples frames")
    }
    async fn analyze_image(&self, image: &Path, prompt: &str) -> Result<AnalysisResult, VisionError> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        self.prompts.lock().unwrap().push(prompt.to_string());
        self.paths.lock().unwrap().push(image.to_path_buf());
        let text = match self.tier {
            Tier::Text => format!("screen text sample {n}"),
            Tier::Reason => {
                r#"{"category":"coding","app":"editor","activity":"refactor","detail":"d"}"#
                    .to_string()
            }
        };
        Ok(AnalysisResult {
            request_id: uuid::Uuid::new_v4(),
            analysis_text: text,
            provider: "recorder".into(),
            model_used: "recorder".into(),
            processing_time_ms: 1,
            token_count: None,
            completed_at: Utc::now(),
        })
    }
    async fn health_check(&self) -> Result<(), VisionError> {
        Ok(())
    }
    fn name(&self) -> &'static str {
        "recorder"
    }
    fn max_video_size(&self) -> u64 {
        0
    }
    fn supports_native_video(&self) -> bool {
        false
    }
    fn model(&self) -> &str {
        "recorder"
    }
}

struct Rig {
    router: Arc<PerceptionRouter>,
    text_calls: Arc<AtomicUsize>,
    reason_calls: Arc<AtomicUsize>,
    reason_prompts: Arc<Mutex<Vec<String>>>,
    text_paths: Arc<Mutex<Vec<std::path::PathBuf>>>,
    reason_paths: Arc<Mutex<Vec<std::path::PathBuf>>>,
}

fn rig(budget: u32) -> Rig {
    let (tc, rc) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let (tp, rp) = (
        Arc::new(Mutex::new(Vec::new())),
        Arc::new(Mutex::new(Vec::new())),
    );
    let (tpath, rpath) = (
        Arc::new(Mutex::new(Vec::new())),
        Arc::new(Mutex::new(Vec::new())),
    );
    Rig {
        router: Arc::new(PerceptionRouter::new(
            Arc::new(Recorder {
                tier: Tier::Text,
                calls: tc.clone(),
                prompts: tp,
                paths: tpath.clone(),
            }),
            Arc::new(Recorder {
                tier: Tier::Reason,
                calls: rc.clone(),
                prompts: rp.clone(),
                paths: rpath.clone(),
            }),
            budget,
        )),
        text_calls: tc,
        reason_calls: rc,
        reason_prompts: rp,
        text_paths: tpath,
        reason_paths: rpath,
    }
}

/// A pane region on `display`, in that display's local coordinates.
fn pane(x: u32, y: u32, w: u32, h: u32, display: u32) -> gentle_eye::regions::Region {
    let mut r = gentle_eye::regions::Region::new(
        gentle_eye::target::model::PixelRect { x, y, w, h },
        gentle_eye::regions::Source::Wm,
        gentle_eye::regions::Granularity::Pane,
        0.8,
    );
    r.display_id = display;
    r
}

/// Write `n` samples for one window, as the sampler names them.
fn samples(dir: &Path, display: u32, sequence: u64, n: usize) -> Vec<std::path::PathBuf> {
    let prefix = gentle_eye::dayflow::sampler::sample_prefix(display, sequence);
    (0..n)
        .map(|i| {
            let p = dir.join(format!("{prefix}20260826T1000{i:02}000.png"));
            std::fs::write(&p, b"png").unwrap();
            p
        })
        .collect()
}

#[tokio::test]
async fn a_full_segment_reads_every_sample_cheaply_and_reasons_once() {
    // US3's headline: per-segment perception uses the text tier by default and
    // escalates only for category and meaning.
    let dir = tempfile::tempdir().unwrap();
    samples(dir.path(), 1, 4, 5);
    let r = rig(1_000);
    let s = RoutedChunkSummarizer::new(r.router.clone(), dir.path());

    let chunk = ChunkRef {
        index: 0,
        path: dir.path().join("unused.mp4"),
        start_wall: Utc::now(),
        end_wall: Utc::now(),
        display_id: 1,
        sequence: 4,
        summarized: false,
    };
    let summary = s.summarize_chunk(&chunk, &RollingContext::default()).await.unwrap();

    assert_eq!(r.text_calls.load(Ordering::SeqCst), 5, "every sample read cheaply");
    assert_eq!(r.reason_calls.load(Ordering::SeqCst), 1, "one escalation for the segment");
    assert_eq!(r.router.escalations().len(), 1);
    assert_eq!(summary.app, "editor", "the reasoning tier's answer reaches the caller");

    // and it carried what the cheap tier extracted, per sample
    let prompt = &r.reason_prompts.lock().unwrap()[0];
    for n in 0..5 {
        assert!(
            prompt.contains(&format!("screen text sample {n}")),
            "sample {n}'s text must reach the reasoning tier"
        );
    }
}

#[tokio::test]
async fn the_ladder_stays_reachable_from_the_summarizer() {
    // A regression guard for this wave's costliest defect: the router existed,
    // was correct, was tested — and nothing called it, while its task was
    // marked done. If RoutedChunkSummarizer stops routing, this fails even
    // though every unit test still passes.
    let dir = tempfile::tempdir().unwrap();
    samples(dir.path(), 0, 0, 2);
    let r = rig(1_000);
    let s = RoutedChunkSummarizer::new(r.router.clone(), dir.path());
    let chunk = ChunkRef {
        index: 0,
        path: dir.path().join("x.mp4"),
        start_wall: Utc::now(),
        end_wall: Utc::now(),
        display_id: 0,
        sequence: 0,
        summarized: false,
    };
    s.summarize_chunk(&chunk, &RollingContext::default()).await.unwrap();
    assert!(
        r.text_calls.load(Ordering::SeqCst) > 0,
        "the summarizer must go THROUGH the ladder, not around it"
    );
}

#[tokio::test]
async fn a_segments_perception_cannot_outrun_its_budget() {
    // The region cap and the interval bound the work at the source; the budget
    // is derived from them, so a segment that tries to exceed it is refused
    // rather than billed.
    let dir = tempfile::tempdir().unwrap();
    samples(dir.path(), 0, 0, 6);
    let r = rig(3); // room for 3 calls only
    let s = RoutedChunkSummarizer::new(r.router.clone(), dir.path());
    let chunk = ChunkRef {
        index: 0,
        path: dir.path().join("x.mp4"),
        start_wall: Utc::now(),
        end_wall: Utc::now(),
        display_id: 0,
        sequence: 0,
        summarized: false,
    };
    let err = s.summarize_chunk(&chunk, &RollingContext::default()).await;
    assert!(err.is_err(), "the budget must bind on a real segment, not only in a unit test");
    assert!(
        r.reason_calls.load(Ordering::SeqCst) == 0,
        "and it must bind BEFORE the expensive tier is reached"
    );
}

// T032 asked for an assertion that "the demoted tesseract path is never used as
// the text tier". It was written as a grep over the source files, and that test
// is theatre in both directions: it FAILS on a comment explaining why not to use
// tesseract, and PASSES if the real wrapper is reintroduced under a name that
// does not contain the word. Deleted rather than kept as reassurance.
//
// The property is already held behaviourally: the text tier is whichever
// `VisionProvider` the router was constructed with, and every test here injects
// its own — a local OCR call could not satisfy them, because the assertions are
// on what the INJECTED provider received. A module-boundary rule ("dayflow must
// not import analysis::ocr") is a lint, not a test, and belongs in T050.

#[tokio::test]
async fn the_reasoning_tier_is_given_the_segments_last_frame() {
    // `last` is deliberate: the end state of a segment is what a question about
    // "what was I doing" is usually asking about. A mutation to `first`
    // survived the whole suite, so nothing was pinning it.
    let dir = tempfile::tempdir().unwrap();
    let paths = samples(dir.path(), 0, 0, 4);
    let r = rig(1_000);

    let (_, _lat) = gentle_eye::dayflow::perception::summarize_segment_via_ladder(
        &r.router,
        &paths,
        "categorise",
        0,
        8,
    )
    .await
    .unwrap();

    // The reasoning call is the only one that takes a frame of its own choosing.
    // and every text read went to a real sample, never to the reasoning frame
    // by accident
    let read = r.text_paths.lock().unwrap().clone();
    assert_eq!(read.len(), 4, "one text read per sample when no regions exist");
    assert_eq!(read, paths, "in capture order");

    let seen = r.reason_paths.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0],
        *paths.last().unwrap(),
        "the reasoning tier must see the segment's LAST frame, not its first"
    );
}

#[test]
fn the_budget_scales_with_the_shape_of_the_day() {
    // Coarse all-day tracking must ask for far less than a fine focused
    // session, or the "derived from the sampling shape" claim is empty.
    let coarse = dayflow_budget(180, 1, 4);
    let fine = dayflow_budget(60, 1, 4);
    let multi = dayflow_budget(180, 3, 4);
    assert!(fine > coarse);
    assert_eq!(multi, coarse * 3);
}

#[test]
fn intent_decides_whether_text_is_accumulated_at_all() {
    // D12: Activity is the default and must not pay to keep a transcript.
    assert!(aggregator_for(DayflowIntent::Activity).is_none());
    // A REALISTIC capture size. Below about three lines the metric is
    // degenerate — the score can only be 0, 0.5 or 1 — so a two-line fixture
    // would be testing the limitation rather than the behaviour.
    let mut agg = aggregator_for(DayflowIntent::Content).expect("Content aggregates");
    let first: Vec<String> = (0..12).map(|i| format!("line {i}")).collect();
    let scrolled: Vec<String> = (2..14).map(|i| format!("line {i}")).collect();
    agg.absorb(&first.join("\n"));
    agg.absorb(&scrolled.join("\n"));
    assert_eq!(agg.blocks().len(), 1, "overlapping captures are one document");
    assert_eq!(
        agg.blocks()[0].lines().filter(|l| *l == "line 5").count(),
        1,
        "and the overlap is not duplicated"
    );
}

#[test]
fn residency_is_a_request_parameter_sized_by_the_segment_cadence() {
    // R27: the governor honours keep_alive per request, so the tier unloads by
    // itself when sampling stops — a pinger would hold 7.4 GB on a shared
    // machine after the session died.
    //
    // And it is sized by the SEGMENT cadence, because dayflow's text calls fire
    // in a burst at segment close: a window sized from the 180s sample interval
    // expires long before the next burst, so `Resident` would hold memory AND
    // pay every cold load.
    let segment = std::time::Duration::from_secs(900);
    assert_eq!(ResidencyPolicy::default(), ResidencyPolicy::OnDemand);
    assert!(ResidencyPolicy::OnDemand.keep_alive(segment).is_none());
    let ka = ResidencyPolicy::Resident.keep_alive(segment).unwrap();
    let secs: u64 = ka.trim_end_matches('s').parse().unwrap();
    assert!(secs > segment.as_secs(), "must outlast one segment to bridge two");
}

#[tokio::test]
async fn a_reasoning_request_must_say_why_before_anything_is_spent() {
    // FR-007/010: the audit trail answers "why was today expensive?", so an
    // escalation with no reason is a bug rather than a log line.
    let r = rig(1);
    let err = r
        .router
        .perceive(PerceptionKind::Reason, Path::new("/tmp/x.png"), "why", "")
        .await;
    assert!(err.is_err());
    assert!(r.router.escalations().is_empty());
    // the refused request must not have consumed the only token
    r.router
        .perceive(PerceptionKind::Reason, Path::new("/tmp/x.png"), "why", "categorising")
        .await
        .expect("a valid request still has its budget");
}

#[tokio::test]
async fn region_crops_actually_reach_the_text_tier() {
    // THE GUARD THAT WAS MISSING. `crop_regions` was written, documented and
    // unit-tested while nothing called it, so the shipped pipeline fed whole
    // frames to the text tier — committing the exact failure the function's own
    // doc rails against, in the same commit that added an anti-orphan guard for
    // the router. A correct function nothing calls is not a feature.
    let dir = tempfile::tempdir().unwrap();
    let paths = samples(dir.path(), 0, 0, 2);

    // A frame with two panes, and the regions the capture detected for it.
    for p in &paths {
        let mut img = image::RgbaImage::new(400, 200);
        for (x, _y, px) in img.enumerate_pixels_mut() {
            *px = if x < 200 {
                image::Rgba([255, 0, 0, 255])
            } else {
                image::Rgba([0, 0, 255, 255])
            };
        }
        img.save(p).unwrap();

        let regions = vec![
            gentle_eye::regions::Region::new(
                gentle_eye::target::model::PixelRect { x: 0, y: 0, w: 200, h: 200 },
                gentle_eye::regions::Source::Wm,
                gentle_eye::regions::Granularity::Pane,
                0.8,
            ),
            gentle_eye::regions::Region::new(
                gentle_eye::target::model::PixelRect { x: 200, y: 0, w: 200, h: 200 },
                gentle_eye::regions::Source::Wm,
                gentle_eye::regions::Granularity::Pane,
                0.8,
            ),
        ];
        std::fs::write(
            gentle_eye::dayflow::perception::regions_path(p),
            serde_json::to_string(&regions).unwrap(),
        )
        .unwrap();
    }

    let r = rig(1_000);
    let (_, latency) = gentle_eye::dayflow::perception::summarize_segment_via_ladder(
        &r.router,
        &paths,
        "categorise",
        0,
        8,
    )
    .await
    .unwrap();

    assert_eq!(
        r.text_calls.load(Ordering::SeqCst),
        4,
        "2 samples x 2 regions — the tier must see CROPS, not 2 whole frames"
    );
    assert_eq!(latency.perception_calls, 5, "4 text reads + 1 reasoning call");

    // and each crop is one pane, not the frame
    for p in r.text_paths.lock().unwrap().iter() {
        let img = image::open(p).unwrap();
        assert_eq!(
            (img.width(), img.height()),
            (200, 200),
            "each read must be a single-pane crop at full resolution: {p:?}"
        );
    }
}

#[tokio::test]
async fn a_sample_with_no_regions_is_still_read_whole() {
    // FAIL-OPEN (R13): a missing or unreadable region sidecar must DEGRADE the
    // reading, never drop the sample. Dayflow cannot re-capture yesterday.
    let dir = tempfile::tempdir().unwrap();
    let paths = samples(dir.path(), 0, 0, 3);
    // one sample gets a corrupt sidecar, one gets none, one gets an empty list
    std::fs::write(
        gentle_eye::dayflow::perception::regions_path(&paths[0]),
        b"{ not json",
    )
    .unwrap();
    std::fs::write(gentle_eye::dayflow::perception::regions_path(&paths[2]), b"[]").unwrap();

    let r = rig(1_000);
    let (_, latency) = gentle_eye::dayflow::perception::summarize_segment_via_ladder(
        &r.router,
        &paths,
        "categorise",
        0,
        8,
    )
    .await
    .unwrap();

    assert_eq!(
        r.text_calls.load(Ordering::SeqCst),
        3,
        "every sample is still read, once, as a whole frame"
    );
    assert_eq!(latency.samples, 3);
}

#[tokio::test]
async fn crops_reach_the_text_tier_through_the_production_summarizer() {
    // X-B: the crop guard above calls the ladder DIRECTLY with a literal
    // max_regions, so `RoutedChunkSummarizer` could pass 0 and crops would
    // never happen on the production path — 453 tests still green. That is the
    // orphan class one seam up: not "nothing calls the function" but "the
    // caller neuters the parameter". This drives sidecar -> crops -> text tier
    // through the type the pipeline actually uses.
    let dir = tempfile::tempdir().unwrap();
    let paths = samples(dir.path(), 2, 9, 2);
    for p in &paths {
        let mut img = image::RgbaImage::new(300, 100);
        for (x, _y, px) in img.enumerate_pixels_mut() {
            *px = if x < 150 {
                image::Rgba([255, 0, 0, 255])
            } else {
                image::Rgba([0, 0, 255, 255])
            };
        }
        img.save(p).unwrap();
        let regions = vec![pane(0, 0, 150, 100, 2), pane(150, 0, 150, 100, 2)];
        std::fs::write(
            gentle_eye::dayflow::perception::regions_path(p),
            serde_json::to_string(&regions).unwrap(),
        )
        .unwrap();
    }

    let r = rig(1_000);
    let s = RoutedChunkSummarizer::new(r.router.clone(), dir.path());
    let chunk = ChunkRef {
        index: 0,
        path: dir.path().join("x.mp4"),
        start_wall: Utc::now(),
        end_wall: Utc::now(),
        display_id: 2,
        sequence: 9,
        summarized: false,
    };
    s.summarize_chunk(&chunk, &RollingContext::default()).await.unwrap();

    assert_eq!(
        r.text_calls.load(Ordering::SeqCst),
        4,
        "2 samples x 2 regions THROUGH the summarizer — a max_regions of 0 gives 2"
    );

    // X-A: the recorded latency must actually be kept, not merely computed.
    let recorded = s.latencies();
    assert_eq!(recorded.len(), 1, "FR-013 records a latency PER SEGMENT");
    assert_eq!(recorded[0].samples, 2);
    assert_eq!(recorded[0].perception_calls, 5, "4 crops + 1 reasoning call");
    assert_eq!(recorded[0].samples_read_whole, 0, "regions were present for both");
}

#[tokio::test]
async fn a_sample_whose_regions_all_miss_the_frame_is_still_read() {
    // X-H: the fail-open tests covered missing, corrupt and EMPTY sidecars, but
    // not "regions present and every one skipped" — which is precisely the
    // stale-region story the docs tell (a resize, or boxes from another
    // display). Each fixture was missing a different half of reality again.
    // R13: never drop, dayflow cannot re-capture yesterday.
    let dir = tempfile::tempdir().unwrap();
    let paths = samples(dir.path(), 0, 0, 1);
    let mut img = image::RgbaImage::new(200, 100);
    for (_x, _y, px) in img.enumerate_pixels_mut() {
        *px = image::Rgba([10, 10, 10, 255]);
    }
    img.save(&paths[0]).unwrap();

    // every region is off-frame or belongs to another display
    let regions = vec![pane(9_000, 9_000, 50, 50, 0), pane(0, 0, 50, 50, 7)];
    std::fs::write(
        gentle_eye::dayflow::perception::regions_path(&paths[0]),
        serde_json::to_string(&regions).unwrap(),
    )
    .unwrap();

    let r = rig(1_000);
    let (_, latency) = gentle_eye::dayflow::perception::summarize_segment_via_ladder(
        &r.router, &paths, "categorise", 0, 8,
    )
    .await
    .unwrap();

    assert_eq!(
        r.text_calls.load(Ordering::SeqCst),
        1,
        "the sample must still be read, whole, rather than silently lost"
    );
    assert_eq!(r.text_paths.lock().unwrap()[0], paths[0], "and it is the frame itself");
    // W5 gate: this used to expect 0 ("regions existed — this is not the
    // no-sidecar path"), which made the counter a taxonomy of CAUSES instead of
    // what FR-103 needs it to be — visibility of every read where the crop was
    // absent. This sample WAS read whole; regions that all miss the frame are
    // exactly the stale-region story the fixture tells, and leaving it at 0
    // made that degradation the one whole-frame read invisible to BOTH the
    // capture-side counter and this one.
    assert_eq!(
        latency.samples_read_whole, 1,
        "a frame read whole because no region applied must be COUNTED, not merely survived"
    );
}

#[tokio::test]
async fn a_failed_crop_is_a_counted_whole_frame_read() {
    // W5 gate: the Err arm of the crop match read the frame whole WITHOUT
    // counting it — a degradation invisible to both counters, found because
    // the mutation deleting the count survived the whole suite. The frame
    // here is not a decodable image, so `crop_regions` fails at open; the
    // sidecar's regions are valid and in-frame, so no other arm can absorb it.
    let dir = tempfile::tempdir().unwrap();
    let paths = samples(dir.path(), 0, 0, 1); // writes b"png" — not a real PNG
    std::fs::write(
        gentle_eye::dayflow::perception::regions_path(&paths[0]),
        serde_json::to_string(&vec![pane(0, 0, 50, 50, 0)]).unwrap(),
    )
    .unwrap();

    let r = rig(1_000);
    let (_, latency) = gentle_eye::dayflow::perception::summarize_segment_via_ladder(
        &r.router, &paths, "categorise", 0, 8,
    )
    .await
    .unwrap();

    assert_eq!(
        r.text_calls.load(Ordering::SeqCst),
        1,
        "the sample must still be read, whole, rather than dropped"
    );
    assert_eq!(r.text_paths.lock().unwrap()[0], paths[0], "and it is the frame itself");
    assert_eq!(
        latency.samples_read_whole, 1,
        "a whole-frame read forced by a failed crop must be counted, not merely survived"
    );
}

#[test]
fn a_region_exactly_touching_the_frame_edge_yields_no_crop() {
    // X-E: `x1 <= x0` versus `x1 < x0` is one character, and no fixture had a
    // region that exactly kisses the edge. The consequence is not cosmetic: a
    // zero-width crop makes `save` fail, which errors `crop_regions`, which
    // sends the ladder down its whole-frame fallback for the ENTIRE sample —
    // one stale region abandoning cropping for everything else on screen.
    let dir = tempfile::tempdir().unwrap();
    let frame = dir.path().join("f.png");
    image::RgbaImage::new(200, 100).save(&frame).unwrap();

    let regions = vec![
        pane(200, 0, 50, 50, 0), // starts exactly at the right edge
        pane(0, 100, 50, 50, 0), // starts exactly at the bottom edge
        pane(0, 0, 50, 50, 0),   // genuinely inside
    ];
    let crops =
        gentle_eye::dayflow::perception::crop_regions(&frame, &regions, &dir.path().join("c"), 8, 0)
            .unwrap();
    assert_eq!(crops.len(), 1, "edge-kissing regions produce no crop, and no error");
}

// ── T020/T021: residency is expressible end to end ───────────────────────────

/// A provider that records every keep_alive it was handed.
struct ResidencyRecorder {
    tier: Tier,
    seen: Arc<Mutex<Vec<Option<String>>>>,
}

#[async_trait]
impl VisionProvider for ResidencyRecorder {
    async fn analyze_video(
        &self,
        _: &Path,
        _: &str,
        _: Option<TimeRange>,
    ) -> Result<AnalysisResult, VisionError> {
        unreachable!("dayflow samples frames")
    }
    async fn analyze_image(&self, _: &Path, _: &str) -> Result<AnalysisResult, VisionError> {
        let text = match self.tier {
            Tier::Text => "screen text".to_string(),
            Tier::Reason => {
                r#"{"category":"coding","app":"editor","activity":"a","detail":"d"}"#.to_string()
            }
        };
        Ok(AnalysisResult {
            request_id: uuid::Uuid::new_v4(),
            analysis_text: text,
            provider: "residency".into(),
            model_used: "residency".into(),
            processing_time_ms: 1,
            token_count: None,
            completed_at: Utc::now(),
        })
    }
    async fn health_check(&self) -> Result<(), VisionError> {
        Ok(())
    }
    fn name(&self) -> &'static str {
        "residency"
    }
    fn max_video_size(&self) -> u64 {
        0
    }
    fn supports_native_video(&self) -> bool {
        false
    }
    fn model(&self) -> &str {
        "residency"
    }
    fn set_keep_alive(&self, keep_alive: Option<String>) {
        self.seen.lock().unwrap().push(keep_alive);
    }
}

/// `Resident` reaches the PROVIDER — the value, not just the config. Before
/// T020 nothing sent keep_alive, so `Resident` was a policy the running system
/// could not express, and no test failed to say so.
#[test]
fn a_resident_policy_reaches_the_text_provider_and_not_the_reason_tier() {
    use gentle_eye::dayflow::summarizer::RoutedChunkSummarizer;

    let text_seen = Arc::new(Mutex::new(Vec::new()));
    let reason_seen = Arc::new(Mutex::new(Vec::new()));
    let router = Arc::new(PerceptionRouter::new(
        Arc::new(ResidencyRecorder { tier: Tier::Text, seen: Arc::clone(&text_seen) }),
        Arc::new(ResidencyRecorder { tier: Tier::Reason, seen: Arc::clone(&reason_seen) }),
        dayflow_budget(60, 1, 12),
    ));
    let dir = tempfile::tempdir().unwrap();
    let cadence = std::time::Duration::from_secs(900);

    let _s = RoutedChunkSummarizer::new(Arc::clone(&router), dir.path())
        .with_residency(ResidencyPolicy::Resident, cadence);

    let text = text_seen.lock().unwrap().clone();
    assert_eq!(
        text,
        vec![Some("1860s".to_string())],
        "the text tier must receive the RESOLVED keep_alive (2x cadence + 60s margin)"
    );
    assert!(
        reason_seen.lock().unwrap().is_empty(),
        "the reasoning tier fires once per segment — pinning it holds a second model's \
         memory to save a cost that is already amortised"
    );
}

/// The three policies are distinguishable at the provider. Collapsing them
/// would make `Resident` and `OnDemand` the same request while status still
/// reported a residency policy.
#[test]
fn the_three_residency_policies_reach_the_provider_differently() {
    use gentle_eye::dayflow::summarizer::RoutedChunkSummarizer;

    let cadence = std::time::Duration::from_secs(900);
    let mut got = Vec::new();
    for policy in [ResidencyPolicy::Resident, ResidencyPolicy::OnDemand, ResidencyPolicy::Off] {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let router = Arc::new(PerceptionRouter::new(
            Arc::new(ResidencyRecorder { tier: Tier::Text, seen: Arc::clone(&seen) }),
            Arc::new(ResidencyRecorder { tier: Tier::Reason, seen: Arc::new(Mutex::new(Vec::new())) }),
            dayflow_budget(60, 1, 12),
        ));
        let dir = tempfile::tempdir().unwrap();
        let _s = RoutedChunkSummarizer::new(router, dir.path()).with_residency(policy, cadence);
        got.push(seen.lock().unwrap().clone());
    }
    assert_eq!(got[0], vec![Some("1860s".to_string())], "Resident holds the model");
    assert_eq!(got[1], vec![None], "OnDemand says nothing — it accepts the reload");
    assert_eq!(got[2], vec![Some("0".to_string())], "Off releases immediately");
    assert_ne!(got[0], got[1]);
    assert_ne!(got[1], got[2]);
}

/// Residency measurably WORKS: a `Resident` run pays the cold load once across
/// segments, an `OnDemand` run pays it every segment.
///
/// # What this proves, and what it deliberately does not (W8 gate)
///
/// The provider's warm/cold behaviour is a FIXTURE: `ColdLoadProvider` unloads
/// itself when handed `keep_alive: None`, so "OnDemand pays per segment" is
/// arranged by this file, not observed from a server. Read as a claim about
/// ollama it would be circular. What the test actually establishes is the
/// three links this side of the wire: (1) the loop re-applies the policy at
/// every segment boundary and the RESOLVED value (asserted against
/// `ResidencyPolicy::keep_alive` below, not restated) reaches the provider
/// each time; (2) `SegmentLatency::first_call` isolates a cold load — it is
/// deterministically the first call of a burst, and a mean cannot see it
/// (013/R5); (3) given a provider that honours keep_alive semantics, the three
/// policies produce measurably different runs. That a REAL server honours the
/// value is 013/R27's live measurement, not this test's — and the fixture's
/// unload-on-None models that server only while the segment cadence exceeds
/// the server's own default window (900 s vs ollama's 5 min default; a 60 s
/// cadence would stay warm under OnDemand and this fixture would not).
#[tokio::test]
async fn a_resident_run_pays_the_cold_load_once_and_an_ondemand_run_pays_it_per_segment() {
    /// Warm only if it was asked to stay warm. Records each call's cost.
    struct ColdLoadProvider {
        tier: Tier,
        warm: Arc<Mutex<bool>>,
        keep: Arc<Mutex<Option<String>>>,
    }

    #[async_trait]
    impl VisionProvider for ColdLoadProvider {
        async fn analyze_video(
            &self,
            _: &Path,
            _: &str,
            _: Option<TimeRange>,
        ) -> Result<AnalysisResult, VisionError> {
            unreachable!()
        }
        async fn analyze_image(&self, _: &Path, _: &str) -> Result<AnalysisResult, VisionError> {
            // The guard must NOT be held across the await: a std Mutex guard
            // is not Send, and holding it would also serialise the very thing
            // being measured.
            let was_cold = {
                let mut warm = self.warm.lock().unwrap();
                let cold = !*warm;
                *warm = true;
                cold
            };
            if was_cold {
                // The cold load. Small but real, so first_call can see it.
                tokio::time::sleep(std::time::Duration::from_millis(40)).await;
            }
            let text = match self.tier {
                Tier::Text => "screen text".to_string(),
                Tier::Reason => {
                    r#"{"category":"coding","app":"e","activity":"a","detail":"d"}"#.to_string()
                }
            };
            Ok(AnalysisResult {
                request_id: uuid::Uuid::new_v4(),
                analysis_text: text,
                provider: "cold".into(),
                model_used: "cold".into(),
                processing_time_ms: 1,
                token_count: None,
                completed_at: Utc::now(),
            })
        }
        async fn health_check(&self) -> Result<(), VisionError> {
            Ok(())
        }
        fn name(&self) -> &'static str {
            "cold"
        }
        fn max_video_size(&self) -> u64 {
            0
        }
        fn supports_native_video(&self) -> bool {
            false
        }
        fn model(&self) -> &str {
            "cold"
        }
        fn set_keep_alive(&self, keep_alive: Option<String>) {
            *self.keep.lock().unwrap() = keep_alive.clone();
            // Between segments the model unloads UNLESS residency asked it to
            // stay. That is the whole behaviour residency buys.
            if keep_alive.is_none() {
                *self.warm.lock().unwrap() = false;
            }
        }
    }

    async fn run(policy: ResidencyPolicy) -> Vec<std::time::Duration> {
        let dir = tempfile::tempdir().unwrap();
        let sample = dir.path().join("d0_w000000_x.png");
        // A real 8x8 PNG so crop/read paths behave.
        let img = image::RgbaImage::from_fn(8, 8, |x, y| {
            image::Rgba([(x * 30) as u8, (y * 30) as u8, 128, 255])
        });
        img.save(&sample).unwrap();

        let warm = Arc::new(Mutex::new(false));
        let keep = Arc::new(Mutex::new(None));
        let router = PerceptionRouter::new(
            Arc::new(ColdLoadProvider {
                tier: Tier::Text,
                warm: Arc::clone(&warm),
                keep: Arc::clone(&keep),
            }),
            Arc::new(ColdLoadProvider {
                tier: Tier::Reason,
                warm: Arc::new(Mutex::new(true)),
                keep: Arc::new(Mutex::new(None)),
            }),
            dayflow_budget(60, 1, 12),
        );
        let cadence = std::time::Duration::from_secs(900);
        let mut firsts = Vec::new();
        for _ in 0..3 {
            // Each segment begins by applying the policy — which, for a policy
            // that does not hold the model, drops it.
            router.text_provider().set_keep_alive(policy.keep_alive(cadence));
            let (_s, latency) =
                gentle_eye::dayflow::perception::summarize_segment_via_ladder(
                    &router,
                    std::slice::from_ref(&sample),
                    "what?",
                    0,
                    1,
                )
                    .await
                    .expect("ladder runs");
            firsts.push(latency.first_call);
        }
        // The tie that keeps the fixture honest: the value it acted on is the
        // one the REAL resolver produced for this policy — not a value this
        // test invented to make its own arrangement come out.
        assert_eq!(
            *keep.lock().unwrap(),
            policy.keep_alive(cadence),
            "the provider acted on a keep_alive the resolver never produced"
        );
        firsts
    }

    let resident = run(ResidencyPolicy::Resident).await;
    let on_demand = run(ResidencyPolicy::OnDemand).await;

    let cold = std::time::Duration::from_millis(30);
    assert!(resident[0] >= cold, "the first segment always pays the load: {:?}", resident[0]);
    assert!(
        resident[1] < cold && resident[2] < cold,
        "a Resident run paid the cold load again: {resident:?}"
    );
    assert!(
        on_demand.iter().all(|d| *d >= cold),
        "an OnDemand run should pay the load every segment: {on_demand:?}"
    );
}
