//! Recording orchestration: the `RecordingService` implementation.
//!
//! [`CaptureService`] ties together [`ScreenCapturer`], [`FrameRateController`],
//! and [`PipeEncoder`], persisting recording metadata through a
//! [`StorageManager`](crate::contracts::traits::StorageManager). Each recording
//! runs on a background task (blocking capture loop via `spawn_blocking`) that
//! finalizes the recording in storage on completion/cancellation/error.
//!
//! The capture loop needs a real display + ffmpeg, so it is integration-tested;
//! config validation and the storage-backed query methods are unit-tested.
//!
//! Authored 2026-05-28 from the `RecordingService` contract + the recovered
//! struct shape — the recovered source was junk.

use crate::capture::encoder::PipeEncoder;
use crate::capture::frame_rate::FrameRateController;
use crate::capture::screen::ScreenCapturer;
use crate::contracts::errors::{RecordingError, StorageError};
use crate::contracts::traits::{
    Recording, RecordingConfig, RecordingService, RecordingStatus,
    StorageManager as StorageManagerTrait,
};
use async_trait::async_trait;
use chrono::Utc;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Stop/cancel signalling for a running capture loop.
#[derive(Default)]
struct RecordingControl {
    stop: AtomicBool,
    cancel: AtomicBool,
}

/// A live recording's control handle + background worker.
struct ActiveRecording {
    control: Arc<RecordingControl>,
    worker: tokio::task::JoinHandle<()>,
}

/// Outcome of a successful capture loop.
struct CaptureStats {
    duration_ms: u64,
    file_size_bytes: u64,
}

/// Orchestrates screen recording and persists metadata via storage.
pub struct CaptureService {
    storage: Arc<dyn StorageManagerTrait>,
    active: Arc<Mutex<HashMap<Uuid, ActiveRecording>>>,
    display_index: usize,
}

impl CaptureService {
    /// Create a service that records `display_index` and persists via `storage`.
    pub fn new(storage: Arc<dyn StorageManagerTrait>, display_index: usize) -> Self {
        Self {
            storage,
            active: Arc::new(Mutex::new(HashMap::new())),
            display_index,
        }
    }

    fn map_storage_err(e: StorageError) -> RecordingError {
        match e {
            StorageError::NotFound(id) => RecordingError::NotFound(id),
            other => RecordingError::Internal(other.to_string()),
        }
    }
}

/// Blocking capture loop: capture → encode until stop/cancel/max-duration.
fn run_capture(
    display_index: usize,
    config: &RecordingConfig,
    output: &Path,
    control: &RecordingControl,
) -> Result<CaptureStats, RecordingError> {
    let mut capturer = ScreenCapturer::new(display_index)?;
    let full_w = capturer.width();
    let full_h = capturer.height();

    // If an active target points at THIS display, crop every frame to it. The
    // encoder is sized to the crop, so the whole recording is the sub-region.
    let crop_rect = active_display_crop(display_index, full_w as u32, full_h as u32);
    let (width, height) = match crop_rect {
        Some(r) => (r.w, r.h),
        None => (full_w as u32, full_h as u32),
    };
    let fps = u32::from(config.fps);
    let mut encoder = PipeEncoder::start(width, height, fps, output)?;
    let mut frame_rate = FrameRateController::new(fps);
    let start = Instant::now();
    let max_duration = config.max_duration_seconds.map(Duration::from_secs);

    loop {
        if control.cancel.load(Ordering::Relaxed) {
            return Err(RecordingError::Cancelled);
        }
        if control.stop.load(Ordering::Relaxed) {
            break;
        }
        if let Some(max) = max_duration {
            if start.elapsed() >= max {
                break;
            }
        }
        let now = Instant::now();
        if frame_rate.should_capture(now) {
            let frame = capturer.capture_frame(Duration::from_millis(200))?;
            match crop_rect {
                Some(rect) => {
                    let stride = frame.len().checked_div(full_h).unwrap_or(full_w * 4);
                    let (cropped, _, _) =
                        crate::target::crop::crop_bgra(&frame, full_w, full_h, stride, rect)
                            .map_err(|e| RecordingError::EncoderError(e.to_string()))?;
                    encoder.write_frame(&cropped)?;
                }
                None => encoder.write_frame(&frame)?,
            }
        } else {
            std::thread::sleep(frame_rate.time_until_next(now).min(Duration::from_millis(10)));
        }
    }

    encoder.finish()?;
    let file_size_bytes = std::fs::metadata(output).map(|m| m.len()).unwrap_or(0);
    Ok(CaptureStats {
        duration_ms: start.elapsed().as_millis() as u64,
        file_size_bytes,
    })
}

/// Resolve the active target into a pixel crop rect — but only when it targets
/// THIS display. Returns `None` (full-frame capture) when there's no active
/// target, it targets a different display, or it's a stream target (stream crops
/// are applied by the ffmpeg `crop=` filter in `capture::stream`, not here).
fn active_display_crop(display_index: usize, w: u32, h: u32) -> Option<crate::target::model::PixelRect> {
    let store = crate::target::store::TargetStore::load().ok()?;
    let target = store.active()?;
    match &target.source {
        crate::target::model::TargetSource::Display { index } if *index == display_index => {
            Some(crate::target::geometry::norm_to_pixel(target.region, (w, h), (0, 0)))
        }
        _ => None,
    }
}

