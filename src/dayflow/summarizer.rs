//! Rust-native Map-Reduce chunk summarizer (Wave 3 · PRD dayflow P2 · D1).
//!
//! Ports videolocr's `process_video_chunks_with_gemini`: each chunk is summarized
//! by a [`VisionProvider`] (Gemini native-video by default), threading a rolling
//! `CONTEXT SUMMARY FOR NEXT CHUNK` forward (the Map step), then reducing the
//! per-chunk summaries into a session digest (the Reduce step). Provider-agnostic
//! and fully unit-testable with a stub provider.

use async_trait::async_trait;
use std::sync::Arc;

use crate::contracts::errors::DayflowError;
use crate::contracts::traits::VisionProvider;
use crate::dayflow::models::{ActivityCategory, ChunkRef, ChunkSummary, RollingContext};

/// Summarize one chunk into a structured [`ChunkSummary`], given the rolling
/// context accumulated from prior chunks.
#[async_trait]
pub trait ChunkSummarizer {
    async fn summarize_chunk(
        &self,
        chunk: &ChunkRef,
        prior: &RollingContext,
    ) -> Result<ChunkSummary, DayflowError>;
}

/// A [`VisionProvider`]-backed summarizer. The provider choice (Gemini cloud /
/// Ollama local) is the caller's; this just drives the prompt + parses the result.
pub struct VisionChunkSummarizer {
    provider: Arc<dyn VisionProvider>,
}

impl VisionChunkSummarizer {
    pub fn new(provider: Arc<dyn VisionProvider>) -> Self {
        Self { provider }
    }
}

/// Build the per-chunk prompt, threading the prior rolling context forward
/// (videolocr's `CONTEXT SUMMARY FOR NEXT CHUNK`). The Map step's input.
pub fn build_chunk_prompt(prior: &RollingContext) -> String {
    let context = if prior.is_empty() {
        "(this is the first chunk; no prior context)".to_string()
    } else {
        prior.summary.clone()
    };
    format!(
        "You are summarizing one screen-recording chunk into a single activity record.\n\
         CONTEXT SUMMARY FROM PRIOR CHUNKS:\n{context}\n\n\
         Return ONLY a JSON object: \
         {{\"category\":\"{categories}\",\
         \"app\":\"...\",\"activity\":\"short label\",\"detail\":\"1-2 sentences\"}}",
        categories = ActivityCategory::ALL
            .iter()
            .map(|c| c.wire_name())
            .collect::<Vec<_>>()
            .join("|")
    )
}

#[async_trait]
impl ChunkSummarizer for VisionChunkSummarizer {
    async fn summarize_chunk(
        &self,
        chunk: &ChunkRef,
        prior: &RollingContext,
    ) -> Result<ChunkSummary, DayflowError> {
        let prompt = build_chunk_prompt(prior);
        // VisionError → DayflowError::Summarization via #[from].
        let result = self.provider.analyze_video(&chunk.path, &prompt, None).await?;
        Ok(parse_chunk_summary(chunk, &result.analysis_text))
    }
}

/// A [`ChunkSummarizer`] that routes through the two-tier perception ladder.
///
/// This is what T031 asks for, and the difference from [`VisionChunkSummarizer`]
/// is the cost shape, not the output: that one sends the whole segment to a
/// single vision model, while this reads every sample with the cheap text tier
/// and consults the reasoning tier ONCE per segment. Over an eight-hour day
/// that is the difference between a feature that can run all day and one that
/// cannot (research R19).
pub struct RoutedChunkSummarizer {
    router: Arc<crate::dayflow::perception::PerceptionRouter>,
    sample_dir: std::path::PathBuf,
    max_regions: usize,
    latencies: std::sync::Mutex<Vec<crate::dayflow::perception::SegmentLatency>>,
}

