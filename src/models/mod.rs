//! Data models for the gentle-eye MCP server.
//!
//! This module contains the core data structures used throughout the application:
//! - [`Recording`] - Represents a screen recording session
//! - [`RecordingStatus`] - Current state of a recording
//! - [`EncoderMode`] - Video encoding strategy
//!
//! # Example
//!
//! ```
//! use gentle_eye::models::{Recording, RecordingStatus, EncoderMode};
//! use uuid::Uuid;
//! use chrono::Utc;
//!
//! let recording = Recording {
//!     id: Uuid::new_v4(),
//!     status: RecordingStatus::Recording,
//!     start_time: Utc::now(),
//!     end_time: None,
//!     duration_ms: None,
//!     file_path: None,
//!     fps: 1,
//!     width: 1920,
//!     height: 1080,
//!     file_size_bytes: None,
//!     error_message: None,
//!     display_name: Some("Primary Display".to_string()),
//!     encoder_mode: EncoderMode::FileBased,
//! };
//! ```

pub mod analysis;
pub mod config;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

/// Represents a screen recording session with all associated metadata.
///
/// A `Recording` tracks the entire lifecycle of a screen capture operation,
/// from initiation through completion or error states.
///
/// # Fields
///
/// * `id` - Unique identifier for this recording session
/// * `status` - Current state of the recording
/// * `start_time` - When the recording was initiated
/// * `end_time` - When the recording was stopped (if completed)
/// * `duration_ms` - Total duration in milliseconds (if completed)
/// * `file_path` - Path to the output video file (if completed)
/// * `fps` - Frames per second for the recording
/// * `width` - Video width in pixels
/// * `height` - Video height in pixels
/// * `file_size_bytes` - Size of the output file in bytes (if completed)
/// * `error_message` - Error description if status is Error
/// * `display_name` - Human-readable name of the captured display
/// * `encoder_mode` - Encoding strategy used for this recording
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recording {
    /// Unique identifier for this recording session.
    pub id: Uuid,

    /// Current status of the recording.
    pub status: RecordingStatus,

    /// Timestamp when the recording was started.
    pub start_time: DateTime<Utc>,

    /// Timestamp when the recording was stopped.
    /// `None` if the recording is still in progress.
    pub end_time: Option<DateTime<Utc>>,

    /// Duration of the recording in milliseconds.
    /// `None` if the recording is still in progress.
    pub duration_ms: Option<u64>,

    /// Path to the output video file.
    /// `None` if the recording is still in progress or was cancelled.
    pub file_path: Option<PathBuf>,

    /// Frames per second used for this recording.
    pub fps: u32,

    /// Width of the recording in pixels.
    pub width: u32,

    /// Height of the recording in pixels.
    pub height: u32,

    /// Size of the output file in bytes.
    /// `None` if the recording is still in progress.
    pub file_size_bytes: Option<u64>,

    /// Error message if the recording failed.
    /// `None` if no error occurred.
    pub error_message: Option<String>,

    /// Human-readable name of the display being captured.
    /// `None` if not available or not specified.
    pub display_name: Option<String>,

    /// Encoding mode used for this recording.
    pub encoder_mode: EncoderMode,
}

impl Recording {
    /// Creates a new recording with the given parameters.
    ///
    /// # Arguments
    ///
    /// * `fps` - Frames per second for the recording
    /// * `width` - Video width in pixels
    /// * `height` - Video height in pixels
    /// * `encoder_mode` - Encoding strategy to use
    ///
    /// # Example
    ///
    /// ```
    /// use gentle_eye::models::{Recording, EncoderMode};
    ///
    /// let recording = Recording::new(1, 1920, 1080, EncoderMode::FileBased);
    /// assert_eq!(recording.fps, 1);
    /// ```
    pub fn new(fps: u32, width: u32, height: u32, encoder_mode: EncoderMode) -> Self {
        Self {
            id: Uuid::new_v4(),
            status: RecordingStatus::Recording,
            start_time: Utc::now(),
            end_time: None,
            duration_ms: None,
            file_path: None,
            fps,
            width,
            height,
            file_size_bytes: None,
            error_message: None,
            display_name: None,
            encoder_mode,
        }
    }

    /// Calculates the elapsed time since the recording started.
    ///
    /// Returns the duration in milliseconds from start_time to either
    /// end_time (if completed) or the current time (if still recording).
    pub fn elapsed_ms(&self) -> u64 {
        let end = self.end_time.unwrap_or_else(Utc::now);
        let duration = end.signed_duration_since(self.start_time);
        duration.num_milliseconds().max(0) as u64
    }

    /// Returns `true` if the recording is currently active.
    pub fn is_active(&self) -> bool {
        self.status == RecordingStatus::Recording
    }

