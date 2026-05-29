//! Configuration module for Gentle-Eye MCP server
//!
//! This module provides configuration management including:
//! - TOML configuration file loading from `~/.config/gentle-eye/config.toml`
//! - Environment variable overrides
//! - Default configuration fallbacks
//! - Runtime configuration access
//!
//! # Configuration Priority
//!
//! 1. Environment variables (highest priority)
//! 2. Configuration file
//! 3. Default values (lowest priority)
//!
//! # Environment Variables
//!
//! - `GEMINI_API_KEY` -> `vision.gemini_api_key`
//! - `GENTLE_EYE_DATA` -> `storage.base_dir`
//! - `GENTLE_EYE_FPS` -> `recording.fps`
//! - `GENTLE_EYE_PROVIDER` -> `vision.provider`
//!
//! # Example
//!
//! ```no_run
//! use gentle_eye::config::{AppConfig, ConfigProvider};
//!
//! let config = AppConfig::load().expect("Failed to load config");
//! let fps = config.recording_config().fps;
//! let storage_dir = config.storage_dir();
//! ```

mod loader;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;

pub use loader::load_config;

// ============================================================================
// Configuration Types
// ============================================================================

/// Recording configuration settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingConfig {
    /// Frames per second (1-30, default: 1)
    #[serde(default = "default_fps")]
    pub fps: u32,

    /// Maximum recording duration in seconds (default: 1800 = 30 minutes)
    #[serde(default = "default_max_duration")]
    pub max_duration_seconds: u64,

    /// Encoder mode selection (default: auto)
    #[serde(default = "default_encoder_mode")]
    pub encoder_mode: EncoderMode,
}

impl Default for RecordingConfig {
    fn default() -> Self {
        Self {
            fps: default_fps(),
            max_duration_seconds: default_max_duration(),
            encoder_mode: default_encoder_mode(),
        }
    }
}

/// Video encoding mode selection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum EncoderMode {
    /// Automatically select based on system resources
    #[default]
    Auto,
    /// Frames saved as PNG files, FFmpeg converts at end
    FileBased,
    /// Frames piped directly to FFmpeg stdin
    InMemoryPipe,
}

/// Vision AI provider configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisionConfig {
    /// Vision provider name: "gemini" or "ollama"
    #[serde(default = "default_provider")]
    pub provider: String,

    /// Gemini API key (can be set via GEMINI_API_KEY env var)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gemini_api_key: Option<String>,

    /// Gemini model identifier
    #[serde(default = "default_gemini_model")]
    pub gemini_model: String,

    /// Ollama server host
    #[serde(default = "default_ollama_host")]
    pub ollama_host: String,

    /// Ollama server port
    #[serde(default = "default_ollama_port")]
    pub ollama_port: u16,

    /// Ollama vision model
    #[serde(default = "default_ollama_model")]
    pub ollama_model: String,

    /// Request timeout in seconds
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
}

impl Default for VisionConfig {
    fn default() -> Self {
        Self {
            provider: default_provider(),
            gemini_api_key: None,
            gemini_model: default_gemini_model(),
            ollama_host: default_ollama_host(),
            ollama_port: default_ollama_port(),
            ollama_model: default_ollama_model(),
            timeout_seconds: default_timeout(),
        }
    }
}

/// Storage configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    /// Base directory for all storage (default: ~/.local/share/gentle-eye)
    #[serde(default = "default_base_dir")]
    pub base_dir: PathBuf,

    /// Number of days to keep old recordings before cleanup
    #[serde(default = "default_cleanup_days")]
    pub cleanup_after_days: u32,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            base_dir: default_base_dir(),
            cleanup_after_days: default_cleanup_days(),
        }
    }
}

// ============================================================================
// Default Value Functions
// ============================================================================

fn default_fps() -> u32 {
    1
}

fn default_max_duration() -> u64 {
    1800 // 30 minutes
}

fn default_encoder_mode() -> EncoderMode {
    EncoderMode::Auto
}

fn default_provider() -> String {
    "gemini".to_string()
}

