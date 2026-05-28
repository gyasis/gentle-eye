//! High-level library facade — the easy way to use gentle-eye from other Rust code.
//!
//! gentle-eye is a normal library crate: add it as a dependency and `use` it.
//! These functions wrap the multi-step setup (storage + service + provider) into
//! single calls so a consumer that "just needs to record" (or analyze, or read
//! text) doesn't have to assemble the pieces.
//!
//! ```no_run
//! use gentle_eye::{record, analyze, VisionConfig};
//! use std::time::Duration;
//!
//! # async fn demo() -> anyhow::Result<()> {
//! // Record display 0 for 10s at 2 fps into a directory.
//! let rec = record(0, Duration::from_secs(10), 2, "/tmp/recordings").await?;
//!
//! // Analyze it with the local Ollama provider.
//! if let Some(path) = &rec.file_path {
//!     let cfg = VisionConfig { provider: "ollama".into(), ..Default::default() };
//!     let result = analyze(&cfg, path, "What is shown?", true).await?;
//!     println!("{}", result.analysis_text);
//! }
//! # Ok(())
//! # }
//! ```

use crate::analysis::{GeminiProvider, OllamaProvider};
use crate::capture::CaptureService;
use crate::contracts::errors::{RecordingError, VisionError};
use crate::contracts::traits::{
    AnalysisResult, Recording, RecordingConfig, RecordingService, StorageManager as StorageTrait,
    TimeRange, VisionConfig, VisionProvider,
};
use crate::storage::StorageManager;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// Record `display_index` for `duration` at `fps`, writing the video into
/// `output_dir`, and return the finished [`Recording`] (its `file_path` points
/// at the saved file). Blocks for `duration` then finalizes.
///
/// Requires a tokio runtime (call it from `async` context), a display, and
/// `ffmpeg` on PATH.
pub async fn record(
    display_index: usize,
    duration: Duration,
    fps: u8,
    output_dir: impl Into<PathBuf>,
) -> Result<Recording, RecordingError> {
    let out = output_dir.into();
    std::fs::create_dir_all(&out).map_err(RecordingError::StorageError)?;
    let storage: Arc<dyn StorageTrait> = Arc::new(
        StorageManager::in_memory(out.clone())
            .map_err(|e| RecordingError::Internal(e.to_string()))?,
    );
    let service = CaptureService::new(storage, display_index);
    let config = RecordingConfig {
        fps,
        max_duration_seconds: Some(duration.as_secs()),
        output_dir: out,
        ..Default::default()
    };
    let started = service.start_recording(config).await?;
    let id = started.id;
    tokio::time::sleep(duration + Duration::from_millis(800)).await;
    service.stop_recording(id).await
}

/// Analyze an image or video with a vision provider built from `config`
/// (`config.provider` selects "gemini" or "ollama"). Set `is_video` for video.
pub async fn analyze(
    config: &VisionConfig,
    media: &Path,
    prompt: &str,
    is_video: bool,
) -> Result<AnalysisResult, VisionError> {
    let provider: Box<dyn VisionProvider> = if config.provider == "ollama" {
        Box::new(OllamaProvider::new(config)?)
    } else {
        Box::new(GeminiProvider::new(config)?)
    };
    if is_video {
        provider.analyze_video(media, prompt, None).await
    } else {
        provider.analyze_image(media, prompt).await
    }
}

/// Analyze a sub-range of a video (`start_seconds`..`end_seconds`).
pub async fn analyze_video_range(
    config: &VisionConfig,
    video: &Path,
    prompt: &str,
    start_seconds: f64,
    end_seconds: f64,
) -> Result<AnalysisResult, VisionError> {
    let provider: Box<dyn VisionProvider> = if config.provider == "ollama" {
        Box::new(OllamaProvider::new(config)?)
    } else {
        Box::new(GeminiProvider::new(config)?)
    };
    let timeframe = Some(TimeRange {
        start_seconds,
        end_seconds,
    });
    provider.analyze_video(video, prompt, timeframe).await
}
