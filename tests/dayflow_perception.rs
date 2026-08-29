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
use gentle_eye::config::DayflowIntent;
use gentle_eye::contracts::errors::VisionError;
use gentle_eye::contracts::traits::{AnalysisResult, TimeRange, VisionProvider};
use gentle_eye::dayflow::models::{ChunkRef, RollingContext};
use gentle_eye::dayflow::perception::{
    aggregator_for, dayflow_budget, PerceptionKind, PerceptionRouter, Residency, Tier,
};
use gentle_eye::dayflow::summarizer::{ChunkSummarizer, RoutedChunkSummarizer};

/// Records which tier answered and what it was asked.
struct Recorder {
    tier: Tier,
    calls: Arc<AtomicUsize>,
    prompts: Arc<Mutex<Vec<String>>>,
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
    async fn analyze_image(&self, _: &Path, prompt: &str) -> Result<AnalysisResult, VisionError> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        self.prompts.lock().unwrap().push(prompt.to_string());
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
}

fn rig(budget: u32) -> Rig {
    let (tc, rc) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let (tp, rp) = (
        Arc::new(Mutex::new(Vec::new())),
        Arc::new(Mutex::new(Vec::new())),
    );
    Rig {
        router: Arc::new(PerceptionRouter::new(
            Arc::new(Recorder { tier: Tier::Text, calls: tc.clone(), prompts: tp }),
            Arc::new(Recorder { tier: Tier::Reason, calls: rc.clone(), prompts: rp.clone() }),
            budget,
        )),
        text_calls: tc,
        reason_calls: rc,
        reason_prompts: rp,
    }
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

#[test]
fn the_demoted_tesseract_path_is_never_the_text_tier() {
    // T032 names this explicitly. Local tesseract remains available for the
    // interactive `read_screen_text` tool, but the DAYFLOW text tier is the
    // configured vision model: tesseract on a full desktop screenshot returns
    // fragmentary text with no layout, which is precisely the input that makes
    // a downstream summary confidently wrong.
    let src = std::fs::read_to_string("src/dayflow/perception.rs").unwrap();
    assert!(
        !src.to_lowercase().contains("tesseract"),
        "the dayflow perception ladder must not reach for the demoted local OCR path"
    );
    let summarizer = std::fs::read_to_string("src/dayflow/summarizer.rs").unwrap();
    assert!(!summarizer.to_lowercase().contains("tesseract"));
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
fn residency_is_a_request_parameter_not_a_background_thread() {
    // R27: the governor honours keep_alive per request, so the tier unloads by
    // itself when sampling stops. A pinger would keep 7.4 GB resident on a
    // shared machine after the session died.
    let i = std::time::Duration::from_secs(180);
    assert_eq!(Residency::default(), Residency::OnDemand);
    assert!(Residency::OnDemand.keep_alive(i).is_none());
    assert!(Residency::Resident.keep_alive(i).is_some());
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