fn default_gemini_model() -> String {
    // Current alias (the recovered "gemini-2.0-flash" is stale); validated live.
    "gemini-flash-latest".to_string()
}

fn default_ollama_host() -> String {
    "localhost".to_string()
}

fn default_ollama_port() -> u16 {
    11434
}

fn default_ollama_model() -> String {
    // Vision model present on the LAN box (192.168.0.159); validated live.
    "qwen2.5vl:7b".to_string()
}

fn default_timeout() -> u64 {
    // Local vision models (Ollama) process multi-image requests slowly; give them
    // headroom. Gemini returns well within this.
    300
}

fn default_base_dir() -> PathBuf {
    // Use directories crate for proper XDG paths
    if let Some(data_dir) = dirs_data_local_dir() {
        data_dir.join("gentle-eye")
    } else {
        // Fallback to temp directory
        std::env::temp_dir().join("gentle-eye")
    }
}

fn default_cleanup_days() -> u32 {
    7
}

/// Helper function to get the local data directory
fn dirs_data_local_dir() -> Option<PathBuf> {
    // Try XDG_DATA_HOME first
    if let Ok(xdg_data) = std::env::var("XDG_DATA_HOME") {
        return Some(PathBuf::from(xdg_data));
    }

    // Fall back to ~/.local/share
    if let Ok(home) = std::env::var("HOME") {
        return Some(PathBuf::from(home).join(".local").join("share"));
    }

    // Try home_dir as last resort
    dirs::data_local_dir()
}

// ============================================================================
// Configuration Errors
// ============================================================================

/// Errors that can occur during configuration operations
#[derive(Debug, Error)]
pub enum ConfigError {
    /// Configuration file not found
    #[error("Configuration file not found: {0}")]
    NotFound(PathBuf),

    /// Invalid configuration format or value
    #[error("Invalid configuration: {0}")]
    Invalid(String),

    /// Required environment variable not set
    #[error("Environment variable not set: {0}")]
    EnvVarMissing(String),

    /// Configuration value out of acceptable range
    #[error("Configuration value out of range: {field} = {value} (expected {min}-{max})")]
    ValueOutOfRange {
        field: String,
        value: String,
        min: String,
        max: String,
    },

    /// TOML parse error
    #[error("Configuration parse error: {0}")]
    ParseError(String),

    /// I/O error reading configuration
    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),
}

impl From<toml::de::Error> for ConfigError {
    fn from(e: toml::de::Error) -> Self {
        ConfigError::ParseError(e.to_string())
    }
}

// ============================================================================
// ConfigProvider Trait
// ============================================================================

/// Configuration provider trait for runtime settings
///
/// This trait provides the interface for accessing configuration values
/// used by other modules in the application.
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
// AppConfig Implementation
// ============================================================================

// ----------------------------------------------------------------------------
// Dayflow configuration
// ----------------------------------------------------------------------------

fn default_chunk_minutes() -> u32 {
    15
}
fn default_record_fps() -> f32 {
    0.5
}
fn default_dayflow_provider() -> String {
    "gemini".to_string()
}
fn default_hot_grace_hours() -> u32 {
    48
}
fn default_warm_days() -> u32 {
    14
}
fn default_disk_budget_bytes() -> u64 {
    20 * 1024 * 1024 * 1024 // 20 GiB
}

/// 3-tier retention policy for dayflow recordings.
///
/// The timeline is the permanent artifact; raw video is scaffolding:
/// Hot (raw chunks) → Warm (shrunk timelapse/frames) → Cold (timeline only).
/// A disk-budget guard evicts oldest raw, then oldest warm — never the timeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionConfig {
    /// Keep raw chunks until summarized + this grace window, then shrink.
    #[serde(default = "default_hot_grace_hours")]
    pub hot_grace_hours: u32,
    /// Keep the shrunk (warm) artifact this many days before it's evictable.
    #[serde(default = "default_warm_days")]
    pub warm_days: u32,
    /// Hard disk budget; over this, evict oldest raw then oldest warm.
    #[serde(default = "default_disk_budget_bytes")]
    pub disk_budget_bytes: u64,
}

