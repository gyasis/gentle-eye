//! Gemini vision provider — `VisionProvider` over the Generative Language API.
//!
//! Supports native video (base64-inline) and images. For a requested timeframe
//! the video is first trimmed with FFmpeg. Request building and response parsing
//! are factored into pure functions so they can be unit-tested without network
//! or API keys; the HTTP/FFmpeg paths are exercised by integration tests.
//!
//! Tier-4 synthesis (R-DR13): the original `gemini.rs` was never written (WIP at
//! wipe). Authored 2026-05-28 from the `VisionProvider` contract + PRD FR-015 +
//! the recovered design notes (20 MB cap, inline base64). The default model is a
//! current one — the recovered spec's `gemini-2.0-flash` is stale; the model is
//! always overridable via `VisionConfig.model` / `gentle-eye.toml`.

use crate::contracts::errors::VisionError;
use crate::contracts::traits::{AnalysisResult, TimeRange, VisionConfig, VisionProvider};
use async_trait::async_trait;
use base64::Engine;
use chrono::Utc;
use serde_json::Value;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};
use uuid::Uuid;

const DEFAULT_GEMINI_MAX_VIDEO_SIZE: u64 = 20_971_520; // 20 MB
// Default to the always-newest Flash alias (the recovered spec's
// `gemini-2.0-flash` is stale). Flash is the routine tier; for deep-understanding
// tasks set `VisionConfig.model` to [`DEEP_MODEL`] (`gemini-pro-latest`).
const DEFAULT_MODEL: &str = "gemini-flash-latest";
/// Deep-understanding (Pro) tier — select via config when Flash isn't enough.
pub const DEEP_MODEL: &str = "gemini-pro-latest";

/// Gemini-backed [`VisionProvider`].
pub struct GeminiProvider {
    client: reqwest::Client,
    api_key: String,
    model: String,
    max_video_size: u64,
    timeout_seconds: u64,
}

impl GeminiProvider {
    /// Build from a [`VisionConfig`]. The API key is taken from the config, then
    /// `GEMINI_API_KEY`, then `GOOGLE_API_KEY`.
    pub fn new(config: &VisionConfig) -> Result<Self, VisionError> {
        let api_key = config
            .api_key
            .clone()
            .filter(|k| !k.is_empty())
            .or_else(|| std::env::var("GEMINI_API_KEY").ok())
            .or_else(|| std::env::var("GOOGLE_API_KEY").ok())
            .ok_or_else(|| {
                VisionError::Unavailable(
                    "Gemini API key not set (config.api_key / GEMINI_API_KEY / GOOGLE_API_KEY)"
                        .to_string(),
                )
            })?;
        let timeout_seconds = config.timeout_seconds.max(1);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_seconds))
            .build()
            .map_err(|e| VisionError::Unavailable(e.to_string()))?;
        Ok(Self {
            client,
            api_key,
            model: if config.model.is_empty() {
                DEFAULT_MODEL.to_string()
            } else {
                config.model.clone()
            },
            max_video_size: if config.max_video_size_bytes > 0 {
                config.max_video_size_bytes
            } else {
                DEFAULT_GEMINI_MAX_VIDEO_SIZE
            },
            timeout_seconds,
        })
    }

    fn endpoint(&self) -> String {
        format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
            self.model, self.api_key
        )
    }

    fn map_http_err(&self, e: reqwest::Error) -> VisionError {
        if e.is_timeout() {
            VisionError::Timeout {
                timeout_seconds: self.timeout_seconds,
            }
        } else {
            VisionError::NetworkError(e.to_string())
        }
    }

    /// POST one inline part to `generateContent` and return (text, token_count).
    async fn generate(
        &self,
        mime: &str,
        b64: String,
        prompt: &str,
    ) -> Result<(String, Option<u32>), VisionError> {
        let body = build_gemini_request(mime, &b64, prompt);
        let resp = self
            .client
            .post(self.endpoint())
            .json(&body)
            .send()
            .await
            .map_err(|e| self.map_http_err(e))?;
        let status = resp.status().as_u16();
        let text = resp.text().await.map_err(|e| self.map_http_err(e))?;
        if !(200..300).contains(&status) {
            return Err(classify_http_error(status, &text));
        }
        parse_gemini_response(&text)
    }
}

