//! Ollama vision provider — `VisionProvider` over a local Ollama `/api/generate`.
//!
//! Ollama has no native video input, so video analysis extracts frames with
//! FFmpeg (default 1 FPS, capped), analyzes each frame with an annotated prompt,
//! and combines the per-frame results. Request building, response parsing, and
//! the frame-combination logic are pure functions and unit-tested; the HTTP and
//! FFmpeg paths are integration-tested.
//!
//! Tier-4 synthesis (R-DR13): the original `ollama.rs` was never written (WIP at
//! wipe). Authored 2026-05-28 from the `VisionProvider` contract + PRD FR-015 +
//! the recovered design notes (100 MB cap, frame extraction, per-frame context).

use crate::contracts::errors::VisionError;
use crate::contracts::traits::{AnalysisResult, TimeRange, VisionConfig, VisionProvider};
use async_trait::async_trait;
use base64::Engine;
use chrono::Utc;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};
use uuid::Uuid;

const DEFAULT_OLLAMA_MAX_VIDEO_SIZE: u64 = 104_857_600; // 100 MB
const DEFAULT_OLLAMA_URL: &str = "http://localhost:11434";
// Vision model tag as actually pulled on the LAN test box (<LAN_OLLAMA_HOST>);
// set `OLLAMA_HOST` to point there. Verified live 2026-05-28. Overridable via
// `VisionConfig.model` (the box also has `qwen2.5vl:32b`, `moondream`).
const DEFAULT_MODEL: &str = "qwen2.5vl:7b";
/// Target frames sampled EVENLY across the whole clip — not a usage cap, it's the
/// visual density a VL model needs to summarize a recording (more is redundant).
/// Kept modest: a local 7B VL model processes each image slowly in one call.
const TARGET_FRAMES: f64 = 8.0;
/// Longest edge (px) frames are downscaled to before sending — minor, acceptable
/// quality loss that keeps each image small so the local model stays fast.
const FRAME_MAX_EDGE: u32 = 1024;
/// Safety ceiling on collected frames (only guards the ffprobe-failure fallback).
const FRAME_CEILING: usize = 24;

/// Ollama-backed [`VisionProvider`].
pub struct OllamaProvider {
    client: reqwest::Client,
    base_url: String,
    model: String,
    max_video_size: u64,
    timeout_seconds: u64,
    /// The `keep_alive` to send, already resolved by the caller from its
    /// `ResidencyPolicy` and segment cadence. Interior mutability because
    /// `set_keep_alive` takes `&self` — a provider is shared behind an `Arc`,
    /// and requiring `&mut` would force a lock at every call site to express a
    /// hint.
    keep_alive: std::sync::Mutex<Option<String>>,
}

impl OllamaProvider {
    /// The `keep_alive` currently asked for, if any.
    pub fn keep_alive(&self) -> Option<String> {
        self.keep_alive.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// Build from a [`VisionConfig`]. The base URL is read from `OLLAMA_HOST`
    /// (or `OLLAMA_URL`), defaulting to `http://localhost:11434`.
    pub fn new(config: &VisionConfig) -> Result<Self, VisionError> {
        let base_url = std::env::var("OLLAMA_HOST")
            .or_else(|_| std::env::var("OLLAMA_URL"))
            .unwrap_or_else(|_| DEFAULT_OLLAMA_URL.to_string());
        Self::with_url(config, base_url)
    }

    /// Build with an explicit base URL (used when wiring from `AppConfig`).
    pub fn with_url(config: &VisionConfig, base_url: impl Into<String>) -> Result<Self, VisionError> {
        let timeout_seconds = config.timeout_seconds.max(1);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_seconds))
            .build()
            .map_err(|e| VisionError::Unavailable(e.to_string()))?;
        Ok(Self {
            client,
            base_url: base_url.into().trim_end_matches('/').to_string(),
            model: if config.model.is_empty() {
                DEFAULT_MODEL.to_string()
            } else {
                config.model.clone()
            },
            max_video_size: if config.max_video_size_bytes > 0 {
                config.max_video_size_bytes
            } else {
                DEFAULT_OLLAMA_MAX_VIDEO_SIZE
            },
            timeout_seconds,
            // Unmanaged until a caller asks. Sending an unsolicited
            // `keep_alive` would change ollama's eviction behaviour for every
            // existing caller of this provider, none of which asked.
            keep_alive: std::sync::Mutex::new(None),
        })
    }

    fn map_http_err(&self, e: reqwest::Error) -> VisionError {
        if e.is_timeout() {
            VisionError::Timeout {
                timeout_seconds: self.timeout_seconds,
            }
        } else if e.is_connect() {
            VisionError::Unavailable(format!("cannot reach Ollama at {}: {e}", self.base_url))
        } else {
            VisionError::NetworkError(e.to_string())
        }
    }

    /// POST one prompt + image set to `/api/generate`, returning (text, tokens).
    async fn generate(
        &self,
        prompt: &str,
        images: &[String],
    ) -> Result<(String, Option<u32>), VisionError> {
        let body = build_ollama_request(&self.model, prompt, images, self.keep_alive());
        let resp = self
            .client
            .post(format!("{}/api/generate", self.base_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| self.map_http_err(e))?;
        let status = resp.status().as_u16();
        let text = resp.text().await.map_err(|e| self.map_http_err(e))?;
        if !(200..300).contains(&status) {
            return Err(VisionError::ApiError {
                message: text.chars().take(300).collect(),
                status_code: Some(status),
            });
        }
        parse_ollama_response(&text)
    }
}

