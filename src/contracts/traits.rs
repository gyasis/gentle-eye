//! Core trait definitions for Gentle-Eye modules
//!
//! This file defines the inter-module contracts that enable loose coupling
//! between capture, storage, analysis, and MCP server components.

use async_trait::async_trait;
use std::path::{Path, PathBuf};
use uuid::Uuid;

// Import error types from sibling module
use super::errors::{ConfigError, RecordingError, StorageError, VisionError};

// ============================================================================
// Temporary Type Definitions
// ============================================================================
// NOTE: These types are defined here temporarily until the models module
// is created (tasks T008-T010). Once src/models/ is implemented, these
// should be removed and replaced with:
//
//   pub use crate::models::{
//       AnalysisRequest, AnalysisResult, EncoderMode, Recording, RecordingConfig,
//       RecordingStatus, TimeRange, VisionConfig,
//   };

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Status of a screen recording
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordingStatus {
    /// Recording is in progress
    Recording,
    /// Recording completed successfully
    Completed,
    /// Recording was cancelled
    Cancelled,
    /// Recording failed with an error
    Failed,
}

/// Encoder operating mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncoderMode {
    /// Stream frames directly to FFmpeg (default)
    Streaming,
    /// Write frames to disk first, then encode (for memory pressure)
    FileBased,
}

impl Default for EncoderMode {
    fn default() -> Self {
        Self::Streaming
    }
}

/// Configuration for screen recording
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingConfig {
    /// Frames per second (1-30, default: 1)
    pub fps: u8,
    /// Maximum duration in seconds (optional, default: 300)
    pub max_duration_seconds: Option<u64>,
    /// Output directory for recordings
    pub output_dir: PathBuf,
    /// Encoder mode selection
    pub encoder_mode: EncoderMode,
}

impl Default for RecordingConfig {
    fn default() -> Self {
        Self {
            fps: 1,
            max_duration_seconds: Some(300),
            output_dir: std::env::temp_dir().join("gentle-eye"),
            encoder_mode: EncoderMode::default(),
        }
    }
}

/// Represents a screen recording session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recording {
    /// Unique identifier
    pub id: Uuid,
    /// Current status
    pub status: RecordingStatus,
    /// When recording started
    pub start_time: DateTime<Utc>,
    /// When recording ended (if completed)
    pub end_time: Option<DateTime<Utc>>,
    /// Duration in milliseconds (if completed)
    pub duration_ms: Option<u64>,
    /// Path to output video file (if completed)
    pub file_path: Option<PathBuf>,
    /// File size in bytes (if completed)
    pub file_size_bytes: Option<u64>,
    /// Recording configuration used
    pub config: RecordingConfig,
    /// Error message (if failed)
    pub error_message: Option<String>,
}

impl Recording {
    /// Create a new recording with the given configuration
    pub fn new(config: RecordingConfig) -> Self {
        Self {
            id: Uuid::new_v4(),
            status: RecordingStatus::Recording,
            start_time: Utc::now(),
            end_time: None,
            duration_ms: None,
            file_path: None,
            file_size_bytes: None,
            config,
            error_message: None,
        }
    }
}

/// Time range for video analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeRange {
    /// Start time in seconds
    pub start_seconds: f64,
    /// End time in seconds
    pub end_seconds: f64,
}

/// Configuration for vision AI providers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisionConfig {
    /// Provider name (gemini, ollama)
    pub provider: String,
    /// API key for cloud providers
    pub api_key: Option<String>,
    /// Model identifier
    pub model: String,
    /// Request timeout in seconds
    pub timeout_seconds: u64,
    /// Maximum video file size in bytes
    pub max_video_size_bytes: u64,
}

impl Default for VisionConfig {
    fn default() -> Self {
        Self {
            provider: "gemini".to_string(),
            api_key: None,
            model: "gemini-2.0-flash".to_string(),
            timeout_seconds: 120,
            max_video_size_bytes: 100 * 1024 * 1024, // 100 MB
        }
    }
}

