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

    /// A stable identity for this region, derived from what and where it is.
    ///
    /// **Not the vector index.** `parent` is an index into the slice a single
    /// capture produced, meaningless the moment it is written to a database:
    /// the next capture builds a different vector and index 3 means something
    /// else. Persisting an index gives provenance that looks precise and points
    /// at the wrong pane.
    ///
    /// Derived from `(display_id, granularity, bbox)` so it is deterministic,
    /// and so the SAME pane in two captures carries the SAME id — which is what
    /// makes "this entry came from the editor pane" answerable across a day
    /// rather than only within one frame.
    ///
    /// `granularity` is in the key because a maximized pane has the same box as
    /// the window containing it, which is a routine layout, not a corner case.
    /// Without it those two collide onto one id and "walk back up to the
    /// window" becomes ambiguous — a `parent_region_id` could equal a different
    /// region's `region_id` by construction.
    ///
    /// # A SPECIFIED hash, not `DefaultHasher`
    ///
    /// FNV-1a is written out here rather than using `DefaultHasher`, whose
    /// algorithm std explicitly does not guarantee across releases. That is
    /// fine for a `HashMap` and fatal for an identity **written to disk**: a
    /// toolchain upgrade would silently rebind every stored `region_id`, so
    /// pre-upgrade rows would stop matching post-upgrade captures — breaking
    /// exactly the cross-day query this exists for, with no error anywhere.
    ///
    /// # What it does NOT survive
    ///
    /// The box is exact, so a pane moved or resized by one pixel is a different
    /// region. Identity holds for pixel-stable layouts; detection jitter or a
    /// window drag starts a new identity, and that is a real limit rather than
    /// a bug to work around here.
    pub fn identity(&self) -> u64 {
        const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
        const PRIME: u64 = 0x0000_0100_0000_01b3;
        let mut h = OFFSET;
        let mut feed = |v: u64| {
            for b in v.to_le_bytes() {
                h ^= u64::from(b);
                h = h.wrapping_mul(PRIME);
            }
        };
        feed(u64::from(self.display_id));
        feed(self.granularity as u64);
        feed(u64::from(self.bbox.x));
        feed(u64::from(self.bbox.y));
        feed(u64::from(self.bbox.w));
        feed(u64::from(self.bbox.h));
        h
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

/// Assign a deterministic reading order to `regions` (FR-020).
///
/// Returns indices into `regions`, in the order a person reads them.
///
/// # Geometry, never a model
///
/// A model asked to order regions gives a different answer on the same input
/// from one run to the next, and there is no way to tell a re-ordering from a
/// re-render. Reading order is a property of where things are on screen, and
/// pixels already say where things are.
///
/// # Bounded by the parent tree
///
/// Siblings are ordered among themselves and each region is immediately
/// followed by its own subtree. Without that, two windows side by side get
/// their panes interleaved — window A's left pane, window B's left pane,
/// window A's right pane — an order describing no screen anyone saw.
///
/// # Recursive cuts, not a running band edge
///
/// Siblings are split by GAPS: a horizontal cut wherever no region crosses a
/// given y (giving rows), else a vertical cut wherever none crosses an x
/// (giving columns), recursively.
///
/// Two earlier attempts both used a running band edge and both failed, in
/// opposite directions, which is what makes the edge the wrong model rather
/// than the threshold wrong:
///
/// - ending a band at its TALLEST member let a full-height sidebar or window
///   absorb every row beside it, reading down the columns;
/// - ending it at its SHORTEST let a small chip pin the edge so low that a
///   column overlapping the next one by 480px was exiled to a later band,
///   reading right before left.
///
/// A cut asks a question about the whole set — "is there a line nothing
/// crosses?" — instead of accumulating an edge as regions arrive, so no single
/// member's height can distort it.
///
/// The cut is where nothing CROSSES, not where coverage has a hole: rows that
/// abut exactly still separate, because a region ending at y and one starting
/// at y both fail to cross it.
///
/// # Displays
///
/// Ordered before anything else: a top-left region on a portrait monitor is not
/// comparable to a bottom-right one on a laptop panel, and interleaving two
/// screens by geometry produces an order matching neither.
pub fn reading_order(regions: &[Region]) -> Vec<usize> {
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); regions.len()];
    let mut roots: Vec<usize> = Vec::new();
    for (i, r) in regions.iter().enumerate() {
        match r.parent.map(|p| p as usize) {
            // A parent index out of range, or pointing at itself, is treated as
            // no parent: the region still gets placed. Dropping it would leave
            // provenance that is not a map of the screen.
            Some(p) if p < regions.len() && p != i => children[p].push(i),
            _ => roots.push(i),
        }
    }

    // An EXPLICIT stack, not recursion. `Region` is `Deserialize` and this is a
    // pub fn, so a corrupt or adversarial region set can carry an arbitrarily
    // deep parent chain — and a stack overflow is an ABORT, not an `Err`: it
    // would kill the daemon with nothing to catch and nothing logged. Measured
    // before this change: a 8,000-deep chain aborted the process.
    enum Step {
        Emit(usize),
        Group(Vec<usize>),
    }
    let mut out = Vec::with_capacity(regions.len());
    let mut placed = vec![false; regions.len()];
    let mut stack = vec![Step::Group(roots)];

    while let Some(step) = stack.pop() {
        match step {
            Step::Emit(i) => {
                // The guard is defensive only. Every region has at most one
                // parent, so it lands in exactly one group — roots or one
                // `children[p]` — and is reached at most once; a mutation
                // replacing this branch with `panic!` passes the whole suite.
                // Cycles are handled by the fallback below, not here.
                if !placed[i] {
                    placed[i] = true;
                    out.push(i);
                }
            }
            Step::Group(group) => {
                // Pushed in reverse so popping yields: this region, then its
                // subtree, then the next sibling.
                for &i in cut_order(group, regions).iter().rev() {
                    if !children[i].is_empty() {
                        stack.push(Step::Group(children[i].clone()));
                    }
                    stack.push(Step::Emit(i));
                }
            }
        }
    }

    // A parent cycle leaves its members unreachable from any root — they are
    // never roots and never children of a root's subtree. They are placed here,
    // in index order, so the result is always a total ordering of the input.
    for (i, done) in placed.iter_mut().enumerate() {
        if !*done {
            *done = true;
            out.push(i);
        }
    }
    out
}

