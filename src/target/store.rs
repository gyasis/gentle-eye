//! Target persistence + active-target selection.
//!
//! TG2 — persisted at `~/.config/gentle-eye/targets.json`, mirroring
//! `capture::display::DisplayConfig::{load,save}` (same dir, same `HOME`
//! resolution, same pretty-JSON-on-disk shape). **One** target is active at a
//! time; `set_active` clears any prior active flag.

use super::errors::TargetError;
use super::model::Target;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// The on-disk target catalogue. Construct via [`TargetStore::load`].
#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq)]
pub struct TargetStore {
    #[serde(default)]
    targets: Vec<Target>,
}

impl TargetStore {
    /// `~/.config/gentle-eye/targets.json` (sibling of `display.json`).
    fn config_path() -> Result<PathBuf, TargetError> {
        let home = std::env::var("HOME")
            .map_err(|_| TargetError::Config("Could not find HOME directory".into()))?;
        Ok(PathBuf::from(home).join(".config/gentle-eye/targets.json"))
    }

    /// Load the catalogue, or an empty one if the file doesn't exist yet.
    pub fn load() -> Result<Self, TargetError> {
        let path = Self::config_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = fs::read_to_string(path)?;
        Ok(serde_json::from_str(&content)?)
    }

    /// Persist the catalogue (creates `~/.config/gentle-eye/` if needed).
    pub fn save(&self) -> Result<(), TargetError> {
        let path = Self::config_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let serialized = serde_json::to_string_pretty(self)?;
        fs::write(path, serialized)?;
        Ok(())
    }

    /// All targets, in insertion order.
    pub fn list(&self) -> &[Target] {
        &self.targets
    }

    /// Add (or replace by name) a target. Does not change the active selection
    /// unless the incoming target is itself `active` (then it wins, exclusively).
    pub fn add(&mut self, target: Target) {
        let make_active = target.active;
        self.targets.retain(|t| t.name != target.name);
        self.targets.push(target);
        if make_active {
            let name = self.targets.last().unwrap().name.clone();
            self.set_active_unchecked(&name);
        }
    }

    /// Remove a target by name. Returns whether one was removed.
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.targets.len();
        self.targets.retain(|t| t.name != name);
        self.targets.len() != before
    }

    /// Make exactly one target active (clearing any prior). Errors if `name`
    /// isn't in the store.
    pub fn set_active(&mut self, name: &str) -> Result<(), TargetError> {
        if !self.targets.iter().any(|t| t.name == name) {
            return Err(TargetError::NotFound(name.to_string()));
        }
        self.set_active_unchecked(name);
        Ok(())
    }

    fn set_active_unchecked(&mut self, name: &str) {
        for t in &mut self.targets {
            t.active = t.name == name;
        }
    }

    /// The currently-active target, if any.
    pub fn active(&self) -> Option<&Target> {
        self.targets.iter().find(|t| t.active)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::model::{NormRect, TargetSource};

    fn t(name: &str) -> Target {
        Target::new(
            name,
            TargetSource::Display { index: 0 },
            NormRect::new(0.0, 0.0, 0.5, 0.5),
        )
    }

    #[test]
    fn one_active_at_a_time() {
        let mut s = TargetStore::default();
        s.add(t("a"));
        s.add(t("b"));
        s.set_active("a").unwrap();
        s.set_active("b").unwrap();
        let active: Vec<&str> = s.list().iter().filter(|x| x.active).map(|x| x.name.as_str()).collect();
        assert_eq!(active, vec!["b"]); // exactly one, the most-recently-set
        assert_eq!(s.active().unwrap().name, "b");
    }

    #[test]
    fn set_active_unknown_errors() {
        let mut s = TargetStore::default();
        assert!(matches!(s.set_active("nope"), Err(TargetError::NotFound(_))));
    }

    #[test]
    fn remove_active_clears_active() {
        let mut s = TargetStore::default();
        s.add(t("a"));
        s.set_active("a").unwrap();
        assert!(s.remove("a"));
        assert!(s.active().is_none());
    }

    #[test]
    fn add_replaces_by_name() {
        let mut s = TargetStore::default();
        s.add(t("a"));
        s.add(t("a"));
        assert_eq!(s.list().len(), 1);
    }
}