/// Request for video/image analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisRequest {
    /// Unique request identifier
    pub id: Uuid,
    /// Path to the video or image file
    pub file_path: PathBuf,
    /// Analysis prompt/question
    pub prompt: String,
    /// Optional time range for video analysis
    pub time_range: Option<TimeRange>,
    /// When the request was created
    pub created_at: DateTime<Utc>,
}

/// Result of video/image analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisResult {
    /// Request ID this result corresponds to
    pub request_id: Uuid,
    /// The analysis text response
    pub analysis_text: String,
    /// Vision provider used
    pub provider: String,
    /// Model used for analysis
    pub model_used: String,
    /// Processing time in milliseconds
    pub processing_time_ms: u64,
    /// Token count (if available)
    pub token_count: Option<u32>,
    /// When the analysis completed
    pub completed_at: DateTime<Utc>,
}

// ============================================================================
// Capture Module Traits
// ============================================================================

/// Core recording operations exposed to MCP server
///
/// This trait provides the primary interface for screen recording functionality.
/// Implementations handle capture, encoding, and file output.
#[async_trait]
pub trait RecordingService: Send + Sync {
    /// Start a new recording session
    ///
    /// # Arguments
    /// * `config` - Recording configuration (fps, output dir, etc.)
    ///
    /// # Returns
    /// * `Ok(Recording)` - Recording started successfully with initial metadata
    /// * `Err` - If capture permissions denied, display unavailable, or other error
    async fn start_recording(&self, config: RecordingConfig) -> Result<Recording, RecordingError>;

    /// Stop an active recording and finalize video
    ///
    /// # Arguments
    /// * `id` - Recording UUID from start_recording
    ///
    /// # Returns
    /// * `Ok(Recording)` - Final recording metadata with file path
    /// * `Err` - If recording not found or finalization fails
    async fn stop_recording(&self, id: Uuid) -> Result<Recording, RecordingError>;

    /// Cancel a recording without saving
    ///
    /// # Arguments
    /// * `id` - Recording UUID to cancel
    ///
    /// # Returns
    /// * `Ok(Recording)` - Recording with cancelled status
    /// * `Err` - If recording not found
    async fn cancel_recording(&self, id: Uuid) -> Result<Recording, RecordingError>;

    /// Get current status of a recording
    ///
    /// # Arguments
    /// * `id` - Recording UUID to check
    ///
    /// # Returns
    /// * `Ok(Recording)` - Current recording state
    /// * `Err` - If recording not found
    async fn get_status(&self, id: Uuid) -> Result<Recording, RecordingError>;

    /// List recordings with optional filtering
    ///
    /// # Arguments
    /// * `limit` - Maximum number of recordings to return
    /// * `status_filter` - Optional status to filter by
    ///
    /// # Returns
    /// * `Ok(Vec<Recording>)` - List of matching recordings
    async fn list_recordings(
        &self,
        limit: usize,
        status_filter: Option<RecordingStatus>,
    ) -> Result<Vec<Recording>, RecordingError>;
}

// ============================================================================
// Analysis Module Traits
// ============================================================================

/// Vision AI provider abstraction
///
/// Implementations handle video/image analysis using various AI backends.
/// Constitution requires: Gemini (cloud), Ollama (local), HuggingFace (local)
#[async_trait]
pub trait VisionProvider: Send + Sync {
    /// Analyze video content with a contextual prompt
    ///
    /// # Arguments
    /// * `video_path` - Path to video file
    /// * `prompt` - Analysis question/instruction
    /// * `timeframe` - Optional time range to focus on
    ///
    /// # Returns
    /// * `Ok(AnalysisResult)` - Analysis response with metadata
    /// * `Err` - If file not found, API error, etc.
    async fn analyze_video(
        &self,
        video_path: &Path,
        prompt: &str,
        timeframe: Option<TimeRange>,
    ) -> Result<AnalysisResult, VisionError>;

    /// Analyze a single image/frame
    ///
    /// # Arguments
    /// * `image_path` - Path to image file
    /// * `prompt` - Analysis question/instruction
    ///
    /// # Returns
    /// * `Ok(AnalysisResult)` - Analysis response with metadata
    async fn analyze_image(
        &self,
        image_path: &Path,
        prompt: &str,
    ) -> Result<AnalysisResult, VisionError>;

