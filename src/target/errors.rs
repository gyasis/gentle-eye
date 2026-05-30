//! Target-feature errors, mapped into [`GentleEyeError`] + MCP error codes.
//!
//! Mirrors the `DayflowError` pattern in `contracts/errors.rs`: a domain enum
//! with `From` into the top-level error plus an `mcp_error_code()` mapping.

use crate::contracts::errors::McpErrorCode;
use thiserror::Error;

/// Errors from the `target` (region-of-interest / crop) feature.
#[derive(Debug, Error)]
pub enum TargetError {
    /// Filesystem I/O failure (reading/writing `targets.json`).
    #[error("Target I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// Config (de)serialization or path resolution failure.
    #[error("Target config error: {0}")]
    Config(String),
    /// Named target does not exist in the store.
    #[error("Target not found: {0}")]
    NotFound(String),
    /// Region is outside the unit square or has non-positive area.
    #[error("Invalid region: {0}")]
    InvalidRegion(String),
    /// No target is currently active.
    #[error("No active target")]
    NoActive,
    /// Crop / capture failure.
    #[error("Target capture error: {0}")]
    Capture(String),
    /// Measurement (Phase 2 CV) failure.
    #[error("Target measurement error: {0}")]
    Measure(String),
}

impl TargetError {
    /// Map to the appropriate MCP (JSON-RPC) error code.
    pub fn mcp_error_code(&self) -> McpErrorCode {
        match self {
            TargetError::NotFound(_) => McpErrorCode::NotFound,
            TargetError::InvalidRegion(_) => McpErrorCode::InvalidParams,
            TargetError::NoActive => McpErrorCode::InvalidParams,
            _ => McpErrorCode::InternalError,
        }
    }
}

impl From<serde_json::Error> for TargetError {
    fn from(e: serde_json::Error) -> Self {
        TargetError::Config(e.to_string())
    }
}
