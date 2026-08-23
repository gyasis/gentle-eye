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
    // Vision model present on the LAN box (<LAN_OLLAMA_HOST>); validated live.
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

fn default_segment_seconds() -> u32 {
    900 // 15 minutes
}

fn default_idle_enabled() -> bool {
    true
}

fn default_idle_threshold_seconds() -> u32 {
    300 // 5 minutes
}

fn default_idle_hysteresis_seconds() -> u32 {
    30
}

/// Neutral placeholder. The real governed-lane host is machine-local and is
/// supplied by config file or environment — never committed here.
fn default_perception_endpoint() -> String {
    "http://127.0.0.1:11434".to_string()
}

/// `/api/generate`, NOT `/api/chat` — see [`PerceptionConfig`].
fn default_perception_api_path() -> String {
    "/api/generate".to_string()
}

fn default_text_model() -> String {
    "deepseek-ocr:latest".to_string()
}

/// Pinned. Verbose variants collapse the model — see [`PerceptionConfig`].
fn default_text_prompt() -> String {
    "Free OCR.".to_string()
}

fn default_grounding_prompt() -> String {
    "<image>\n<|grounding|>Convert the document to markdown.".to_string()
}

fn default_reason_model() -> String {
    "ornith-1.5-9b:latest".to_string()
}

fn default_max_regions_per_segment() -> u32 {
    12
}

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

/// Which displays dayflow captures.
///
/// Default is [`DisplaySelection::All`]: every attached display is captured and
/// merged into ONE timeline, with each entry identifying its source display
/// (FR-029). Decided 2026-08-23.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplaySelection {
    /// Capture every attached display.
    All,
    /// Capture only these display indices.
    Only(Vec<u32>),
}

impl Default for DisplaySelection {
    fn default() -> Self {
        Self::All
    }
}

/// Idle-pause policy.
///
/// Capture PAUSES when the user goes idle and resumes on activity (FR-030/031);
/// a paused interval is an explicit gap, never a degraded reading (FR-032).
///
/// Idle comes from the X11 MIT-SCREEN-SAVER idle counter, verified monotonic
/// (T005). Lock detection is deliberately NOT part of this: the X saver `state`
/// field is unusable under GNOME (reports 3, outside the documented 0/1/2
/// range), and lock-based pausing was descoped 2026-08-23 — the idle threshold
/// is the primary and sufficient trigger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdleConfig {
    /// Pause capture while the user is idle.
    #[serde(default = "default_idle_enabled")]
    pub enabled: bool,
    /// Idle seconds before capture pauses.
    #[serde(default = "default_idle_threshold_seconds")]
    pub threshold_seconds: u32,
    /// Dwell applied to BOTH transitions so brief inactivity cannot thrash the
    /// recorder into a burst of tiny segments.
    #[serde(default = "default_idle_hysteresis_seconds")]
    pub hysteresis_seconds: u32,
}

impl Default for IdleConfig {
    fn default() -> Self {
        Self {
            enabled: default_idle_enabled(),
            threshold_seconds: default_idle_threshold_seconds(),
            hysteresis_seconds: default_idle_hysteresis_seconds(),
        }
    }
}

/// Whether the text tier is held resident while a recording is active.
///
/// UNSETTLED — decided at T029, not here. Two measured facts pull opposite ways:
/// the lane reported a model resident with an expiry hours out (suggesting the
/// keep-alive is long and a pinger is pointless), yet a probe in the same
/// session paid a 43.9 s cold load because a 32 GB tenant had evicted the OCR
/// model, against 0.5–3.2 s warm. Default is [`ResidencyPolicy::OnDemand`]
/// because it builds nothing and holds nothing until the question is settled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResidencyPolicy {
    /// Keep the text tier warm for the duration of an active recording.
    Resident,
    /// Accept the reload cost; hold nothing between segments.
    OnDemand,
    /// Do not manage residency at all.
    Off,
}

impl Default for ResidencyPolicy {
    fn default() -> Self {
        Self::OnDemand
    }
}

