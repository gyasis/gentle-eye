//! Display enumeration, selection, and labeling for screen capture.
//!
//! Reconstructed 2026-05-28 (paired-debate + Gemini): the recovered file had its
//! entire top (imports + type defs) missing and all standalone `}` lines dropped.
//! Types reconstructed from the impl + test ground-truth; braces restored. The
//! `DisplayConfig::{load,save}` bodies are minimal stubs (persistence wiring is a
//! follow-up); the serde round-trip tests exercise the real (de)serialization.

use chrono::Utc;
use crate::contracts::errors::{GentleEyeError, RecordingError};
use scrap::Display;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct DisplayInfo {
    pub index: usize,
    pub width: u32,
    pub height: u32,
    pub is_primary: bool,
    pub label: Option<String>,
    pub auto_name: String,
}

impl DisplayInfo {
    pub fn new(index: usize, width: u32, height: u32, is_primary: bool) -> Self {
        let mut info = Self {
            index,
            width,
            height,
            is_primary,
            label: None,
            auto_name: String::new(),
        };
        info.auto_name = info.generate_auto_name();
        info
    }

    pub fn with_label(
        index: usize,
        width: u32,
        height: u32,
        is_primary: bool,
        label: String,
    ) -> Self {
        let mut info = Self::new(index, width, height, is_primary);
        info.label = Some(label);
        info
    }

    pub fn display_name(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.auto_name)
    }

    pub fn generate_auto_name(&self) -> String {
        format!(
            "Display {} ({}x{}{})",
            self.index + 1,
            self.width,
            self.height,
            if self.is_primary { ", Primary" } else { "" }
        )
    }

    pub fn pixel_count(&self) -> u64 {
        self.width as u64 * self.height as u64
    }

    pub fn aspect_ratio(&self) -> f64 {
        if self.height == 0 {
            return 0.0;
        }
        self.width as f64 / self.height as f64
    }

    pub fn aspect_ratio_name(&self) -> &str {
        let ratio = self.aspect_ratio();
        if (ratio - 16.0 / 9.0).abs() < 0.01 {
            "16:9"
        } else if (ratio - 16.0 / 10.0).abs() < 0.01 {
            "16:10"
        } else if (ratio - 4.0 / 3.0).abs() < 0.01 {
            "4:3"
        } else if (ratio - 21.0 / 9.0).abs() < 0.01 {
            "21:9"
        } else {
            "Unknown"
        }
    }
}

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct DisplayConfig {
    pub labels: HashMap<usize, String>,
    pub default_display: Option<String>,
    pub last_updated: Option<String>,
}

impl DisplayConfig {
    fn config_path() -> Result<PathBuf, DisplayError> {
        let home = std::env::var("HOME")
            .map_err(|_| DisplayError::ConfigError("Could not find HOME directory".into()))?;
        Ok(PathBuf::from(home).join(".config/gentle-eye/display.json"))
    }

    pub fn load() -> Result<Self, DisplayError> {
        let path = Self::config_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let content =
            fs::read_to_string(path).map_err(|e| DisplayError::ConfigError(e.to_string()))?;
        serde_json::from_str(&content).map_err(|e| DisplayError::ConfigError(e.to_string()))
    }

    pub fn save(&self) -> Result<(), DisplayError> {
        let path = Self::config_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| DisplayError::ConfigError(e.to_string()))?;
        }
        let serialized = serde_json::to_string_pretty(self)
            .map_err(|e| DisplayError::ConfigError(e.to_string()))?;
        fs::write(path, serialized).map_err(|e| DisplayError::ConfigError(e.to_string()))
    }

    pub fn set_label(&mut self, index: usize, label: String) {
        self.labels.insert(index, label);
        self.last_updated = Some(Utc::now().to_rfc3339());
    }

    pub fn remove_label(&mut self, index: usize) -> Option<String> {
        let removed = self.labels.remove(&index);
        if removed.is_some() {
            self.last_updated = Some(Utc::now().to_rfc3339());
        }
        removed
    }

    pub fn get_label(&self, index: usize, info: &DisplayInfo) -> String {
        self.labels
            .get(&index)
            .cloned()
            .unwrap_or_else(|| info.display_name().to_string())
    }

    pub fn find_by_label(&self, label: &str, displays: &[DisplayInfo]) -> Option<usize> {
        let target = label.to_lowercase();
        for (idx, l) in &self.labels {
            if l.to_lowercase() == target {
                return Some(*idx);
            }
        }
        for d in displays {
            if let Some(l) = &d.label {
                if l.to_lowercase() == target {
                    return Some(d.index);
                }
            }
            if d.auto_name.to_lowercase() == target {
                return Some(d.index);
            }
        }
        None
    }
}

