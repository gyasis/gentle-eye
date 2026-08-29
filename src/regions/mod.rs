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
    /// Pixel box in the coordinate space of [`Region::display_id`] — see that
    /// field for the invariant and its current producer gap.
    ///
    /// (This line previously read "screen-absolute", contradicting the
    /// invariant fifteen lines below. Consumers were reading whichever half
    /// they happened to see first, so the wrong one is corrected rather than
    /// left for the next reader to pick between.)
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
    /// Which display this region lives on.
    ///
    /// Required once more than one display is captured: two regions with
    /// identical bboxes on different screens are otherwise indistinguishable,
    /// and reading order must sort WITHIN a display before merging across them
    /// — a top-left region on a portrait panel is not comparable to a
    /// bottom-right one on a laptop panel.
    ///
    /// `bbox` is expressed in the coordinate space of THIS `display_id` —
    /// display-local, not virtual-desktop global. On a desk whose displays sit
    /// at x-offsets 0, 1920 and 5360 the global form would make every crop and
    /// every comparison carry an offset it does not need.
    ///
    /// **Current producer gap, stated rather than hidden:** the WM provider
    /// (`providers::wm`) translates window coordinates to ROOT, and emits
    /// `display_id: 0`. That is self-consistent while only one capture surface
    /// exists — display 0 is effectively the whole virtual desktop — but it is
    /// NOT display-local, and the per-display sampler (T010/T011) must set both
    /// the id and the local origin when it captures each screen separately.
    /// Until then, treat multi-display geometry from `detect()` as unpopulated
    /// rather than as satisfying this invariant.
    #[serde(default)]
    pub display_id: u32,
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
            display_id: 0,
            provenance: vec![source],
        }
    }
    /// Set the display this region lives on.
    pub fn on_display(mut self, display_id: u32) -> Self {
        self.display_id = display_id;
        self
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
            // Display FIRST: two regions with identical display-local geometry on
            // different screens have IoU 1.0. Without this guard they fuse into
            // one and a whole display vanishes from the timeline, silently.
            if k.display_id == r.display_id
                && k.granularity == r.granularity
                && iou(&k.bbox, &r.bbox) >= thresh
            {
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
    let mut out = fuse(all, 0.7);
    assign_parents(&mut out);
    out
}

// ── cascade resolution (E4) ──────────────────────────────────────────────────

fn contains(outer: &crate::target::model::PixelRect, inner: &crate::target::model::PixelRect) -> bool {
    outer.x <= inner.x
        && outer.y <= inner.y
        && outer.x + outer.w >= inner.x + inner.w
        && outer.y + outer.h >= inner.y + inner.h
}
fn same_box(a: &crate::target::model::PixelRect, b: &crate::target::model::PixelRect) -> bool {
    a.x == b.x && a.y == b.y && a.w == b.w && a.h == b.h
}

/// Link each region to its tightest container (a coarser-or-equal region that
/// strictly contains it) via `parent` = the container's index in `regions`. This
/// turns the flat fused list into the trickle-down hierarchy (window → pane →
/// element) consumers drill through.
pub fn assign_parents(regions: &mut [Region]) {
    let n = regions.len();
    let boxes: Vec<_> = regions.iter().map(|r| r.bbox).collect();
    let gran: Vec<_> = regions.iter().map(|r| r.granularity).collect();
    let disp: Vec<_> = regions.iter().map(|r| r.display_id).collect();
    for i in 0..n {
        let mut best: Option<usize> = None;
        let mut best_area = u64::MAX;
        for j in 0..n {
            if i == j {
                continue;
            }
            // Containment is only meaningful WITHIN a display: bboxes are
            // display-local, so a pane on one screen can sit "inside" a window
            // on another purely by coordinate coincidence.
            if disp[j] == disp[i]
                && gran[j] <= gran[i]
                && contains(&boxes[j], &boxes[i])
                && !same_box(&boxes[j], &boxes[i])
            {
                let area = boxes[j].w as u64 * boxes[j].h as u64;
                if area < best_area {
                    best_area = area;
                    best = Some(j);
                }
            }
        }
        regions[i].parent = best.map(|j| j as u64);
    }
}

/// Resolve a natural-language target to a region index — **structural + geometric,
/// no VLM**. Handles ordinals ("second pane"), position ("left/right/top/bottom/
/// middle"), and label/role match ("the terminal", "Cancel button"). Returns the
/// best-matching region's index, or `None`. (Pixel/VLM escalation is E7–E10;
/// this is the `gentle-eye locate` primitive over the structural region set.)
pub fn locate(query: &str, regions: &[Region]) -> Option<usize> {
    if regions.is_empty() {
        return None;
    }
    let low = query.to_lowercase();
    // Narrow the candidate pool by an explicit noun, if the query names one.
    let want: Option<Granularity> = if low.contains("window") {
        Some(Granularity::Window)
    } else if low.contains("pane") || low.contains("panel") {
        Some(Granularity::Pane)
    } else if low.contains("button") || low.contains("icon") || low.contains("menu") {
        Some(Granularity::Element)
    } else if low.contains("label") || low.contains(" text") {
        Some(Granularity::Text)
    } else {
        None
    };
    let filtered: Vec<usize> = (0..regions.len())
        .filter(|&i| want.map_or(true, |g| regions[i].granularity == g))
        .collect();
    let pool = if filtered.is_empty() {
        (0..regions.len()).collect::<Vec<_>>()
    } else {
        filtered
    };
    // 1) ordinal / positional geometric pick, then 2) label/role match.
    spatial_pick(&low, regions, &pool).or_else(|| label_match(&low, regions, &pool))
}

/// OCR fallback for `locate` (Phase-3-lite): when the structural pick fails, crop the FRAME to each
/// region and pick the region whose on-screen TEXT best matches the query's content words. Enables
/// content-named focus ("the youtube panel") in no-a11y apps where WM titles / AT-SPI don't carry
/// the word. `frame_png` is the caller's screenshot of the watched display (display-local pixels);
/// `origin` is that display's screen origin, so screen-absolute region bboxes map to frame-local
/// crops. Returns the best-scoring region index, or `None` if nothing matches.
pub fn locate_ocr(query: &str, regions: &[Region], frame_png: &str, origin: (i64, i64)) -> Option<usize> {
    let words = content_words(query);
    if words.is_empty() || regions.is_empty() {
        return None;
    }
    let img = image::open(frame_png).ok()?;
    let (fw, fh) = (img.width() as i64, img.height() as i64);
    let mut best: Option<(usize, usize)> = None; // (region index, match score)
    for (i, r) in regions.iter().enumerate() {
        let x = (r.bbox.x as i64 - origin.0).clamp(0, fw - 1);
        let y = (r.bbox.y as i64 - origin.1).clamp(0, fh - 1);
        let w = (r.bbox.w as i64).min(fw - x).max(1);
        let h = (r.bbox.h as i64).min(fh - y).max(1);
        let crop = img.crop_imm(x as u32, y as u32, w as u32, h as u32);
        let tmp = std::env::temp_dir().join(format!("ge-locate-ocr-{i}.png"));
        if crop.save(&tmp).is_err() {
            continue;
        }
        let text = crate::analysis::ocr::ocr_image(&tmp).unwrap_or_default().to_lowercase();
        let _ = std::fs::remove_file(&tmp);
        let score = words.iter().filter(|w| text.contains(w.as_str())).count();
        if score > 0 && best.map_or(true, |(_, bs)| score > bs) {
            best = Some((i, score));
        }
    }
    best.map(|(i, _)| i)
}

/// Query words minus focus-verbs + generic region nouns → the meaningful CONTENT words to match
/// against on-screen text (so "focus on the youtube panel" → `["youtube"]`).
fn content_words(query: &str) -> Vec<String> {
    const STOP: &[&str] = &[
        "focus", "the", "please", "show", "watch", "look", "into", "that", "this", "onto", "lock",
        "grab", "zoom", "capture", "panel", "pane", "window", "region", "screen", "view", "part",
        "section", "area", "thing", "side", "left", "right", "top", "bottom", "middle", "center",
        "centre", "upper", "lower", "first", "second", "third", "fourth", "fifth",
    ];
    query
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 2 && !STOP.contains(w))
        .map(|w| w.to_string())
        .collect()
}

