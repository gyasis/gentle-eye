//! `regions` — the Region ENGINE.
//!
//! A uniform [`Region`] currency + pluggable [`RegionProvider`]s organized into a
//! **capability cascade** (monitor → window → pane → element → text; cheapest /
//! most-structural source first, escalate to pixels only on a miss). Any consumer
//! (Lookout is the first) asks for regions without reimplementing detection.
//!
//! See `docs/REGION_ENGINE.md` and PRD `gentle_eye_region_engine_2026-07-01`.
//!
//! Build-out order: **E1 (this file)** = the model + trait + fusion.
//! **E2** = [`providers::wm`] (X11 EWMH window rects). E4 = the cascade resolver.

pub mod providers;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::target::model::PixelRect;

/// Where a [`Region`] came from — its detection source (drives `trust` + provenance).
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// Window manager (X11 EWMH) — exact top-level window rects.
    Wm,
    /// Accessibility tree (AT-SPI) — semantic element boxes.
    AtSpi,
    /// Saliency: a high-contrast rectangle vs a busy background.
    Contrast,
    /// opencv HoughLinesP — panel/pane dividers.
    Hough,
    /// gentle-eye variance-gutter segmenter.
    Segment,
    /// Learned element detector (YOLO).
    Yolo,
    /// Text detection (OCR).
    Ocr,
    /// Vision-language model — **index/attention only, never raw coordinates**.
    Vlm,
}

/// How big a region is — the cascade drills coarse → fine down this ladder.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Granularity {
    Monitor,
    Window,
    Pane,
    Element,
    Text,
}

/// Provider cost — orders the cascade (cheap/structural first, heavy last).
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Cost {
    /// WM, AT-SPI — ~zero compute, structural ground truth.
    Free,
    /// CV segment / contrast / OCR.
    Cheap,
    /// YOLO subprocess, VLM.
    Heavy,
}

/// The common currency every provider emits and every consumer reads.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
pub struct Region {
    /// Screen-absolute pixel box (callers clamp to the display).
    pub bbox: PixelRect,
    pub source: Source,
    pub granularity: Granularity,
    /// 0..1 — source-tier prior × source confidence. Drives escalation.
    pub trust: f32,
    /// Semantic role when known (AtSpi/Yolo): `"button"`, `"textbox"`, `"editor"`…
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Name / text content when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// The region this was drilled out of (the cascade edge).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<u64>,
    /// Full source chain when fused (seeded to `[source]`).
    pub provenance: Vec<Source>,
}

impl Region {
    /// A fresh region from a single source (provenance seeded to that source).
    pub fn new(bbox: PixelRect, source: Source, granularity: Granularity, trust: f32) -> Self {
        Region {
            bbox,
            source,
            granularity,
            trust,
            role: None,
            label: None,
            parent: None,
            provenance: vec![source],
        }
    }
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }
    pub fn with_role(mut self, role: impl Into<String>) -> Self {
        self.role = Some(role.into());
        self
    }
}

/// A pluggable detection source. Providers are ordered by (`granularity`, `cost`)
/// and consulted cheapest-first by the cascade resolver (E4). `probe` gates
/// whether a provider can answer inside a given region (e.g. WM only at the root).
pub trait RegionProvider {
    fn source(&self) -> Source;
    fn granularity(&self) -> Granularity;
    fn cost(&self) -> Cost;
    /// Can this provider answer inside `within`?
    fn probe(&self, within: &Region) -> bool;
    /// The child regions this provider finds inside `within`.
    fn regions(&self, within: &Region) -> Vec<Region>;
}

// ── fusion ───────────────────────────────────────────────────────────────────

/// Intersection-over-union of two pixel boxes, in `[0, 1]`.
pub fn iou(a: &PixelRect, b: &PixelRect) -> f32 {
    let (ax2, ay2) = (a.x + a.w, a.y + a.h);
    let (bx2, by2) = (b.x + b.w, b.y + b.h);
    let ix1 = a.x.max(b.x);
    let iy1 = a.y.max(b.y);
    let ix2 = ax2.min(bx2);
    let iy2 = ay2.min(by2);
    if ix2 <= ix1 || iy2 <= iy1 {
        return 0.0;
    }
    let inter = (ix2 - ix1) as f32 * (iy2 - iy1) as f32;
    let area_a = a.w as f32 * a.h as f32;
    let area_b = b.w as f32 * b.h as f32;
    let union = area_a + area_b - inter;
    if union <= 0.0 {
        0.0
    } else {
        inter / union
    }
}