#[derive(Debug, Error)]
pub enum DisplayError {
    #[error("No displays found on the system")]
    NoDisplaysFound,
    #[error("Invalid display index: requested {requested}, but only {available} available")]
    InvalidIndex { requested: usize, available: usize },
    #[error("Label not found: {0}")]
    LabelNotFound(String),
    #[error("Failed to enumerate displays: {0}")]
    EnumerationFailed(String),
    #[error("Display was disconnected")]
    DisplayDisconnected,
    #[error("Configuration error: {0}")]
    ConfigError(String),
}

impl From<DisplayError> for GentleEyeError {
    fn from(e: DisplayError) -> Self {
        GentleEyeError::Recording(RecordingError::Internal(e.to_string()))
    }
}

pub struct DisplayManager {
    pub displays: Vec<DisplayInfo>,
    pub selected_index: usize,
    pub config: DisplayConfig,
}

impl DisplayManager {
    pub fn new(initial_selection: Option<usize>) -> Result<Self, DisplayError> {
        let config = DisplayConfig::load().unwrap_or_default();
        let displays = Self::enumerate_displays_internal(&config)?;
        if displays.is_empty() {
            return Err(DisplayError::NoDisplaysFound);
        }
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
            None => config
                .default_display
                .as_ref()
                .and_then(|default| config.find_by_label(default, &displays))
                .unwrap_or(0),
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
    }

    pub fn list_available() -> Result<Vec<DisplayInfo>, DisplayError> {
        let config = DisplayConfig::load().unwrap_or_default();
        Self::enumerate_displays_internal(&config)
    }

    fn enumerate_displays_internal(config: &DisplayConfig) -> Result<Vec<DisplayInfo>, DisplayError> {
        let scrap_displays = Display::all().map_err(|e| {
            DisplayError::EnumerationFailed(format!("Failed to enumerate displays: {}", e))
        })?;
        if scrap_displays.is_empty() {
            return Err(DisplayError::NoDisplaysFound);
        }
        let primary_result = Display::primary();
        let primary_dims = primary_result.ok().map(|d| (d.width(), d.height()));
        let displays: Vec<DisplayInfo> = scrap_displays
            .into_iter()
            .enumerate()
            .map(|(index, display)| {
                let width = display.width();
                let height = display.height();
                let is_primary = index == 0
                    || primary_dims
                        .map(|(pw, ph)| pw == width && ph == height)
                        .unwrap_or(false);
                let mut info =
                    DisplayInfo::new(index, width as u32, height as u32, is_primary && index == 0);
                if let Some(label) = config.labels.get(&index) {
                    info.label = Some(label.clone());
                }
                info
            })
            .collect();
        tracing::debug!("Enumerated {} displays", displays.len());
        Ok(displays)
    }

    pub fn list_displays(&self) -> &[DisplayInfo] {
        &self.displays
    }

    pub fn select_by_index(&mut self, index: usize) -> Result<(), DisplayError> {
        if index >= self.displays.len() {
            return Err(DisplayError::InvalidIndex {
                requested: index,
                available: self.displays.len(),
            });
        }
        self.selected_index = index;
        Ok(())
    }

    pub fn select_by_label(&mut self, label: &str) -> Result<(), DisplayError> {
        let index = self
            .config
            .find_by_label(label, &self.displays)
            .ok_or_else(|| DisplayError::LabelNotFound(label.to_string()))?;
        self.selected_index = index;
        Ok(())
    }

    pub fn set_display_label(&mut self, index: usize, label: String) -> Result<(), DisplayError> {
        if index >= self.displays.len() {
            return Err(DisplayError::InvalidIndex {
                requested: index,
                available: self.displays.len(),
            });
        }
        self.config.set_label(index, label.clone());
        self.displays[index].label = Some(label.clone());
        Ok(())
    }