fn spatial_pick(low: &str, regions: &[Region], pool: &[usize]) -> Option<usize> {
    let vertical = ["top", "bottom", "upper", "lower"].iter().any(|k| low.contains(k));
    let mut order = pool.to_vec();
    if vertical {
        order.sort_by_key(|&i| (regions[i].bbox.y, regions[i].bbox.x));
    } else {
        order.sort_by_key(|&i| (regions[i].bbox.x, regions[i].bbox.y));
    }
    let n = order.len();
    if n == 0 {
        return None;
    }
    let hit = |ks: &[&str]| ks.iter().any(|k| low.contains(k));
    let p = if let Some(k) = ordinal_position(low) {
        k.saturating_sub(1).min(n - 1)
    } else if hit(&["last", "final", "rightmost", "far right", "right side", "the right"]) {
        n - 1
    } else if hit(&["leftmost", "far left", "left side", "the left"]) {
        0
    } else if hit(&["middle", "center", "centre", "central"]) {
        n / 2
    } else if vertical && hit(&["top", "upper"]) {
        0
    } else if vertical && hit(&["bottom", "lower"]) {
        n - 1
    } else {
        return None;
    };
    Some(order[p])
}

fn label_match(low: &str, regions: &[Region], pool: &[usize]) -> Option<usize> {
    let mut best: Option<usize> = None;
    let mut best_score = 0usize;
    for &i in pool {
        let label = regions[i].label.as_deref().unwrap_or("").to_lowercase();
        let role = regions[i].role.as_deref().unwrap_or("").to_lowercase();
        let hay = format!("{label} {role}");
        if hay.trim().is_empty() {
            continue;
        }
        let tokens = low.split_whitespace().filter(|w| w.len() > 2 && hay.contains(*w)).count();
        let contained = !label.is_empty() && (low.contains(&label) || label.contains(low.trim()));
        let score = tokens + if contained { 3 } else { 0 };
        if score > best_score {
            best_score = score;
            best = Some(i);
        }
    }
    (best_score > 0).then_some(best).flatten()
}

