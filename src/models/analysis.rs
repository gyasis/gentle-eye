// ⚠️ SYNTHESIZED (deterministic unescape from path-line container) 2026-05-26 — verify before promoting.
//! Analysis data models for video AI processing.
//!
//! This module contains the core data structures used for submitting
//! videos to a vision AI provider and capturing the results:
//! - [`AnalysisRequest`] - A request to analyze a video
//! - [`AnalysisResult`] - The result of a video analysis
//! - [`TimeRange`] - An optional time range within a video

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use uuid::Uuid;

/// The `AnalysisRequest` struct encapsulates all the information needed to submit
/// a video for AI analysis, including the video location, prompt, and
/// optional time constraints.
///
/// # Fields
///
/// * `id` - Unique identifier for this analysis request
/// * `recording_id` - Optional link to an associated Recording
/// * `video_path` - Path to the video file to analyze
/// * `prompt` - The question or instruction for the AI
/// * `provider` - Name of the vision provider (e.g., "gemini", "ollama")
/// * `timestamp` - When this request was created
/// * `timeframe` - Optional time range within the video to analyze
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisRequest {
    /// Unique identifier for this analysis request.
    pub id: Uuid,

    /// Optional reference to an associated Recording.
    /// `None` if analyzing an external video file.
    pub recording_id: Option<Uuid>,

    /// Path to the video file to be analyzed.
    pub video_path: PathBuf,

    /// The prompt or question to send to the vision AI.
    /// Should describe what information to extract from the video.
    pub prompt: String,

    /// Name of the vision provider to use (e.g., "gemini", "ollama").
    pub provider: String,

    /// Timestamp when this request was created.
    pub timestamp: DateTime<Utc>,

    /// Optional time range within the video to analyze.
    /// If `None`, the entire video will be analyzed.
    pub timeframe: Option<TimeRange>,
}

impl AnalysisRequest {
    /// Creates a new analysis request with the given parameters.
    ///
    /// # Arguments
    ///
    /// * `video_path` - Path to the video file
    /// * `prompt` - The question or instruction for the AI
    /// * `provider` - Name of the vision provider
    ///
    /// # Example
    ///
    /// ```
    /// use gentle_eye::models::analysis::AnalysisRequest;
    /// use std::path::PathBuf;
    ///
    /// let request = AnalysisRequest::new(
    ///     PathBuf::from("/tmp/video.mp4"),
    ///     "What is shown in this video?".to_string(),
    ///     "gemini".to_string(),
    /// );
    /// ```
    pub fn new(video_path: PathBuf, prompt: String, provider: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            recording_id: None,
            video_path,
            prompt,
            provider,
            timestamp: Utc::now(),
            timeframe: None,
        }
    }

    /// Creates a new analysis request for a specific recording.
    ///
    /// # Arguments
    ///
    /// * `recording_id` - UUID of the associated recording
    /// * `video_path` - Path to the video file
    /// * `prompt` - The question or instruction for the AI
    /// * `provider` - Name of the vision provider
    pub fn for_recording(
        recording_id: Uuid,
        video_path: PathBuf,
        prompt: String,
        provider: String,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            recording_id: Some(recording_id),
            video_path,
            prompt,
            provider,
            timestamp: Utc::now(),
            timeframe: None,
        }
    }

    /// Sets a time range for the analysis.
    ///
    /// # Arguments
    ///
    /// * `timeframe` - The time range within the video to analyze
    pub fn with_timeframe(mut self, timeframe: TimeRange) -> Self {
        self.timeframe = Some(timeframe);
        self
    }
}

/// Represents a time range within a video for partial analysis.
///
/// Used to specify a segment of the video to analyze rather than
/// the entire duration. Times are specified in seconds from the
/// start of the video.
///
/// # Invariants
///
/// * `start_seconds` must be non-negative
/// * `end_seconds` must be greater than `start_seconds`
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct TimeRange {
    /// Start time in seconds from the beginning of the video.
    pub start_seconds: f64,

    /// End time in seconds from the beginning of the video.
    pub end_seconds: f64,
}

