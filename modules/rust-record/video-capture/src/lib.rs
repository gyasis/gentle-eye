_**Agent (claude-opus-4-5-20251101) (sidechain)**_
Tool use: **Read** `./modules/rust-record/video-capture/src/lib.rs`
pub mod capture;
pub mod config;
pub mod display_manager;
pub mod encoder;
pub mod error;
pub mod frame_rate;
pub mod memory;
pub mod metadata;
pub mod prerequisites;
pub mod service;
pub mod storage;
// Re-export main types for convenience
pub use capture::{CaptureEvent, CaptureResult, RawFrame, ScreenCapturer};
pub use config::Config;
pub use display_manager::{DisplayInfo, DisplayManager};
pub use encoder::{EncoderMode, EncoderState, Frame, PipeEncoder, VideoEncoder};
pub use error::{CaptureError, ConfigError, DisplayError, EncoderError, MemoryError, MetadataError, ServiceError, StorageError};
pub use frame_rate::FrameRateMode;
pub use memory::{MemoryConfig, MemoryMonitor, MemoryPressure, MemoryStats};
pub use metadata::{CaptureRegion, RecordingMetadata};
pub use service::{DayflowService, RecordingTool, SessionState, SessionStatus};
pub use storage::{
    atomic_write, generate_frame_filename, validate_path_for_creation, validate_path_within_dir,
    CleanupStats, ClipMetadata, RecordingInfo, SessionMetadata, SessionPaths, StorageBuffer,
    StorageManager,
_**Agent (claude-opus-4-5-20251101) (sidechain)**_
The video_capture module exports `DisplayInfo` and `DisplayManager`. Now let me check how the main crate imports from this module.
_**Agent (claude-opus-4-5-20251101) (sidechain)**_
Tool use: **Read** `./Cargo.toml`
name = "gentle-eye"
version = "0.1.0"
edition = "2021"
description = "MCP server for screen recording and AI video analysis"