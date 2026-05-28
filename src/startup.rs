//! Startup validation — environment/permission checks run before serving.
//!
//! Reconstructed 2026-05-28 (integration-aware, from the `bin` contract). The
//! recovered file held only the `validate_startup` skeleton + the check names; the
//! helper bodies and the result/error types were lost in session-log junk. The
//! types + dispatch are reconstructed; the per-check helpers are STUBS that pass
//! (real probes — FFmpeg presence, storage writability, env vars, capture perms —
//! are a follow-up).
use crate::config::AppConfig;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StartupError {
    #[error("FFmpeg not found. Install with: {install_command}")]
    FfmpegNotFound { install_command: String },
    #[error("Storage directory not accessible: {path:?} ({reason})")]
    StorageDirectoryNotAccessible { path: PathBuf, reason: String },
    #[error("Storage directory not writable: {path:?}")]
    StorageDirectoryNotWritable { path: PathBuf },
    #[error("{0}")]
    ScreenCapturePermissionDenied(String),
    #[error("Environment variable {var_name} not set ({hint})")]
    EnvVarMissing { var_name: String, hint: String },
    #[error("Configuration error: {0}")]
    ConfigError(String),
}

/// Result of a single startup check.
pub struct CheckResult {
    pub check_name: String,
    pub passed: bool,
    pub warning: Option<String>,
    pub error: Option<StartupError>,
}

/// Aggregate of all startup checks.
pub struct StartupValidation {
    pub all_passed: bool,
    pub warning_count: usize,
    checks: Vec<CheckResult>,
}

impl StartupValidation {
    pub fn from_checks(checks: Vec<CheckResult>) -> Self {
        let all_passed = checks.iter().all(|c| c.passed);
        let warning_count = checks.iter().filter(|c| c.warning.is_some()).count();
        Self {
            all_passed,
            warning_count,
            checks,
        }
    }

    pub fn warnings(&self) -> Vec<&String> {
        self.checks.iter().filter_map(|c| c.warning.as_ref()).collect()
    }

    pub fn errors(&self) -> Vec<&StartupError> {
        self.checks.iter().filter_map(|c| c.error.as_ref()).collect()
    }
}

pub fn validate_startup(config: &AppConfig) -> StartupValidation {
    let checks = vec![
        check_ffmpeg(),
        check_storage_directory(&config.storage.base_dir),
        check_environment_variables(&config.vision.provider),
        check_screen_capture_permission(),
    ];
    StartupValidation::from_checks(checks)
}

// --- per-check helpers (STUBS — pass; real probes are a follow-up) ---

fn passing(name: &str) -> CheckResult {
    CheckResult {
        check_name: name.to_string(),
        passed: true,
        warning: None,
        error: None,
    }
}

fn check_ffmpeg() -> CheckResult {
    passing("ffmpeg")
}

fn check_storage_directory(_base_dir: &std::path::Path) -> CheckResult {
    passing("storage_directory")
}

fn check_environment_variables(_provider: &str) -> CheckResult {
    passing("environment_variables")
}

fn check_screen_capture_permission() -> CheckResult {
    passing("screen_capture_permission")
}
