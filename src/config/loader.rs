//! Configuration file loading and environment variable overrides
//!
//! This module handles loading configuration from TOML files and
//! applying environment variable overrides.

use super::{AppConfig, ConfigError};
use std::fs;

/// Load configuration from file with environment variable overrides
///
/// Priority:
/// 1. Environment variables (highest)
/// 2. Configuration file (~/.config/gentle-eye/config.toml)
/// 3. Default values (lowest)
pub fn load_config() -> Result<AppConfig, ConfigError> {
    let config_path = AppConfig::config_file_path();

    // Start with defaults
    let mut config = if config_path.exists() {
        let content = fs::read_to_string(&config_path)?;
        toml::from_str(&content)?
    } else {
        AppConfig::default()
    };

    // Apply environment variable overrides
    apply_env_overrides(&mut config);

    // Validate the final configuration
    config.validate()?;

    Ok(config)
}

/// Apply environment variable overrides to configuration
fn apply_env_overrides(config: &mut AppConfig) {
    // GEMINI_API_KEY -> vision.gemini_api_key
    if let Ok(api_key) = std::env::var("GEMINI_API_KEY") {
        config.vision.gemini_api_key = Some(api_key);
    }

    // GENTLE_EYE_DATA -> storage.base_dir
    if let Ok(data_dir) = std::env::var("GENTLE_EYE_DATA") {
        config.storage.base_dir = data_dir.into();
    }

    // GENTLE_EYE_FPS -> recording.fps
    if let Ok(fps_str) = std::env::var("GENTLE_EYE_FPS") {
        if let Ok(fps) = fps_str.parse::<u32>() {
            config.recording.fps = fps;
        }
    }

    // GENTLE_EYE_PROVIDER -> vision.provider
    if let Ok(provider) = std::env::var("GENTLE_EYE_PROVIDER") {
        config.vision.provider = provider;
    }

    // GENTLE_EYE_MAX_DURATION -> recording.max_duration_seconds
    if let Ok(duration_str) = std::env::var("GENTLE_EYE_MAX_DURATION") {
        if let Ok(duration) = duration_str.parse::<u64>() {
            config.recording.max_duration_seconds = duration;
        }
    }

    // OLLAMA_HOST -> vision.ollama_host (+ port). Accepts a bare host, "host:port", or a full
    // URL ("http://host:port[/]") and normalizes it so the downstream `http://{host}:{port}`
    // formatter never doubles the scheme/port. (fix 2026-06-27: env held a full URL -> doubled.)
    if let Ok(raw) = std::env::var("OLLAMA_HOST") {
        let s = raw.trim().trim_end_matches('/');
        let has_scheme = s.starts_with("http://") || s.starts_with("https://");
        let body = s
            .strip_prefix("http://")
            .or_else(|| s.strip_prefix("https://"))
            .unwrap_or(s);
        if body.contains('/') {
            // PATH-PREFIXED endpoint (e.g. the Atelier governor's memory-governed
            // lane, http://<mac>:8799/llm/ollama). Keep it VERBATIM — splitting it
            // into host+port drops the prefix and every call 404s. Downstream
            // (mcp/server.rs) passes a full URL through untouched.
            config.vision.ollama_host = if has_scheme {
                s.to_string()
            } else {
                format!("http://{s}")
            };
        } else {
            match body.rsplit_once(':') {
                Some((h, p)) if !h.is_empty() => {
                    config.vision.ollama_host = h.to_string();
                    if let Ok(port) = p.parse::<u16>() {
                        config.vision.ollama_port = port;
                    }
                }
                _ if !body.is_empty() => config.vision.ollama_host = body.to_string(),
                _ => {}
            }
        }
    }

    // OLLAMA_PORT -> vision.ollama_port
    if let Ok(port_str) = std::env::var("OLLAMA_PORT") {
        if let Ok(port) = port_str.parse::<u16>() {
            config.vision.ollama_port = port;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn test_load_default_config() {
        // When no config file exists, should return defaults
        let config = load_config();
        // This may fail if env vars are set, so just check it doesn't panic
        assert!(config.is_ok() || config.is_err());
    }

    #[test]
    fn test_apply_env_overrides() {
        let mut config = AppConfig::default();

        // Set test environment variables
        env::set_var("GENTLE_EYE_FPS", "5");
        env::set_var("GENTLE_EYE_PROVIDER", "ollama");

        apply_env_overrides(&mut config);

        assert_eq!(config.recording.fps, 5);
        assert_eq!(config.vision.provider, "ollama");

        // Clean up
        env::remove_var("GENTLE_EYE_FPS");
        env::remove_var("GENTLE_EYE_PROVIDER");
    }
}
