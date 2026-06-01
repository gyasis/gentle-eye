//! Configuration data models for recording and vision analysis.
//!
//! This module contains configuration structures for controlling
//! recording behavior and vision AI provider settings.
//!
//! # Example
//!
//! ```
//! use gentle_eye::models::config::{RecordingConfig, VisionConfig};
//! use gentle_eye::models::EncoderMode;
//! use std::path::PathBuf;
//!
//! // Create a custom recording configuration
//! let recording_config = RecordingConfig {
//!     fps: 2,
//!     output_dir: Some(PathBuf::from("/tmp/recordings")),
//!     encoder_mode: Some(EncoderMode::FileBased),
//!     display_id: None,
//!     max_duration_seconds: 3600,
//! };
//!
//! // Use default vision configuration
//! let vision_config = VisionConfig::default();
//! assert_eq!(vision_config.provider, "gemini");
//! ```

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::EncoderMode;

/// Default frames per second for recording.
fn default_fps() -> u32 {
    1
}

/// Default maximum recording duration in seconds (30 minutes).
fn default_max_duration() -> u64 {
    1800
}

/// Configuration for screen recording operations.
///
/// Controls the behavior of screen capture including frame rate,
/// output location, encoding mode, and duration limits.
///
/// # Defaults
///
/// * `fps` - 1 frame per second (suitable for low-motion capture)
/// * `output_dir` - System temp directory
/// * `encoder_mode` - None (system default)
/// * `display_id` - None (primary display)
/// * `max_duration_seconds` - 1800 (30 minutes)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingConfig {
    /// Frames per second for the recording.
    /// Lower values reduce file size, higher values capture more detail.
    /// Default: 1 FPS (suitable for documentation capture).
    #[serde(default = "default_fps")]
    pub fps: u32,

    /// Directory where recordings will be saved.
    /// If `None`, uses the system temp directory.
    pub output_dir: Option<PathBuf>,

    /// Encoding mode to use for the recording.
    /// If `None`, uses the system default (FileBased).
    pub encoder_mode: Option<EncoderMode>,

    /// ID of the display to capture.
    /// If `None`, captures the primary display.
    pub display_id: Option<u32>,

    /// Maximum duration for a recording in seconds.
    /// Recording will auto-stop when this limit is reached.
    /// Default: 1800 seconds (30 minutes).
    #[serde(default = "default_max_duration")]
    pub max_duration_seconds: u64,
}

impl Default for RecordingConfig {
    fn default() -> Self {
        Self {
            fps: 1,
            output_dir: None,
            encoder_mode: None,
            display_id: None,
            max_duration_seconds: 1800,
        }
    }
}

impl RecordingConfig {
    /// Creates a new RecordingConfig with default values.
    ///
    /// # Example
    ///
    /// ```
    /// use gentle_eye::models::config::RecordingConfig;
    ///
    /// let config = RecordingConfig::new();
    /// assert_eq!(config.fps, 1);
    /// assert_eq!(config.max_duration_seconds, 1800);
    /// ```
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a RecordingConfig with custom FPS.
    ///
    /// # Arguments
    ///
    /// * `fps` - Frames per second for the recording
    ///
    /// # Example
    ///
    /// ```
    /// use gentle_eye::models::config::RecordingConfig;
    ///
    /// let config = RecordingConfig::with_fps(10);
    /// assert_eq!(config.fps, 10);
    /// ```
    pub fn with_fps(fps: u32) -> Self {
        Self {
            fps,
            ..Default::default()
        }
    }

    /// Sets the output directory for recordings.
    ///
    /// # Arguments
    ///
    /// * `dir` - Path to the output directory
    pub fn output_dir(mut self, dir: PathBuf) -> Self {
        self.output_dir = Some(dir);
        self
    }

    /// Sets the encoder mode for recordings.
    ///
    /// # Arguments
    ///
    /// * `mode` - The encoder mode to use
    pub fn encoder_mode(mut self, mode: EncoderMode) -> Self {
        self.encoder_mode = Some(mode);
        self
    }

