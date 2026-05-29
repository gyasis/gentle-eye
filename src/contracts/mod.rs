//! Contract definitions for Gentle-Eye
//!
//! This module contains all trait definitions and error types that define
//! the interfaces between Gentle-Eye's components. These contracts enable
//! loose coupling and allow for multiple implementations (e.g., different
//! vision providers, mock implementations for testing).
//!
//! # Modules
//!
//! - [`errors`] - Error types with MCP error code mapping
//! - [`traits`] - Trait definitions for services
//!
//! # Re-exports
//!
//! All public types are re-exported at the module level for convenience.

pub mod errors;
pub mod traits;

// Re-export all error types
pub use errors::{
    ConfigError, ConfigResult, DayflowError, GentleEyeError, GentleEyeResult, McpErrorCode,
    RecordingError, RecordingResult, StorageError, StorageResult, VisionError, VisionResult,
};

// Re-export all traits
pub use traits::{ConfigProvider, RecordingService, StorageManager, VisionProvider};

// Re-export MCP types
pub use traits::{ServerInfo, ToolContent, ToolResult};

// Re-export storage types
pub use traits::RecordingList;

// Re-export model types from traits (which re-exports from models module)
pub use traits::{
    AnalysisRequest, AnalysisResult, EncoderMode, Recording, RecordingConfig, RecordingStatus,
    TimeRange, VisionConfig,
};
