//! Tool input/output types for Gentle-Eye MCP server
//!
//! This module defines the request and response types for all MCP tools.
//! These types are derived from the JSON schemas in `contracts/mcp-tools.json`.

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// ============================================================================
// start_recording Tool
// ============================================================================

/// Input parameters for the `start_recording` tool
///
/// Starts a new screen recording session. Returns a unique recording ID
/// that can be used to stop the recording or check its status.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct StartRecordingInput {
    /// Frames per second for the recording (1-30, default: 1).
    ///
    /// Pick fps by how long you intend to record (duration-aware heuristic; see
    /// `docs/FPS_AND_DAYFLOW.md`):
    /// - ≤ ~30 s, motion matters → 15 (smooth, tiny file)
    /// - ~30 s – 15 min, debugging → 1 (a sequence of actions; cheap)
    /// - 15 min+ (all-day "dayflow") → sub-1 fps timelapse (0.2–0.5), which the
    ///   dedicated dayflow tools handle (chunk + Map-Reduce), since this tool
    ///   clamps to ≥1.
    #[schemars(range(min = 1, max = 30))]
    pub fps: Option<u32>,

    /// Optional directory to save the recording
    ///
    /// Must be an absolute path. If not provided, uses the default temp directory.
    pub output_dir: Option<String>,

    /// Maximum recording duration in seconds (1-7200, default: 1800)
    ///
    /// Recording will auto-stop if this limit is reached.
    #[schemars(range(min = 1, max = 7200))]
    pub max_duration_seconds: Option<u64>,
}

/// Output for the `start_recording` tool
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct StartRecordingOutput {
    /// Unique identifier (UUID) for this recording session
    pub recording_id: String,

    /// Current status of the recording
    pub status: String,

    /// Human-readable confirmation message
    pub message: String,
}

// ============================================================================
// stop_recording Tool
// ============================================================================

/// Input parameters for the `stop_recording` tool
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct StopRecordingInput {
    /// The recording ID returned from start_recording
    pub recording_id: String,
}

