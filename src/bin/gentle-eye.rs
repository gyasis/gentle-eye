use std::process::ExitCode;
use anyhow::Result;
use tokio::signal;
use tracing::{error, info, warn};
use tracing_subscriber::{fmt, EnvFilter};
use gentle_eye::config::AppConfig;
use gentle_eye::mcp::GentleEyeServer;
use gentle_eye::startup::{validate_startup, StartupError};
// ============================================================================
// Main Entry Point
// ============================================================================
_**Agent (claude-opus-4-5-20251101) (sidechain)**_
Tool use: **Edit**
The file /home/gyasis/Documents/code/gentle-eye/src/bin/gentle-eye.rs has been updated. Here's the result of running `cat -n` on a snippet of the edited file:
        eprintln!("Warning: Failed to set up logging: {}", e);
/// Run the MCP server with graceful shutdown
async fn run() -> Result<()> {
    info!(
        "Starting Gentle-Eye MCP server v{}",
        env!("CARGO_PKG_VERSION")
    // Load configuration first
    let config = match AppConfig::load() {
        Ok(cfg) => {
            info!("Configuration loaded successfully");
            cfg
        Err(e) => {
            error!("Failed to load configuration: {}", e);
            return Err(anyhow::anyhow!("Configuration error: {}", e));
    // Run startup validation checks
    info!("Running startup validation checks...");
    let validation = validate_startup(&config);
    // Log all warnings
    for warning in validation.warnings() {
        warn!("Startup warning: {}", warning);
    // Check for critical failures
    if !validation.all_passed {
        for err in validation.errors() {
            error!("Startup check failed: {}", err);
            // Provide helpful error messages for common issues
            match err {
                StartupError::FfmpegNotFound { install_command } => {
                    error!("FFmpeg is required for video encoding.");
                    error!("Install it with: {}", install_command);
                }
                StartupError::StorageDirectoryNotAccessible { path, reason } => {
                    error!("Cannot access storage directory: {:?}", path);
                    error!("Reason: {}", reason);
                    error!("Fix permissions or specify a different directory in config.");
                }
                StartupError::StorageDirectoryNotWritable { path } => {
                    error!("Storage directory is not writable: {:?}", path);
                    error!("Check permissions: chmod 755 {:?}", path);
                }
                StartupError::ScreenCapturePermissionDenied(msg) => {
                    error!("{}", msg);
                }
                StartupError::EnvVarMissing { var_name, hint } => {
                    error!("Required environment variable {} is not set.", var_name);
                    error!("Hint: {}", hint);
                }
                StartupError::ConfigError(msg) => {
                    error!("Configuration error: {}", msg);
                }
                _ => {}
            }
        return Err(anyhow::anyhow!("Startup validation failed. See errors above."));
    info!(
        "Startup validation passed with {} warning(s)",
        validation.warning_count
    // Create the server with validated configuration
    let server = GentleEyeServer::new().await?;
    info!(
        "Configuration: provider={}, fps={}",
        server.config().vision.provider,
        server.config().recording.fps
    // Spawn the server task
    let server_handle = tokio::spawn(async move {
        server.serve_stdio().await
_**Agent (claude-opus-4-5-20251101) (sidechain)**_
Now let me check on the MCP server implementation to understand how it implements tools/list. Let me read the server.rs file.
_**Agent (claude-opus-4-5-20251101) (sidechain)**_
Tool use: **Read** `./src/mcp/server.rs`
//! MCP Server implementation for Gentle-Eye
//! This module provides the main `GentleEyeServer` struct that implements
//! the MCP `ServerHandler` trait. The server exposes 7 tools for screen
//! recording and AI video analysis.
//! # Tools
//! - `start_recording` - Start a new screen recording session
//! - `stop_recording` - Stop an active recording and save the video
//! - `get_recording_status` - Get the current status of a recording
//! - `analyze_video` - Analyze a video using vision AI
//! - `list_recordings` - List recent recordings with metadata
//! - `cancel_recording` - Cancel a recording without saving
//! - `get_vision_provider_info` - Get information about the vision AI provider
use chrono::Utc;
use rmcp::{
    ServerHandler,
    model::ServerInfo,