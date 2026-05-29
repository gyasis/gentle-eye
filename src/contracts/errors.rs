//! Error types for the gentle-eye MCP server, with MCP error-code mapping.
//!
//! Recovered 2026-05-28 from `RECOVERY/sessions/_seg5_s108378` (the original was
//! never Write-captured; its body survived as a Read page). The enum bodies
//! (RecordingError tail, VisionError, StorageError, ConfigError, McpErrorCode,
//! the `mcp_error_code` impls, `From` conversions, and Result aliases) are
//! [RECOVERED] verbatim. The header — imports, the `GentleEyeError` enum, and
//! `RecordingError`'s first four variants — is [RECONSTRUCTED] from the
//! `mcp_error_code` match arms plus the `contracts/mod.rs` re-export contract.

use std::path::PathBuf;
use thiserror::Error;
use uuid::Uuid;

// [RECONSTRUCTED] top-level error wrapping the domain errors
// (variants determined by the `GentleEyeError::mcp_error_code` match arms below).
/// Top-level error for the gentle-eye server.
#[derive(Debug, Error)]
pub enum GentleEyeError {
    #[error(transparent)]
    Recording(#[from] RecordingError),
    #[error(transparent)]
    Vision(#[from] VisionError),
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error(transparent)]
    Config(#[from] ConfigError),
    /// Dayflow (continuous activity-timeline) error
    #[error(transparent)]
    Dayflow(#[from] DayflowError),
    /// MCP-protocol-level error
    #[error("MCP error: {0}")]
    Mcp(String),
}

/// Dayflow-mode errors (continuous recording → chunk summarization → timeline).
#[derive(Debug, Error)]
pub enum DayflowError {
    /// No dayflow session is currently active.
    #[error("No active dayflow session")]
    NoActiveSession,
    /// A dayflow session is already running.
    #[error("A dayflow session is already running")]
    AlreadyRunning,
    /// Underlying recording/capture failure.
    #[error("Capture error: {0}")]
    Capture(#[from] RecordingError),
    /// Chunk summarization (vision) failure.
    #[error("Summarization error: {0}")]
    Summarization(#[from] VisionError),
    /// Timeline persistence failure.
    #[error("Timeline storage error: {0}")]
    Timeline(#[from] StorageError),
    /// Retention / shrink / evict failure.
    #[error("Retention error: {0}")]
    Retention(String),
    /// Invalid dayflow configuration or request.
    #[error("Invalid dayflow request: {0}")]
    Invalid(String),
    /// Internal/unexpected error.
    #[error("Internal error: {0}")]
    Internal(String),
}

/// Recording-related errors
#[derive(Debug, Error)]
pub enum RecordingError {
    // [RECONSTRUCTED] first four variants (from RecordingError::mcp_error_code match arms).
    /// Requested recording not found
    #[error("Recording not found: {0}")]
    NotFound(Uuid),
    /// A recording is already in progress
    #[error("A recording is already in progress")]
    AlreadyRecording,
    /// No active recording to operate on
    #[error("No active recording")]
    NoActiveRecording,
    /// Screen-capture permission denied
    #[error("Permission denied for screen capture")]
    PermissionDenied,
    // [RECOVERED] from here down.
    /// No display available for capture
    #[error("No display available for capture")]
    NoDisplay,
    /// FFmpeg encoder failure
    #[error("Video encoding failed: {0}")]
    EncoderError(String),
    /// File system error during recording
    #[error("Storage error: {0}")]
    StorageError(#[from] std::io::Error),
    /// Recording exceeded configured maximum duration
    #[error("Recording exceeded maximum duration of {0} seconds")]
    MaxDurationExceeded(u64),
    /// Not enough disk space to continue recording
    #[error("Insufficient disk space. Available: {available_bytes}, Required: {required_bytes}")]
    InsufficientStorage {
        available_bytes: u64,
        required_bytes: u64,
    },
    /// Memory pressure forced encoder mode switch
    #[error("High memory pressure detected, switching to file-based encoding")]
    MemoryPressure,
    /// Recording was cancelled by user
    #[error("Recording cancelled")]
    Cancelled,
    /// Invalid recording configuration
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),
    /// Internal/unexpected error
    #[error("Internal error: {0}")]
    Internal(String),
}

/// Vision AI provider errors
#[derive(Debug, Error)]
pub enum VisionError {
    /// Provider is not available or misconfigured
    #[error("Vision provider unavailable: {0}")]
    Unavailable(String),
    /// Requested video file does not exist
    #[error("Video file not found: {0}")]
    FileNotFound(PathBuf),
    /// Video format not supported by provider
    #[error("Invalid or unsupported video format: {0}")]
    InvalidFormat(String),
    /// Video exceeds provider's size limit
    #[error("Video file too large: {size_bytes} bytes (maximum: {max_bytes} bytes)")]
    FileTooLarge { size_bytes: u64, max_bytes: u64 },
    /// Vision API returned an error
    #[error("API error: {message}")]
    ApiError {
        message: String,
        status_code: Option<u16>,
    },
    /// Network communication failure
    #[error("Network error: {0}")]
    NetworkError(String),
    /// API rate limit exceeded
    #[error("Rate limited. Retry after {retry_after_seconds} seconds.")]
    RateLimited { retry_after_seconds: u64 },
    /// API authentication failed
    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),
    /// Request timed out
    #[error("Request timed out after {timeout_seconds} seconds")]
    Timeout { timeout_seconds: u64 },
    /// Frame extraction from video failed (for providers that don't support native video)
    #[error("Frame extraction failed: {0}")]
    FrameExtractionFailed(String),
    /// Prompt was invalid or too long
    #[error("Invalid prompt: {0}")]
    InvalidPrompt(String),
    /// Timeframe specified is invalid
    #[error("Invalid timeframe: start ({start_seconds}s) must be less than end ({end_seconds}s)")]
    InvalidTimeframe {
        start_seconds: f64,
        end_seconds: f64,
    },
}

/// Storage and persistence errors
#[derive(Debug, Error)]
pub enum StorageError {
    /// Requested recording not found in database
    #[error("Recording not found: {0}")]
    NotFound(Uuid),
    /// SQLite database error
    #[error("Database error: {0}")]
    DatabaseError(String),
    /// File system operation failed
    #[error("File system error: {0}")]
    FileSystemError(#[from] std::io::Error),
    /// JSON serialization/deserialization failed
    #[error("Serialization error: {0}")]
    SerializationError(String),
    /// Storage directory does not exist or is not writable
    #[error("Storage directory not accessible: {0}")]
    DirectoryNotAccessible(PathBuf),
    /// Database migration failed
    #[error("Database migration failed: {0}")]
    MigrationError(String),
}

/// Configuration errors
#[derive(Debug, Error)]
pub enum ConfigError {
    /// Configuration file not found
    #[error("Configuration file not found: {0}")]
    NotFound(PathBuf),
    /// Configuration file is invalid
    #[error("Invalid configuration: {0}")]
    Invalid(String),
    /// Required environment variable not set
    #[error("Environment variable not set: {var_name}")]
    EnvVarMissing { var_name: String },
    /// Configuration value out of acceptable range
    #[error("Configuration value out of range: {field} = {value} (expected {min}-{max})")]
    ValueOutOfRange {
        field: String,
        value: String,
        min: String,
        max: String,
    },
    /// Parse error in configuration file
    #[error("Configuration parse error: {0}")]
    ParseError(String),
}

// ============================================================================
// Error Code Mapping for MCP
// ============================================================================

/// MCP error codes following JSON-RPC conventions
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpErrorCode {
    /// Invalid method parameter(s)
    InvalidParams = -32602,
    /// Internal error
    InternalError = -32603,
    /// Resource not found (custom)
    NotFound = -32001,
    /// Permission denied (custom)
    PermissionDenied = -32002,
    /// Rate limited (custom)
    RateLimited = -32003,
    /// Service unavailable (custom)
    ServiceUnavailable = -32004,
}

impl GentleEyeError {
    /// Get the appropriate MCP error code for this error
    pub fn mcp_error_code(&self) -> McpErrorCode {
        match self {
            GentleEyeError::Recording(e) => e.mcp_error_code(),
            GentleEyeError::Vision(e) => e.mcp_error_code(),
            GentleEyeError::Storage(e) => e.mcp_error_code(),
            GentleEyeError::Config(_) => McpErrorCode::InternalError,
            GentleEyeError::Dayflow(e) => e.mcp_error_code(),
            GentleEyeError::Mcp(_) => McpErrorCode::InternalError,
        }
    }
}

impl DayflowError {
    /// Get the appropriate MCP error code for this error.
    pub fn mcp_error_code(&self) -> McpErrorCode {
        match self {
            DayflowError::NoActiveSession => McpErrorCode::InvalidParams,
            DayflowError::AlreadyRunning => McpErrorCode::InvalidParams,
            DayflowError::Invalid(_) => McpErrorCode::InvalidParams,
            DayflowError::Capture(e) => e.mcp_error_code(),
            DayflowError::Summarization(e) => e.mcp_error_code(),
            DayflowError::Timeline(e) => e.mcp_error_code(),
            _ => McpErrorCode::InternalError,
        }
    }
}

impl RecordingError {
    /// Get the appropriate MCP error code for this error
    pub fn mcp_error_code(&self) -> McpErrorCode {
        match self {
            RecordingError::NotFound(_) => McpErrorCode::NotFound,
            RecordingError::AlreadyRecording => McpErrorCode::InvalidParams,
            RecordingError::NoActiveRecording => McpErrorCode::InvalidParams,
            RecordingError::PermissionDenied => McpErrorCode::PermissionDenied,
            RecordingError::NoDisplay => McpErrorCode::ServiceUnavailable,
            RecordingError::InvalidConfig(_) => McpErrorCode::InvalidParams,
            _ => McpErrorCode::InternalError,
        }
    }
}

impl VisionError {
    /// Get the appropriate MCP error code for this error
    pub fn mcp_error_code(&self) -> McpErrorCode {
        match self {
            VisionError::Unavailable(_) => McpErrorCode::ServiceUnavailable,
            VisionError::FileNotFound(_) => McpErrorCode::NotFound,
            VisionError::InvalidFormat(_) => McpErrorCode::InvalidParams,
            VisionError::FileTooLarge { .. } => McpErrorCode::InvalidParams,
            VisionError::RateLimited { .. } => McpErrorCode::RateLimited,
            VisionError::AuthenticationFailed(_) => McpErrorCode::PermissionDenied,
            VisionError::InvalidPrompt(_) => McpErrorCode::InvalidParams,
            VisionError::InvalidTimeframe { .. } => McpErrorCode::InvalidParams,
            _ => McpErrorCode::InternalError,
        }
    }
}

impl StorageError {
    /// Get the appropriate MCP error code for this error
    pub fn mcp_error_code(&self) -> McpErrorCode {
        match self {
            StorageError::NotFound(_) => McpErrorCode::NotFound,
            StorageError::DirectoryNotAccessible(_) => McpErrorCode::PermissionDenied,
            _ => McpErrorCode::InternalError,
        }
    }
}

// ============================================================================
// Conversion implementations
// ============================================================================

impl From<rusqlite::Error> for StorageError {
    fn from(e: rusqlite::Error) -> Self {
        StorageError::DatabaseError(e.to_string())
    }
}

impl From<serde_json::Error> for StorageError {
    fn from(e: serde_json::Error) -> Self {
        StorageError::SerializationError(e.to_string())
    }
}

impl From<reqwest::Error> for VisionError {
    fn from(e: reqwest::Error) -> Self {
        if e.is_timeout() {
            VisionError::Timeout {
                timeout_seconds: 60,
            }
        } else if e.is_connect() {
            VisionError::NetworkError(format!("Connection failed: {}", e))
        } else {
            VisionError::NetworkError(e.to_string())
        }
    }
}

impl From<toml::de::Error> for ConfigError {
    fn from(e: toml::de::Error) -> Self {
        ConfigError::ParseError(e.to_string())
    }
}

// ============================================================================
// Result type aliases
// ============================================================================

pub type RecordingResult<T> = Result<T, RecordingError>;
pub type VisionResult<T> = Result<T, VisionError>;
pub type StorageResult<T> = Result<T, StorageError>;
pub type ConfigResult<T> = Result<T, ConfigError>;
pub type GentleEyeResult<T> = Result<T, GentleEyeError>;