/// Update a recording's persisted state once its capture loop has ended.
async fn finalize_recording(
    storage: Arc<dyn StorageManagerTrait>,
    id: Uuid,
    output: PathBuf,
    result: Result<Result<CaptureStats, RecordingError>, tokio::task::JoinError>,
) {
    let Ok(mut rec) = storage.load_recording(id).await else {
        return;
    };
    rec.end_time = Some(Utc::now());
    match result {
        Ok(Ok(stats)) => {
            rec.status = RecordingStatus::Completed;
            rec.duration_ms = Some(stats.duration_ms);
            rec.file_path = Some(output);
            rec.file_size_bytes = Some(stats.file_size_bytes);
        }
        Ok(Err(RecordingError::Cancelled)) => {
            rec.status = RecordingStatus::Cancelled;
            let _ = std::fs::remove_file(&output);
        }
        Ok(Err(e)) => {
            rec.status = RecordingStatus::Failed;
            rec.error_message = Some(e.to_string());
        }
        Err(join_err) => {
            rec.status = RecordingStatus::Failed;
            rec.error_message = Some(format!("capture worker failed: {join_err}"));
        }
    }
    let _ = storage.save_recording(&rec).await;
}

#[async_trait]
impl RecordingService for CaptureService {
    async fn start_recording(&self, config: RecordingConfig) -> Result<Recording, RecordingError> {
        if !(1..=30).contains(&config.fps) {
            return Err(RecordingError::InvalidConfig(format!(
                "fps must be between 1 and 30, got {}",
                config.fps
            )));
        }
        let recording = Recording::new(config.clone());
        let id = recording.id;
        self.storage
            .save_recording(&recording)
            .await
            .map_err(Self::map_storage_err)?;

        let control = Arc::new(RecordingControl::default());
        let output = self.storage.generate_recording_path(id);
        let worker = {
            let storage = self.storage.clone();
            let control = control.clone();
            let display = self.display_index;
            let out = output.clone();
            tokio::spawn(async move {
                let result = tokio::task::spawn_blocking(move || {
                    run_capture(display, &config, &out, &control)
                })
                .await;
                finalize_recording(storage, id, output, result).await;
            })
        };
        self.active
            .lock()
            .expect("active-recordings mutex poisoned")
            .insert(id, ActiveRecording { control, worker });
        Ok(recording)
    }

    async fn stop_recording(&self, id: Uuid) -> Result<Recording, RecordingError> {
        let active = self
            .active
            .lock()
            .expect("active-recordings mutex poisoned")
            .remove(&id);
        if let Some(ar) = active {
            ar.control.stop.store(true, Ordering::Relaxed);
            let _ = ar.worker.await;
        }
        self.storage
            .load_recording(id)
            .await
            .map_err(Self::map_storage_err)
    }

    async fn cancel_recording(&self, id: Uuid) -> Result<Recording, RecordingError> {
        let active = self
            .active
            .lock()
            .expect("active-recordings mutex poisoned")
            .remove(&id);
        if let Some(ar) = active {
            ar.control.cancel.store(true, Ordering::Relaxed);
            let _ = ar.worker.await;
        }
        self.storage
            .load_recording(id)
            .await
            .map_err(Self::map_storage_err)
    }

    async fn get_status(&self, id: Uuid) -> Result<Recording, RecordingError> {
        self.storage
            .load_recording(id)
            .await
            .map_err(Self::map_storage_err)
    }

    async fn list_recordings(
        &self,
        limit: usize,
        status_filter: Option<RecordingStatus>,
    ) -> Result<Vec<Recording>, RecordingError> {
        self.storage
            .list_recordings(limit, 0, status_filter)
            .await
            .map(|list| list.recordings)
            .map_err(Self::map_storage_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StorageManager;

    fn service() -> CaptureService {
        let storage: Arc<dyn StorageManagerTrait> =
            Arc::new(StorageManager::in_memory("/tmp/gentle-eye-svc").unwrap());
        CaptureService::new(storage, 0)
    }

    #[tokio::test]
    async fn rejects_invalid_fps() {
        let svc = service();
        let mut config = RecordingConfig::default();
        config.fps = 0;
        let err = svc.start_recording(config).await.unwrap_err();
        assert!(matches!(err, RecordingError::InvalidConfig(_)));
    }

    #[tokio::test]
    async fn status_of_unknown_is_not_found() {
        let svc = service();
        let err = svc.get_status(Uuid::new_v4()).await.unwrap_err();
        assert!(matches!(err, RecordingError::NotFound(_)));
    }

    #[tokio::test]
    async fn list_is_empty_initially() {
        let svc = service();
        assert!(svc.list_recordings(10, None).await.unwrap().is_empty());
    }
}