impl TimeRange {
    /// Creates a new time range.
    ///
    /// # Arguments
    ///
    /// * `start_seconds` - Start time in seconds
    /// * `end_seconds` - End time in seconds
    ///
    /// # Panics
    ///
    /// Panics if `start_seconds` is negative or `end_seconds <= start_seconds`.
    ///
    /// # Example
    ///
    /// ```
    /// use gentle_eye::models::analysis::TimeRange;
    ///
    /// let range = TimeRange::new(10.0, 30.0);
    /// assert_eq!(range.duration_seconds(), 20.0);
    /// ```
    pub fn new(start_seconds: f64, end_seconds: f64) -> Self {
        assert!(
            start_seconds >= 0.0,
            "start_seconds must be non-negative, got {}",
            start_seconds
        );
        assert!(
            end_seconds > start_seconds,
            "end_seconds ({}) must be greater than start_seconds ({})",
            end_seconds,
            start_seconds
        );

        Self {
            start_seconds,
            end_seconds,
        }
    }

    /// Attempts to create a new time range, returning `None` if invalid.
    ///
    /// # Arguments
    ///
    /// * `start_seconds` - Start time in seconds
    /// * `end_seconds` - End time in seconds
    ///
    /// # Returns
    ///
    /// `Some(TimeRange)` if valid, `None` if the range is invalid.
    pub fn try_new(start_seconds: f64, end_seconds: f64) -> Option<Self> {
        if start_seconds >= 0.0 && end_seconds > start_seconds {
            Some(Self {
                start_seconds,
                end_seconds,
            })
        } else {
            None
        }
    }

    /// Returns the duration of this time range in seconds.
    pub fn duration_seconds(&self) -> f64 {
        self.end_seconds - self.start_seconds
    }

    /// Returns the duration of this time range in milliseconds.
    pub fn duration_ms(&self) -> u64 {
        (self.duration_seconds() * 1000.0) as u64
    }

    /// Checks if a given time falls within this range.
    ///
    /// # Arguments
    ///
    /// * `time_seconds` - Time to check in seconds
    pub fn contains(&self, time_seconds: f64) -> bool {
        time_seconds >= self.start_seconds && time_seconds <= self.end_seconds
    }
}

/// Represents the result of a video analysis from a vision AI provider.
///
/// Contains the analysis text, metadata about the processing, and
/// success/failure information.
///
/// # Fields
///
/// * `id` - Unique identifier for this result
/// * `request_id` - Reference to the originating AnalysisRequest
/// * `analysis_text` - The AI-generated analysis or error message
/// * `model_used` - Specific model that performed the analysis
/// * `token_count` - Number of tokens used (if available)
/// * `processing_time_ms` - Time taken for analysis in milliseconds
/// * `timestamp` - When this result was generated
/// * `success` - Whether the analysis completed successfully
/// * `error_message` - Error details if analysis failed
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisResult {
    /// Unique identifier for this analysis result.
    pub id: Uuid,

    /// Reference to the originating analysis request.
    pub request_id: Uuid,

    /// The analysis text generated by the vision AI.
    /// Contains the actual analysis if successful, or may be empty on failure.
    pub analysis_text: String,

    /// The specific model that performed the analysis.
    /// For example: "gemini-2.0-flash" or "llava:latest".
    pub model_used: String,

    /// Number of tokens used for the analysis.
    /// `None` if the provider doesn't report token usage.
    pub token_count: Option<u32>,

    /// Time taken to process the analysis in milliseconds.
    pub processing_time_ms: u64,

    /// Timestamp when this result was generated.
    pub timestamp: DateTime<Utc>,

    /// Whether the analysis completed successfully.
    pub success: bool,

    /// Error message if the analysis failed.
    /// `None` if the analysis was successful.
    pub error_message: Option<String>,
}