impl Default for RetentionConfig {
    fn default() -> Self {
        Self {
            hot_grace_hours: default_hot_grace_hours(),
            warm_days: default_warm_days(),
            disk_budget_bytes: default_disk_budget_bytes(),
        }
    }
}

/// Dayflow-mode settings (continuous recording → chunk summarization → timeline).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DayflowConfig {
    /// On-the-fly chunk length in minutes (matches Gemini ~1fps native sampling math).
    #[serde(default = "default_chunk_minutes")]
    pub chunk_minutes: u32,
    /// Low capture fps for dayflow (timelapse tier).
    #[serde(default = "default_record_fps")]
    pub record_fps: f32,
    /// Default summarization provider: "gemini" (cloud, default) or "ollama" (local).
    #[serde(default = "default_dayflow_provider")]
    pub default_provider: String,
    /// Retention / shrink / evict policy.
    #[serde(default)]
    pub retention: RetentionConfig,
}

impl Default for DayflowConfig {
    fn default() -> Self {
        Self {
            chunk_minutes: default_chunk_minutes(),
            record_fps: default_record_fps(),
            default_provider: default_dayflow_provider(),
            retention: RetentionConfig::default(),
        }
    }
}

/// Complete application configuration
///
/// This struct implements the `ConfigProvider` trait and holds all
/// configuration settings for the Gentle-Eye MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// Recording settings
    #[serde(default)]
    pub recording: RecordingConfig,

    /// Vision AI provider settings
    #[serde(default)]
    pub vision: VisionConfig,

    /// Storage settings
    #[serde(default)]
    pub storage: StorageConfig,

    /// Dayflow (activity-timeline) settings
    #[serde(default)]
    pub dayflow: DayflowConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            recording: RecordingConfig::default(),
            vision: VisionConfig::default(),
            storage: StorageConfig::default(),
            dayflow: DayflowConfig::default(),
        }
    }
}

impl AppConfig {
    /// Load configuration using the priority: env vars > config file > defaults
    ///
    /// # Returns
    ///
    /// * `Ok(AppConfig)` - Successfully loaded configuration
    /// * `Err(ConfigError)` - If configuration is invalid
    ///
    /// # Example
    ///
    /// ```no_run
    /// use gentle_eye::config::AppConfig;
    ///
    /// let config = AppConfig::load().expect("Failed to load config");
    /// println!("FPS: {}", config.recording.fps);
    /// ```
    pub fn load() -> Result<Self, ConfigError> {
        loader::load_config()
    }

    /// Create a new configuration with all default values
    pub fn with_defaults() -> Self {
        Self::default()
    }

    /// Validate configuration values
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Configuration is valid
    /// * `Err(ConfigError)` - If any value is out of range
    pub fn validate(&self) -> Result<(), ConfigError> {
        // Validate FPS range
        if self.recording.fps < 1 || self.recording.fps > 30 {
            return Err(ConfigError::ValueOutOfRange {
                field: "recording.fps".to_string(),
                value: self.recording.fps.to_string(),
                min: "1".to_string(),
                max: "30".to_string(),
            });
        }

        // Validate max duration range
        if self.recording.max_duration_seconds < 1 || self.recording.max_duration_seconds > 7200 {
            return Err(ConfigError::ValueOutOfRange {
                field: "recording.max_duration_seconds".to_string(),
                value: self.recording.max_duration_seconds.to_string(),
                min: "1".to_string(),
                max: "7200".to_string(),
            });
        }

        // Validate provider
        let valid_providers = ["gemini", "ollama"];
        if !valid_providers.contains(&self.vision.provider.as_str()) {
            return Err(ConfigError::Invalid(format!(
                "Invalid vision provider: {}. Valid options: gemini, ollama",
                self.vision.provider
            )));
        }

        // Validate Ollama port
        if self.vision.ollama_port == 0 {
            return Err(ConfigError::ValueOutOfRange {
                field: "vision.ollama_port".to_string(),
                value: self.vision.ollama_port.to_string(),
                min: "1".to_string(),
                max: "65535".to_string(),
            });
        }

        // Validate timeout
        if self.vision.timeout_seconds < 1 || self.vision.timeout_seconds > 300 {
            return Err(ConfigError::ValueOutOfRange {
                field: "vision.timeout_seconds".to_string(),
                value: self.vision.timeout_seconds.to_string(),
                min: "1".to_string(),
                max: "300".to_string(),
            });
        }

        Ok(())
    }

