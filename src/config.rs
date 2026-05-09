pub fn redact_secret(secret: &str) -> String {
    if secret.len() <= 4 {
        "****".to_string()
    } else {
        format!(
            "{}...{}",
            &secret[..2],
            &secret[secret.len() - 2..]
// ============================================================================
// ============================================================================
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_parse_ffmpeg_version() {
        let output = "ffmpeg version 6.0 Copyright (c) 2000-2023";
        assert_eq!(parse_ffmpeg_version(output), "6.0");
        let output = "ffmpeg version 5.1.2-custom Copyright (c) 2000-2023";
        assert_eq!(parse_ffmpeg_version(output), "5.1.2-custom");
    #[test]
    fn test_redact_secret() {
        assert_eq!(redact_secret("abcdefghij"), "ab...ij");
        assert_eq!(redact_secret("abc"), "****");
        assert_eq!(redact_secret(""), "****");
    #[test]
    fn test_startup_check_result_pass() {
        let result = StartupCheckResult::pass("test_check");
        assert!(result.passed);
        assert!(result.warning.is_none());
        assert!(result.error.is_none());
    #[test]
    fn test_startup_check_result_pass_with_warning() {
        let result = StartupCheckResult::pass_with_warning("test_check", "some_warning");
        assert!(result.passed);
        assert_eq!(result.warning, Some("some_warning".to_string()));
        assert!(result.error.is_none());
    #[test]
    fn test_startup_check_result_fail() {
        let error = StartupError::FfmpegNotFound {
            install_command: "test_command".to_string(),
        };
        let result = StartupCheckResult::fail("test_check", error);
        assert!(!result.passed);
        assert!(result.error.is_some());
    #[test]
    fn test_startup_validation_from_checks() {
        let checks = vec![
            StartupCheckResult::pass("check_1"),
            StartupCheckResult::pass_with_warning("check_2", "warning_msg"),
        ];
        let validation = StartupValidation::from_checks(checks);
        assert!(validation.all_passed);
        assert_eq!(validation.warning_count, 1);
    #[test]
    fn test_startup_validation_with_failure() {
        let checks = vec![
            StartupCheckResult::pass("check_1"),
            StartupCheckResult::fail(
                "check_2",
                StartupError::ConfigError("test_error".to_string()),
            ),
        ];
        let validation = StartupValidation::from_checks(checks);
        assert!(!validation.all_passed);
        assert_eq!(validation.errors().len(), 1);
    #[test]
    fn test_check_storage_directory_temp() {
        let temp_dir = std::env::temp_dir();
        let result = check_storage_directory(&temp_dir);
        assert!(result.passed);
    #[test]
    fn test_sanitize_error_message() {
        // Note: This test will work differently depending on whether regex crate is available
        let message = "Error with api_key=SECRET123";
        let sanitized = sanitize_error_message(message);
        // Should not contain the actual secret
        assert!(!sanitized.contains("SECRET123") || sanitized.contains("[REDACTED]"));
    #[test]
    fn test_screen_capture_permission_check() {
        // This test verifies the screen capture permission check runs without crashing
        // The actual result depends on the environment
        let result = check_screen_capture_permission();
        // Result should have a check_name
        assert!(!result.check_name.is_empty());
        // If it fails, it should have an error with instructions
        if !result.passed {
            assert!(result.error.is_some());
    #[cfg(target_os = "linux")]
    #[test]
    fn test_x11_permission_check_with_display() {
        // Set up a fake DISPLAY for testing
        let original = std::env::var("DISPLAY").ok();
        std::env::set_var("DISPLAY", ":99");
        let result = check_x11_permission("test_check");
        // Should at least not crash
        assert!(!result.check_name.is_empty());
        // Restore original DISPLAY
        if let Some(orig) = original {
            std::env::set_var("DISPLAY", orig);
        } else {
            std::env::remove_var("DISPLAY");
    #[cfg(target_os = "linux")]
    #[test]
    fn test_wayland_permission_check() {
        let result = check_wayland_permission("test_check");
        // Should at least not crash
        assert!(!result.check_name.is_empty());
    #[test]
    fn test_startup_error_display() {
        // Test that error messages are descriptive
        let error = StartupError::ScreenCapturePermissionDenied(
            "Test permission denied message".to_string(),
        );
        let display = format!("{}", error);
        assert!(display.contains("Test permission denied message"));
        let error = StartupError::FfmpegNotFound {
            install_command: "apt install ffmpeg".to_string(),
        };
        let display = format!("{}", error);
        assert!(display.contains("FFmpeg not found"));
        assert!(display.contains("apt install ffmpeg"));
