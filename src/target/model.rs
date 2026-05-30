//! Target data model: normalized regions, pixel regions, sources, and the
//! persisted [`Target`].
//!
//! Coordinates are **normalized 0–1** at the model layer (resolution-agnostic,
//! the contract the agent speaks). Conversion to absolute pixels lives in
//! [`super::geometry`].

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A region-of-interest in **normalized** 0–1 coordinates, relative to its
/// capture source. `(x, y)` is the top-left corner; `(w, h)` the size.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq)]
pub struct NormRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl NormRect {
    /// Construct a normalized rect (not validated — call [`Self::is_valid`]).
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self { x, y, w, h }
    }

    /// True iff the rect lies within the unit square with positive area.
    ///
    /// A tiny epsilon slack on the far edges absorbs float round-off so a
    /// full-width/height box (`x+w == 1.0`) is accepted.
    pub fn is_valid(&self) -> bool {
        const EPS: f64 = 1e-9;
        (0.0..=1.0).contains(&self.x)
            && (0.0..=1.0).contains(&self.y)
            && self.w > 0.0
            && self.h > 0.0
            && self.x + self.w <= 1.0 + EPS
            && self.y + self.h <= 1.0 + EPS
    }
}

/// A region in absolute pixels on a source.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// What a target is cropped from.
///
/// Struct variants (rather than tuple variants) so the type derives a clean,
/// internally-tagged `JsonSchema` — tuple variants aren't supported by
/// `#[serde(tag = ...)]` and read poorly as MCP tool input.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TargetSource {
    /// A local display, by enumeration index (0 = first display).
    Display { index: usize },
    /// A stream URL (`rtsp://` / `rtmp://` / `http(s)://` / `srt://` / …).
    Stream { url: String },
}

/// A named, persisted region-of-interest. **One** target is `active` at a time
/// (enforced by [`super::store::TargetStore`]).
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
pub struct Target {
    pub name: String,
    pub source: TargetSource,
    pub region: NormRect,
    #[serde(default)]
    pub active: bool,
}

impl Target {
    /// Construct an inactive target.
    pub fn new(name: impl Into<String>, source: TargetSource, region: NormRect) -> Self {
        Self {
            name: name.into(),
            source,
            region,
            active: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn norm_rect_validity() {
        assert!(NormRect::new(0.0, 0.0, 1.0, 1.0).is_valid());
        assert!(NormRect::new(0.25, 0.5, 0.5, 0.25).is_valid());
        assert!(!NormRect::new(-0.1, 0.0, 0.5, 0.5).is_valid()); // negative origin
        assert!(!NormRect::new(0.6, 0.0, 0.5, 0.5).is_valid()); // x+w > 1
        assert!(!NormRect::new(0.0, 0.0, 0.0, 0.5).is_valid()); // zero width
    }

    #[test]
    fn target_serde_round_trip() {
        let t = Target::new(
            "editor",
            TargetSource::Display { index: 1 },
            NormRect::new(0.5, 0.0, 0.25, 1.0),
        );
        let json = serde_json::to_string(&t).unwrap();
        let back: Target = serde_json::from_str(&json).unwrap();
        assert_eq!(t, back);

        // Stream variant round-trips too.
        let s = Target::new(
            "cam",
            TargetSource::Stream {
                url: "rtsp://cam/live".into(),
            },
            NormRect::new(0.0, 0.0, 0.5, 0.5),
        );
        let back: Target = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(s, back);
    }
}