/// Order one sibling set by recursive horizontal-then-vertical cuts.
fn cut_order(group: Vec<usize>, regions: &[Region]) -> Vec<usize> {
    let mut out = Vec::with_capacity(group.len());
    let mut stack = vec![group];

    while let Some(g) = stack.pop() {
        if g.len() <= 1 {
            out.extend(g);
            continue;
        }
        // Displays first and absolutely: never compared by geometry.
        if let Some(parts) = split_by_display(&g, regions) {
            stack.extend(parts.into_iter().rev());
            continue;
        }
        if let Some(parts) = split(&g, regions, Axis::Y) {
            stack.extend(parts.into_iter().rev());
            continue;
        }
        if let Some(parts) = split(&g, regions, Axis::X) {
            stack.extend(parts.into_iter().rev());
            continue;
        }
        // Nothing separates these — genuinely overlapping content. Fall back to
        // a total, arrival-order-independent sort so the answer is still the
        // same on every run.
        let mut rest = g;
        rest.sort_by_key(|&i| (regions[i].bbox.y, regions[i].bbox.x, i));
        out.extend(rest);
    }
    out
}

enum Axis {
    X,
    Y,
}

fn split_by_display(group: &[usize], regions: &[Region]) -> Option<Vec<Vec<usize>>> {
    let mut ids: Vec<u32> = group.iter().map(|&i| regions[i].display_id).collect();
    ids.sort_unstable();
    ids.dedup();
    if ids.len() < 2 {
        return None;
    }
    Some(
        ids.into_iter()
            .map(|d| {
                group.iter().copied().filter(|&i| regions[i].display_id == d).collect()
            })
            .collect(),
    )
}