/// Fuse overlapping regions (IoU-NMS): for each cluster of same-granularity boxes
/// with `iou >= thresh`, keep the **highest-trust** box and **merge** the
/// provenance of the ones it absorbs. Provenance/trust stay honest for downstream
/// escalation ("only a contrast box → verify") and "why is this wrong" reasoning.
pub fn fuse(mut regions: Vec<Region>, thresh: f32) -> Vec<Region> {
    // highest trust first → the survivor of each cluster is the most-trusted box.
    regions.sort_by(|a, b| {
        b.trust
            .partial_cmp(&a.trust)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut kept: Vec<Region> = Vec::new();
    'next: for r in regions {
        for k in kept.iter_mut() {
            if k.granularity == r.granularity && iou(&k.bbox, &r.bbox) >= thresh {
                for s in &r.provenance {
                    if !k.provenance.contains(s) {
                        k.provenance.push(*s);
                    }
                }
                continue 'next;
            }
        }
        kept.push(r);
    }
    kept
}

/// Minimal detector (pre-cascade, **E5**): gather regions from the applicable free
/// providers and [`fuse`] them. `depth` gates how deep to go — `Window` = WM only;
/// `Pane`/`Element`/`Text` also walk the AT-SPI tree. The full capability cascade
/// with per-hop escalation to CV/VLM is E4; this is the direct "give me the current
/// regions" call the CLI/MCP (and Lookout) use today.
pub fn detect(depth: Granularity) -> Vec<Region> {
    use crate::target::model::PixelRect;
    use providers::{atspi::AtSpiProvider, wm::WmProvider};

    // A monitor-granularity root so the window/element providers `probe` true.
    let root = Region::new(PixelRect { x: 0, y: 0, w: 0, h: 0 }, Source::Wm, Granularity::Monitor, 1.0);
    let mut all = WmProvider.regions(&root);
    if depth >= Granularity::Pane {
        all.extend(AtSpiProvider.regions(&root));
    }
    fuse(all, 0.7)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(x: u32, y: u32, w: u32, h: u32, src: Source, t: f32) -> Region {
        Region::new(PixelRect { x, y, w, h }, src, Granularity::Window, t)
    }

    #[test]
    fn iou_basics() {
        let a = PixelRect { x: 0, y: 0, w: 100, h: 100 };
        assert_eq!(iou(&a, &a), 1.0);
        let disjoint = PixelRect { x: 200, y: 200, w: 10, h: 10 };
        assert_eq!(iou(&a, &disjoint), 0.0);
        let half = PixelRect { x: 50, y: 0, w: 100, h: 100 }; // 5000 inter / 15000 union
        let v = iou(&a, &half);
        assert!((v - 0.3333).abs() < 0.01, "got {v}");
    }

    #[test]
    fn fuse_keeps_highest_trust_and_merges_provenance() {
        let regions = vec![
            win(0, 0, 100, 100, Source::Contrast, 0.40),
            win(2, 2, 98, 98, Source::Wm, 0.90), // ~same box, higher trust → survivor
            win(500, 0, 100, 100, Source::Wm, 0.90), // disjoint → survives separately
        ];
        let out = fuse(regions, 0.6);
        assert_eq!(out.len(), 2, "the two overlapping boxes fuse into one");
        let merged = out.iter().find(|x| x.bbox.x < 10).unwrap();
        assert_eq!(merged.source, Source::Wm, "highest-trust box wins the cluster");
        assert!(merged.provenance.contains(&Source::Contrast), "absorbed provenance kept");
        assert!(merged.provenance.contains(&Source::Wm));
    }

    #[test]
    fn region_builders() {
        let r = Region::new(PixelRect { x: 1, y: 2, w: 3, h: 4 }, Source::AtSpi, Granularity::Element, 0.95)
            .with_role("button")
            .with_label("Submit");
        assert_eq!(r.role.as_deref(), Some("button"));
        assert_eq!(r.label.as_deref(), Some("Submit"));
        assert_eq!(r.provenance, vec![Source::AtSpi]);
    }
}