/// Two-tier perception: cheap local text extraction, escalating to a vision
/// model only for meaning (D6/D7).
///
/// # The endpoint is load-bearing
///
/// `api_path` defaults to `/api/generate`, **not** `/api/chat`. The text tier is
/// an OCR specialist, not a chat model: routing it through the chat endpoint
/// wraps the prompt in a chat template and the template bleeds into the output
/// as `>user` / `>system` / `<|im_end|>` markers. Measured 2026-08-23 — same
/// image and prompt, `/api/generate` returns clean verbatim text.
///
/// # The prompt is load-bearing
///
/// `text_prompt` is pinned. A verbose "transcribe verbatim, do not reformat"
/// instruction does not degrade the answer, it destroys it: 42.9 s and 7366
/// tokens of a degenerate repetition loop, returned with a 200 and no error.
/// `Free OCR.` returned perfect text in 0.5 s warm.
///
/// # No host here
///
/// `endpoint` deliberately defaults to a neutral loopback address. The real
/// governed-lane host is machine-local and supplied by config file or
/// environment — never committed to this repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerceptionConfig {
    /// Base URL of the governed model lane. Supply the real host via config or
    /// environment; the default is a neutral placeholder.
    #[serde(default = "default_perception_endpoint")]
    pub endpoint: String,
    /// Generation path. MUST be `/api/generate` for the text tier — see the
    /// type-level docs.
    #[serde(default = "default_perception_api_path")]
    pub api_path: String,
    /// Text tier: the cheap OCR specialist that handles nearly all volume.
    #[serde(default = "default_text_model")]
    pub text_model: String,
    /// Pinned text-extraction prompt. Do not make this verbose.
    #[serde(default = "default_text_prompt")]
    pub text_prompt: String,
    /// Prompt that additionally returns per-block bounding boxes, at roughly 6x
    /// the latency of `text_prompt`. Used deliberately when intra-region layout
    /// is wanted, never as the default path.
    #[serde(default = "default_grounding_prompt")]
    pub grounding_prompt: String,
    /// Reason tier: spent only on semantic or relational questions.
    #[serde(default = "default_reason_model")]
    pub reason_model: String,
    /// Whether the text tier is held resident during a recording.
    #[serde(default)]
    pub residency: ResidencyPolicy,
    /// Hard cap on regions perceived per segment per display. Bounds work at the
    /// source so the rate-limit budget is a safety net rather than the shaper.
    #[serde(default = "default_max_regions_per_segment")]
    pub max_regions_per_segment: u32,
}

impl Default for PerceptionConfig {
    fn default() -> Self {
        Self {
            endpoint: default_perception_endpoint(),
            api_path: default_perception_api_path(),
            text_model: default_text_model(),
            text_prompt: default_text_prompt(),
            grounding_prompt: default_grounding_prompt(),
            reason_model: default_reason_model(),
            residency: ResidencyPolicy::default(),
            max_regions_per_segment: default_max_regions_per_segment(),
        }
    }
}

/// Dayflow-mode settings (continuous recording → chunk summarization → timeline).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DayflowConfig {
    /// Segment length in SECONDS — the authoritative interval (FR-034).
    ///
    /// User-configurable to any value and changeable mid-day; a change takes
    /// effect at the next boundary and never re-times an existing entry
    /// (FR-035). Seconds rather than minutes so short intervals are expressible
    /// for tests and smoke runs.
    ///
    /// A day may therefore contain segments of DIFFERENT lengths. Nothing
    /// downstream may derive a duration by multiplying a count by this value —
    /// always read the segment's own recorded start and end.
    #[serde(default = "default_segment_seconds")]
    pub segment_seconds: u32,
    /// Legacy interval in minutes, retained so existing config files keep
    /// parsing. [`DayflowConfig::segment_duration`] is the accessor to use;
    /// `segment_seconds` wins.
    #[serde(default = "default_chunk_minutes")]
    pub chunk_minutes: u32,
    /// Low capture fps for dayflow (timelapse tier).
    #[serde(default = "default_record_fps")]
    pub record_fps: f32,
    /// Default summarization provider: "gemini" (cloud, opt-in) or "ollama" (local).
    #[serde(default = "default_dayflow_provider")]
    pub default_provider: String,
    /// Which displays are captured (FR-029).
    #[serde(default)]
    pub displays: DisplaySelection,
    /// Idle-pause policy (FR-030/031/032).
    #[serde(default)]
    pub idle: IdleConfig,
    /// Two-tier perception configuration (D6/D7/D8).
    #[serde(default)]
    pub perception: PerceptionConfig,
    /// Retention / shrink / evict policy.
    #[serde(default)]
    pub retention: RetentionConfig,
}