impl AnalysisResult {
    /// Creates a successful analysis result.
    ///
    /// # Arguments
    ///
    /// * `request_id` - UUID of the originating request
    /// * `analysis_text` - The generated analysis text
    /// * `model_used` - Name of the model that performed the analysis
    /// * `processing_time_ms` - Time taken for analysis
    ///
    /// # Example
    ///
    /// ```
    /// use gentle_eye::models::analysis::AnalysisResult;
    /// use uuid::Uuid;
    ///
    /// let result = AnalysisResult::success(
    ///     Uuid::new_v4(),
    ///     "The video shows a user navigating a web application.".to_string(),
    ///     "gemini-2.0-flash".to_string(),
    ///     1500,
    /// );
    /// assert!(result.success);
    /// ```
    pub fn success(
        request_id: Uuid,
        analysis_text: String,
        model_used: String,
        processing_time_ms: u64,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            request_id,
            analysis_text,
            model_used,
            token_count: None,
            processing_time_ms,
            timestamp: Utc::now(),
            success: true,
            error_message: None,
        }
    }

    /// Creates a failed analysis result.
    ///
    /// # Arguments
    ///
    /// * `request_id` - UUID of the originating request
    /// * `error_message` - Description of the failure
    /// * `model_used` - Name of the model (or provider) that failed
    /// * `processing_time_ms` - Time elapsed before failure
    ///
    /// # Example
    ///
    /// ```
    /// use gentle_eye::models::analysis::AnalysisResult;
    /// use uuid::Uuid;
    ///
    /// let result = AnalysisResult::failure(
    ///     Uuid::new_v4(),
    ///     "API rate limit exceeded".to_string(),
    ///     "gemini-2.0-flash".to_string(),
    ///     100,
    /// );
    /// assert!(!result.success);
    /// ```
    pub fn failure(
        request_id: Uuid,
        error_message: String,
        model_used: String,
        processing_time_ms: u64,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            request_id,
            analysis_text: String::new(),
            model_used,
            token_count: None,
            processing_time_ms,
            timestamp: Utc::now(),
            success: false,
            error_message: Some(error_message),
        }
    }

    /// Sets the token count for this result.
    ///
    /// # Arguments
    ///
    /// * `count` - Number of tokens used
    pub fn with_token_count(mut self, count: u32) -> Self {
        self.token_count = Some(count);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_analysis_request_new() {
        let request = AnalysisRequest::new(
            PathBuf::from("/tmp/video.mp4"),
            "Describe this video".to_string(),
            "gemini".to_string(),
        );

        assert_eq!(request.video_path, PathBuf::from("/tmp/video.mp4"));
        assert_eq!(request.prompt, "Describe this video");
        assert_eq!(request.provider, "gemini");
        assert!(request.recording_id.is_none());
        assert!(request.timeframe.is_none());
    }

    #[test]
    fn test_analysis_request_for_recording() {
        let recording_id = Uuid::new_v4();
        let request = AnalysisRequest::for_recording(
            recording_id,
            PathBuf::from("/tmp/video.mp4"),
            "Describe this video".to_string(),
            "gemini".to_string(),
        );

        assert_eq!(request.recording_id, Some(recording_id));
    }

    #[test]
    fn test_analysis_request_with_timeframe() {
        let request = AnalysisRequest::new(
            PathBuf::from("/tmp/video.mp4"),
            "Describe this video".to_string(),
            "gemini".to_string(),
        )
        .with_timeframe(TimeRange::new(10.0, 30.0));

        assert!(request.timeframe.is_some());
        assert_eq!(request.timeframe.unwrap().start_seconds, 10.0);
        assert_eq!(request.timeframe.unwrap().end_seconds, 30.0);
    }

    #[test]
    fn test_time_range_new() {
        let range = TimeRange::new(10.0, 30.0);
        assert_eq!(range.start_seconds, 10.0);
        assert_eq!(range.end_seconds, 30.0);
    }

    #[test]
    #[should_panic(expected = "start_seconds must be non-negative")]
    fn test_time_range_negative_start() {
        TimeRange::new(-5.0, 30.0);
    }

    #[test]
    #[should_panic(expected = "end_seconds")]
    fn test_time_range_invalid_order() {
        TimeRange::new(30.0, 10.0);
    }

    #[test]
    fn test_time_range_try_new() {
        assert!(TimeRange::try_new(10.0, 30.0).is_some());
        assert!(TimeRange::try_new(-5.0, 30.0).is_none());
        assert!(TimeRange::try_new(30.0, 10.0).is_none());
        assert!(TimeRange::try_new(10.0, 10.0).is_none());
    }

    #[test]
    fn test_time_range_duration() {
        let range = TimeRange::new(10.0, 30.0);
        assert_eq!(range.duration_seconds(), 20.0);
        assert_eq!(range.duration_ms(), 20000);
    }

    #[test]
    fn test_time_range_contains() {
        let range = TimeRange::new(10.0, 30.0);
        assert!(range.contains(10.0));
        assert!(range.contains(20.0));
        assert!(range.contains(30.0));
        assert!(!range.contains(5.0));
        assert!(!range.contains(35.0));
    }

    #[test]
    fn test_analysis_result_success() {
        let request_id = Uuid::new_v4();
        let result = AnalysisResult::success(
            request_id,
            "Analysis text".to_string(),
            "gemini-2.0-flash".to_string(),
            1500,
        );

        assert_eq!(result.request_id, request_id);
        assert_eq!(result.analysis_text, "Analysis text");
        assert_eq!(result.model_used, "gemini-2.0-flash");
        assert_eq!(result.processing_time_ms, 1500);
        assert!(result.success);
        assert!(result.error_message.is_none());
    }

    #[test]
    fn test_analysis_result_failure() {
        let request_id = Uuid::new_v4();
        let result = AnalysisResult::failure(
            request_id,
            "Rate limit exceeded".to_string(),
            "gemini-2.0-flash".to_string(),
            100,
        );

        assert_eq!(result.request_id, request_id);
        assert!(result.analysis_text.is_empty());
        assert!(!result.success);
        assert_eq!(result.error_message, Some("Rate limit exceeded".to_string()));
    }

    #[test]
    fn test_analysis_result_with_token_count() {
        let request_id = Uuid::new_v4();
        let result = AnalysisResult::success(
            request_id,
            "Analysis text".to_string(),
            "gemini-2.0-flash".to_string(),
            1500,
        )
        .with_token_count(500);

        assert_eq!(result.token_count, Some(500));
    }

    #[test]
    fn test_analysis_request_serialization() {
        let request = AnalysisRequest::new(
            PathBuf::from("/tmp/video.mp4"),
            "Describe this video".to_string(),
            "gemini".to_string(),
        );

        let json = serde_json::to_string(&request).expect("serialization failed");
        let deserialized: AnalysisRequest =
            serde_json::from_str(&json).expect("deserialization failed");

        assert_eq!(request.id, deserialized.id);
        assert_eq!(request.prompt, deserialized.prompt);
        assert_eq!(request.provider, deserialized.provider);
    }

    #[test]
    fn test_analysis_result_serialization() {
        let result = AnalysisResult::success(
            Uuid::new_v4(),
            "Analysis text".to_string(),
            "gemini-2.0-flash".to_string(),
            1500,
        );

        let json = serde_json::to_string(&result).expect("serialization failed");
        let deserialized: AnalysisResult =
            serde_json::from_str(&json).expect("deserialization failed");

        assert_eq!(result.id, deserialized.id);
        assert_eq!(result.success, deserialized.success);
    }
}
