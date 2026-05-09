pub fn validate_startup(config: &AppConfig) -> StartupValidation {
    let checks = vec![
        // Check FFmpeg
        check_ffmpeg(),
        // Check storage directory
        check_storage_directory(&config.storage.base_dir),
        // Check environment variables
        check_environment_variables(&config.vision.provider),
        // Check screen capture permissions (best-effort, platform-specific)
        check_screen_capture_permission(),
    // Log results
    for check in &checks {
        if check.passed {
            if let Some(ref warning) = check.warning {
                warn!("{}: PASSED with warning: {}", check.check_name, warning);
            } else {
                info!("{}: PASSED", check.check_name);
            }
        } else if let Some(ref error) = check.error {
            warn!("{}: FAILED - {}", check.check_name, error);
    StartupValidation::from_checks(checks)
</details></tool-use>
_**Agent (claude-sonnet-4-6 2026-03-11 18:20:00Z)**_
Now make both edits to `startup.rs`:
_**Agent (claude-sonnet-4-6 2026-03-11 18:20:09Z)**_
<tool-use data-tool-type="write" data-tool-name="Edit"><details>
<summary>Tool use: **Edit**</summary>
The file /home/gyasis/Documents/code/gentle-eye/src/startup.rs has been updated successfully.
</details></tool-use>
_**Agent (claude-sonnet-4-6 2026-03-11 18:23:25Z)**_
<tool-use data-tool-type="write" data-tool-name="Edit"><details>
<summary>Tool use: **Edit**</summary>
The file /home/gyasis/Documents/code/gentle-eye/src/startup.rs has been updated successfully.
</details></tool-use>
_**Agent (claude-sonnet-4-6 2026-03-11 18:23:30Z)**_
Now find where to insert the new function 
 just before the helper functions section:
_**Agent (claude-sonnet-4-6 2026-03-11 18:23:32Z)**_
<tool-use data-tool-type="search" data-tool-name="Grep"><details>
<summary>Tool use: **Grep**</summary>
 path `/home/gyasis/Documents/code/gentle-eye/src/startup.rs` pattern `// ============================================================================\n// Helper Functions`
579:// ============================================================================
580:// Helper Functions
</details></tool-use>
_**Agent (claude-sonnet-4-6 2026-03-11 18:23:36Z)**_
<tool-use data-tool-type="read" data-tool-name="Read"><details>
<summary>Tool use: **Read**</summary>
 `/home/gyasis/Documents/code/gentle-eye/src/startup.rs`
    info!("Screen capture test passed - permissions verified");
    Ok(())
// ============================================================================
// Helper Functions
// ============================================================================
/// Parse FFmpeg version from output
fn parse_ffmpeg_version(output: &str) -> String {
</details></tool-use>
_**Agent (claude-sonnet-4-6 2026-03-11 18:25:05Z)**_
<tool-use data-tool-type="write" data-tool-name="Edit"><details>
<summary>Tool use: **Edit**</summary>
The file /home/gyasis/Documents/code/gentle-eye/src/startup.rs has been updated successfully.
</details></tool-use>
_**Agent (claude-sonnet-4-6 2026-03-11 18:25:09Z)**_
Now update `gentle-eye.toml.example` to document the new fields:
_**Agent (claude-sonnet-4-6 2026-03-11 18:25:09Z)**_
<tool-use data-tool-type="read" data-tool-name="Read"><details>
<summary>Tool use: **Read**</summary>
 `/home/gyasis/Documents/code/gentle-eye/gentle-eye.toml.example`