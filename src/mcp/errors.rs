//! MCP-layer errors. ⚠️ Minimal reconstruction (Wave-4); recovered file was
//! markdown design-notes, not source.
use thiserror::Error;

#[derive(Debug, Error)]
pub enum McpError {
    #[error("Tool error: {0}")]
    Tool(String),
    #[error("Invalid request: {0}")]
    InvalidRequest(String),
    #[error("Internal MCP error: {0}")]
    Internal(String),
}