    /// Sets the display ID to capture.
    ///
    /// # Arguments
    ///
    /// * `id` - The display ID to capture
    pub fn display_id(mut self, id: u32) -> Self {
        self.display_id = Some(id);
        self
    }

    /// Sets the maximum recording duration in seconds.
    ///
    /// # Arguments
    ///
    /// * `seconds` - Maximum duration in seconds
    pub fn max_duration(mut self, seconds: u64) -> Self {
        self.max_duration_seconds = seconds;
        self
    }

    /// Returns the effective encoder mode.
    ///
    /// If no encoder mode is specified, returns the default (FileBased).
    pub fn effective_encoder_mode(&self) -> EncoderMode {
        self.encoder_mode.unwrap_or_default()
    }

    /// Validates the configuration.
    ///
    /// # Returns
    ///
    /// `Ok(())` if configuration is valid, `Err` with description otherwise.
    ///
    /// # Validation Rules
    ///
    /// * `fps` must be between 1 and 60
    /// * `max_duration_seconds` must be between 1 and 86400 (24 hours)
    pub fn validate(&self) -> Result<(), String> {
        if self.fps < 1 || self.fps > 60 {
            return Err(format!(
                "fps must be between 1 and 60, got {}",
                self.fps
            ));
        }

        if self.max_duration_seconds < 1 || self.max_duration_seconds > 86400 {
            return Err(format!(
                "max_duration_seconds must be between 1 and 86400, got {}",
                self.max_duration_seconds
            ));
        }

        Ok(())
    }
}

/// Default vision provider name.
fn default_provider() -> String {
    "gemini".into()
}

/// Default Gemini model name.
fn default_gemini_model() -> String {
    "gemini-3.5-flash".into()
}

/// Default Ollama host address.
fn default_ollama_host() -> String {
    "localhost".into()
}

/// Default Ollama port number.
fn default_ollama_port() -> u16 {
    11434
}

/// Default Ollama model name.
fn default_ollama_model() -> String {
    "llava".into()
}

/// Default request timeout in seconds.
fn default_timeout() -> u64 {
    60
}

