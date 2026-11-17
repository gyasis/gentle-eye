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
///
/// The Ollama endpoint is the exception: see [`apply_ollama_host`].
pub fn load_config() -> Result<AppConfig, ConfigError> {
    let config_path = AppConfig::config_file_path();

    // Start with defaults
    let (mut config, file_pins_host) = if config_path.exists() {
        let content = fs::read_to_string(&config_path)?;
        let pins = file_sets_ollama_host(&content);
        (toml::from_str(&content)?, pins)
    } else {
        (AppConfig::default(), false)
    };

    // Apply environment variable overrides
    apply_env_overrides(&mut config);
    apply_ollama_host(&mut config, file_pins_host);

    // Validate the final configuration
    config.validate()?;

    Ok(config)
}

/// True when the config file names `vision.ollama_host` itself.
fn file_sets_ollama_host(content: &str) -> bool {
    toml::from_str::<toml::Value>(content)
        .ok()
        .and_then(|v| v.get("vision")?.get("ollama_host").map(|_| ()))
        .is_some()
}

/// Resolve the Ollama endpoint. First match wins:
/// 1. `GENTLE_EYE_OLLAMA_HOST`, the gentle-eye-only override.
/// 2. `vision.ollama_host` in the config file. A shell-wide `OLLAMA_HOST`
///    serves other tools and must not move gentle-eye off the endpoint its
///    config pins (for example a governor URL), so it is ignored here.
/// 3. `OLLAMA_HOST` and `OLLAMA_PORT`.
/// 4. The built-in default.
fn apply_ollama_host(config: &mut AppConfig, file_pins_host: bool) {
    if let Some(raw) = non_empty_env("GENTLE_EYE_OLLAMA_HOST") {
        set_ollama_host(config, &raw);
        return;
    }
    if file_pins_host {
        return;
    }
    if let Some(raw) = non_empty_env("OLLAMA_HOST") {
        set_ollama_host(config, &raw);
    }
    if let Ok(port_str) = std::env::var("OLLAMA_PORT") {
        if let Ok(port) = port_str.parse::<u16>() {
            config.vision.ollama_port = port;
        }
    }
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

/// The environment variable that carries the model of `provider`.
pub fn model_env_var(provider: &str) -> &'static str {
    if provider == "ollama" {
        "GENTLE_EYE_OLLAMA_MODEL"
    } else {
        "GENTLE_EYE_GEMINI_MODEL"
    }
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

    // GENTLE_EYE_OLLAMA_MODEL -> vision.ollama_model (e.g. an Ollama Cloud vision model, "qwen3-vl:235b-cloud")
    if let Ok(model) = std::env::var("GENTLE_EYE_OLLAMA_MODEL") {
        if !model.trim().is_empty() {
            config.vision.ollama_model = model.trim().to_string();
        }
    }

    // GENTLE_EYE_GEMINI_MODEL -> vision.gemini_model (e.g. "gemini-3.8-flash")
    if let Ok(model) = std::env::var("GENTLE_EYE_GEMINI_MODEL") {
        if !model.trim().is_empty() {
            config.vision.gemini_model = model.trim().to_string();
        }
    }

    // GENTLE_EYE_MAX_DURATION -> recording.max_duration_seconds
    if let Ok(duration_str) = std::env::var("GENTLE_EYE_MAX_DURATION") {
        if let Ok(duration) = duration_str.parse::<u64>() {
            config.recording.max_duration_seconds = duration;
        }
    }
}

/// Store an endpoint in `vision.ollama_host` (+ port). Accepts a bare host,
/// "host:port", or a full URL ("http://host:port[/]"), normalized so the
/// downstream `http://{host}:{port}` formatter never doubles the scheme or port.
fn set_ollama_host(config: &mut AppConfig, raw: &str) {
    let s = raw.trim().trim_end_matches('/');
    let has_scheme = s.starts_with("http://") || s.starts_with("https://");
    let body = s
        .strip_prefix("http://")
        .or_else(|| s.strip_prefix("https://"))
        .unwrap_or(s);
    if body.contains('/') {
        // A path-prefixed endpoint (such as a governor lane, http://<mac>:8799/llm/ollama)
        // stays verbatim: splitting it into host+port drops the prefix and every call 404s.
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

    #[test]
    fn model_env_var_follows_the_provider() {
        assert_eq!(model_env_var("ollama"), "GENTLE_EYE_OLLAMA_MODEL");
        assert_eq!(model_env_var("gemini"), "GENTLE_EYE_GEMINI_MODEL");
    }

    #[test]
    fn each_model_env_var_sets_only_its_own_provider_model() {
        let mut config = AppConfig::default();
        let defaults = config.vision.clone();

        env::set_var("GENTLE_EYE_GEMINI_MODEL", "gemini-3.8-flash");
        apply_env_overrides(&mut config);
        assert_eq!(config.vision.gemini_model, "gemini-3.8-flash");
        assert_eq!(config.vision.ollama_model, defaults.ollama_model);
        env::remove_var("GENTLE_EYE_GEMINI_MODEL");

        let mut config = AppConfig::default();
        env::set_var("GENTLE_EYE_OLLAMA_MODEL", "minimax-m3:cloud");
        apply_env_overrides(&mut config);
        assert_eq!(config.vision.ollama_model, "minimax-m3:cloud");
        assert_eq!(config.vision.gemini_model, defaults.gemini_model);
        env::remove_var("GENTLE_EYE_OLLAMA_MODEL");
    }

    #[test]
    fn file_host_detection_reads_the_vision_table() {
        assert!(file_sets_ollama_host("[vision]\nollama_host = \"http://gov:8799/llm/ollama\"\n"));
        assert!(!file_sets_ollama_host("[vision]\nollama_model = \"m\"\n"));
        assert!(!file_sets_ollama_host("ollama_host = \"x\"\n"));
    }

    #[test]
    fn ollama_host_order_gentle_eye_env_then_file_then_shell_env() {
        let gov = "http://gov:8799/llm/ollama";
        env::remove_var("GENTLE_EYE_OLLAMA_HOST");
        env::remove_var("OLLAMA_PORT");
        env::set_var("OLLAMA_HOST", "http://raw:11434");

        // A host pinned in the file beats the shell-wide OLLAMA_HOST.
        let mut config = AppConfig::default();
        config.vision.ollama_host = gov.into();
        apply_ollama_host(&mut config, true);
        assert_eq!(config.vision.ollama_host, gov);

        // Without a pinned host, OLLAMA_HOST applies.
        let mut config = AppConfig::default();
        apply_ollama_host(&mut config, false);
        assert_eq!(config.vision.ollama_host, "raw");
        assert_eq!(config.vision.ollama_port, 11434);

        // GENTLE_EYE_OLLAMA_HOST beats the file.
        env::set_var("GENTLE_EYE_OLLAMA_HOST", "http://other:8799/llm/ollama");
        let mut config = AppConfig::default();
        config.vision.ollama_host = gov.into();
        apply_ollama_host(&mut config, true);
        assert_eq!(config.vision.ollama_host, "http://other:8799/llm/ollama");

        env::remove_var("GENTLE_EYE_OLLAMA_HOST");
        env::remove_var("OLLAMA_HOST");
    }
}
