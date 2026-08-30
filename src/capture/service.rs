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
            // The delete path is validated like the write path: the output
            // travelled through an async boundary, and "we generated it" is
            // not a safety argument (see start_recording). A refusal leaves
            // the file; it must never delete outside the recording directory.
            let validator =
                crate::security::path_validator::PathValidator::new(storage.base_dir());
            match validator.validate(&output) {
                Ok(safe) => {
                    let _ = std::fs::remove_file(safe);
                }
                Err(e) => {
                    tracing::warn!(error = %e, path = %output.display(),
                        "refusing to delete a cancelled recording outside the recording dir");
                }
            }
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

        // The write path is validated BEFORE anything is persisted or spawned.
        // `generate_recording_path` is a trait method: any implementation (or a
        // misconfigured base dir) can hand back a path outside the recording
        // directory, and the encoder would happily create it. Refusing here —
        // rather than trusting "storage generated it" — keeps every byte this
        // service writes inside the tree the validator allows.
        let output = self.storage.generate_recording_path(id);
        let validator =
            crate::security::path_validator::PathValidator::new(self.storage.base_dir());
        if let Err(e) = validator.validate(&output) {
            return Err(RecordingError::Internal(format!(
                "refusing to record to {}: {e}",
                output.display()
            )));
        }

        self.storage
            .save_recording(&recording)
            .await
            .map_err(Self::map_storage_err)?;

        let control = Arc::new(RecordingControl::default());
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

    /// A storage whose generated recording path escapes its own base dir —
    /// the exact situation the T047 validation exists to refuse. Everything
    /// else delegates to a real in-memory [`StorageManager`].
    struct EscapingStorage {
        inner: StorageManager,
        escape_to: PathBuf,
    }

    #[async_trait]
    impl StorageManagerTrait for EscapingStorage {
        fn base_dir(&self) -> &Path {
            self.inner.base_dir()
        }
        fn generate_recording_path(&self, id: Uuid) -> PathBuf {
            self.escape_to.join(format!("{id}.mp4"))
        }
        async fn save_recording(&self, r: &Recording) -> Result<(), StorageError> {
            self.inner.save_recording(r).await
        }
        async fn load_recording(&self, id: Uuid) -> Result<Recording, StorageError> {
            self.inner.load_recording(id).await
        }
        async fn delete_recording(&self, id: Uuid) -> Result<(), StorageError> {
            self.inner.delete_recording(id).await
        }
        async fn list_recordings(
            &self,
            limit: usize,
            offset: usize,
            f: Option<RecordingStatus>,
        ) -> Result<crate::contracts::traits::RecordingList, StorageError> {
            self.inner.list_recordings(limit, offset, f).await
        }
        async fn storage_used(&self) -> Result<u64, StorageError> {
            self.inner.storage_used().await
        }
        async fn cleanup_old_recordings(&self, d: u32) -> Result<u32, StorageError> {
            self.inner.cleanup_old_recordings(d).await
        }
    }

    #[tokio::test]
    async fn a_recording_path_escaping_the_base_dir_is_refused_before_anything_starts() {
        // The escape target is a directory this process could genuinely write
        // (R33's lesson: /etc/passwd fails on permissions whether or not the
        // validator runs). Without the validation, start_recording spawns the
        // worker and returns Ok — so Ok-vs-Err is the code refusing, not the OS.
        let base = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let storage: Arc<dyn StorageManagerTrait> = Arc::new(EscapingStorage {
            inner: StorageManager::in_memory(base.path()).unwrap(),
            escape_to: elsewhere.path().to_path_buf(),
        });
        let svc = CaptureService::new(storage.clone(), 0);

        let err = svc
            .start_recording(RecordingConfig::default())
            .await
            .expect_err("a write path outside the base dir must be refused");
        assert!(
            err.to_string().contains("refusing to record"),
            "refused by the validator, not by some later failure: {err}"
        );
        // Refused BEFORE persisting: no dangling recording row was created.
        assert!(svc.list_recordings(10, None).await.unwrap().is_empty());
        // And nothing was written where the escaping path pointed.
        assert_eq!(std::fs::read_dir(elsewhere.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn a_cancelled_recording_outside_the_base_dir_is_not_deleted() {
        // Drives the real finalize path with a Cancelled result. The victim
        // file is one this process could genuinely delete; it must survive
        // because the VALIDATOR refuses, not because the OS does.
        let base = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let storage: Arc<dyn StorageManagerTrait> =
            Arc::new(StorageManager::in_memory(base.path()).unwrap());

        let rec = Recording::new(RecordingConfig::default());
        storage.save_recording(&rec).await.unwrap();
        let victim = elsewhere.path().join("someone-elses.mp4");
        std::fs::write(&victim, b"not ours").unwrap();

        finalize_recording(
            storage.clone(),
            rec.id,
            victim.clone(),
            Ok(Err(RecordingError::Cancelled)),
        )
        .await;

        assert!(victim.exists(), "a delete outside the recording dir must be refused");
        let after = storage.load_recording(rec.id).await.unwrap();
        assert_eq!(after.status, RecordingStatus::Cancelled, "the finalize itself completed");
    }

    #[tokio::test]
    async fn a_cancelled_recording_inside_the_base_dir_is_cleaned_up() {
        // The companion: the same path with an in-tree file DOES delete, so the
        // refusal above cannot be a delete that never runs at all.
        let base = tempfile::tempdir().unwrap();
        let storage: Arc<dyn StorageManagerTrait> =
            Arc::new(StorageManager::in_memory(base.path()).unwrap());

        let rec = Recording::new(RecordingConfig::default());
        storage.save_recording(&rec).await.unwrap();
        let inside = base.path().join(format!("{}.mp4", rec.id));
        std::fs::write(&inside, b"partial capture").unwrap();

        finalize_recording(
            storage.clone(),
            rec.id,
            inside.clone(),
            Ok(Err(RecordingError::Cancelled)),
        )
        .await;

        assert!(!inside.exists(), "the in-tree partial file is removed");
    }

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