/// Output for the `stop_recording` tool
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct StopRecordingOutput {
    /// The recording ID that was stopped
    pub recording_id: String,

    /// Final status of the recording (completed or error)
    pub status: String,

    /// Absolute path to the saved video file
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,

    /// Total recording duration in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,

    /// Size of the output video file in bytes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_size_bytes: Option<u64>,

    /// Error details if status is 'error'
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

// ============================================================================
// get_recording_status Tool
// ============================================================================

/// Input parameters for the `get_recording_status` tool
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct GetRecordingStatusInput {
    /// The recording ID to check
    pub recording_id: String,
}

/// Output for the `get_recording_status` tool
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct GetRecordingStatusOutput {
    /// The recording ID
    pub recording_id: String,

    /// Current status (recording, finalizing, completed, error, cancelled)
    pub status: String,

    /// When the recording started (ISO 8601)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_time: Option<DateTime<Utc>>,

    /// Elapsed time in milliseconds (if still recording)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,

    /// Path to video file (if completed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,

    /// Error details (if status is error)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

// ============================================================================
// analyze_video Tool
// ============================================================================

/// Timeframe specification for video analysis
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct TimeframeInput {
    /// Start time in seconds from video beginning
    #[schemars(range(min = 0.0))]
    pub start_seconds: f64,

    /// End time in seconds from video beginning
    pub end_seconds: f64,
}

/// Input parameters for the `analyze_video` tool
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct AnalyzeVideoInput {
    /// Absolute path to the video file to analyze
    pub video_path: String,

    /// Analysis prompt describing what you want to know about the video
    ///
    /// Examples:
    /// - "What terminal commands were executed?"
    /// - "Describe the error that appeared"
    /// - "What was the user trying to accomplish?"
    #[schemars(length(max = 10000))]
    pub prompt: String,

    /// Optional time range to focus the analysis on
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeframe: Option<TimeframeInput>,
}

/// Output for the `analyze_video` tool
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct AnalyzeVideoOutput {
    /// The vision AI's analysis of the video content
    pub analysis_text: String,

    /// The AI model that performed the analysis
    pub model_used: String,

    /// Number of tokens used (if available from provider)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_count: Option<u64>,

    /// Time taken to generate the analysis in milliseconds
    pub processing_time_ms: u64,
}

// ============================================================================
// list_recordings Tool
// ============================================================================

/// Input parameters for the `list_recordings` tool
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ListRecordingsInput {
    /// Maximum number of recordings to return (1-100, default: 10)
    #[schemars(range(min = 1, max = 100))]
    pub limit: Option<u32>,

    /// Filter by recording status (recording, completed, error, all)
    ///
    /// Default: "all"
    pub status_filter: Option<String>,
}

/// Summary of a single recording for list results
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct RecordingSummary {
    /// Recording UUID
    pub id: String,

    /// Recording status
    pub status: String,

    /// When recording started (ISO 8601)
    pub start_time: DateTime<Utc>,

    /// Duration in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,

    /// Path to video file
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,

    /// File size in bytes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_size_bytes: Option<u64>,
}

/// Output for the `list_recordings` tool
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ListRecordingsOutput {
    /// Array of recording summaries
    pub recordings: Vec<RecordingSummary>,

    /// Total number of recordings matching the filter
    pub total_count: u32,
}

// ============================================================================
// cancel_recording Tool
// ============================================================================

/// Input parameters for the `cancel_recording` tool
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct CancelRecordingInput {
    /// The recording ID to cancel
    pub recording_id: String,
}

/// Output for the `cancel_recording` tool
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct CancelRecordingOutput {
    /// The recording ID that was cancelled
    pub recording_id: String,

    /// Status after cancellation (always "cancelled")
    pub status: String,

    /// Confirmation message
    pub message: String,
}

// ============================================================================
// get_vision_provider_info Tool
// ============================================================================

/// Output for the `get_vision_provider_info` tool
///
/// Note: This tool has no input parameters.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct GetVisionProviderInfoOutput {
    /// Name of the configured provider (gemini, ollama)
    pub provider: String,

    /// Model being used for analysis
    pub model: String,

    /// Maximum supported video file size in bytes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_video_size_bytes: Option<u64>,

    /// Whether provider supports direct video input (vs frame extraction)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_native_video: Option<bool>,

    /// Whether the provider is currently available
    pub available: bool,

    /// Error details if provider is not available
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

// ============================================================================
// read_screen_text Tool
// ============================================================================

/// Input parameters for the `read_screen_text` tool — fast, LOCAL OCR (tesseract).
///
/// Provide exactly one of `image_path` or `video_path`.
///
/// CHOOSING A VISION METHOD (full guidance: `docs/VISION_METHODS.md`):
/// - `read_screen_text` (this tool) — OCR. **Local, private, fast, free.** Good for
///   crisp/light UI text and quick extraction. WEAK on dense, dark, multi-column
///   screens (terminals/IDEs) — the text comes back garbled.
/// - For ACCURATE text on a dense/dark screen, use `analyze_video` (the cloud vision
///   provider, e.g. Gemini) with a "transcribe all on-screen text" prompt: it reads
///   the full frame far better than OCR — at the cost of sending the image off-box.
///   For an ultrawide, TILE into columns and analyze each at full resolution for
///   near-perfect fidelity.
/// - PRIVACY: prefer OCR / local vision for sensitive screens; use the cloud provider
///   only when fidelity matters and the content is OK to share.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ReadScreenTextInput {
    /// Absolute path to an image to OCR.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_path: Option<String>,

    /// Absolute path to a video to OCR (frames are sampled and merged).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video_path: Option<String>,
}

/// Output for the `read_screen_text` tool.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ReadScreenTextOutput {
    /// The extracted on-screen text.
    pub text: String,

    /// What was OCR'd ("image" or "video").
    pub source: String,
}

// ============================================================================
// capture_stream_frame Tool
// ============================================================================

/// Input parameters for the `capture_stream_frame` tool.
///
/// Grabs a single frame from a live stream URL (RTSP/HTTP/SRT — e.g. a
/// Blackmagic ATEM output) and saves it as a PNG for later analysis.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct CaptureStreamFrameInput {
    /// The stream URL to grab a frame from (rtsp://, http(s)://, srt://, …).
    pub stream_url: String,

    /// Optional directory to save the PNG into (default: temp dir).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_dir: Option<String>,
}

/// Output for the `capture_stream_frame` tool.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct CaptureStreamFrameOutput {
    /// Absolute path to the saved PNG file. Pass this to `analyze_video`
    /// (image mode) for AI analysis.
    pub file_path: String,

    /// Width of the captured frame in pixels.
    pub width: u32,

    /// Height of the captured frame in pixels.
    pub height: u32,

    /// Size of the PNG file in bytes.
    pub file_size_bytes: u64,

    /// The stream URL that was used for capture.
    pub stream_url: String,

    /// ISO 8601 timestamp of when the frame was captured.
    pub captured_at: String,

    /// Human-readable confirmation message.
    pub message: String,
}