/// Configuration for vision AI providers.
///
/// Supports multiple vision AI backends including cloud-based (Gemini)
/// and local (Ollama) providers for video analysis.
///
/// # Defaults
///
/// * `provider` - "gemini"
/// * `gemini_model` - "gemini-2.0-flash"
/// * `ollama_host` - "localhost"
/// * `ollama_port` - 11434
/// * `ollama_model` - "llava"
/// * `timeout_seconds` - 60
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisionConfig {
    /// The vision provider to use ("gemini" or "ollama").
    #[serde(default = "default_provider")]
    pub provider: String,

    /// API key for Gemini provider.
    /// Required when using the Gemini provider.
    pub gemini_api_key: Option<String>,

    /// Model name for Gemini API.
    /// Default: "gemini-2.0-flash"
    #[serde(default = "default_gemini_model")]
    pub gemini_model: String,

    /// Host address for Ollama server.
    /// Default: "localhost"
    #[serde(default = "default_ollama_host")]
    pub ollama_host: String,

    /// Port number for Ollama server.
    /// Default: 11434
    #[serde(default = "default_ollama_port")]
    pub ollama_port: u16,

    /// Model name for Ollama.
    /// Default: "llava"
    #[serde(default = "default_ollama_model")]
    pub ollama_model: String,

    /// Request timeout in seconds.
    /// Default: 60 seconds
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

impl VisionConfig {
    /// Creates a new VisionConfig with default values.
    ///
    /// # Example
    ///
    /// ```
    /// use gentle_eye::models::config::VisionConfig;
    ///
    /// let config = VisionConfig::new();
    /// assert_eq!(config.provider, "gemini");
    /// ```
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a VisionConfig for Gemini provider.
    ///
    /// # Arguments
    ///
    /// * `api_key` - The Gemini API key
    ///
    /// # Example
    ///
    /// ```
    /// use gentle_eye::models::config::VisionConfig;
    ///
    /// let config = VisionConfig::gemini("my-api-key".to_string());
    /// assert_eq!(config.provider, "gemini");
    /// assert!(config.gemini_api_key.is_some());
    /// ```
    pub fn gemini(api_key: String) -> Self {
        Self {
            provider: "gemini".into(),
            gemini_api_key: Some(api_key),
            ..Default::default()
        }
    }

    /// Creates a VisionConfig for Ollama provider.
    ///
    /// # Example
    ///
    /// ```
    /// use gentle_eye::models::config::VisionConfig;
    ///
    /// let config = VisionConfig::ollama();
    /// assert_eq!(config.provider, "ollama");
    /// ```
    pub fn ollama() -> Self {
        Self {
            provider: "ollama".into(),
            ..Default::default()
        }
    }

    /// Creates a VisionConfig for Ollama with custom host and port.
    ///
    /// # Arguments
    ///
    /// * `host` - The Ollama server host
    /// * `port` - The Ollama server port
    ///
    /// # Example
    ///
    /// ```
    /// use gentle_eye::models::config::VisionConfig;
    ///
    /// let config = VisionConfig::ollama_at("192.168.1.100".to_string(), 11434);
    /// assert_eq!(config.ollama_host, "192.168.1.100");
    /// ```
    pub fn ollama_at(host: String, port: u16) -> Self {
        Self {
            provider: "ollama".into(),
            ollama_host: host,
            ollama_port: port,
            ..Default::default()
        }
    }

    /// Sets the Gemini model.
    ///
    /// # Arguments
    ///
    /// * `model` - The model name to use
    pub fn with_gemini_model(mut self, model: String) -> Self {
        self.gemini_model = model;
        self
    }

    /// Sets the Ollama model.
    ///
    /// # Arguments
    ///
    /// * `model` - The model name to use
    pub fn with_ollama_model(mut self, model: String) -> Self {
        self.ollama_model = model;
        self
    }

    /// Sets the request timeout.
    ///
    /// # Arguments
    ///
    /// * `seconds` - Timeout in seconds
    pub fn with_timeout(mut self, seconds: u64) -> Self {
        self.timeout_seconds = seconds;
        self
    }

    /// Returns true if the Gemini provider is selected.
    pub fn is_gemini(&self) -> bool {
        self.provider.eq_ignore_ascii_case("gemini")
    }

    /// Returns true if the Ollama provider is selected.
    pub fn is_ollama(&self) -> bool {
        self.provider.eq_ignore_ascii_case("ollama")
    }

    /// Returns the Ollama endpoint URL.
    ///
    /// # Example
    ///
    /// ```
    /// use gentle_eye::models::config::VisionConfig;
    ///
    /// let config = VisionConfig::ollama();
    /// assert_eq!(config.ollama_endpoint(), "http://localhost:11434");
    /// ```
    pub fn ollama_endpoint(&self) -> String {
        // Robust against a host that already carries a scheme and/or port (a
        // misconfig that previously produced `http://http://host:11434:11434`).
        // Normalize to a bare host, then prefix scheme + port exactly once.
        let host = self
            .ollama_host
            .trim()
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_end_matches('/');
        let host = host.split(':').next().unwrap_or(host);
        format!("http://{}:{}", host, self.ollama_port)
    }

    /// Returns the current model name based on the selected provider.
    pub fn current_model(&self) -> &str {
        if self.is_gemini() {
            &self.gemini_model
        } else {
            &self.ollama_model
        }
    }

    /// Validates the configuration.
    ///
    /// # Returns
    ///
    /// `Ok(())` if configuration is valid, `Err` with description otherwise.
    ///
    /// # Validation Rules
    ///
    /// * Provider must be "gemini" or "ollama"
    /// * Gemini provider requires an API key
    /// * Timeout must be between 1 and 3600 seconds
    /// * Ollama port must be non-zero
    pub fn validate(&self) -> Result<(), String> {
        if !self.is_gemini() && !self.is_ollama() {
            return Err(format!(
                "provider must be 'gemini' or 'ollama', got '{}'",
                self.provider
            ));
        }

        if self.is_gemini() && self.gemini_api_key.is_none() {
            return Err("gemini provider requires gemini_api_key".to_string());
        }

        if self.timeout_seconds < 1 || self.timeout_seconds > 3600 {
            return Err(format!(
                "timeout_seconds must be between 1 and 3600, got {}",
                self.timeout_seconds
            ));
        }

        if self.ollama_port == 0 {
            return Err("ollama_port must be non-zero".to_string());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // RecordingConfig tests

    #[test]
    fn test_recording_config_default() {
        let config = RecordingConfig::default();

        assert_eq!(config.fps, 1);
        assert!(config.output_dir.is_none());
        assert!(config.encoder_mode.is_none());
        assert!(config.display_id.is_none());
        assert_eq!(config.max_duration_seconds, 1800);
    }

    #[test]
    fn test_recording_config_with_fps() {
        let config = RecordingConfig::with_fps(10);
        assert_eq!(config.fps, 10);
    }

    #[test]
    fn test_recording_config_builder() {
        let config = RecordingConfig::new()
            .output_dir(PathBuf::from("/tmp/recordings"))
            .encoder_mode(EncoderMode::InMemoryPipe)
            .display_id(1)
            .max_duration(3600);

        assert_eq!(config.output_dir, Some(PathBuf::from("/tmp/recordings")));
        assert_eq!(config.encoder_mode, Some(EncoderMode::InMemoryPipe));
        assert_eq!(config.display_id, Some(1));
        assert_eq!(config.max_duration_seconds, 3600);
    }

    #[test]
    fn test_recording_config_effective_encoder_mode() {
        let config = RecordingConfig::default();
        assert_eq!(config.effective_encoder_mode(), EncoderMode::FileBased);

        let config = RecordingConfig::new().encoder_mode(EncoderMode::InMemoryPipe);
        assert_eq!(config.effective_encoder_mode(), EncoderMode::InMemoryPipe);
    }

    #[test]
    fn test_recording_config_validate_valid() {
        let config = RecordingConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_recording_config_validate_fps_too_low() {
        let mut config = RecordingConfig::default();
        config.fps = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_recording_config_validate_fps_too_high() {
        let mut config = RecordingConfig::default();
        config.fps = 61;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_recording_config_validate_duration_too_low() {
        let mut config = RecordingConfig::default();
        config.max_duration_seconds = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_recording_config_validate_duration_too_high() {
        let mut config = RecordingConfig::default();
        config.max_duration_seconds = 86401;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_recording_config_serialization() {
        let config = RecordingConfig::with_fps(5)
            .output_dir(PathBuf::from("/tmp"))
            .max_duration(600);

        let json = serde_json::to_string(&config).expect("serialization failed");
        let deserialized: RecordingConfig =
            serde_json::from_str(&json).expect("deserialization failed");

        assert_eq!(config.fps, deserialized.fps);
        assert_eq!(config.output_dir, deserialized.output_dir);
        assert_eq!(config.max_duration_seconds, deserialized.max_duration_seconds);
    }

    // VisionConfig tests

    #[test]
    fn test_vision_config_default() {
        let config = VisionConfig::default();

        assert_eq!(config.provider, "gemini");
        assert!(config.gemini_api_key.is_none());
        assert_eq!(config.gemini_model, "gemini-2.0-flash");
        assert_eq!(config.ollama_host, "localhost");
        assert_eq!(config.ollama_port, 11434);
        assert_eq!(config.ollama_model, "llava");
        assert_eq!(config.timeout_seconds, 60);
    }

    #[test]
    fn test_vision_config_gemini() {
        let config = VisionConfig::gemini("test-key".to_string());

        assert_eq!(config.provider, "gemini");
        assert_eq!(config.gemini_api_key, Some("test-key".to_string()));
    }

    #[test]
    fn test_vision_config_ollama() {
        let config = VisionConfig::ollama();
        assert_eq!(config.provider, "ollama");
    }

    #[test]
    fn test_vision_config_ollama_at() {
        let config = VisionConfig::ollama_at("192.168.1.100".to_string(), 8080);

        assert_eq!(config.provider, "ollama");
        assert_eq!(config.ollama_host, "192.168.1.100");
        assert_eq!(config.ollama_port, 8080);
    }

    #[test]
    fn test_vision_config_builder() {
        let config = VisionConfig::gemini("test-key".to_string())
            .with_gemini_model("gemini-pro".to_string())
            .with_timeout(120);

        assert_eq!(config.gemini_model, "gemini-pro");
        assert_eq!(config.timeout_seconds, 120);
    }

    #[test]
    fn test_vision_config_is_gemini() {
        let config = VisionConfig::gemini("test-key".to_string());
        assert!(config.is_gemini());
        assert!(!config.is_ollama());
    }

    #[test]
    fn test_vision_config_is_ollama() {
        let config = VisionConfig::ollama();
        assert!(config.is_ollama());
        assert!(!config.is_gemini());
    }

    #[test]
    fn test_vision_config_ollama_endpoint() {
        let config = VisionConfig::ollama();
        assert_eq!(config.ollama_endpoint(), "http://localhost:11434");

        let config = VisionConfig::ollama_at("192.168.1.100".to_string(), 8080);
        assert_eq!(config.ollama_endpoint(), "http://192.168.1.100:8080");
    }

    #[test]
    fn test_vision_config_current_model() {
        let config = VisionConfig::gemini("test-key".to_string());
        assert_eq!(config.current_model(), "gemini-2.0-flash");

        let config = VisionConfig::ollama();
        assert_eq!(config.current_model(), "llava");
    }

    #[test]
    fn test_vision_config_validate_valid_gemini() {
        let config = VisionConfig::gemini("test-key".to_string());
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_vision_config_validate_valid_ollama() {
        let config = VisionConfig::ollama();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_vision_config_validate_invalid_provider() {
        let mut config = VisionConfig::default();
        config.provider = "invalid".to_string();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_vision_config_validate_gemini_no_key() {
        let config = VisionConfig::default(); // gemini without key
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_vision_config_validate_timeout_too_low() {
        let mut config = VisionConfig::ollama();
        config.timeout_seconds = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_vision_config_validate_timeout_too_high() {
        let mut config = VisionConfig::ollama();
        config.timeout_seconds = 3601;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_vision_config_validate_ollama_port_zero() {
        let mut config = VisionConfig::ollama();
        config.ollama_port = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_vision_config_serialization() {
        let config = VisionConfig::gemini("test-key".to_string())
            .with_gemini_model("gemini-pro".to_string());

        let json = serde_json::to_string(&config).expect("serialization failed");
        let deserialized: VisionConfig =
            serde_json::from_str(&json).expect("deserialization failed");

        assert_eq!(config.provider, deserialized.provider);
        assert_eq!(config.gemini_model, deserialized.gemini_model);
    }

    #[test]
    fn test_vision_config_deserialization_with_defaults() {
        let json = r#"{"provider": "ollama"}"#;
        let config: VisionConfig = serde_json::from_str(json).expect("deserialization failed");

        assert_eq!(config.provider, "ollama");
        assert_eq!(config.gemini_model, "gemini-2.0-flash"); // default
        assert_eq!(config.ollama_host, "localhost"); // default
        assert_eq!(config.ollama_port, 11434); // default
        assert_eq!(config.timeout_seconds, 60); // default
    }
}
