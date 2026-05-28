//! MCP (Model Context Protocol) server module for Gentle-Eye
//!
//! This module implements the MCP server that exposes screen recording
//! and AI video analysis tools to Claude and other MCP-compatible clients.
//!
//! # Architecture
//!
//! The MCP server uses the `rmcp` crate for protocol handling:
//! - [`server`] - The main `GentleEyeServer` struct implementing `ServerHandler`
//! - [`tools`] - Tool input/output types matching the JSON schema from mcp-tools.json
//! - [`errors`] - Error types with MCP error code conversions
//!
//! # MCP Tools
//!
//! The server exposes these tools to MCP clients:
//!
//! | Tool | Description |
//! |------|-------------|
//! | `start_recording` | Start a new screen recording session |
//! | `stop_recording` | Stop an active recording and save the video |
//! | `get_recording_status` | Get the current status of a recording |
//! | `analyze_video` | Analyze a video using vision AI |
//! | `list_recordings` | List recent recordings with metadata |
//! | `cancel_recording` | Cancel a recording without saving |
//! | `get_vision_provider_info` | Get information about the vision AI provider |
//!
//! # Example
//!
//! ```no_run
//! use gentle_eye::mcp::GentleEyeServer;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let server = GentleEyeServer::new().await?;
//!     server.serve_stdio().await?;
//!     Ok(())
//! }
//! ```

pub mod errors;
pub mod server;
pub mod tools;

// Re-export main types for convenience
pub use errors::McpError;
pub use server::GentleEyeServer;
pub use tools::{
    AnalyzeVideoInput, AnalyzeVideoOutput, CancelRecordingInput, CancelRecordingOutput,
    GetRecordingStatusInput, GetRecordingStatusOutput, GetVisionProviderInfoOutput,
    ListRecordingsInput, ListRecordingsOutput, RecordingSummary, StartRecordingInput,
    StartRecordingOutput, StopRecordingInput, StopRecordingOutput, TimeframeInput,
};
