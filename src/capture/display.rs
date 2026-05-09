impl DisplayManager {
    /// Create a new DisplayManager with optional initial display selection.
    /// # Arguments
    /// * `initial_selection` - Optional display index to select initially.
    ///   - `None`: Select the primary display (index 0)
    ///   - `Some(n)`: Select display at index n
    /// # Returns
    /// * `Ok(DisplayManager)` - Successfully created manager
    /// * `Err(DisplayError::NoDisplaysFound)` - No displays available
    /// * `Err(DisplayError::InvalidIndex)` - Requested index out of range
    /// # Example
    /// \```no_run
    /// use gentle_eye::capture::display::DisplayManager;
    /// // Select primary display
    /// let manager = DisplayManager::new(None)?;
    /// // Select specific display
    /// let manager = DisplayManager::new(Some(1))?;
    /// # Ok::<(), gentle_eye::capture::display::DisplayError>(())
    /// \```
    pub fn new(initial_selection: Option<usize>) -> Result<Self, DisplayError> {
        // Load saved configuration
        let config = DisplayConfig::load().unwrap_or_default();
        // Enumerate displays
        let displays = Self::enumerate_displays_internal(&config)?;
        if displays.is_empty() {
            return Err(DisplayError::NoDisplaysFound);
        // Determine initial selection
        let selected_index = match initial_selection {
            Some(idx) => {
                if idx >= displays.len() {
                    return Err(DisplayError::InvalidIndex {
                        requested: idx,
                        available: displays.len(),
                    });
                }
                idx
            }
            None => {
                // Try to use default from config, fallback to 0
                config
                    .default_display
                    .as_ref()
                    .and_then(|default| config.find_by_label(default, &displays))
                    .unwrap_or(0)
            }
        };
        tracing::info!(
            "DisplayManager initialized with {} display(s), selected: {} ({})",
            displays.len(),
            selected_index,
            displays[selected_index].display_name()
        );
        Ok(Self {
            displays,
            selected_index,
            config,
        })
    /// Enumerate all available displays on the system.
    /// This is a static method that queries the system for all connected
    /// displays without requiring a DisplayManager instance.
    /// # Returns
    /// * `Ok(Vec<DisplayInfo>)` - List of available displays
    /// * `Err(DisplayError::EnumerationFailed)` - Failed to query displays
    pub fn list_available() -> Result<Vec<DisplayInfo>, DisplayError> {
        let config = DisplayConfig::load().unwrap_or_default();
        Self::enumerate_displays_internal(&config)
    /// Internal display enumeration with label application.
    fn enumerate_displays_internal(config: &DisplayConfig) -> Result<Vec<DisplayInfo>, DisplayError> {
        let scrap_displays = Display::all().map_err(|e| {
            DisplayError::EnumerationFailed(format!("Failed to enumerate displays: {}", e))
        })?;
        if scrap_displays.is_empty() {
            return Err(DisplayError::NoDisplaysFound);
        // Get primary display to identify it in the list
        let primary_result = Display::primary();
        let primary_dims = primary_result.ok().map(|d| (d.width(), d.height()));
        let displays: Vec<DisplayInfo> = scrap_displays
            .into_iter()
            .enumerate()
            .map(|(index, display)| {
                let width = display.width();
                let height = display.height();
                // First display (index 0) is typically primary on most systems,
                // but we also check dimensions match the primary display
                let is_primary = index == 0
                    || primary_dims
                        .map(|(pw, ph)| pw == width && ph == height)
                        .unwrap_or(false);
                let mut info = DisplayInfo::new(index, width, height, is_primary && index == 0);
                // Apply saved label if available
                if let Some(label) = config.labels.get(&index) {
                    info.label = Some(label.clone());
                }
                info
            })
            .collect();
        tracing::debug!("Enumerated {} displays", displays.len());
        for d in &displays {
            tracing::debug!(
                "  Display {}: {}x{} (primary: {}, label: {:?})",
                d.index,
                d.width,
                d.height,
                d.is_primary,
                d.label
            );
        Ok(displays)
    /// Get the cached list of displays with labels applied.
    pub fn list_displays(&self) -> &[DisplayInfo] {
        &self.displays
    /// Select a display by index.
    /// # Arguments
    /// * `index` - Zero-based display index
    /// # Returns
    /// * `Ok(())` - Selection successful
    /// * `Err(DisplayError::InvalidIndex)` - Index out of range
    pub fn select_by_index(&mut self, index: usize) -> Result<(), DisplayError> {
        if index >= self.displays.len() {
            return Err(DisplayError::InvalidIndex {
                requested: index,
                available: self.displays.len(),
            });
        self.selected_index = index;
        tracing::info!(
            "Selected display {} ({})",
            index,
            self.displays[index].display_name()
        );
        Ok(())
    /// Select a display by label.
    /// # Arguments
    /// * `label` - The label to search for (case-insensitive)
    /// # Returns
    /// * `Ok(())` - Selection successful
    /// * `Err(DisplayError::LabelNotFound)` - No display with that label
    pub fn select_by_label(&mut self, label: &str) -> Result<(), DisplayError> {
        let index = self
            .config
            .find_by_label(label, &self.displays)
            .ok_or_else(|| DisplayError::LabelNotFound(label.to_string()))?;
        self.selected_index = index;
        tracing::info!(
            "Selected display by label '{}' -> index {}",
            label,
            index
        );
        Ok(())
    /// Set a label for a display.
    /// # Arguments
    /// * `index` - The display index
    /// * `label` - The user-friendly label to assign
    /// # Returns
    /// * `Ok(())` - Label set successfully
    /// * `Err(DisplayError::InvalidIndex)` - Index out of range
    pub fn set_display_label(&mut self, index: usize, label: String) -> Result<(), DisplayError> {
        if index >= self.displays.len() {
            return Err(DisplayError::InvalidIndex {
                requested: index,
                available: self.displays.len(),
            });
        // Update config
        self.config.set_label(index, label.clone());
        // Update cached display info
        self.displays[index].label = Some(label.clone());
        tracing::info!("Set label for display {}: '{}'", index, label);
        Ok(())
    /// Remove a label from a display.
    /// # Arguments
    /// * `index` - The display index
    /// # Returns
    /// * `Ok(())` - Label removed successfully
    /// * `Err(DisplayError::InvalidIndex)` - Index out of range
    pub fn remove_display_label(&mut self, index: usize) -> Result<(), DisplayError> {
        if index >= self.displays.len() {
            return Err(DisplayError::InvalidIndex {
                requested: index,
                available: self.displays.len(),
            });
        self.config.remove_label(index);
        self.displays[index].label = None;
        tracing::info!("Removed label from display {}", index);
        Ok(())
    /// Get a scrap::Display handle for the currently selected display.
    /// This method returns a fresh `Display` handle suitable for creating
    /// a `Capturer`. The handle is obtained fresh each time to ensure
    /// it reflects the current display state.
    /// # Returns
    /// * `Ok(Display)` - Display handle for capture
    /// * `Err(DisplayError::EnumerationFailed)` - Failed to get display
    /// * `Err(DisplayError::DisplayDisconnected)` - Selected display no longer available
    pub fn get_selected_display(&self) -> Result<Display, DisplayError> {
        let displays = Display::all().map_err(|e| {
            DisplayError::EnumerationFailed(format!("Failed to enumerate displays: {}", e))
        })?;
        displays
            .into_iter()
            .nth(self.selected_index)
            .ok_or(DisplayError::DisplayDisconnected)
    /// Get information about the currently selected display.
    pub fn selected_info(&self) -> &DisplayInfo {
        &self.displays[self.selected_index]
    /// Get the index of the currently selected display.
    pub fn selected_index(&self) -> usize {
        self.selected_index
    /// Get the number of available displays.
    pub fn display_count(&self) -> usize {
        self.displays.len()
    /// Save the current configuration to disk.
    /// # Returns
    /// * `Ok(())` - Configuration saved successfully
    /// * `Err(DisplayError::ConfigError)` - Failed to save configuration
    pub fn save_config(&self) -> Result<(), DisplayError> {
        self.config.save()
    /// Set the default display to use on startup.
    /// # Arguments
    /// * `default` - Display label or index (as string)
    pub fn set_default_display(&mut self, default: String) {
        self.config.default_display = Some(default);
        self.config.last_updated = Some(Utc::now().to_rfc3339());
    /// Refresh the display list by re-enumerating system displays.
    /// This method updates the internal display cache. If the currently
    /// selected display is still available (matched by dimensions),
    /// the selection is preserved. Otherwise, the selection falls back
    /// to the primary display (index 0).
    /// # Returns
    /// * `Ok(bool)` - `true` if the display list changed, `false` otherwise
    /// * `Err(DisplayError)` - Failed to enumerate displays
    pub fn refresh(&mut self) -> Result<bool, DisplayError> {
        let new_displays = Self::enumerate_displays_internal(&self.config)?;
        // Check if display list changed
        let changed = new_displays.len() != self.displays.len()
            || new_displays
                .iter()
                .zip(self.displays.iter())
                .any(|(a, b)| a.width != b.width || a.height != b.height);
        if changed {
            tracing::info!(
                "Display configuration changed: {} -> {} displays",
                self.displays.len(),
                new_displays.len()
            );
            // Try to preserve selection by matching display dimensions
            let current_info = &self.displays[self.selected_index];
            let new_index = new_displays
                .iter()
                .position(|d| d.width == current_info.width && d.height == current_info.height)
                .unwrap_or(0);
            if new_index != self.selected_index {
                tracing::warn!(
                    "Selected display changed from index {} to {}",
                    self.selected_index,
                    new_index
                );
            }
            self.displays = new_displays;
            self.selected_index = new_index;
        Ok(changed)
    /// Check if the currently selected display is still available.
    pub fn is_selected_available(&self) -> bool {
        Display::all()
            .map(|displays| displays.into_iter().nth(self.selected_index).is_some())
            .unwrap_or(false)
    /// Get the configuration reference (read-only).
    pub fn config(&self) -> &DisplayConfig {
        &self.config
// ============================================================================
// ============================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    #[test]
    fn test_display_info_creation() {
        let info = DisplayInfo::new(0, 1920, 1080, true);
        assert_eq!(info.index, 0);
        assert_eq!(info.width, 1920);
        assert_eq!(info.height, 1080);
        assert!(info.is_primary);
        assert!(info.label.is_none());
        assert_eq!(info.auto_name, "Display 1 (1920x1080, Primary)");
    #[test]
    fn test_display_info_with_label() {
        let info = DisplayInfo::with_label(1, 2560, 1440, false, "external".to_string());
        assert_eq!(info.index, 1);
        assert_eq!(info.label, Some("external".to_string()));
        assert_eq!(info.display_name(), "external");
    #[test]
    fn test_display_info_auto_name() {
        let primary = DisplayInfo::new(0, 1920, 1080, true);
        assert_eq!(primary.generate_auto_name(), "Display 1 (1920x1080, Primary)");
        let secondary = DisplayInfo::new(1, 2560, 1440, false);
        assert_eq!(secondary.generate_auto_name(), "Display 2 (2560x1440)");
    #[test]
    fn test_display_info_pixel_count() {
        let info = DisplayInfo::new(0, 1920, 1080, true);
        assert_eq!(info.pixel_count(), 1920 * 1080);
    #[test]
    fn test_display_info_aspect_ratio() {
        let info = DisplayInfo::new(0, 1920, 1080, true);
        let ratio = info.aspect_ratio();
        assert!((ratio - 16.0 / 9.0).abs() < 0.01);
        assert_eq!(info.aspect_ratio_name(), "16:9");
    #[test]
    fn test_display_info_serialization() {
        let info = DisplayInfo::with_label(0, 1920, 1080, true, "main".to_string());
        let json = serde_json::to_string(&info).unwrap();
        let deserialized: DisplayInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(info, deserialized);
    #[test]
    fn test_display_config_default() {
        let config = DisplayConfig::default();
        assert!(config.labels.is_empty());
        assert!(config.default_display.is_none());
        assert!(config.last_updated.is_none());
    #[test]
    fn test_display_config_labels() {
        let mut config = DisplayConfig::default();
        config.set_label(0, "main-laptop".to_string());
        config.set_label(1, "external-monitor".to_string());
        assert_eq!(config.labels.get(&0), Some(&"main-laptop".to_string()));
        assert_eq!(
            config.labels.get(&1),
            Some(&"external-monitor".to_string())
        );
        assert!(config.last_updated.is_some());
    #[test]
    fn test_display_config_get_label() {
        let mut config = DisplayConfig::default();
        let info = DisplayInfo::new(0, 1920, 1080, true);
        // Without label, should return auto-generated name
        let label = config.get_label(0, &info);
        assert_eq!(label, "Display 1 (1920x1080, Primary)");
        // With label, should return the label
        config.set_label(0, "main".to_string());
        let label = config.get_label(0, &info);
        assert_eq!(label, "main");
    #[test]
    fn test_display_config_find_by_label() {
        let mut config = DisplayConfig::default();
        config.set_label(0, "laptop".to_string());
        config.set_label(1, "external".to_string());
        let displays = vec![
            DisplayInfo::new(0, 1920, 1080, true),
            DisplayInfo::new(1, 2560, 1440, false),
        ];
        assert_eq!(config.find_by_label("laptop", &displays), Some(0));
        assert_eq!(config.find_by_label("EXTERNAL", &displays), Some(1)); // Case-insensitive
        assert_eq!(config.find_by_label("unknown", &displays), None);
    #[test]
    fn test_display_config_remove_label() {
        let mut config = DisplayConfig::default();
        config.set_label(0, "main".to_string());
        let removed = config.remove_label(0);
        assert_eq!(removed, Some("main".to_string()));
        assert!(config.labels.get(&0).is_none());
        let removed_again = config.remove_label(0);
        assert!(removed_again.is_none());
    #[test]
    fn test_display_config_serialization() {
        let mut config = DisplayConfig::default();
        config.set_label(0, "main".to_string());
        config.default_display = Some("main".to_string());
        let json = serde_json::to_string_pretty(&config).unwrap();
        let deserialized: DisplayConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config.labels, deserialized.labels);
        assert_eq!(config.default_display, deserialized.default_display);
    // Integration tests that require actual displays
    #[test]
    fn test_display_manager_new() {
        // This test will pass or fail depending on the environment
        let result = DisplayManager::new(None);
        match result {
            Ok(manager) => {
                assert!(manager.display_count() > 0);
                assert!(manager.selected_index() < manager.display_count());
            }
            Err(DisplayError::NoDisplaysFound) => {
                // Expected in headless environments
            }
            Err(e) => {
                // Other errors are also acceptable in test environments
                eprintln!("DisplayManager test: {}", e);
            }
    #[test]
    fn test_display_manager_invalid_index() {
        // Skip if no displays available
        if DisplayManager::list_available().is_err() {
            return;
        let result = DisplayManager::new(Some(999));
        assert!(matches!(result, Err(DisplayError::InvalidIndex { .. })));
    #[test]
    fn test_display_manager_select_by_index() {
        let mut manager = match DisplayManager::new(None) {
            Ok(m) => m,
            Err(_) => return, // Skip if no displays
        };
        // Select same display should work
        assert!(manager.select_by_index(0).is_ok());
        // Select invalid index should fail
        let result = manager.select_by_index(999);
        assert!(matches!(result, Err(DisplayError::InvalidIndex { .. })));
    #[test]
    fn test_display_manager_labels() {
        let mut manager = match DisplayManager::new(None) {
            Ok(m) => m,
            Err(_) => return, // Skip if no displays
        };
        // Set label
        assert!(manager
            .set_display_label(0, "test-label".to_string())
            .is_ok());
        // Verify label is set
        assert_eq!(
            manager.list_displays()[0].label,
            Some("test-label".to_string())
        );
        assert_eq!(manager.list_displays()[0].display_name(), "test-label");
        // Select by label
        assert!(manager.select_by_label("test-label").is_ok());
        assert_eq!(manager.selected_index(), 0);
        // Remove label
        assert!(manager.remove_display_label(0).is_ok());
        assert!(manager.list_displays()[0].label.is_none());
    #[test]
    fn test_display_manager_select_by_label_not_found() {
        let mut manager = match DisplayManager::new(None) {
            Ok(m) => m,
            Err(_) => return, // Skip if no displays
        };
        let result = manager.select_by_label("nonexistent-label");
        assert!(matches!(result, Err(DisplayError::LabelNotFound(_))));
    #[test]
    fn test_display_manager_refresh() {
        let mut manager = match DisplayManager::new(None) {
            Ok(m) => m,
            Err(_) => return, // Skip if no displays
        };
        // Refresh should succeed
        let result = manager.refresh();
        assert!(result.is_ok());
    #[test]
    fn test_display_error_to_gentle_eye_error() {
        let display_err = DisplayError::NoDisplaysFound;
        let gentle_err: GentleEyeError = display_err.into();
        assert!(matches!(gentle_err, GentleEyeError::Recording(_)));
File created successfully at: /home/gyasis/Documents/code/gentle-eye/src/capture/display.rs