impl RoutedChunkSummarizer {
    /// Route through `router`, resolving each window's samples under `sample_dir`.
    pub fn new(
        router: Arc<crate::dayflow::perception::PerceptionRouter>,
        sample_dir: impl Into<std::path::PathBuf>,
    ) -> Self {
        Self {
            router,
            sample_dir: sample_dir.into(),
            max_regions: crate::config::PerceptionConfig::default().max_regions_per_segment
                as usize,
            latencies: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Apply a residency policy to the router's text tier, ON CONSTRUCTION.
    ///
    /// Deliberately part of building the summariser rather than a separate call
    /// the caller must remember: a residency that is configured but never
    /// applied is exactly the state `ResidencyPolicy::Resident` was in before
    /// T020 — a policy the running system could not express, with nothing
    /// failing to say so.
    ///
    /// Takes the SEGMENT cadence because that is the gap the model must survive
    /// to stay warm; 013 measured that sizing it from the sample interval
    /// expired the window before the next burst.
    pub fn with_residency(
        self,
        policy: crate::config::ResidencyPolicy,
        segment_cadence: std::time::Duration,
    ) -> Self {
        self.router.apply_residency(policy, segment_cadence);
        self
    }

    /// Bound the crops taken per sample. This is the same number the perception
    /// budget is derived from, so the two must be configured together.
    pub fn with_max_regions(mut self, n: usize) -> Self {
        self.max_regions = n;
        self
    }

    /// Per-segment perception cost so far (FR-013).
    pub fn latencies(&self) -> Vec<crate::dayflow::perception::SegmentLatency> {
        match self.latencies.lock() {
            Ok(v) => v.clone(),
            Err(p) => p.into_inner().clone(),
        }
    }

    /// The sample files belonging to `chunk`, in capture order.
    ///
    /// Resolved by the sampler's own [`sample_prefix`](crate::dayflow::sampler::sample_prefix)
    /// rather than a second copy of the naming rule, so the reader cannot drift
    /// from the writer. The names embed a sortable timestamp, so a lexical sort
    /// IS capture order.
    pub fn samples_for(&self, chunk: &ChunkRef) -> Result<Vec<std::path::PathBuf>, DayflowError> {
        // `sequence`, NEVER `index`: index is a per-run counter that resets on
        // every pause, resume, interval change and display change, so after any
        // pause it would resolve to a DIFFERENT window's samples — silently
        // summarising the wrong slice of the day. The durable identity is
        // (session, display_id, sequence), which is what the sampler wrote.
        let prefix = crate::dayflow::sampler::sample_prefix(chunk.display_id, chunk.sequence);
        let entries = std::fs::read_dir(&self.sample_dir).map_err(|e| {
            DayflowError::Internal(format!("read {}: {e}", self.sample_dir.display()))
        })?;
        let mut found: Vec<std::path::PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(&prefix) && n.ends_with(".png"))
            })
            .collect();
        found.sort();
        Ok(found)
    }
}

#[async_trait]
impl ChunkSummarizer for RoutedChunkSummarizer {
    async fn summarize_chunk(
        &self,
        chunk: &ChunkRef,
        prior: &RollingContext,
    ) -> Result<ChunkSummary, DayflowError> {
        let samples = self.samples_for(chunk)?;
        let (text, latency) = crate::dayflow::perception::summarize_segment_via_ladder(
            &self.router,
            &samples,
            &build_chunk_prompt(prior),
            chunk.display_id,
            self.max_regions,
        )
        .await?;
        tracing::info!(
            sequence = chunk.sequence,
            samples = latency.samples,
            calls = latency.perception_calls,
            total_ms = latency.total.as_millis() as u64,
            first_call_ms = latency.first_call.as_millis() as u64,
            mean_warm_ms = latency.mean_warm_call().as_millis() as u64,
            cold_load = latency.paid_a_cold_load(),
            read_whole = latency.samples_read_whole,
            "dayflow segment perception"
        );
        if let Ok(mut v) = self.latencies.lock() {
            v.push(latency);
        }
        Ok(parse_chunk_summary(chunk, &text))
    }
}