    /// Get the configuration file path
    ///
    /// Default: `~/.config/gentle-eye/config.toml`
    pub fn config_file_path() -> PathBuf {
        if let Ok(xdg_config) = std::env::var("XDG_CONFIG_HOME") {
            PathBuf::from(xdg_config).join("gentle-eye").join("config.toml")
        } else if let Ok(home) = std::env::var("HOME") {
            PathBuf::from(home)
                .join(".config")
                .join("gentle-eye")
                .join("config.toml")
        } else if let Some(config_dir) = dirs::config_dir() {
            config_dir.join("gentle-eye").join("config.toml")
        } else {
            PathBuf::from("/etc/gentle-eye/config.toml")
        }
    }
}

impl ConfigProvider for AppConfig {
    fn recording_config(&self) -> RecordingConfig {
        self.recording.clone()
    }

    fn vision_config(&self) -> VisionConfig {
        self.vision.clone()
    }

    fn storage_dir(&self) -> PathBuf {
        self.storage.base_dir.clone()
    }

    fn reload(&mut self) -> Result<(), ConfigError> {
        let new_config = loader::load_config()?;
        *self = new_config;
        Ok(())
    }
}

// ============================================================================
// Re-exports for convenience
// ============================================================================

/// Directory functions from the `dirs` crate
mod dirs {
    use std::path::PathBuf;

    pub fn config_dir() -> Option<PathBuf> {
        directories::BaseDirs::new().map(|d| d.config_dir().to_path_buf())
    }

    pub fn data_local_dir() -> Option<PathBuf> {
        directories::BaseDirs::new().map(|d| d.data_local_dir().to_path_buf())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = AppConfig::default();
        assert_eq!(config.recording.fps, 1);
        assert_eq!(config.recording.max_duration_seconds, 1800);
        assert_eq!(config.vision.provider, "gemini");
        assert_eq!(config.vision.timeout_seconds, 300);
    }

    #[test]
    fn test_default_fps() {
        assert_eq!(default_fps(), 1);
    }

    #[test]
    fn test_default_provider() {
        assert_eq!(default_provider(), "gemini");
    }

    #[test]
    fn test_default_storage_dir() {
        let dir = default_base_dir();
        assert!(dir.to_string_lossy().contains("gentle-eye"));
    }

    #[test]
    fn test_validate_valid_config() {
        let config = AppConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_invalid_fps_too_low() {
        let mut config = AppConfig::default();
        config.recording.fps = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_invalid_fps_too_high() {
        let mut config = AppConfig::default();
        config.recording.fps = 31;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_invalid_provider() {
        let mut config = AppConfig::default();
        config.vision.provider = "invalid".to_string();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_invalid_timeout() {
        let mut config = AppConfig::default();
        config.vision.timeout_seconds = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_file_path() {
        let path = AppConfig::config_file_path();
        assert!(path.to_string_lossy().contains("gentle-eye"));
        assert!(path.to_string_lossy().ends_with("config.toml"));
    }

    #[test]
    fn test_encoder_mode_default() {
        assert_eq!(EncoderMode::default(), EncoderMode::Auto);
    }

    #[test]
    fn test_recording_config_serde() {
        let config = RecordingConfig::default();
        let toml_str = toml::to_string(&config).unwrap();
        let parsed: RecordingConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.fps, config.fps);
    }

    #[test]
    fn test_vision_config_serde() {
        let config = VisionConfig::default();
        let toml_str = toml::to_string(&config).unwrap();
        let parsed: VisionConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.provider, config.provider);
    }
}
