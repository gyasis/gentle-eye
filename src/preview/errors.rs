//! Preview-pane errors, mapped into [`GentleEyeError`] + MCP error codes
//! (mirrors the `TargetError` pattern).

use crate::contracts::errors::McpErrorCode;
use thiserror::Error;

/// Errors from the `preview` (media pane) feature.
#[derive(Debug, Error)]
pub enum PreviewError {
    /// Filesystem I/O failure.
    #[error("Preview I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// Requested file not found.
    #[error("File not found: {0}")]
    NotFound(String),
    /// No captures available to preview.
    #[error("No captures found to preview")]
    NoCaptures,
    /// Failed to spawn the player (ffplay / OS opener).
    #[error("Failed to launch preview: {0}")]
    Spawn(String),
    /// HTTP gallery server error.
    #[error("Preview server error: {0}")]
    Http(String),
}

impl PreviewError {
    /// Map to the appropriate MCP (JSON-RPC) error code.
    pub fn mcp_error_code(&self) -> McpErrorCode {
        match self {
            PreviewError::NotFound(_) | PreviewError::NoCaptures => McpErrorCode::NotFound,
            _ => McpErrorCode::InternalError,
        }
    }
}