/// Parse the provider's text into a structured [`ChunkSummary`] (JSON when
/// present; otherwise a safe fallback that preserves the raw text in `detail`).
fn parse_chunk_summary(chunk: &ChunkRef, text: &str) -> ChunkSummary {
    let parsed = extract_json(text)
        .and_then(|j| serde_json::from_str::<serde_json::Value>(&j).ok());
    let (category, app, activity, detail) = match parsed {
        Some(v) => (
            v.get("category")
                .and_then(|x| x.as_str())
                .map(category_from)
                .unwrap_or(ActivityCategory::Other),
            v.get("app").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            v.get("activity").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            v.get("detail").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        ),
        None => (
            ActivityCategory::Other,
            String::new(),
            text.chars().take(80).collect(),
            text.to_string(),
        ),
    };
    ChunkSummary {
        chunk_index: chunk.index,
        start_time: chunk.start_wall,
        end_time: chunk.end_wall,
        category,
        app,
        activity,
        detail,
    }
}

fn category_from(s: &str) -> ActivityCategory {
    // TRIMMED as well as lowercased: a model emitting `"  docs  "` is emitting
    // docs, and filing it as Other loses a classification for a whitespace
    // difference nobody would call an error.
    serde_json::from_str(&format!("\"{}\"", s.trim().to_lowercase()))
        .unwrap_or(ActivityCategory::Other)
}

fn category_str(c: &ActivityCategory) -> String {
    serde_json::to_string(c)
        .unwrap_or_default()
        .trim_matches('"')
        .to_string()
}

fn extract_json(text: &str) -> Option<String> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    (end > start).then(|| text[start..=end].to_string())
}

/// Map step: advance the rolling context with a freshly-summarized chunk, so the
/// next chunk's prompt carries everything seen so far.
pub fn advance_context(prior: &RollingContext, summary: &ChunkSummary) -> RollingContext {
    let line = format!(
        "[chunk {}] {} — {}",
        summary.chunk_index,
        category_str(&summary.category),
        summary.activity
    );
    let merged = if prior.is_empty() {
        line
    } else {
        format!("{}\n{}", prior.summary, line)
    };
    RollingContext { summary: merged }
}

/// Reduce step: combine per-chunk summaries into a human-readable session digest.
pub fn reduce(summaries: &[ChunkSummary]) -> String {
    if summaries.is_empty() {
        return "No activity recorded.".to_string();
    }
    let mut lines = vec![format!("Session digest ({} chunks):", summaries.len())];
    for cs in summaries {
        lines.push(format!(
            "- {}–{}: [{}] {}",
            cs.start_time.format("%H:%M"),
            cs.end_time.format("%H:%M"),
            category_str(&cs.category),
            cs.activity
        ));
    }
    lines.join("\n")
}