    pub fn remove_display_label(&mut self, index: usize) -> Result<(), DisplayError> {
        if index >= self.displays.len() {
            return Err(DisplayError::InvalidIndex {
                requested: index,
                available: self.displays.len(),
            });
        }
        self.config.remove_label(index);
        self.displays[index].label = None;
        Ok(())
    }

    pub fn get_selected_display(&self) -> Result<Display, DisplayError> {
        let displays = Display::all().map_err(|e| {
            DisplayError::EnumerationFailed(format!("Failed to enumerate displays: {}", e))
        })?;
        displays
            .into_iter()
            .nth(self.selected_index)
            .ok_or(DisplayError::DisplayDisconnected)
    }

    pub fn selected_info(&self) -> &DisplayInfo {
        &self.displays[self.selected_index]
    }

    pub fn selected_index(&self) -> usize {
        self.selected_index
    }

    pub fn display_count(&self) -> usize {
        self.displays.len()
    }

    pub fn save_config(&self) -> Result<(), DisplayError> {
        self.config.save()
    }

    pub fn set_default_display(&mut self, default: String) {
        self.config.default_display = Some(default);
        self.config.last_updated = Some(Utc::now().to_rfc3339());
    }

    pub fn refresh(&mut self) -> Result<bool, DisplayError> {
        let new_displays = Self::enumerate_displays_internal(&self.config)?;
        let changed = new_displays.len() != self.displays.len()
            || new_displays
                .iter()
                .zip(self.displays.iter())
                .any(|(a, b)| a.width != b.width || a.height != b.height);
        if changed {
            let current_info = &self.displays[self.selected_index];
            let new_index = new_displays
                .iter()
                .position(|d| d.width == current_info.width && d.height == current_info.height)
                .unwrap_or(0);
            self.displays = new_displays;
            self.selected_index = new_index;
        }
        Ok(changed)
    }

    pub fn is_selected_available(&self) -> bool {
        Display::all()
            .map(|displays| displays.into_iter().nth(self.selected_index).is_some())
            .unwrap_or(false)
    }

    pub fn config(&self) -> &DisplayConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_info_creation() {
        let info = DisplayInfo::new(0, 1920, 1080, true);
        assert_eq!(info.index, 0);
        assert_eq!(info.width, 1920);
        assert_eq!(info.height, 1080);
        assert!(info.is_primary);
        assert!(info.label.is_none());
        assert_eq!(info.auto_name, "Display 1 (1920x1080, Primary)");
    }

    #[test]
    fn test_display_info_with_label() {
        let info = DisplayInfo::with_label(1, 2560, 1440, false, "external".to_string());
        assert_eq!(info.index, 1);
        assert_eq!(info.label, Some("external".to_string()));
        assert_eq!(info.display_name(), "external");
    }

    #[test]
    fn test_display_info_auto_name() {
        let primary = DisplayInfo::new(0, 1920, 1080, true);
        assert_eq!(primary.generate_auto_name(), "Display 1 (1920x1080, Primary)");
        let secondary = DisplayInfo::new(1, 2560, 1440, false);
        assert_eq!(secondary.generate_auto_name(), "Display 2 (2560x1440)");
    }

    #[test]
    fn test_display_info_pixel_count() {
        let info = DisplayInfo::new(0, 1920, 1080, true);
        assert_eq!(info.pixel_count(), 1920 * 1080);
    }

    #[test]
    fn test_display_info_aspect_ratio() {
        let info = DisplayInfo::new(0, 1920, 1080, true);
        let ratio = info.aspect_ratio();
        assert!((ratio - 16.0 / 9.0).abs() < 0.01);
        assert_eq!(info.aspect_ratio_name(), "16:9");
    }

    #[test]
    fn test_display_info_serialization() {
        let info = DisplayInfo::with_label(0, 1920, 1080, true, "main".to_string());
        let json = serde_json::to_string(&info).unwrap();
        let deserialized: DisplayInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(info, deserialized);
    }

    #[test]
    fn test_display_config_default() {
        let config = DisplayConfig::default();
        assert!(config.labels.is_empty());
        assert!(config.default_display.is_none());
        assert!(config.last_updated.is_none());
    }