/// Parse a 1-based ordinal/cardinal position ("second" / "2nd" / "pane 2" / "#3").
fn ordinal_position(low: &str) -> Option<usize> {
    const WORDS: &[(&str, usize)] = &[
        ("first", 1), ("1st", 1), ("second", 2), ("2nd", 2), ("third", 3), ("3rd", 3),
        ("fourth", 4), ("4th", 4), ("fifth", 5), ("5th", 5), ("sixth", 6), ("6th", 6),
        ("seventh", 7), ("7th", 7), ("eighth", 8), ("8th", 8), ("ninth", 9), ("9th", 9),
        ("tenth", 10), ("10th", 10),
    ];
    for (w, k) in WORDS {
        if low.contains(w) {
            return Some(*k);
        }
    }
    for tok in low.split(|c: char| !c.is_ascii_digit()) {
        if let Ok(k) = tok.parse::<usize>() {
            if (1..=12).contains(&k) {
                return Some(k);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(x: u32, y: u32, w: u32, h: u32, src: Source, t: f32) -> Region {
        Region::new(PixelRect { x, y, w, h }, src, Granularity::Window, t)
    }

    #[test]
    fn fuse_must_not_merge_identical_geometry_across_displays() {
        // THE bug this guard exists for: bboxes are display-local, so the same
        // pane position on two screens has IoU 1.0. Without a display check
        // `fuse` collapses them into one region and an entire display vanishes
        // from the timeline — silently, which is the worst kind.
        let bbox = PixelRect { x: 100, y: 100, w: 400, h: 300 };
        let regions = vec![
            Region::new(bbox, Source::Wm, Granularity::Pane, 0.9).on_display(0),
            Region::new(bbox, Source::Wm, Granularity::Pane, 0.8).on_display(2),
        ];
        assert_eq!(iou(&bbox, &bbox), 1.0, "precondition: geometry is identical");

        let kept = fuse(regions, 0.5);
        assert_eq!(kept.len(), 2, "both displays must survive fusion, got {kept:?}");
        let mut displays: Vec<u32> = kept.iter().map(|r| r.display_id).collect();
        displays.sort_unstable();
        assert_eq!(displays, vec![0, 2]);
    }

    #[test]
    fn fuse_still_merges_duplicates_on_the_same_display() {
        // The guard must not disable fusion outright — same screen, overlapping
        // boxes from two providers should still collapse to one.
        let a = PixelRect { x: 0, y: 0, w: 100, h: 100 };
        let b = PixelRect { x: 2, y: 2, w: 100, h: 100 };
        let kept = fuse(
            vec![
                Region::new(a, Source::Wm, Granularity::Window, 0.9).on_display(1),
                Region::new(b, Source::AtSpi, Granularity::Window, 0.6).on_display(1),
            ],
            0.5,
        );
        assert_eq!(kept.len(), 1, "same display + high IoU must still fuse");
        assert_eq!(kept[0].display_id, 1);
        assert!(kept[0].provenance.len() >= 2, "provenance must merge");
    }

    #[test]
    fn a_region_is_never_parented_to_one_on_another_display() {
        // Containment is only meaningful within a display. A pane at (10,10) on
        // display 1 sits "inside" a window at (0,0,500,500) on display 0 purely
        // by coordinate coincidence.
        let mut regions = vec![
            Region::new(
                PixelRect { x: 0, y: 0, w: 500, h: 500 },
                Source::Wm,
                Granularity::Window,
                0.9,
            )
            .on_display(0),
            Region::new(
                PixelRect { x: 10, y: 10, w: 100, h: 100 },
                Source::Wm,
                Granularity::Pane,
                0.9,
            )
            .on_display(1),
        ];
        assign_parents(&mut regions);
        assert_eq!(
            regions[1].parent, None,
            "a pane on display 1 must not be parented to a window on display 0"
        );

        // ...and the same pane ON display 0 IS parented, so the guard did not
        // simply disable parenting.
        regions[1].display_id = 0;
        assign_parents(&mut regions);
        assert_eq!(regions[1].parent, Some(0), "same display must still parent");
    }

    #[test]
    fn a_region_defaults_to_the_first_display_and_on_display_sets_it() {
        let r = Region::new(
            PixelRect { x: 0, y: 0, w: 10, h: 10 },
            Source::Wm,
            Granularity::Window,
            1.0,
        );
        assert_eq!(r.display_id, 0, "single-display callers need no change");
        assert_eq!(r.on_display(3).display_id, 3);
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

    #[test]
    fn locate_ordinal_positional_and_label() {
        let regions = vec![
            Region::new(PixelRect { x: 0, y: 0, w: 100, h: 500 }, Source::AtSpi, Granularity::Pane, 0.9),
            Region::new(PixelRect { x: 200, y: 0, w: 100, h: 500 }, Source::AtSpi, Granularity::Pane, 0.9),
            Region::new(PixelRect { x: 400, y: 0, w: 100, h: 500 }, Source::AtSpi, Granularity::Pane, 0.9),
            Region::new(PixelRect { x: 50, y: 50, w: 60, h: 20 }, Source::AtSpi, Granularity::Element, 0.9)
                .with_role("push button")
                .with_label("Cancel"),
        ];
        assert_eq!(locate("focus on the second pane", &regions), Some(1)); // ordinal
        assert_eq!(locate("the rightmost pane", &regions), Some(2)); // positional
        assert_eq!(locate("the Cancel button", &regions), Some(3)); // label/role
    }

    #[test]
    fn assign_parents_nests_by_containment() {
        let mut regions = vec![
            Region::new(PixelRect { x: 0, y: 0, w: 1000, h: 1000 }, Source::Wm, Granularity::Window, 0.98),
            Region::new(PixelRect { x: 100, y: 100, w: 200, h: 200 }, Source::AtSpi, Granularity::Element, 0.9),
        ];
        assign_parents(&mut regions);
        assert_eq!(regions[1].parent, Some(0), "element nests under the window");
        assert_eq!(regions[0].parent, None, "window has no container");
    }
}