#[async_trait]
impl VisionProvider for GeminiProvider {
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

        // Trim to the timeframe if requested (kept alive until the request sends).
        let _segment;
        let send_path = match &timeframe {
            Some(tf) => {
                let seg = extract_video_segment(video_path, tf)?;
                let p = seg.path().to_path_buf();
                _segment = Some(seg);
                p
            }
            None => video_path.to_path_buf(),
        };

        let mime = guess_mime(&send_path);
        let b64 = read_as_base64(&send_path)?;
        let started = Instant::now();
        let (analysis_text, token_count) = self.generate(&mime, b64, prompt).await?;
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
        let mime = guess_mime(image_path);
        let b64 = read_as_base64(image_path)?;
        let started = Instant::now();
        let (analysis_text, token_count) = self.generate(&mime, b64, prompt).await?;
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
        if self.api_key.is_empty() {
            return Err(VisionError::AuthenticationFailed(
                "Gemini API key is empty".to_string(),
            ));
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "gemini"
    }

    fn max_video_size(&self) -> u64 {
        self.max_video_size
    }

    fn supports_native_video(&self) -> bool {
        true
    }

    fn model(&self) -> &str {
        &self.model
    }
}

// ---- pure helpers (unit-tested) -------------------------------------------

/// Build a `generateContent` request body with one text + one inline-data part.
fn build_gemini_request(mime: &str, b64: &str, prompt: &str) -> Value {
    serde_json::json!({
        "contents": [{
            "parts": [
                { "text": prompt },
                { "inline_data": { "mime_type": mime, "data": b64 } }
            ]
        }]
    })
}

/// Extract the analysis text + total token count from a Gemini JSON response.
fn parse_gemini_response(body: &str) -> Result<(String, Option<u32>), VisionError> {
    let v: Value = serde_json::from_str(body).map_err(|e| VisionError::ApiError {
        message: format!("invalid JSON from Gemini: {e}"),
        status_code: None,
    })?;
    if let Some(err) = v.get("error") {
        let message = err
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown Gemini error")
            .to_string();
        let status_code = err.get("code").and_then(Value::as_u64).map(|c| c as u16);
        return Err(VisionError::ApiError {
            message,
            status_code,
        });
    }
    let text = v
        .pointer("/candidates/0/content/parts/0/text")
        .and_then(Value::as_str)
        .ok_or_else(|| VisionError::ApiError {
            message: "Gemini response contained no text".to_string(),
            status_code: None,
        })?
        .to_string();
    let token_count = v
        .pointer("/usageMetadata/totalTokenCount")
        .and_then(Value::as_u64)
        .map(|t| t as u32);
    Ok((text, token_count))
}

/// Map an HTTP error status + body to the most specific [`VisionError`].
fn classify_http_error(status: u16, body: &str) -> VisionError {
    let message = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| {
            v.pointer("/error/message")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| body.chars().take(300).collect());
    match status {
        401 | 403 => VisionError::AuthenticationFailed(message),
        429 => VisionError::RateLimited {
            retry_after_seconds: 60,
        },
        _ => VisionError::ApiError {
            message,
            status_code: Some(status),
        },
    }
}