// ============================================================================
// define_target / focus_target Tools
// ============================================================================

/// Input for the `define_target` tool.
///
/// A *target* is an OBS-style crop on a display or stream. You — the agent —
/// pick WHAT to focus on and pass a **rough region in normalized 0–1
/// coordinates**: `region.x`/`y` is the top-left corner, `region.w`/`h` the
/// size, both as fractions of the source. Example: the 2nd of 4 equal code
/// columns on an ultrawide = `{x: 0.25, y: 0.0, w: 0.25, h: 1.0}`.
///
/// `define_target` returns a **confirmation image** of the resulting crop so
/// you can SEE what you selected and re-call with an adjusted region if it's
/// off. (Phase 2 will snap the box to real edges; for now use the image to
/// self-correct.)
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct DefineTargetInput {
    /// Unique name for this target (e.g. "editor", "left-pane", "atem-cam").
    pub name: String,
    /// What to crop from: `{"kind":"display","index":0}` or
    /// `{"kind":"stream","url":"rtsp://…"}`.
    pub source: crate::target::model::TargetSource,
    /// The region of interest in NORMALIZED 0–1 coordinates.
    pub region: crate::target::model::NormRect,
    /// Make this the active target (default: true). Only one is active at a time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub set_active: Option<bool>,
}

/// Output for the `define_target` tool.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct DefineTargetOutput {
    /// The target's name.
    pub name: String,
    /// Whether this target is now the active one.
    pub active: bool,
    /// The resolved absolute pixel rect, when a confirmation capture succeeded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pixel_rect: Option<crate::target::model::PixelRect>,
    /// Path to the cropped confirmation PNG, when a frame could be captured.
    /// Pass it to `analyze_video` (image mode) or inspect it to self-correct.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confirmation_image: Option<String>,
    /// Human-readable status (incl. why a confirmation image may be absent).
    pub message: String,
}

/// Input for the `focus_target` tool — switch the active target by name.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct FocusTargetInput {
    /// The name of a previously-defined target to make active.
    pub name: String,
}

/// Output for the `focus_target` tool.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct FocusTargetOutput {
    /// The now-active target's name.
    pub name: String,
    /// Always true on success (the named target is active).
    pub active: bool,
    /// The active target's normalized region.
    pub region: crate::target::model::NormRect,
    /// Human-readable confirmation message.
    pub message: String,
}

// ============================================================================
// measure_target Tool (Phase 2)
// ============================================================================

/// Input for the `measure_target` tool — Zoom-then-Snap measurement.
///
/// Give a ROUGH normalized region; the pure-Rust CV snaps it to the nearest
/// strong edges and detects any tiled-pane grid. Inspect the returned overlay
/// image (green = edges found, red = the snapped box) and the `snapped_rect`,
/// then pass `snapped_rect` to `define_target`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct MeasureTargetInput {
    /// What to measure on: `{"kind":"display","index":0}` or `{"kind":"stream","url":"…"}`.
    pub source: crate::target::model::TargetSource,
    /// The rough region in NORMALIZED 0–1 coordinates.
    pub region: crate::target::model::NormRect,
    /// Also locate a hand-drawn red marker and return its bbox (default false).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub find_red_marker: Option<bool>,
}

