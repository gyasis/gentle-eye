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
    /// Frames per second for the recording (1-30, default: 1)
    ///
    /// Lower values (1-5) are recommended for typical debugging sessions
    /// to reduce file size.
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

/// Input parameters for the `read_screen_text` tool (OCR).
///
/// Provide exactly one of `image_path` or `video_path`.
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