    #[test]
    fn test_display_config_labels() {
        let mut config = DisplayConfig::default();
        config.set_label(0, "main-laptop".to_string());
        config.set_label(1, "external-monitor".to_string());
        assert_eq!(config.labels.get(&0), Some(&"main-laptop".to_string()));
        assert_eq!(config.labels.get(&1), Some(&"external-monitor".to_string()));
        assert!(config.last_updated.is_some());
    }

    #[test]
    fn test_display_config_get_label() {
        let mut config = DisplayConfig::default();
        let info = DisplayInfo::new(0, 1920, 1080, true);
        let label = config.get_label(0, &info);
        assert_eq!(label, "Display 1 (1920x1080, Primary)");
        config.set_label(0, "main".to_string());
        let label = config.get_label(0, &info);
        assert_eq!(label, "main");
    }

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
        assert_eq!(config.find_by_label("EXTERNAL", &displays), Some(1));
        assert_eq!(config.find_by_label("unknown", &displays), None);
    }

    #[test]
    fn test_display_config_remove_label() {
        let mut config = DisplayConfig::default();
        config.set_label(0, "main".to_string());
        let removed = config.remove_label(0);
        assert_eq!(removed, Some("main".to_string()));
        assert!(!config.labels.contains_key(&0));
        let removed_again = config.remove_label(0);
        assert!(removed_again.is_none());
    }

    #[test]
    fn test_display_config_serialization() {
        let mut config = DisplayConfig::default();
        config.set_label(0, "main".to_string());
        config.default_display = Some("main".to_string());
        let json = serde_json::to_string_pretty(&config).unwrap();
        let deserialized: DisplayConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config.labels, deserialized.labels);
        assert_eq!(config.default_display, deserialized.default_display);
    }

    #[test]
    fn test_display_manager_new() {
        let result = DisplayManager::new(None);
        match result {
            Ok(manager) => {
                assert!(manager.display_count() > 0);
                assert!(manager.selected_index() < manager.display_count());
            }
            Err(DisplayError::NoDisplaysFound) => {}
            Err(e) => {
                eprintln!("DisplayManager test: {}", e);
            }
        }
    }

    #[test]
    fn test_display_manager_invalid_index() {
        if DisplayManager::list_available().is_err() {
            return;
        }
        let result = DisplayManager::new(Some(999));
        assert!(matches!(result, Err(DisplayError::InvalidIndex { .. })));
    }

    #[test]
    fn test_display_manager_select_by_index() {
        let mut manager = match DisplayManager::new(None) {
            Ok(m) => m,
            Err(_) => return,
        };
        assert!(manager.select_by_index(0).is_ok());
        let result = manager.select_by_index(999);
        assert!(matches!(result, Err(DisplayError::InvalidIndex { .. })));
    }

    #[test]
    fn test_display_manager_labels() {
        let mut manager = match DisplayManager::new(None) {
            Ok(m) => m,
            Err(_) => return,
        };
        assert!(manager
            .set_display_label(0, "test-label".to_string())
            .is_ok());
        assert_eq!(
            manager.list_displays()[0].label,
            Some("test-label".to_string())
        );
        assert_eq!(manager.list_displays()[0].display_name(), "test-label");
        assert!(manager.select_by_label("test-label").is_ok());
        assert_eq!(manager.selected_index(), 0);
        assert!(manager.remove_display_label(0).is_ok());
        assert!(manager.list_displays()[0].label.is_none());
    }

    #[test]
    fn test_display_manager_select_by_label_not_found() {
        let mut manager = match DisplayManager::new(None) {
            Ok(m) => m,
            Err(_) => return,
        };
        let result = manager.select_by_label("nonexistent-label");
        assert!(matches!(result, Err(DisplayError::LabelNotFound(_))));
    }

    #[test]
    fn test_display_manager_refresh() {
        let mut manager = match DisplayManager::new(None) {
            Ok(m) => m,
            Err(_) => return,
        };
        let result = manager.refresh();
        assert!(result.is_ok());
    }

    #[test]
    fn test_display_error_to_gentle_eye_error() {
        let display_err = DisplayError::NoDisplaysFound;
        let gentle_err: GentleEyeError = display_err.into();
        assert!(matches!(gentle_err, GentleEyeError::Recording(_)));
    }
}