/// Output for the `measure_target` tool.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct MeasureTargetOutput {
    /// The snapped measurement (normalized rect, confidence, detected grid).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<crate::target::measure::MeasurementResult>,
    /// Bounding box of a detected red marker (when `find_red_marker` was set).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub red_marker: Option<crate::target::model::PixelRect>,
    /// Path to the "Redline Overlay" diagnostic PNG, when one could be rendered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overlay_image: Option<String>,
    /// Human-readable status.
    pub message: String,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_start_recording_input_defaults() {
        let json = r#"{}"#;
        let input: StartRecordingInput = serde_json::from_str(json).unwrap();
        assert!(input.fps.is_none());
        assert!(input.output_dir.is_none());
        assert!(input.max_duration_seconds.is_none());
    }

    #[test]
    fn test_start_recording_input_with_values() {
        let json = r#"{"fps": 5, "output_dir": "/tmp/recordings", "max_duration_seconds": 600}"#;
        let input: StartRecordingInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.fps, Some(5));
        assert_eq!(input.output_dir, Some("/tmp/recordings".to_string()));
        assert_eq!(input.max_duration_seconds, Some(600));
    }

    #[test]
    fn test_stop_recording_input() {
        let json = r#"{"recording_id": "550e8400-e29b-41d4-a716-446655440000"}"#;
        let input: StopRecordingInput = serde_json::from_str(json).unwrap();
        assert_eq!(
            input.recording_id,
            "550e8400-e29b-41d4-a716-446655440000"
        );
    }

    #[test]
    fn test_analyze_video_input_with_timeframe() {
        let json = r#"{
            "video_path": "/tmp/recording.mp4",
            "prompt": "What happened?",
            "timeframe": {"start_seconds": 10.5, "end_seconds": 20.0}
        }"#;
        let input: AnalyzeVideoInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.video_path, "/tmp/recording.mp4");
        assert_eq!(input.prompt, "What happened?");
        let tf = input.timeframe.unwrap();
        assert_eq!(tf.start_seconds, 10.5);
        assert_eq!(tf.end_seconds, 20.0);
    }

    #[test]
    fn test_list_recordings_input_defaults() {
        let json = r#"{}"#;
        let input: ListRecordingsInput = serde_json::from_str(json).unwrap();
        assert!(input.limit.is_none());
        assert!(input.status_filter.is_none());
    }

    #[test]
    fn test_start_recording_output_serialization() {
        let output = StartRecordingOutput {
            recording_id: "test-id".to_string(),
            status: "recording".to_string(),
            message: "Recording started".to_string(),
        };
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("test-id"));
        assert!(json.contains("recording"));
    }

    #[test]
    fn test_stop_recording_output_optional_fields() {
        let output = StopRecordingOutput {
            recording_id: "test-id".to_string(),
            status: "completed".to_string(),
            file_path: Some("/tmp/video.mp4".to_string()),
            duration_ms: Some(5000),
            file_size_bytes: Some(1024000),
            error_message: None,
        };
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("file_path"));
        assert!(json.contains("duration_ms"));
        assert!(!json.contains("error_message")); // None should be skipped
    }

    #[test]
    fn test_vision_provider_info_output() {
        let output = GetVisionProviderInfoOutput {
            provider: "gemini".to_string(),
            model: "gemini-2.0-flash".to_string(),
            max_video_size_bytes: Some(100_000_000),
            supports_native_video: Some(true),
            available: true,
            error_message: None,
        };
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("gemini"));
        assert!(json.contains("gemini-2.0-flash"));
    }
}
// ---- Dayflow (US6) ---------------------------------------------------------

/// Input for the `start_dayflow` tool.
///
/// Starts continuous activity tracking. Dayflow SAMPLES the screen at an
/// interval — it is not a recording — so an all-day session costs a few frames
/// per minute rather than a video stream.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct StartDayflowInput {
    /// Displays to capture. Empty means every display.
    pub displays: Option<Vec<u32>>,
    /// Capture ONE named window instead of whole displays, matched on its
    /// title or class.
    pub window: Option<String>,
    /// Capture one persisted named target (a saved region of interest).
    pub target: Option<String>,
    /// Capture an INPUT — a stream or capture-device URL. Content that may
    /// never have been rendered on this machine's screen.
    pub input: Option<String>,
    /// `session` (explicit start/stop, the default) or `daemon` (rolls all day).
    pub mode: Option<String>,
}

/// Input for the `stop_dayflow` tool.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct StopDayflowInput {}

/// Input for the `dayflow_status` tool.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct DayflowStatusInput {}

/// Input for the `get_timeline` tool.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct GetTimelineInput {
    /// Start of the range, RFC 3339. Defaults to the start of today.
    pub from: Option<String>,
    /// End of the range, RFC 3339. Defaults to now.
    pub to: Option<String>,
    /// Return the categorized standup digest for the range instead of the raw
    /// entries (FR-028) — the same digest the CLI's `--standup` and HTTP's
    /// `/dayflow/standup` produce, computed by the one shared engine.
    #[serde(default)]
    pub standup: Option<bool>,
}

/// Input for the `ask_day` tool.
///
/// Answers strictly from recorded entries. When the range holds no record it
/// says so rather than inventing one — for a record of someone's day, an
/// invented answer is worse than no answer.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct AskDayInput {
    /// The question, e.g. "what was I doing at 2pm?".
    pub question: String,
    /// Start of the range to ground on, RFC 3339. Defaults to the start of today.
    pub from: Option<String>,
    /// End of the range, RFC 3339. Defaults to now.
    pub to: Option<String>,
}