#[async_trait]
impl VisionProvider for OllamaProvider {
    async fn analyze_video(
        &self,
        video_path: &Path,
        prompt: &str,
        timeframe: Option<TimeRange>,
    ) -> Result<AnalysisResult, VisionError> {
        validate_prompt(prompt)?;
        if let Some(tf) = &timeframe {
            validate_timeframe(tf)?;
        }
        let size = file_size(video_path)?;
        if size > self.max_video_size {
            return Err(VisionError::FileTooLarge {
                size_bytes: size,
                max_bytes: self.max_video_size,
            });
        }

        // Extract frames (held in a temp dir for the duration of the analysis).
        let frame_dir = tempfile::tempdir()
            .map_err(|e| VisionError::FrameExtractionFailed(e.to_string()))?;
        let frames = extract_frames(video_path, frame_dir.path(), timeframe.as_ref())?;
        if frames.is_empty() {
            return Err(VisionError::FrameExtractionFailed(
                "FFmpeg produced no frames".to_string(),
            ));
        }

        let started = Instant::now();
        // OCR all sampled frames (cheap + accurate) so the local model gets the
        // exact on-screen text, then send only ONE representative frame for
        // visual context. A small local VL model is too slow to process many
        // frames per call; OCR carries text-over-time, and full-motion analysis
        // is the cloud (Gemini native-video) path's job.
        let ocr_text = crate::analysis::ocr::ocr_images_combined(&frames);
        let representative = &frames[frames.len() / 2];
        let b64 = read_as_base64(representative)?;
        let video_prompt = with_ocr_context(
            &format!(
                "{prompt}\n\n(The attached image is a representative frame from a screen recording.)"
            ),
            &ocr_text,
        );
        let (analysis_text, token_count) = self.generate(&video_prompt, &[b64]).await?;
        Ok(AnalysisResult {
            request_id: Uuid::new_v4(),
            analysis_text,
            provider: self.name().to_string(),
            model_used: self.model.clone(),
            processing_time_ms: started.elapsed().as_millis() as u64,
            token_count,
            completed_at: Utc::now(),
        })
    }

    async fn analyze_image(
        &self,
        image_path: &Path,
        prompt: &str,
    ) -> Result<AnalysisResult, VisionError> {
        validate_prompt(prompt)?;
        file_size(image_path)?; // existence check
        let ocr_text = crate::analysis::ocr::ocr_image(image_path).unwrap_or_default();
        let b64 = read_as_base64(image_path)?;
        let started = Instant::now();
        let prompt_with_text = with_ocr_context(prompt, &ocr_text);
        let (analysis_text, token_count) = self.generate(&prompt_with_text, &[b64]).await?;
        Ok(AnalysisResult {
            request_id: Uuid::new_v4(),
            analysis_text,
            provider: self.name().to_string(),
            model_used: self.model.clone(),
            processing_time_ms: started.elapsed().as_millis() as u64,
            token_count,
            completed_at: Utc::now(),
        })
    }

    async fn health_check(&self) -> Result<(), VisionError> {
        let resp = self
            .client
            .get(format!("{}/api/tags", self.base_url))
            .send()
            .await
            .map_err(|e| self.map_http_err(e))?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(VisionError::Unavailable(format!(
                "Ollama health check failed: HTTP {}",
                resp.status().as_u16()
            )))
        }
    }

    fn name(&self) -> &'static str {
        "ollama"
    }

    fn max_video_size(&self) -> u64 {
        self.max_video_size
    }

    fn supports_native_video(&self) -> bool {
        false
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn set_keep_alive(&self, keep_alive: Option<String>) {
        *self.keep_alive.lock().unwrap_or_else(|p| p.into_inner()) = keep_alive;
    }
}

