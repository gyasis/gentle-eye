//! `ContrastProvider` (E8) — window/content-level regions by CHROMA saliency.
//!
//! Isolates the bounding box of low-saturation "content" (white/gray UI, text)
//! against a saturated background (a colored desktop wallpaper). Grayscale variance
//! fails here — a textured wallpaper has as many edges as a window — but a **color
//! channel separates cleanly**: window UI is desaturated, the wallpaper is not.
//! (Validated in OpenCV on a real textured desktop: saturation masking isolated the
//! window at IoU 0.86 vs ~0.27 for variance.)
//!
//! Method: per coarse cell, mean HSV-style saturation `S=(max-min)/max` from the
//! captured BGRA frame → mark cells whose mean S is low as "content" → take the
//! LARGEST connected component (so a disjoint dock/top-bar isn't merged in) → bbox.
//! Pure-Rust, no system libs. The fallback for windowless / AT-SPI-less content;
//! lower `trust` (0.6) — a pixel guess, not a named rect. Kept OUT of the default
//! `detect()` union (WM already names managed windows exactly).

use std::time::Duration;

use anyhow::Result;

use crate::capture::screen::ScreenCapturer;
use crate::regions::{Cost, Granularity, Region, RegionProvider, Source};
use crate::target::model::PixelRect;

const GRID_X: usize = 80;
const GRID_Y: usize = 45;
const SAT_THRESH: f64 = 0.35; // a cell is "content" if its mean saturation < 0.35

/// Chroma-saliency region provider (per display). `display` = the display index.
pub struct ContrastProvider {
    pub display: usize,
}

impl ContrastProvider {
    /// Bounding box of the salient low-saturation content on `display`, or `None`
    /// if the frame is empty / has no desaturated region.
    pub fn salient_region(display: usize) -> Result<Option<Region>> {
        let mut cap = ScreenCapturer::new(display)?;
        let (w, h) = (cap.width(), cap.height());
        if w == 0 || h == 0 {
            return Ok(None);
        }
        let buf = cap.capture_frame(Duration::from_secs(2))?;
        let stride = buf.len().checked_div(h).unwrap_or(w * 4);
        let (cw, ch) = ((w / GRID_X).max(1), (h / GRID_Y).max(1));

        // Per-cell mean saturation → "content" mask (low saturation = desaturated UI).
        let mut content = vec![false; GRID_X * GRID_Y];
        for gy in 0..GRID_Y {
            for gx in 0..GRID_X {
                let (x0, y0) = (gx * cw, gy * ch);
                let (x1, y1) = ((x0 + cw).min(w), (y0 + ch).min(h));
                let (mut sat_sum, mut n) = (0f64, 0f64);
                let mut y = y0;
                while y < y1 {
                    let mut x = x0;
                    while x < x1 {
                        let o = y * stride + x * 4;
                        if o + 2 < buf.len() {
                            let (b, g, r) = (buf[o] as f64, buf[o + 1] as f64, buf[o + 2] as f64);
                            let max = r.max(g).max(b);
                            let min = r.min(g).min(b);
                            let s = if max <= 0.0 { 0.0 } else { (max - min) / max };
                            sat_sum += s;
                            n += 1.0;
                        }
                        x += 2;
                    }
                    y += 2;
                }
                if n > 0.0 {
                    content[gy * GRID_X + gx] = (sat_sum / n) < SAT_THRESH;
                }
            }
        }

        // Largest 4-connected component of content cells (a disjoint dock/top-bar
        // stays separate and doesn't inflate the box).
        let mut visited = vec![false; GRID_X * GRID_Y];
        let mut best: Vec<usize> = Vec::new();
        for start in 0..GRID_X * GRID_Y {
            if !content[start] || visited[start] {
                continue;
            }
            let mut stack = vec![start];
            visited[start] = true;
            let mut comp = Vec::new();
            while let Some(c) = stack.pop() {
                comp.push(c);
                let (cx, cy) = (c % GRID_X, c / GRID_X);
                let mut push = |nx: usize, ny: usize, stack: &mut Vec<usize>, visited: &mut [bool]| {
                    let nc = ny * GRID_X + nx;
                    if content[nc] && !visited[nc] {
                        visited[nc] = true;
                        stack.push(nc);
                    }
                };
                if cx > 0 {
                    push(cx - 1, cy, &mut stack, &mut visited);
                }
                if cx + 1 < GRID_X {
                    push(cx + 1, cy, &mut stack, &mut visited);
                }
                if cy > 0 {
                    push(cx, cy - 1, &mut stack, &mut visited);
                }
                if cy + 1 < GRID_Y {
                    push(cx, cy + 1, &mut stack, &mut visited);
                }
            }
            if comp.len() > best.len() {
                best = comp;
            }
        }
        if best.is_empty() {
            return Ok(None);
        }

        let (mut minx, mut miny, mut maxx, mut maxy) = (GRID_X, GRID_Y, 0usize, 0usize);
        for &c in &best {
            let (cx, cy) = (c % GRID_X, c / GRID_X);
            minx = minx.min(cx);
            miny = miny.min(cy);
            maxx = maxx.max(cx);
            maxy = maxy.max(cy);
        }
        let x = (minx * cw) as u32;
        let y = (miny * ch) as u32;
        let bbox = PixelRect {
            x,
            y,
            w: (((maxx + 1) * cw).min(w) as u32).saturating_sub(x).max(1),
            h: (((maxy + 1) * ch).min(h) as u32).saturating_sub(y).max(1),
        };
        Ok(Some(Region::new(bbox, Source::Contrast, Granularity::Window, 0.6)))
    }
}

impl RegionProvider for ContrastProvider {
    fn source(&self) -> Source {
        Source::Contrast
    }
    fn granularity(&self) -> Granularity {
        Granularity::Window
    }
    fn cost(&self) -> Cost {
        Cost::Cheap
    }
    fn probe(&self, within: &Region) -> bool {
        within.granularity == Granularity::Monitor
    }
    fn regions(&self, _within: &Region) -> Vec<Region> {
        Self::salient_region(self.display).ok().flatten().into_iter().collect()
    }
}
