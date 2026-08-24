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

fn default_day_interval_seconds() -> u32 {
    180 // one frame every 3 minutes — all-day tracking is the coarse one
}

fn default_focused_interval_seconds() -> u32 {
    60 // one frame a minute — a bounded, focused ask
}

fn default_skip_unchanged() -> bool {
    true
}

fn default_video_enabled() -> bool {
    false // gentle-eye already records video; dayflow's artifact is the timeline
}

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

/// How often dayflow SAMPLES a frame, per tracking granularity.
///
/// # Dayflow samples; it does not record video
///
/// This is the distinction that governs the feature's cost. gentle-eye already
/// has real-time video recording; dayflow exists to **track what a user was
/// doing**, and it does that by taking periodic snapshots — not by streaming an
/// encoder for eight hours. Sampling once a minute instead of at 0.5 fps is
/// thirty times less work, and on an idle screen the delta check
/// ([`SamplingConfig::skip_unchanged`]) drives it toward zero.
///
/// # Two granularities
///
/// | mode | intent | default |
/// |---|---|---|
/// | [`DayflowMode::Daemon`] | all-day background tracking — generalized, fast, cheap | one frame every **3 minutes** |
/// | [`DayflowMode::Session`] | a bounded, focused ask — "track my dev work for this hour" | one frame every **minute** |
///
/// All-day is deliberately the coarser of the two: it runs unattended for hours,
/// so its interval is what decides whether the feature is cheap or wasteful.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SamplingConfig {
    /// Seconds between samples during all-day (daemon) tracking.
    #[serde(default = "default_day_interval_seconds")]
    pub day_interval_seconds: u32,
    /// Seconds between samples during a bounded, focused session.
    #[serde(default = "default_focused_interval_seconds")]
    pub focused_interval_seconds: u32,
    /// Skip perception for a sample whose regions are unchanged from the previous
    /// one. On a static screen this collapses steady-state cost toward zero — the
    /// single largest saving available, because reading is most of a working day.
    #[serde(default = "default_skip_unchanged")]
    pub skip_unchanged: bool,
}

impl Default for SamplingConfig {
    fn default() -> Self {
        Self {
            day_interval_seconds: default_day_interval_seconds(),
            focused_interval_seconds: default_focused_interval_seconds(),
            skip_unchanged: default_skip_unchanged(),
        }
    }
}

impl SamplingConfig {
    /// Never sample faster than once every 10 s, even in focused mode. Below this
    /// dayflow stops being an activity tracker and becomes the video recorder it
    /// is explicitly not.
    pub const MIN_INTERVAL_SECONDS: u32 = 10;
    /// Never coarser than once an hour, or a segment can contain no samples.
    pub const MAX_INTERVAL_SECONDS: u32 = 3600;

    /// The sampling interval for a given mode.
    pub fn interval_for(&self, mode: crate::dayflow::models::DayflowMode) -> std::time::Duration {
        use crate::dayflow::models::DayflowMode;
        let secs = match mode {
            DayflowMode::Daemon => self.day_interval_seconds,
            DayflowMode::Session => self.focused_interval_seconds,
        };
        std::time::Duration::from_secs(u64::from(secs))
    }
}

/// Optional video output.
///
/// **Off by default, and that is the point.** gentle-eye already provides video
/// recording as its own feature; dayflow's artifact is the timeline. When
/// enabled, sampled frames are assembled into a timelapse at window close as a
/// convenience for human review — never as an input to perception, which reads
/// frames directly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DayflowVideoConfig {
    /// Assemble sampled frames into a timelapse artifact per window.
    #[serde(default = "default_video_enabled")]
    pub enabled: bool,
}