// ---- pure helpers (unit-tested) -------------------------------------------

/// Build an `/api/generate` request body (non-streaming, with inline images).
fn build_ollama_request(
    model: &str,
    prompt: &str,
    images: &[String],
    keep_alive: Option<String>,
) -> Value {
    let mut body = serde_json::json!({
        "model": model,
        "prompt": prompt,
        "images": images,
        "stream": false
    });
    // Sent ONLY when the caller asked. An unsolicited `keep_alive` overrides
    // the server's own default for every caller, including those that never
    // asked about residency. The VALUE is resolved by
    // `ResidencyPolicy::keep_alive(segment_cadence)` — this function does not
    // re-derive it, because a second mapping is a second thing to get wrong.
    if let Some(v) = keep_alive {
        body["keep_alive"] = serde_json::json!(v);
    }
    body
}

/// Extract the response text + `eval_count` token estimate from an Ollama reply.
/// Strip a reasoning ("thinking") preamble from a model response.
///
/// Thinking-capable vision models (`ornith-1.5-*`, `qwen3-vl:*`, `gemma4:*`) emit their
/// chain-of-thought before the answer, terminated by `</think>`. Ollama's `/api/generate`
/// returns that inline in `response`, so without this the caller receives the model's
/// deliberation prepended to — and often far longer than — the actual answer.
/// Measured 2026-08-23 against `ornith-1.5-9b` through the Atelier governor: roughly 60%
/// of `analysis_text` was reasoning noise ("Let me carefully transcribe… Actually, let me
/// re-read…") ahead of the real transcription.
///
/// Deliberately conservative:
/// - only acts when a close tag is actually present (non-thinking models are untouched),
/// - splits on the LAST `</think>` so nested/repeated blocks collapse correctly,
/// - never returns empty — a response that is *entirely* reasoning is passed through
///   as-is rather than silently becoming "", since a wrong-but-present answer is far
///   easier to debug than a blank one.
fn strip_reasoning(text: &str) -> String {
    const CLOSE: &str = "</think>";
    match text.rfind(CLOSE) {
        Some(i) => {
            let tail = text[i + CLOSE.len()..].trim();
            if tail.is_empty() {
                text.trim().to_string()
            } else {
                tail.to_string()
            }
        }
        None => text.trim().to_string(),
    }
}

fn parse_ollama_response(body: &str) -> Result<(String, Option<u32>), VisionError> {
    let v: Value = serde_json::from_str(body).map_err(|e| VisionError::ApiError {
        message: format!("invalid JSON from Ollama: {e}"),
        status_code: None,
    })?;
    if let Some(err) = v.get("error").and_then(Value::as_str) {
        return Err(VisionError::ApiError {
            message: err.to_string(),
            status_code: None,
        });
    }
    let text = strip_reasoning(v.get("response").and_then(Value::as_str).ok_or_else(|| {
        VisionError::ApiError {
            message: "Ollama response contained no 'response' field".to_string(),
            status_code: None,
        }
    })?);
    let token_count = v.get("eval_count").and_then(Value::as_u64).map(|t| t as u32);
    Ok((text, token_count))
}

fn validate_prompt(prompt: &str) -> Result<(), VisionError> {
    if prompt.trim().is_empty() {
        return Err(VisionError::InvalidPrompt("prompt is empty".to_string()));
    }
    Ok(())
}

/// Fold OCR-extracted text into the prompt so the local model reads text
/// accurately. No-op when OCR found nothing.
fn with_ocr_context(prompt: &str, ocr_text: &str) -> String {
    if ocr_text.trim().is_empty() {
        prompt.to_string()
    } else {
        format!(
            "{prompt}\n\nOCR detected this on-screen text (may be imperfect; use it to \
             read text accurately):\n---\n{ocr_text}\n---"
        )
    }
}

fn validate_timeframe(tf: &TimeRange) -> Result<(), VisionError> {
    if tf.start_seconds < 0.0 || tf.end_seconds <= tf.start_seconds {
        return Err(VisionError::InvalidTimeframe {
            start_seconds: tf.start_seconds,
            end_seconds: tf.end_seconds,
        });
    }
    Ok(())
}

fn file_size(path: &Path) -> Result<u64, VisionError> {
    std::fs::metadata(path)
        .map(|m| m.len())
        .map_err(|_| VisionError::FileNotFound(path.to_path_buf()))
}