impl Default for DayflowConfig {
    fn default() -> Self {
        Self {
            segment_seconds: default_segment_seconds(),
            chunk_minutes: default_chunk_minutes(),
            record_fps: default_record_fps(),
            default_provider: default_dayflow_provider(),
            displays: DisplaySelection::default(),
            idle: IdleConfig::default(),
            perception: PerceptionConfig::default(),
            retention: RetentionConfig::default(),
        }
    }
}

impl DayflowConfig {
    /// The configured segment length.
    ///
    /// `segment_seconds` is authoritative; `chunk_minutes` is consulted only
    /// when `segment_seconds` is zero, so a legacy config file that sets only
    /// the old key still behaves as its author intended.
    pub fn segment_duration(&self) -> std::time::Duration {
        let secs = if self.segment_seconds > 0 {
            u64::from(self.segment_seconds)
        } else {
            u64::from(self.chunk_minutes) * 60
        };
        std::time::Duration::from_secs(secs)
    }

    /// Full URL the perception tiers post to.
    pub fn perception_url(&self) -> String {
        format!(
            "{}{}",
            self.perception.endpoint.trim_end_matches('/'),
            &self.perception.api_path
        )
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
    fn dayflow_default_round_trips_through_toml() {
        let cfg = DayflowConfig::default();
        let text = toml::to_string(&cfg).expect("serialize");
        let back: DayflowConfig = toml::from_str(&text).expect("deserialize");
        assert_eq!(back.segment_seconds, cfg.segment_seconds);
        assert_eq!(back.perception.api_path, cfg.perception.api_path);
        assert_eq!(back.perception.text_model, cfg.perception.text_model);
        assert_eq!(back.idle.threshold_seconds, cfg.idle.threshold_seconds);
        assert_eq!(back.displays, cfg.displays);
    }

    #[test]
    fn legacy_config_with_only_chunk_minutes_still_parses() {
        // A config file written before `segment_seconds` existed must keep working
        // and must keep meaning what its author intended (FR-035 back-compat).
        let cfg: DayflowConfig =
            toml::from_str("chunk_minutes = 30\nsegment_seconds = 0\n").expect("legacy parse");
        assert_eq!(cfg.chunk_minutes, 30);
        assert_eq!(cfg.segment_duration().as_secs(), 30 * 60);
    }

    #[test]
    fn segment_seconds_wins_over_legacy_chunk_minutes() {
        let cfg: DayflowConfig =
            toml::from_str("chunk_minutes = 15\nsegment_seconds = 1800\n").expect("parse");
        assert_eq!(cfg.segment_duration().as_secs(), 1800);
    }

    #[test]
    fn perception_uses_generate_not_chat() {
        // Measured 2026-08-23: the text tier is an OCR specialist, not a chat
        // model. /api/chat wraps the prompt in a chat template and the template
        // bleeds into the output as >user / >system / <|im_end|> markers.
        let cfg = DayflowConfig::default();
        assert_eq!(cfg.perception.api_path, "/api/generate");
        assert!(!cfg.perception.api_path.contains("chat"));
        assert!(cfg.perception_url().ends_with("/api/generate"));
    }

    #[test]
    fn text_prompt_stays_terse() {
        // A verbose instruction does not degrade this model, it destroys it:
        // 42.9s and 7366 tokens of a degenerate repetition loop, returned with a
        // 200 and no error. Guard the pinned prompt against well-meaning edits.
        let cfg = DayflowConfig::default();
        assert!(
            cfg.perception.text_prompt.len() < 40,
            "text prompt must stay terse, got {} chars: {:?}",
            cfg.perception.text_prompt.len(),
            cfg.perception.text_prompt
        );
        assert!(!cfg.perception.text_prompt.to_lowercase().contains("do not"));
    }

    #[test]
    fn perception_endpoint_leaks_no_private_host() {
        // This repository is public. The governed-lane host is machine-local and
        // must never be committed (see the crate's dependency/infra hygiene).
        let cfg = DayflowConfig::default();
        let ep = &cfg.perception.endpoint;
        assert!(
            ep.contains("127.0.0.1") || ep.contains("localhost"),
            "default perception endpoint must be neutral loopback, got {ep:?}"
        );
        for leak in ["192.168.", "10.", "172.16.", ".local"] {
            assert!(!ep.contains(leak), "endpoint leaks a private host: {ep:?}");
        }
    }

    #[test]
    fn dayflow_captures_all_displays_by_default() {
        assert_eq!(DayflowConfig::default().displays, DisplaySelection::All);
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