impl Default for DayflowVideoConfig {
    fn default() -> Self {
        Self { enabled: default_video_enabled() }
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
    /// Intended operating range is **10–15 minutes**; the default is 900 s
    /// (15 min). Permitted range is [`DayflowConfig::MIN_SEGMENT_SECONDS`]
    /// (5 min) to [`DayflowConfig::MAX_SEGMENT_SECONDS`] (1 h), enforced by
    /// `AppConfig::validate`. Changeable mid-day: a change takes effect at the
    /// next boundary and never re-times an existing entry (FR-035).
    ///
    /// Stored in seconds so the value is exact and so 10 vs 15 minutes is a
    /// plain number, NOT so that second-scale intervals are usable — those are
    /// rejected by validation.
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
    /// How often a frame is SAMPLED, per tracking granularity. Dayflow samples;
    /// it does not stream video.
    #[serde(default)]
    pub sampling: SamplingConfig,
    /// Optional timelapse output. Off by default — the timeline is the artifact.
    #[serde(default)]
    pub video: DayflowVideoConfig,
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
            sampling: SamplingConfig::default(),
            video: DayflowVideoConfig::default(),
            displays: DisplaySelection::default(),
            idle: IdleConfig::default(),
            perception: PerceptionConfig::default(),
            retention: RetentionConfig::default(),
        }
    }
}

impl DayflowConfig {
    /// Hard floor on a segment: **5 minutes**.
    ///
    /// Dayflow is an all-day recorder, not a frame grabber. Below this the
    /// per-segment perception cost (one pass per region per display) cannot keep
    /// up with the cadence, the timeline fills with fragments too short to
    /// describe an activity, and the segment count per day becomes unmanageable.
    /// Tests that need sub-minimum intervals drive ffmpeg directly rather than
    /// going through a validated config.
    pub const MIN_SEGMENT_SECONDS: u32 = 300;

    /// The segment floor and the sampling interval INTERACT: a segment must be
    /// able to hold at least two samples, so the 5-minute floor is only reachable
    /// with a sampling interval of 150 s or finer. The default 3-minute all-day
    /// interval implies a segment of at least 6 minutes. `validate` enforces it.
    ///
    /// Sanity ceiling: **1 hour**.
    ///
    /// A longer interval delays BOTH the first timeline entry and the first
    /// liveness signal — degraded detection is defined in segment intervals
    /// (SC-006), so an interval this long already means an hour of silence
    /// before a fault is visible.
    pub const MAX_SEGMENT_SECONDS: u32 = 3600;

    /// The intended operating range: **10 to 15 minutes**.
    ///
    /// Not enforced — 5 minutes to 1 hour is permitted — but this is the band
    /// the design is tuned for and the default sits at its top.
    pub const RECOMMENDED_SEGMENT_SECONDS: std::ops::RangeInclusive<u32> = 600..=900;

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