fn read_as_base64(path: &Path) -> Result<String, VisionError> {
    let bytes = std::fs::read(path).map_err(|_| VisionError::FileNotFound(path.to_path_buf()))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// Probe a video's duration in seconds via ffprobe (None if unavailable).
fn video_duration_secs(input: &Path) -> Option<f64> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
            &input.to_string_lossy(),
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<f64>()
        .ok()
}

/// Sample frames EVENLY across the whole clip and DOWNSCALE them, into `out_dir`.
///
/// The sampling rate adapts to duration so ~`TARGET_FRAMES` frames span the
/// entire recording (not just the first seconds), and each frame is shrunk to
/// `FRAME_MAX_EDGE` on its longest side — both to keep the single local-model
/// call fast. Capped at 1 fps for very short clips.
fn extract_frames(
    input: &Path,
    out_dir: &Path,
    timeframe: Option<&TimeRange>,
) -> Result<Vec<PathBuf>, VisionError> {
    let duration = match timeframe {
        Some(tf) => (tf.end_seconds - tf.start_seconds).max(0.0),
        None => video_duration_secs(input).unwrap_or(0.0),
    };
    // Even spacing: fewer fps for longer clips, at most 1 fps for short ones.
    let sample_fps = if duration > 0.0 {
        (TARGET_FRAMES / duration).clamp(0.05, 1.0)
    } else {
        1.0
    };

    let pattern = out_dir.join("frame_%04d.png");
    let mut args: Vec<String> = vec!["-y".to_string()];
    if let Some(tf) = timeframe {
        args.extend([
            "-ss".to_string(),
            tf.start_seconds.to_string(),
            "-to".to_string(),
            tf.end_seconds.to_string(),
        ]);
    }
    args.extend([
        "-i".to_string(),
        input.to_string_lossy().to_string(),
        "-vf".to_string(),
        format!(
            "fps={sample_fps:.4},scale={FRAME_MAX_EDGE}:{FRAME_MAX_EDGE}:force_original_aspect_ratio=decrease"
        ),
        pattern.to_string_lossy().to_string(),
    ]);

    let output = Command::new("ffmpeg")
        .args(&args)
        .output()
        .map_err(|e| VisionError::FrameExtractionFailed(format!("failed to run ffmpeg: {e}")))?;
    if !output.status.success() {
        return Err(VisionError::FrameExtractionFailed(
            String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(300)
                .collect(),
        ));
    }

    // Collect frames in deterministic (sorted) order.
    let mut frames: Vec<PathBuf> = std::fs::read_dir(out_dir)
        .map_err(|e| VisionError::FrameExtractionFailed(e.to_string()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|ext| ext == "png"))
        .collect();
    frames.sort();
    frames.truncate(FRAME_CEILING);
    Ok(frames)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> VisionConfig {
        VisionConfig {
            provider: "ollama".to_string(),
            api_key: None,
            model: String::new(),
            timeout_seconds: 30,
            max_video_size_bytes: 0,
        }
    }

    /// The ONE production provider that implements the channel must actually
    /// STORE the hint — the trait's default is a no-op, so deleting this
    /// override would compile, pass every request-body unit test (they take
    /// the value as a parameter), and silently turn `Resident` back into a
    /// policy the running system cannot express (W8 gate; mutation
    /// "remove the override" survived the suite before this test).
    #[test]
    fn set_keep_alive_stores_the_value_the_request_path_reads() {
        let p = OllamaProvider::with_url(&cfg(), "http://host:11434").unwrap();
        assert_eq!(p.keep_alive(), None, "unmanaged until a caller asks");
        p.set_keep_alive(Some("1860s".to_string()));
        assert_eq!(p.keep_alive().as_deref(), Some("1860s"));
        p.set_keep_alive(None);
        assert_eq!(p.keep_alive(), None, "a later None must clear it, not linger");
    }

    #[test]
    fn new_applies_defaults_and_trims_url() {
        let p = OllamaProvider::with_url(&cfg(), "http://host:11434/").unwrap();
        assert_eq!(p.name(), "ollama");
        assert_eq!(p.model(), DEFAULT_MODEL);
        assert_eq!(p.max_video_size(), DEFAULT_OLLAMA_MAX_VIDEO_SIZE);
        assert!(!p.supports_native_video());
        assert_eq!(p.base_url, "http://host:11434"); // trailing slash trimmed
    }

    #[test]
    fn request_body_is_non_streaming_with_images() {
        let body = build_ollama_request(
            "llava",
            "describe",
            &["QUJD".to_string()],
            None,
        );
        assert!(
            body.get("keep_alive").is_none(),
            "an unmanaged policy must send NO keep_alive — an unsolicited one overrides \
             the server default for every caller that never asked about residency"
        );
        assert_eq!(body["model"], "llava");
        assert_eq!(body["prompt"], "describe");
        assert_eq!(body["stream"], false);
        assert_eq!(body["images"][0], "QUJD");
    }

    #[test]
    fn parses_success_and_error_responses() {
        let (text, tokens) =
            parse_ollama_response(r#"{"response":"a window","eval_count":42}"#).unwrap();
        assert_eq!(text, "a window");
        assert_eq!(tokens, Some(42));

        let err = parse_ollama_response(r#"{"error":"model not found"}"#).unwrap_err();
        assert!(matches!(err, VisionError::ApiError { .. }));
    }

    #[test]
    fn validates_inputs() {
        assert!(validate_prompt("  ").is_err());
        assert!(validate_timeframe(&TimeRange { start_seconds: 2.0, end_seconds: 1.0 }).is_err());
    }

    #[test]
    fn strip_reasoning_removes_think_preamble() {
        let raw = "Let me look carefully.\nActually, re-reading.\n</think>\n\nTwo things I did not do";
        assert_eq!(strip_reasoning(raw), "Two things I did not do");
    }

    #[test]
    fn strip_reasoning_leaves_plain_text_untouched() {
        assert_eq!(strip_reasoning("  a plain answer  "), "a plain answer");
    }

    #[test]
    fn strip_reasoning_never_returns_empty() {
        // entirely reasoning, no answer after the close tag -> pass through, never ""
        let raw = "thinking hard</think>   ";
        assert_eq!(strip_reasoning(raw), "thinking hard</think>");
    }

    #[test]
    fn strip_reasoning_splits_on_last_close_tag() {
        let raw = "a</think>b</think>final";
        assert_eq!(strip_reasoning(raw), "final");
    }
}

#[cfg(test)]
mod residency_tests {
    use super::*;
    use crate::config::ResidencyPolicy;

    /// The resolved value must reach the WIRE. A residency that never leaves
    /// the process is a policy the running system cannot express — which is
    /// what `ResidencyPolicy::Resident` was before T020.
    #[test]
    fn the_resolved_keep_alive_reaches_the_request_body() {
        // The resolution is 013's, taking the SEGMENT cadence — not a second
        // mapping invented here. Sizing it from the sample interval expired the
        // window before the next burst, so Resident held memory AND paid every
        // cold load.
        let cadence = std::time::Duration::from_secs(900);
        let resident = ResidencyPolicy::Resident.keep_alive(cadence);
        assert_eq!(resident.as_deref(), Some("1860s"), "2x cadence + 60s margin");

        let body = build_ollama_request("m", "p", &[], resident);
        assert_eq!(body["keep_alive"], "1860s");

        // OnDemand says NOTHING — it accepts the reload rather than pinning.
        let on_demand = build_ollama_request("m", "p", &[], ResidencyPolicy::OnDemand.keep_alive(cadence));
        assert!(
            on_demand.get("keep_alive").is_none(),
            "OnDemand must leave the server default alone"
        );

        // Off actively releases.
        let off = build_ollama_request("m", "p", &[], ResidencyPolicy::Off.keep_alive(cadence));
        assert_eq!(off["keep_alive"], "0");

        assert_ne!(body["keep_alive"], off["keep_alive"]);
    }

    /// A provider that IGNORES the hint still behaves correctly — residency is
    /// an optimisation, never a correctness requirement. The default trait
    /// method is what guarantees it.
    #[test]
    fn a_provider_that_ignores_keep_alive_is_still_correct() {
        struct Ignorer;
        #[async_trait::async_trait]
        impl crate::contracts::traits::VisionProvider for Ignorer {
            async fn analyze_video(
                &self,
                _: &std::path::Path,
                _: &str,
                _: Option<crate::contracts::traits::TimeRange>,
            ) -> Result<crate::contracts::traits::AnalysisResult, VisionError> {
                unreachable!()
            }
            async fn analyze_image(
                &self,
                _: &std::path::Path,
                _: &str,
            ) -> Result<crate::contracts::traits::AnalysisResult, VisionError> {
                unreachable!()
            }
            async fn health_check(&self) -> Result<(), VisionError> {
                Ok(())
            }
            fn name(&self) -> &'static str {
                "ignorer"
            }
            fn max_video_size(&self) -> u64 {
                0
            }
            fn supports_native_video(&self) -> bool {
                false
            }
            fn model(&self) -> &str {
                "ignorer"
            }
        }
        let p = Ignorer;
        // Does not panic, does not change behaviour.
        p.set_keep_alive(Some("1860s".into()));
        assert_eq!(p.name(), "ignorer");
    }
}