/// Map-Reduce driver: summarize each chunk threading context (Map), then reduce.
pub async fn summarize_chunks(
    summarizer: &dyn ChunkSummarizer,
    chunks: &[ChunkRef],
) -> Result<(Vec<ChunkSummary>, String), DayflowError> {
    let mut ctx = RollingContext::default();
    let mut summaries = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        let summary = summarizer.summarize_chunk(chunk, &ctx).await?;
        ctx = advance_context(&ctx, &summary);
        summaries.push(summary);
    }
    let digest = reduce(&summaries);
    Ok((summaries, digest))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::errors::VisionError;
    use crate::contracts::traits::{AnalysisResult, TimeRange};
    use crate::dayflow::chunking::plan_chunks;
    use chrono::{TimeZone, Utc};
    use std::path::Path;
    use std::sync::Mutex;
    use uuid::Uuid;

    /// Stub provider: records each prompt it received (to prove context threading)
    /// and returns canned JSON per call.
    struct StubProvider {
        prompts: Mutex<Vec<String>>,
        replies: Vec<String>,
    }

    #[async_trait]
    impl VisionProvider for StubProvider {
        async fn analyze_video(
            &self,
            _video: &Path,
            prompt: &str,
            _tf: Option<TimeRange>,
        ) -> Result<AnalysisResult, VisionError> {
            let idx = {
                let mut g = self.prompts.lock().unwrap();
                g.push(prompt.to_string());
                g.len() - 1
            };
            let text = self
                .replies
                .get(idx)
                .cloned()
                .unwrap_or_default();
            Ok(AnalysisResult {
                request_id: Uuid::new_v4(),
                analysis_text: text,
                provider: "stub".into(),
                model_used: "stub".into(),
                processing_time_ms: 0,
                token_count: None,
                completed_at: Utc::now(),
            })
        }
        async fn analyze_image(&self, _: &Path, _: &str) -> Result<AnalysisResult, VisionError> {
            Err(VisionError::Unavailable("stub".into()))
        }
        async fn health_check(&self) -> Result<(), VisionError> {
            Ok(())
        }
        fn name(&self) -> &'static str {
            "stub"
        }
        fn max_video_size(&self) -> u64 {
            u64::MAX
        }
        fn supports_native_video(&self) -> bool {
            true
        }
        fn model(&self) -> &str {
            "stub"
        }
    }

    fn t0() -> chrono::DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000, 0).unwrap()
    }

    #[tokio::test]
    async fn map_reduce_threads_context_parses_and_reduces() {
        let replies = vec![
            r#"{"category":"coding","app":"editor","activity":"writing rust","detail":"impl summarizer"}"#.to_string(),
            r#"{"category":"docs","app":"browser","activity":"reading docs","detail":"rmcp docs"}"#.to_string(),
            r#"{"category":"comms","app":"slack","activity":"standup","detail":"team sync"}"#.to_string(),
        ];
        let stub = Arc::new(StubProvider {
            prompts: Mutex::new(Vec::new()),
            replies,
        });
        let summarizer = VisionChunkSummarizer::new(stub.clone());
        let chunks = plan_chunks(t0(), 45 * 60, 15, Path::new("/rec")); // 3 chunks

        let (summaries, digest) = summarize_chunks(&summarizer, &chunks).await.unwrap();

        // T230 — structured parse of each chunk.
        assert_eq!(summaries.len(), 3);
        assert_eq!(summaries[0].category, ActivityCategory::Coding);
        assert_eq!(summaries[0].activity, "writing rust");
        assert_eq!(summaries[0].chunk_index, 0);
        assert_eq!(summaries[0].start_time, chunks[0].start_wall);

        // T231 — the rolling context threads forward into later prompts.
        let prompts = stub.prompts.lock().unwrap();
        assert!(prompts[0].contains("no prior context"));
        assert!(prompts[1].contains("[chunk 0]"), "chunk 1 prompt carries chunk 0");
        assert!(prompts[1].contains("writing rust"));
        assert!(prompts[2].contains("[chunk 1]"), "chunk 2 prompt carries chunk 1");
        assert!(prompts[2].contains("reading docs"));

        // T232/T233 — the reduce digest mentions every chunk's activity.
        assert!(digest.contains("writing rust"));
        assert!(digest.contains("reading docs"));
        assert!(digest.contains("standup"));
        assert!(digest.contains("3 chunks"));
    }

    #[tokio::test]
    async fn non_json_reply_falls_back_safely() {
        let stub = Arc::new(StubProvider {
            prompts: Mutex::new(Vec::new()),
            replies: vec!["the user was coding in the editor".to_string()],
        });
        let summarizer = VisionChunkSummarizer::new(stub);
        let chunks = plan_chunks(t0(), 10 * 60, 15, Path::new("/rec")); // 1 chunk
        let (summaries, _) = summarize_chunks(&summarizer, &chunks).await.unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].category, ActivityCategory::Other);
        assert!(summaries[0].detail.contains("coding in the editor"));
    }

    // ─── T031: the ladder is actually reachable from the summarizer ────────

    #[tokio::test]
    async fn the_routed_summarizer_reads_a_windows_samples_and_reasons_once() {
        // The wiring, end to end: a ChunkSummarizer whose per-sample work goes
        // to the text tier and whose ONE reasoning call carries what those
        // samples extracted. Previously the router existed but nothing called
        // it, so every guarantee it makes protected traffic that did not exist.
        use crate::dayflow::perception::{PerceptionRouter, Tier};
        use crate::dayflow::sampler::sample_prefix;

        let dir = tempfile::tempdir().unwrap();
        // Three samples of window 7 on display 2, plus decoys that must be
        // ignored: another window, another display, and a non-PNG.
        for name in [
            format!("{}20260826T100000000.png", sample_prefix(2, 7)),
            format!("{}20260826T100300000.png", sample_prefix(2, 7)),
            format!("{}20260826T100600000.png", sample_prefix(2, 7)),
            format!("{}20260826T100000000.png", sample_prefix(2, 8)),
            format!("{}20260826T100000000.png", sample_prefix(3, 7)),
            format!("{}notes.txt", sample_prefix(2, 7)),
        ] {
            std::fs::write(dir.path().join(name), b"x").unwrap();
        }

        let text_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let reason_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let router = Arc::new(PerceptionRouter::new(
            Arc::new(TierProvider { tier: Tier::Text, calls: text_calls.clone() }),
            Arc::new(TierProvider { tier: Tier::Reason, calls: reason_calls.clone() }),
            1_000,
        ));
        let s = RoutedChunkSummarizer::new(router.clone(), dir.path());

        // index and sequence DELIBERATELY differ: index resets on every pause,
        // so a resolver keyed on it reads another window's samples after one.
        let chunk = ChunkRef {
            index: 0,
            path: dir.path().join("unused.mp4"),
            start_wall: Utc::now(),
            end_wall: Utc::now(),
            display_id: 2,
            sequence: 7,
            summarized: false,
        };

        let resolved = s.samples_for(&chunk).unwrap();
        assert_eq!(
            resolved.len(),
            3,
            "resolved by SEQUENCE (7), not index (0): {resolved:?}"
        );
        assert!(
            resolved.iter().all(|p| p.to_string_lossy().contains("w000007")),
            "an index-keyed resolver would have picked window 0's samples"
        );
        assert!(resolved.windows(2).all(|w| w[0] <= w[1]), "and in capture order");

        let summary = s.summarize_chunk(&chunk, &RollingContext::default()).await.unwrap();
        assert_eq!(text_calls.load(std::sync::atomic::Ordering::SeqCst), 3, "one read per sample");
        assert_eq!(
            reason_calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "and the segment is reasoned about ONCE"
        );
        assert_eq!(router.escalations().len(), 1, "with an audit record");
        assert_eq!(summary.category, ActivityCategory::Coding, "the reasoning tier's answer");
    }

    /// Answers as whichever tier it stands for, so a test can tell which one
    /// produced the summary.
    struct TierProvider {
        tier: crate::dayflow::perception::Tier,
        calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait]
    impl VisionProvider for TierProvider {
        async fn analyze_video(
            &self,
            _: &std::path::Path,
            _: &str,
            _: Option<crate::contracts::traits::TimeRange>,
        ) -> Result<crate::contracts::traits::AnalysisResult, VisionError> {
            unreachable!("the ladder never analyses video")
        }
        async fn analyze_image(
            &self,
            _: &std::path::Path,
            _: &str,
        ) -> Result<crate::contracts::traits::AnalysisResult, VisionError> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let text = match self.tier {
                crate::dayflow::perception::Tier::Text => "raw screen text".to_string(),
                crate::dayflow::perception::Tier::Reason => {
                    r#"{"category":"coding","app":"editor","activity":"a","detail":"d"}"#.to_string()
                }
            };
            Ok(crate::contracts::traits::AnalysisResult {
                request_id: uuid::Uuid::new_v4(),
                analysis_text: text,
                provider: "stub".into(),
                model_used: "stub".into(),
                processing_time_ms: 1,
                token_count: None,
                completed_at: Utc::now(),
            })
        }
        async fn health_check(&self) -> Result<(), VisionError> {
            Ok(())
        }
        fn name(&self) -> &'static str {
            "stub"
        }
        fn max_video_size(&self) -> u64 {
            0
        }
        fn supports_native_video(&self) -> bool {
            false
        }
        fn model(&self) -> &str {
            "stub"
        }
    }

    #[test]
    fn the_prompt_lists_every_category_in_the_taxonomy() {
        // T045. The list used to be a string literal, which drifts silently:
        // add a variant and the model is never told about it, so every entry
        // that should carry it comes back as `Other` and nothing reports a
        // problem. Derived from the enum now, and asserted against it.
        let prompt = build_chunk_prompt(&RollingContext::default());
        // The ALTERNATION, not seven independent substrings: joining the names
        // with "" instead of "|" satisfies `contains` for every member while
        // offering the model one garbage token as its whole schema — and that
        // mutation survived the suite.
        let alternation = ActivityCategory::ALL
            .iter()
            .map(|c| c.wire_name())
            .collect::<Vec<_>>()
            .join("|");
        assert!(
            prompt.contains(&alternation),
            "the prompt must offer the categories as alternatives, not as one \
             run-together token. Expected `{alternation}` in: {prompt}"
        );
    }

    #[test]
    fn every_category_the_parser_produces_is_a_taxonomy_member() {
        // Including the ones it does not recognise: an unknown token must fall
        // back to `Other` rather than being dropped, because dropping the
        // category silently loses the entry's classification while keeping the
        // entry — the reader then sees an uncategorised hour and no reason why.
        let chunk = ChunkRef {
            index: 0,
            path: std::path::PathBuf::from("/tmp/x.mp4"),
            start_wall: Utc::now(),
            end_wall: Utc::now(),
            display_id: 0,
            sequence: 0,
            summarized: false,
        };
        // `ALL.contains(&category)` is a TAUTOLOGY — it cannot fail for any
        // value of the type — so the previous version of this test pinned
        // nothing: changing either fallback from Other to Meeting survived the
        // whole suite, and "wat" silently became a meeting.
        //
        // Assert the actual mapping instead, in both directions.
        for c in ActivityCategory::ALL {
            let json = format!(
                r#"{{"category":"{}","app":"a","activity":"b","detail":"c"}}"#,
                c.wire_name()
            );
            assert_eq!(
                parse_chunk_summary(&chunk, &json).category,
                *c,
                "`{}` must round-trip to itself",
                c.wire_name()
            );
        }

        // Case and padding are variance, not error.
        for (token, expect) in [("CODING", ActivityCategory::Coding), ("  docs  ", ActivityCategory::Docs)] {
            let json = format!(r#"{{"category":"{token}","app":"a","activity":"b","detail":"c"}}"#);
            assert_eq!(parse_chunk_summary(&chunk, &json).category, expect, "token {token:?}");
        }

        // Anything unrecognisable is Other — NOT some other real category,
        // which would file unknown work under a specific heading and look
        // deliberate in the digest.
        for token in ["wat", "", "null", "42", "meetingg"] {
            let json = format!(r#"{{"category":"{token}","app":"a","activity":"b","detail":"c"}}"#);
            assert_eq!(
                parse_chunk_summary(&chunk, &json).category,
                ActivityCategory::Other,
                "unknown token {token:?} must fall back to Other"
            );
        }

        // And a MISSING category key, which is a different code path.
        let no_key = r#"{"app":"a","activity":"b","detail":"c"}"#;
        assert_eq!(parse_chunk_summary(&chunk, no_key).category, ActivityCategory::Other);
    }
}