    /// Check if provider is available and properly configured
    ///
    /// # Returns
    /// * `Ok(())` - Provider is ready
    /// * `Err` - Provider unavailable with reason
    async fn health_check(&self) -> Result<(), VisionError>;

    /// Get provider name for logging/debugging
    fn name(&self) -> &'static str;

    /// Get maximum supported video file size in bytes
    fn max_video_size(&self) -> u64;

    /// Whether provider supports native video input (vs frame extraction)
    fn supports_native_video(&self) -> bool;

    /// Get the model identifier being used
    fn model(&self) -> &str;
}

// ============================================================================
// Storage Module Traits
// ============================================================================

/// Result of listing recordings with pagination metadata
#[derive(Debug, Clone)]
pub struct RecordingList {
    /// The recordings matching the query
    pub recordings: Vec<Recording>,
    /// Total count of recordings matching filter (for pagination)
    pub total_count: usize,
}

/// Storage operations for recordings and metadata
#[async_trait]
pub trait StorageManager: Send + Sync {
    /// Get the base storage directory
    fn base_dir(&self) -> &Path;

    /// Generate a path for a new recording
    fn generate_recording_path(&self, id: Uuid) -> PathBuf;

    /// Save recording metadata to database
    async fn save_recording(&self, recording: &Recording) -> Result<(), StorageError>;

    /// Load recording metadata from database
    async fn load_recording(&self, id: Uuid) -> Result<Recording, StorageError>;

    /// Delete recording and associated files
    async fn delete_recording(&self, id: Uuid) -> Result<(), StorageError>;

    /// List recordings with pagination
    ///
    /// # Returns
    /// * `RecordingList` containing both the recordings and total_count for pagination
    async fn list_recordings(
        &self,
        limit: usize,
        offset: usize,
        status_filter: Option<RecordingStatus>,
    ) -> Result<RecordingList, StorageError>;

    /// Get total storage used by recordings in bytes
    async fn storage_used(&self) -> Result<u64, StorageError>;

    /// Clean up old recordings to free space
    async fn cleanup_old_recordings(&self, max_age_days: u32) -> Result<u32, StorageError>;
}

// ============================================================================
// MCP Server Traits
// ============================================================================

/// Server information provided to MCP hosts
#[derive(Debug, Clone)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
}

/// MCP tool result content
#[derive(Debug, Clone)]
pub enum ToolContent {
    Text(String),
    Json(serde_json::Value),
}

/// Result of calling an MCP tool
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub content: Vec<ToolContent>,
    pub is_error: bool,
}

impl ToolResult {
    pub fn success(content: Vec<ToolContent>) -> Self {
        Self {
            content,
            is_error: false,
        }
    }

    pub fn error(message: String) -> Self {
        Self {
            content: vec![ToolContent::Text(message)],
            is_error: true,
        }
    }
}

// ============================================================================
// Configuration Traits
// ============================================================================

/// Configuration provider for runtime settings
pub trait ConfigProvider: Send + Sync {
    /// Get recording configuration
    fn recording_config(&self) -> RecordingConfig;

    /// Get vision provider configuration
    fn vision_config(&self) -> VisionConfig;

    /// Get storage base directory
    fn storage_dir(&self) -> PathBuf;

    /// Reload configuration from disk
    fn reload(&mut self) -> Result<(), ConfigError>;
}

// ============================================================================
// Error Conversion for MCP
// ============================================================================

/// Convert domain errors to MCP-compatible errors
impl From<RecordingError> for ToolResult {
    fn from(e: RecordingError) -> Self {
        ToolResult::error(e.to_string())
    }
}

impl From<VisionError> for ToolResult {
    fn from(e: VisionError) -> Self {
        ToolResult::error(e.to_string())
    }
}

impl From<StorageError> for ToolResult {
    fn from(e: StorageError) -> Self {
        ToolResult::error(e.to_string())
    }
}