    /// Validate the segment interval. **Dayflow-scoped on purpose.**
    ///
    /// This is NOT called from `AppConfig::validate`, and must not be. gentle-eye
    /// is a general screen-recording library whose core use is real-time and
    /// short-clip capture at 1–30 fps; the 5-minute floor is a property of the
    /// dayflow FEATURE, not of the library. Wiring it into the library-wide
    /// validator would let a stale `dayflow.*` value fail the config load for a
    /// user who is only recording a ten-second clip.
    ///
    /// Call this when a dayflow session or daemon STARTS — the one moment the
    /// interval actually has to make sense.
    pub fn validate(&self) -> Result<(), ConfigError> {
        let seg = self.segment_duration().as_secs();
        if seg < u64::from(Self::MIN_SEGMENT_SECONDS) || seg > u64::from(Self::MAX_SEGMENT_SECONDS) {
            return Err(ConfigError::ValueOutOfRange {
                field: "dayflow.segment_seconds".to_string(),
                value: seg.to_string(),
                min: Self::MIN_SEGMENT_SECONDS.to_string(),
                max: Self::MAX_SEGMENT_SECONDS.to_string(),
            });
        }

        for (field, secs) in [
            ("dayflow.sampling.day_interval_seconds", self.sampling.day_interval_seconds),
            ("dayflow.sampling.focused_interval_seconds", self.sampling.focused_interval_seconds),
        ] {
            if secs < SamplingConfig::MIN_INTERVAL_SECONDS
                || secs > SamplingConfig::MAX_INTERVAL_SECONDS
            {
                return Err(ConfigError::ValueOutOfRange {
                    field: field.to_string(),
                    value: secs.to_string(),
                    min: SamplingConfig::MIN_INTERVAL_SECONDS.to_string(),
                    max: SamplingConfig::MAX_INTERVAL_SECONDS.to_string(),
                });
            }
        }

        // All-day tracking must never sample FINER than a focused session — that
        // inversion is how an unattended recorder quietly becomes the expensive
        // one, which is the whole thing this design avoids.
        if self.sampling.day_interval_seconds < self.sampling.focused_interval_seconds {
            return Err(ConfigError::Invalid(format!(
                "all-day sampling ({}s) must not be finer than focused sampling ({}s) — \
                 the unattended mode has to be the cheap one",
                self.sampling.day_interval_seconds, self.sampling.focused_interval_seconds
            )));
        }

        // A segment must be able to contain at least two samples, or it cannot
        // show change and the timeline entry has nothing to describe.
        let seg = self.segment_duration().as_secs();
        let coarsest = u64::from(self.sampling.day_interval_seconds);
        if seg < coarsest * 2 {
            return Err(ConfigError::Invalid(format!(
                "a {seg}s segment cannot hold two samples at a {coarsest}s interval — \
                 widen the segment or sample more often"
            )));
        }

        Ok(())
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
    fn dayflow_samples_it_does_not_stream_video() {
        let d = DayflowConfig::default();
        // video is OFF: gentle-eye already records video; dayflow's artifact is
        // the timeline. Flipping this default silently reintroduces the cost.
        assert!(!d.video.enabled, "dayflow video must default to OFF");
        // all-day is the COARSE one; focused is the fine one
        assert_eq!(d.sampling.day_interval_seconds, 180);
        assert_eq!(d.sampling.focused_interval_seconds, 60);
        assert!(d.sampling.skip_unchanged, "delta-skip is the largest saving; default on");
    }

    #[test]
    fn sampling_interval_follows_the_record_mode() {
        use crate::dayflow::models::DayflowMode;
        let d = DayflowConfig::default();
        assert_eq!(d.sampling.interval_for(DayflowMode::Daemon).as_secs(), 180);
        assert_eq!(d.sampling.interval_for(DayflowMode::Session).as_secs(), 60);
        assert!(
            d.sampling.interval_for(DayflowMode::Daemon)
                > d.sampling.interval_for(DayflowMode::Session),
            "unattended all-day tracking must be the cheaper of the two"
        );
    }

    #[test]
    fn all_day_sampling_may_not_be_finer_than_focused() {
        // The inversion that would make the unattended mode the expensive one.
        let mut d = DayflowConfig::default();
        d.sampling.day_interval_seconds = 30;
        d.sampling.focused_interval_seconds = 60;
        assert!(d.validate().is_err(), "all-day finer than focused must be rejected");
    }

    #[test]
    fn sampling_may_not_become_a_video_recorder() {
        let mut d = DayflowConfig::default();
        for too_fast in [1, 2, 5, 9] {
            d.sampling.focused_interval_seconds = too_fast;
            assert!(
                d.validate().is_err(),
                "a {too_fast}s sampling interval is video recording, not activity tracking"
            );
        }
        d.sampling.focused_interval_seconds = SamplingConfig::MIN_INTERVAL_SECONDS;
        assert!(d.validate().is_ok(), "exactly the floor is allowed");
    }

    #[test]
    fn a_segment_must_be_able_to_hold_two_samples() {
        // One sample per segment cannot show change, so the entry has nothing to
        // describe; zero samples is a silently empty timeline.
        let mut d = DayflowConfig::default();
        d.segment_seconds = 300; // 5 min, the floor
        d.sampling.day_interval_seconds = 180; // 3 min -> only 1 fits
        assert!(d.validate().is_err(), "5min segment cannot hold two 3min samples");
        d.sampling.day_interval_seconds = 150; // 2.5 min -> exactly 2 fit
        assert!(d.validate().is_ok());
    }

    #[test]
    fn default_sampling_is_far_cheaper_than_continuous_capture() {
        // The measured cost driver. One frame across this machine's three
        // displays is ~37.4 MiB of raw BGRA (T006), so the sample COUNT is what
        // decides whether an 8-hour day is affordable.
        let d = DayflowConfig::default();
        let workday_secs = 8 * 60 * 60;
        let samples = workday_secs / d.sampling.day_interval_seconds; // per display
        let continuous_at_half_fps = workday_secs / 2; // 0.5 fps for comparison
        assert_eq!(samples, 160, "8h at a 3min interval is 160 samples per display");
        assert!(
            continuous_at_half_fps / samples >= 80,
            "default sampling must be at least 80x cheaper than 0.5fps continuous              capture; got {}x",
            continuous_at_half_fps / samples
        );
    }

    #[test]
    fn dayflow_interval_must_not_gate_the_whole_library() {
        // gentle-eye's core use is real-time / short-clip recording at 1-30 fps.
        // The dayflow 5-minute floor is a FEATURE constraint and must never be
        // able to fail the library-wide config load: a user recording a ten
        // second clip should not be blocked by a stale dayflow value.
        let mut cfg = AppConfig::default();
        cfg.recording.fps = 30; // real-time capture
        cfg.recording.max_duration_seconds = 10; // a short clip
        cfg.dayflow.segment_seconds = 1; // nonsense FOR DAYFLOW
        cfg.dayflow.chunk_minutes = 0;
        assert!(
            cfg.validate().is_ok(),
            "a nonsense dayflow interval must NOT block a real-time recording config"
        );
        // ...while the dayflow-scoped validator still rejects it.
        assert!(
            cfg.dayflow.validate().is_err(),
            "the dayflow-scoped validator must still enforce its own floor"
        );
    }

    #[test]
    fn segment_interval_floor_is_five_minutes() {
        // Dayflow is an all-day recorder. A second-scale "segment" is not a
        // small config choice, it is a different product.
        let mut cfg = DayflowConfig::default();
        // Sample finely enough that the two-samples-per-segment rule is not what
        // is under test here — this test is about the segment floor alone. The
        // interaction between the two knobs has its own test.
        cfg.sampling.day_interval_seconds = 60;
        cfg.sampling.focused_interval_seconds = 60;
        for bad in [1, 30, 60, 299] {
            cfg.segment_seconds = bad;
            assert!(
                cfg.validate().is_err(),
                "{bad}s segment must be rejected (floor is {}s)",
                DayflowConfig::MIN_SEGMENT_SECONDS
            );
        }
        cfg.segment_seconds = DayflowConfig::MIN_SEGMENT_SECONDS;
        assert!(
            cfg.validate().is_ok(),
            "exactly 5 minutes must be accepted when sampling fits inside it"
        );
    }

    #[test]
    fn segment_interval_accepts_the_intended_ten_to_fifteen_minutes() {
        let mut cfg = DayflowConfig::default();
        for good in [600, 720, 900] {
            cfg.segment_seconds = good;
            assert!(cfg.validate().is_ok(), "{good}s is in the intended range");
        }
        assert!(DayflowConfig::RECOMMENDED_SEGMENT_SECONDS.contains(&600));
        assert!(DayflowConfig::RECOMMENDED_SEGMENT_SECONDS.contains(&900));
        // the default sits at the top of the intended band
        assert_eq!(AppConfig::default().dayflow.segment_seconds, 900);
    }

    #[test]
    fn segment_interval_rejects_an_absurdly_long_one() {
        let mut cfg = DayflowConfig::default();
        cfg.segment_seconds = DayflowConfig::MAX_SEGMENT_SECONDS + 1;
        assert!(cfg.validate().is_err(), "beyond 1h must be rejected");
    }

    #[test]
    fn legacy_chunk_minutes_cannot_bypass_the_floor() {
        // Validation reads the EFFECTIVE duration, so an old config file that
        // only sets chunk_minutes is held to the same floor as a new one.
        let mut cfg = DayflowConfig::default();
        cfg.segment_seconds = 0; // defer to the legacy field
        cfg.chunk_minutes = 1; // 60s — below the floor
        assert!(
            cfg.validate().is_err(),
            "a 1-minute legacy interval must be rejected, not silently honoured"
        );
        cfg.chunk_minutes = 10;
        assert!(cfg.validate().is_ok(), "a 10-minute legacy interval is fine");
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