fn validate_prompt(prompt: &str) -> Result<(), VisionError> {
    if prompt.trim().is_empty() {
        return Err(VisionError::InvalidPrompt("prompt is empty".to_string()));
    }
    Ok(())
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

fn guess_mime(path: &Path) -> String {
    mime_guess::from_path(path)
        .first_or_octet_stream()
        .to_string()
}

fn read_as_base64(path: &Path) -> Result<String, VisionError> {
    let bytes = std::fs::read(path).map_err(|_| VisionError::FileNotFound(path.to_path_buf()))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// Trim `input` to `[start, end]` using FFmpeg into a temp `.mp4`.
fn extract_video_segment(input: &Path, tf: &TimeRange) -> Result<tempfile::NamedTempFile, VisionError> {
    let out = tempfile::Builder::new()
        .suffix(".mp4")
        .tempfile()
        .map_err(|e| VisionError::FrameExtractionFailed(e.to_string()))?;
    let status = Command::new("ffmpeg")
        .args([
            "-y",
            "-i",
            &input.to_string_lossy(),
            "-ss",
            &tf.start_seconds.to_string(),
            "-to",
            &tf.end_seconds.to_string(),
            "-c",
            "copy",
            &out.path().to_string_lossy(),
        ])
        .output()
        .map_err(|e| VisionError::FrameExtractionFailed(format!("failed to run ffmpeg: {e}")))?;
    if !status.status.success() {
        return Err(VisionError::FrameExtractionFailed(
            String::from_utf8_lossy(&status.stderr)
                .chars()
                .take(300)
                .collect(),
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> VisionConfig {
        VisionConfig {
            provider: "gemini".to_string(),
            api_key: Some("test-key".to_string()),
            model: String::new(),
            timeout_seconds: 30,
            max_video_size_bytes: 0,
        }
    }

    #[test]
    fn new_applies_defaults() {
        let p = GeminiProvider::new(&cfg()).unwrap();
        assert_eq!(p.name(), "gemini");
        assert_eq!(p.model(), DEFAULT_MODEL);
        assert_eq!(p.max_video_size(), DEFAULT_GEMINI_MAX_VIDEO_SIZE);
        assert!(p.supports_native_video());
    }

    #[test]
    fn request_body_has_prompt_and_inline_data() {
        let body = build_gemini_request("video/mp4", "QUJD", "what happened?");
        let parts = &body["contents"][0]["parts"];
        assert_eq!(parts[0]["text"], "what happened?");
        assert_eq!(parts[1]["inline_data"]["mime_type"], "video/mp4");
        assert_eq!(parts[1]["inline_data"]["data"], "QUJD");
    }

    #[test]
    fn parses_success_response() {
        let body = r#"{
            "candidates":[{"content":{"parts":[{"text":"A terminal error."}]}}],
            "usageMetadata":{"totalTokenCount":1234}
        }"#;
        let (text, tokens) = parse_gemini_response(body).unwrap();
        assert_eq!(text, "A terminal error.");
        assert_eq!(tokens, Some(1234));
    }

    #[test]
    fn parses_error_response() {
        let body = r#"{"error":{"code":400,"message":"bad request"}}"#;
        let err = parse_gemini_response(body).unwrap_err();
        match err {
            VisionError::ApiError { message, status_code } => {
                assert_eq!(message, "bad request");
                assert_eq!(status_code, Some(400));
            }
            other => panic!("expected ApiError, got {other:?}"),
        }
    }

    #[test]
    fn classifies_http_errors() {
        assert!(matches!(
            classify_http_error(401, "{}"),
            VisionError::AuthenticationFailed(_)
        ));
        assert!(matches!(
            classify_http_error(429, "{}"),
            VisionError::RateLimited { .. }
        ));
        assert!(matches!(
            classify_http_error(500, r#"{"error":{"message":"boom"}}"#),
            VisionError::ApiError { status_code: Some(500), .. }
        ));
    }

    #[test]
    fn validates_prompt_and_timeframe() {
        assert!(validate_prompt("").is_err());
        assert!(validate_prompt("ok").is_ok());
        assert!(validate_timeframe(&TimeRange { start_seconds: 5.0, end_seconds: 2.0 }).is_err());
        assert!(validate_timeframe(&TimeRange { start_seconds: 1.0, end_seconds: 2.0 }).is_ok());
    }

    #[test]
    fn guesses_video_mime() {
        assert_eq!(guess_mime(Path::new("/tmp/x.mp4")), "video/mp4");
    }
}
