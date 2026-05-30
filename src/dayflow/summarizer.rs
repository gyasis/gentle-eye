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
         {{\"category\":\"coding|docs|comms|browsing|meeting|idle|other\",\
         \"app\":\"...\",\"activity\":\"short label\",\"detail\":\"1-2 sentences\"}}"
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
    serde_json::from_str(&format!("\"{}\"", s.to_lowercase())).unwrap_or(ActivityCategory::Other)
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
}