/// Split `group` wherever no region CROSSES the cut line on `axis`.
///
/// Returns `None` when the group cannot be cut on this axis.
fn split(group: &[usize], regions: &[Region], axis: Axis) -> Option<Vec<Vec<usize>>> {
    /// Where a region begins, and how far it extends, on the axis being cut.
    type Extent = fn(&Region) -> u32;
    let (start, size): (Extent, Extent) = match axis {
        Axis::Y => (|r| r.bbox.y, |r| r.bbox.h),
        Axis::X => (|r| r.bbox.x, |r| r.bbox.w),
    };
    let mut sorted = group.to_vec();
    // Ties broken by the other axis then by index, so the result cannot depend
    // on the order regions were detected in.
    sorted.sort_by_key(|&i| {
        let r = &regions[i];
        match axis {
            Axis::Y => (r.bbox.y, r.bbox.x, i),
            Axis::X => (r.bbox.x, r.bbox.y, i),
        }
    });

    // Split at the FIRST cut only, then let recursion find the rest. Cutting at
    // every gap in one pass mixes the axes: a sidebar beside a 2x2 grid has an
    // x-gap left of the content AND one between its columns, so a single pass
    // yields sidebar | left-column | right-column — reading down the columns.
    // Taking one cut and recursing lets the content be split into ROWS first,
    // which is what a person does.
    let mut reach = 0u32;
    for (n, &i) in sorted.iter().enumerate() {
        let r = &regions[i];
        // `>=` so exactly abutting rows separate: a region ending at y and one
        // starting at y both fail to CROSS y, which is a cut even though the
        // coverage has no hole in it.
        if n > 0 && start(r) >= reach {
            return Some(vec![sorted[..n].to_vec(), sorted[n..].to_vec()]);
        }
        reach = reach.max(start(r).saturating_add(size(r)));
    }
    None
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

    fn r_at(x: u32, y: u32, w: u32, h: u32, display: u32) -> Region {
        let mut r = Region::new(
            crate::target::model::PixelRect { x, y, w, h },
            Source::Wm,
            Granularity::Pane,
            0.8,
        );
        r.display_id = display;
        r
    }

    #[test]
    fn reading_order_is_identical_across_runs_on_the_same_input() {
        // FR-020 asks for determinism, which is the whole reason this is
        // geometry and not a model: a model gives a different answer on the
        // same input from one run to the next, and nothing can tell a
        // re-ordering from a re-render.
        let regions = vec![
            r_at(600, 10, 300, 400, 0),
            r_at(10, 10, 300, 400, 0),
            r_at(10, 500, 900, 200, 0),
            r_at(320, 12, 260, 400, 0),
        ];
        let first = reading_order(&regions);
        for _ in 0..20 {
            assert_eq!(reading_order(&regions), first, "same input, same order");
        }
        assert_eq!(first.len(), regions.len(), "every region is placed exactly once");
        let mut seen = first.clone();
        seen.sort_unstable();
        assert_eq!(seen, vec![0, 1, 2, 3], "no region dropped or duplicated");
    }

    #[test]
    fn side_by_side_panes_read_left_to_right_not_interleaved_by_pixel_row() {
        // Three columns whose tops differ by a few pixels. Sorting by `y` alone
        // interleaves them — the same corruption cropping exists to prevent,
        // arriving one layer later, as a timeline that reads across two
        // documents line by line.
        let regions = vec![
            r_at(640, 12, 300, 500, 0), // right column, 2px lower
            r_at(10, 10, 300, 500, 0),  // left column
            r_at(325, 14, 300, 500, 0), // middle column, 4px lower
        ];
        let order = reading_order(&regions);
        assert_eq!(
            order,
            vec![1, 2, 0],
            "left, middle, right — a raw y-sort would give right, left, middle"
        );
    }

    #[test]
    fn a_row_below_another_reads_after_all_of_it() {
        // Banding must not swallow everything into one band: a region genuinely
        // below the previous row starts a new one.
        let regions = vec![
            r_at(10, 600, 300, 200, 0), // second row, left
            r_at(400, 10, 300, 500, 0), // first row, right
            r_at(10, 10, 300, 500, 0),  // first row, left
            r_at(400, 600, 300, 200, 0), // second row, right
        ];
        assert_eq!(reading_order(&regions), vec![2, 1, 0, 3]);
    }

    #[test]
    fn displays_are_never_interleaved_by_geometry() {
        // A top-left region on a portrait panel is not comparable to a
        // bottom-right one on a laptop panel: bboxes are display-LOCAL, so two
        // screens ordered together by `y` produce an order matching neither.
        let regions = vec![
            r_at(10, 500, 300, 200, 1), // display 1, low
            r_at(10, 10, 300, 200, 0),  // display 0, high
            r_at(10, 20, 300, 200, 1),  // display 1, high
            r_at(10, 700, 300, 200, 0), // display 0, low
        ];
        let order = reading_order(&regions);
        assert_eq!(order, vec![1, 3, 2, 0], "all of display 0, then all of display 1");

        let displays: Vec<u32> = order.iter().map(|&i| regions[i].display_id).collect();
        assert_eq!(displays, vec![0, 0, 1, 1], "never interleaved");
    }

    #[test]
    fn the_order_does_not_depend_on_the_order_regions_arrived_in() {
        // Detection order is an accident of which provider ran first. If it
        // leaked into reading order, the same screen would read differently
        // depending on whether the WM or the OCR pass found a pane first.
        let a = r_at(10, 10, 300, 400, 0);
        let b = r_at(400, 12, 300, 400, 0);
        let c = r_at(10, 500, 690, 200, 0);

        let forward = vec![a.clone(), b.clone(), c.clone()];
        let backward = vec![c, b, a];

        let f: Vec<_> = reading_order(&forward).iter().map(|&i| forward[i].bbox).collect();
        let g: Vec<_> = reading_order(&backward).iter().map(|&i| backward[i].bbox).collect();
        assert_eq!(f, g, "the same screen must read the same way whatever order it arrived in");
    }


    #[test]
    fn a_window_containing_panes_reads_row_by_row_not_column_by_column() {
        // The shape `assign_parents` actually produces, and the one every
        // shipped fixture avoided. The window is full height, so a band that
        // ended at its TALLEST member absorbed every row beneath it into one
        // band — which then sorted column-major: down the left column, then
        // down the right. Wrong on the ordinary shape of a capture.
        let mut regions = vec![
            Region::new(
                crate::target::model::PixelRect { x: 0, y: 0, w: 1000, h: 1000 },
                Source::Wm,
                Granularity::Window,
                1.0,
            ),
            r_at(10, 10, 480, 480, 0),   // top-left
            r_at(510, 10, 480, 480, 0),  // top-right
            r_at(10, 510, 480, 480, 0),  // bottom-left
            r_at(510, 510, 480, 480, 0), // bottom-right
        ];
        assign_parents(&mut regions);

        let order = reading_order(&regions);
        assert_eq!(
            order,
            vec![0, 1, 2, 3, 4],
            "window, then top-left, top-right, bottom-left, bottom-right"
        );
    }

    #[test]
    fn a_full_height_sidebar_does_not_swallow_the_rows_beside_it() {
        // No nesting needed: ANY tall region did this. The sidebar spans both
        // content rows, so a tallest-member band merged them and read down the
        // columns instead of across the rows.
        let regions = vec![
            r_at(0, 0, 200, 1000, 0),    // sidebar, full height
            r_at(220, 0, 380, 480, 0),   // row 1 left
            r_at(620, 0, 380, 480, 0),   // row 1 right
            r_at(220, 520, 380, 480, 0), // row 2 left
            r_at(620, 520, 380, 480, 0), // row 2 right
        ];
        let order = reading_order(&regions);
        assert_eq!(
            order,
            vec![0, 1, 2, 3, 4],
            "sidebar, then row 1 left-to-right, then row 2 — not down the columns"
        );
    }

    #[test]
    fn two_windows_side_by_side_never_interleave_their_panes() {
        // Ordering bounded by the parent tree (T034's actual words). Without
        // it: window A's left pane, window B's left pane, window A's right
        // pane — an order describing no screen anyone saw.
        let mut regions = vec![
            Region::new(
                crate::target::model::PixelRect { x: 0, y: 0, w: 500, h: 1000 },
                Source::Wm,
                Granularity::Window,
                1.0,
            ),
            Region::new(
                crate::target::model::PixelRect { x: 500, y: 0, w: 500, h: 1000 },
                Source::Wm,
                Granularity::Window,
                1.0,
            ),
            r_at(10, 10, 480, 480, 0),   // A top
            r_at(510, 10, 480, 480, 0),  // B top
            r_at(10, 510, 480, 480, 0),  // A bottom
            r_at(510, 510, 480, 480, 0), // B bottom
        ];
        assign_parents(&mut regions);
        let order = reading_order(&regions);

        let a_panes: Vec<usize> = order.iter().copied().filter(|&i| i == 2 || i == 4).collect();
        let b_panes: Vec<usize> = order.iter().copied().filter(|&i| i == 3 || i == 5).collect();
        let a_last = order.iter().position(|&i| i == a_panes[1]).unwrap();
        let b_first = order.iter().position(|&i| i == b_panes[0]).unwrap();
        assert!(
            a_last < b_first,
            "all of window A's panes precede all of window B's: {order:?}"
        );
    }

    #[test]
    fn a_region_starting_exactly_where_the_band_ends_begins_a_new_row() {
        // `<` versus `<=` on the band boundary is one character and no fixture
        // had exact abutment — a mutation flipping it survived. Rows that touch
        // exactly are the common case in a tiled layout, not a curiosity.
        let regions = vec![
            r_at(0, 0, 100, 100, 0),
            r_at(200, 0, 100, 100, 0),
            r_at(0, 100, 100, 100, 0), // starts exactly where row 1 ends
            r_at(200, 100, 100, 100, 0),
        ];
        assert_eq!(
            reading_order(&regions),
            vec![0, 1, 2, 3],
            "abutting rows are two rows, not one band read column-wise"
        );
    }

    #[test]
    fn a_window_and_a_maximized_pane_inside_it_have_different_identities() {
        // A maximized pane has the same box as its window. Hashing geometry
        // alone collides them, so two provenance rows carry one region_id and
        // "walk back up to the window" becomes ambiguous.
        let bbox = crate::target::model::PixelRect { x: 0, y: 0, w: 1920, h: 1080 };
        let window = Region::new(bbox, Source::Wm, Granularity::Window, 1.0);
        let pane = Region::new(bbox, Source::Wm, Granularity::Pane, 0.9);
        assert_ne!(
            window.identity(),
            pane.identity(),
            "same box, different thing — the ids must differ"
        );
    }

    #[test]
    fn identity_is_a_fixed_value_not_whatever_the_toolchain_hashes_to() {
        // `DefaultHasher`'s algorithm is explicitly unguaranteed across std
        // releases. That is fine for a HashMap and fatal for an id WRITTEN TO
        // DISK: a toolchain upgrade would silently rebind every stored
        // region_id and pre-upgrade rows would stop matching new captures, with
        // no error. Pinning the value is what makes the guarantee real.
        let r = Region::new(
            crate::target::model::PixelRect { x: 100, y: 200, w: 300, h: 400 },
            Source::Wm,
            Granularity::Pane,
            0.8,
        );
        assert_eq!(
            r.identity(),
            0x5acd_062a_2f31_e657,
            "the identity of a given region must never change; if this fails, \
             every region_id already written to disk has been orphaned"
        );
    }


    #[test]
    fn a_small_chip_above_a_column_does_not_exile_the_column_beside_it() {
        // The INVERSE of the sidebar failure, and the reason a running band
        // edge is the wrong model rather than the threshold being wrong.
        //
        // Ending a band at its shortest member let this 10px chip pin the edge
        // at y=10, so the left column — which overlaps the right one for 480 of
        // its 488 pixels — started a new band and was read AFTER it. Right
        // before left, on a layout with no nesting and nothing unusual about it.
        let regions = vec![
            r_at(600, 0, 100, 10, 0),   // a small chip, top-right
            r_at(600, 4, 300, 496, 0),  // right column, starts just below it
            r_at(0, 12, 300, 488, 0),   // left column, overlaps the right by ~480px
        ];
        let order = reading_order(&regions);
        assert_eq!(
            order,
            vec![2, 0, 1],
            "left column first, then the chip and the right column: {order:?}"
        );
    }

    #[test]
    fn a_short_region_inside_a_tall_ones_span_does_not_end_the_group() {
        // `reach` must be the MAXIMUM extent seen, not the last member's.
        // Tracking only the last one lets a short region inside a tall one's
        // span drop the reach to its own bottom, so the next region — still
        // well inside the tall one — is cut off into a new row.
        //
        // Two regions cannot show this: the difference needs a tall region, a
        // short one within its span, and a third below the SHORT one's bottom
        // but still inside the TALL one's. A mutation replacing max with the
        // last member survived the whole suite until this fixture existed.
        let regions = vec![
            r_at(300, 0, 100, 1000, 0), // 0: tall right column
            r_at(300, 10, 100, 20, 0),  // 1: short, inside its span, ends at 30
            r_at(0, 40, 100, 50, 0),    // 2: left, below 1's bottom, inside 0's span
        ];
        let order = reading_order(&regions);
        assert_eq!(
            order,
            vec![2, 0, 1],
            "the left region is read first — it sits beside the tall column, not \
             below it: {order:?}"
        );
    }

    #[test]
    fn three_levels_of_nesting_keep_each_subtree_together() {
        // Monitor -> window -> pane -> element. Two recursion levels, which no
        // fixture exercised: a pane's elements must stay with their pane rather
        // than being ordered against the other pane's elements.
        let mut regions = vec![
            Region::new(
                crate::target::model::PixelRect { x: 0, y: 0, w: 1000, h: 600 },
                Source::Wm,
                Granularity::Window,
                1.0,
            ),
            r_at(10, 10, 480, 580, 0),  // 1: left pane
            r_at(510, 10, 480, 580, 0), // 2: right pane
            r_at(20, 20, 200, 40, 0),   // 3: element in the left pane
            r_at(20, 80, 200, 40, 0),   // 4: element in the left pane
            r_at(520, 20, 200, 40, 0),  // 5: element in the right pane
        ];
        assign_parents(&mut regions);
        let order = reading_order(&regions);
        assert_eq!(
            order,
            vec![0, 1, 3, 4, 2, 5],
            "window, left pane and ITS elements, then right pane and its own: {order:?}"
        );
    }

    #[test]
    fn a_deeply_nested_region_set_does_not_abort_the_process() {
        // `Region` is Deserialize and this is a pub fn, so a corrupt or
        // adversarial region set can carry an arbitrarily deep parent chain.
        // Recursion made that a STACK OVERFLOW — an abort, not an `Err`: it
        // kills the daemon with nothing to catch and nothing logged. Measured
        // before the explicit-stack rewrite: 8,000 deep aborted the process.
        let depth = 50_000;
        let mut regions: Vec<Region> = (0..depth).map(|i| r_at(0, i as u32, 10, 10, 0)).collect();
        for (i, r) in regions.iter_mut().enumerate().skip(1) {
            r.parent = Some((i - 1) as u64);
        }
        let order = reading_order(&regions);
        assert_eq!(order.len(), depth, "every region placed, and the process survived");
        assert_eq!(order[0], 0, "the chain reads from its root");
        assert_eq!(order[depth - 1], depth - 1);
    }

    #[test]
    fn a_parent_cycle_still_places_every_region() {
        // Nothing in the type system prevents a hand-built or deserialized
        // region set from containing a cycle. Recursing forever, or silently
        // dropping the cycle members, would both be worse than emitting them:
        // provenance that omits regions is not a map of the screen.
        let mut regions = vec![r_at(0, 0, 10, 10, 0), r_at(20, 0, 10, 10, 0)];
        regions[0].parent = Some(1);
        regions[1].parent = Some(0);
        let mut order = reading_order(&regions);
        order.sort_unstable();
        assert_eq!(order, vec![0, 1], "every region placed exactly once");

        // and a parent index that points nowhere
        let mut dangling = vec![r_at(0, 0, 10, 10, 0)];
        dangling[0].parent = Some(99);
        assert_eq!(reading_order(&dangling), vec![0]);
    }

    #[test]
    fn an_empty_or_single_region_set_is_handled() {
        assert!(reading_order(&[]).is_empty());
        assert_eq!(reading_order(&[r_at(5, 5, 10, 10, 0)]), vec![0]);
    }

}
