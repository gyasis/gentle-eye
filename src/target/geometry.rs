//! Normalized↔pixel coordinate mapping (multi-monitor / ultrawide aware).
//!
//! TG1 — the critical "boring" utility per the PRD. Pure functions, no I/O.
//!
//! A [`NormRect`] is resolution-agnostic (the contract the agent speaks). To
//! crop, we need absolute pixels on the source. `(resolution, display offset)`
//! turns one into the other:
//!
//! - `resolution = (width, height)` of the source in pixels.
//! - `offset = (origin_x, origin_y)` of the source within the global desktop
//!   coordinate space — non-zero for secondary monitors on an extended desktop
//!   (e.g. a second display to the right of a 3440-wide ultrawide starts at
//!   `x = 3440`). The offset shifts the *pixel* rect; it never changes the
//!   normalized box, which is always relative to its own source.
//!
//! Out-of-unit-square inputs are clamped to the source bounds rather than
//! rejected, so a slightly-too-wide agent box still yields a usable crop (the
//! Phase-2 snap + confirmation loop refine it).

use super::model::{NormRect, PixelRect};

/// Round half-up to the nearest integer pixel, clamped to `u32`.
fn round_px(v: f64) -> u32 {
    if v <= 0.0 {
        0
    } else {
        v.round() as u32
    }
}

/// Map a normalized 0–1 rect on a source of `resolution = (w, h)` at global
/// `offset = (ox, oy)` to an absolute [`PixelRect`].
///
/// The normalized rect is clamped into the unit square first, so the resulting
/// pixel rect always lies within the source. Width/height are guaranteed `>= 1`
/// for any positive-area input (a sub-pixel box still crops one pixel).
pub fn norm_to_pixel(r: NormRect, resolution: (u32, u32), offset: (i32, i32)) -> PixelRect {
    let (res_w, res_h) = (resolution.0 as f64, resolution.1 as f64);

    // Clamp the normalized box into the unit square, preserving positive area.
    let x = r.x.clamp(0.0, 1.0);
    let y = r.y.clamp(0.0, 1.0);
    let w = r.w.clamp(0.0, 1.0 - x);
    let h = r.h.clamp(0.0, 1.0 - y);

    let px = round_px(x * res_w);
    let py = round_px(y * res_h);
    // Width/height: at least 1px for any positive normalized extent, and never
    // spilling past the source's right/bottom edge.
    let pw = round_px(w * res_w).max(1).min(resolution.0.saturating_sub(px).max(1));
    let ph = round_px(h * res_h).max(1).min(resolution.1.saturating_sub(py).max(1));

    PixelRect {
        x: px.saturating_add_signed(offset.0),
        y: py.saturating_add_signed(offset.1),
        w: pw,
        h: ph,
    }
}

/// Inverse of [`norm_to_pixel`]: map an absolute pixel rect (including the
/// global offset) back to a normalized 0–1 rect relative to its source.
pub fn pixel_to_norm(r: PixelRect, resolution: (u32, u32), offset: (i32, i32)) -> NormRect {
    let (res_w, res_h) = (resolution.0 as f64, resolution.1 as f64);

    // Strip the display offset to get source-local pixels.
    let local_x = (r.x as i64 - offset.0 as i64).max(0) as f64;
    let local_y = (r.y as i64 - offset.1 as i64).max(0) as f64;

    let nx = if res_w > 0.0 { local_x / res_w } else { 0.0 };
    let ny = if res_h > 0.0 { local_y / res_h } else { 0.0 };
    let nw = if res_w > 0.0 { r.w as f64 / res_w } else { 0.0 };
    let nh = if res_h > 0.0 { r.h as f64 / res_h } else { 0.0 };

    NormRect::new(nx, ny, nw, nh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ultrawide_second_pane_of_four() {
        // 3440×1440 ultrawide, the 2nd of 4 equal columns (x=0.25, w=0.25).
        let r = NormRect::new(0.25, 0.0, 0.25, 1.0);
        let px = norm_to_pixel(r, (3440, 1440), (0, 0));
        assert_eq!(px, PixelRect { x: 860, y: 0, w: 860, h: 1440 });
    }

    #[test]
    fn multi_monitor_offset_shifts_pixels() {
        // 1920×1080 secondary monitor to the right of a 3440 ultrawide.
        let r = NormRect::new(0.0, 0.0, 0.5, 0.5);
        let px = norm_to_pixel(r, (1920, 1080), (3440, 0));
        assert_eq!(px, PixelRect { x: 3440, y: 0, w: 960, h: 540 });
    }

    #[test]
    fn round_trip_within_one_pixel() {
        let res = (2560, 1440);
        let off = (100, -50);
        let orig = NormRect::new(0.3125, 0.5, 0.25, 0.25);
        let px = norm_to_pixel(orig, res, off);
        let back = pixel_to_norm(px, res, off);
        // Re-mapping the recovered norm rect must land within 1px of the original.
        let re = norm_to_pixel(back, res, off);
        assert!((re.x as i64 - px.x as i64).abs() <= 1);
        assert!((re.y as i64 - px.y as i64).abs() <= 1);
        assert!((re.w as i64 - px.w as i64).abs() <= 1);
        assert!((re.h as i64 - px.h as i64).abs() <= 1);
    }

    #[test]
    fn clamps_past_the_edge() {
        // A box that overflows the right/bottom edge is clamped into bounds.
        let r = NormRect::new(0.8, 0.8, 0.5, 0.5);
        let px = norm_to_pixel(r, (1000, 1000), (0, 0));
        assert_eq!(px.x, 800);
        assert_eq!(px.y, 800);
        assert_eq!(px.x + px.w, 1000); // never spills past the source
        assert_eq!(px.y + px.h, 1000);
    }

    #[test]
    fn degenerate_box_still_yields_one_pixel() {
        let r = NormRect::new(0.5, 0.5, 0.0001, 0.0001);
        let px = norm_to_pixel(r, (1920, 1080), (0, 0));
        assert!(px.w >= 1 && px.h >= 1);
    }
}