    /// Returns `true` if the recording has finished (completed, error, or cancelled).
    pub fn is_finished(&self) -> bool {
        matches!(
            self.status,
            RecordingStatus::Completed | RecordingStatus::Error | RecordingStatus::Cancelled
        )
    }
}

/// Represents the current status of a recording session.
///
/// The status follows a state machine pattern:
/// - `Recording` -> `Finalizing` -> `Completed`
/// - `Recording` -> `Cancelled`
/// - Any state -> `Error` (on failure)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordingStatus {
    /// Recording is actively capturing frames.
    Recording,

    /// Recording has stopped and video is being finalized.
    Finalizing,

    /// Recording completed successfully with output file.
    Completed,

    /// Recording failed with an error.
    Error,

    /// Recording was cancelled before completion.
    Cancelled,
}

impl RecordingStatus {
    /// Returns a human-readable description of the status.
    pub fn as_str(&self) -> &'static str {
        match self {
            RecordingStatus::Recording => "recording",
            RecordingStatus::Finalizing => "finalizing",
            RecordingStatus::Completed => "completed",
            RecordingStatus::Error => "error",
            RecordingStatus::Cancelled => "cancelled",
        }
    }
}

impl std::fmt::Display for RecordingStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Specifies the encoding strategy for video output.
///
/// The encoder mode affects performance, memory usage, and reliability:
/// - `FileBased` writes frames directly to disk (lower memory, more I/O)
/// - `InMemoryPipe` buffers frames in memory (higher memory, faster encoding)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum EncoderMode {
    /// Write frames directly to a file on disk.
    /// Lower memory usage but more disk I/O.
    #[default]
    FileBased,

    /// Buffer frames in memory and pipe to encoder.
    /// Higher memory usage but faster encoding.
    InMemoryPipe,
}

impl EncoderMode {
    /// Returns a human-readable description of the encoder mode.
    pub fn as_str(&self) -> &'static str {
        match self {
            EncoderMode::FileBased => "file_based",
            EncoderMode::InMemoryPipe => "in_memory_pipe",
        }
    }
}

impl std::fmt::Display for EncoderMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recording_new() {
        let recording = Recording::new(1, 1920, 1080, EncoderMode::FileBased);

        assert_eq!(recording.fps, 1);
        assert_eq!(recording.width, 1920);
        assert_eq!(recording.height, 1080);
        assert_eq!(recording.status, RecordingStatus::Recording);
        assert_eq!(recording.encoder_mode, EncoderMode::FileBased);
        assert!(recording.end_time.is_none());
        assert!(recording.duration_ms.is_none());
        assert!(recording.file_path.is_none());
    }

    #[test]
    fn test_recording_is_active() {
        let recording = Recording::new(1, 1920, 1080, EncoderMode::FileBased);
        assert!(recording.is_active());
    }

    #[test]
    fn test_recording_is_finished() {
        let mut recording = Recording::new(1, 1920, 1080, EncoderMode::FileBased);

        assert!(!recording.is_finished());

        recording.status = RecordingStatus::Completed;
        assert!(recording.is_finished());

        recording.status = RecordingStatus::Error;
        assert!(recording.is_finished());

        recording.status = RecordingStatus::Cancelled;
        assert!(recording.is_finished());
    }

    #[test]
    fn test_recording_status_display() {
        assert_eq!(RecordingStatus::Recording.to_string(), "recording");
        assert_eq!(RecordingStatus::Finalizing.to_string(), "finalizing");
        assert_eq!(RecordingStatus::Completed.to_string(), "completed");
        assert_eq!(RecordingStatus::Error.to_string(), "error");
        assert_eq!(RecordingStatus::Cancelled.to_string(), "cancelled");
    }

    #[test]
    fn test_encoder_mode_display() {
        assert_eq!(EncoderMode::FileBased.to_string(), "file_based");
        assert_eq!(EncoderMode::InMemoryPipe.to_string(), "in_memory_pipe");
    }

    #[test]
    fn test_encoder_mode_default() {
        assert_eq!(EncoderMode::default(), EncoderMode::FileBased);
    }

    #[test]
    fn test_recording_serialization() {
        let recording = Recording::new(1, 1920, 1080, EncoderMode::FileBased);
        let json = serde_json::to_string(&recording).expect("serialization failed");
        let deserialized: Recording =
            serde_json::from_str(&json).expect("deserialization failed");

        assert_eq!(recording.id, deserialized.id);
        assert_eq!(recording.status, deserialized.status);
        assert_eq!(recording.fps, deserialized.fps);
    }

    #[test]
    fn test_recording_status_serialization() {
        let status = RecordingStatus::Recording;
        let json = serde_json::to_string(&status).expect("serialization failed");
        assert_eq!(json, "\"recording\"");

        let deserialized: RecordingStatus =
            serde_json::from_str(&json).expect("deserialization failed");
        assert_eq!(status, deserialized);
    }
}
