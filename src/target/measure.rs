//! Phase 2 "measurement mode" — Zoom-then-Snap with pure-Rust CV (`imageproc`).
//!
//! The agent (VLM) supplies a *rough* normalized region; this module snaps it to
//! the nearest strong edges, detects the tiled-pane grid via a projection
//! profile, and can locate a hand-drawn red marker — returning a
//! [`MeasurementResult`] plus a "Redline Overlay" diagnostic the VLM inspects to
//! confirm or re-target. No system libraries: `image` + `imageproc` only.

use super::geometry::{norm_to_pixel, pixel_to_norm};
use super::model::{NormRect, PixelRect};
use crate::target::errors::TargetError;
use image::{GrayImage, Luma, RgbImage};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Result of snapping a rough region to real image geometry.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct MeasurementResult {
    /// The snapped region, back in normalized 0–1 coordinates.
    pub snapped_rect: NormRect,
    /// Width/height of the snapped pixel rect.
    pub aspect_ratio: f64,
    /// 0–1 confidence — how much of the border lay on a strong edge.
    pub confidence: f64,
    /// If the source looks like a tiled grid: `(columns, rows)`.
    pub detected_grid: Option<(u32, u32)>,
    /// 0–1 fraction of the four borders that aligned to detected edges.
    pub edge_alignment: f64,
}

/// Convert a BGRA frame (scrap byte order, possibly row-padded) to grayscale.
pub fn bgra_to_gray(bgra: &[u8], w: usize, h: usize, stride: usize) -> Result<GrayImage, TargetError> {
    if stride < w * 4 || bgra.len() < stride * h {
        return Err(TargetError::Measure("buffer too small for stride*height".into()));
    }
    let mut img = GrayImage::new(w as u32, h as u32);
    for y in 0..h {
        for x in 0..w {
            let i = y * stride + x * 4;
            let (b, g, r) = (bgra[i] as f32, bgra[i + 1] as f32, bgra[i + 2] as f32);
            // Rec. 601 luma.
            let luma = (0.114 * b + 0.587 * g + 0.299 * r).round().clamp(0.0, 255.0) as u8;
            img.put_pixel(x as u32, y as u32, Luma([luma]));
        }
    }
    Ok(img)
}

/// Snap a rough pixel rect to the strongest edges within a `pad`-fraction window
/// around each border. Returns the snapped rect, edge-alignment, and confidence.
pub fn snap_to_edges(gray: &GrayImage, rough: PixelRect, pad: f64) -> (PixelRect, f64) {
    let (iw, ih) = (gray.width(), gray.height());
    let edges = imageproc::edges::canny(gray, 30.0, 90.0);

    let pad_x = ((rough.w as f64) * pad).round() as i64;
    let pad_y = ((rough.h as f64) * pad).round() as i64;

    // Column edge density within the rough vertical span.
    let col_density = |x: u32| -> u32 {
        let (y0, y1) = (rough.y, (rough.y + rough.h).min(ih));
        (y0..y1).filter(|&y| edges.get_pixel(x, y).0[0] > 0).count() as u32
    };
    let row_density = |y: u32| -> u32 {
        let (x0, x1) = (rough.x, (rough.x + rough.w).min(iw));
        (x0..x1).filter(|&x| edges.get_pixel(x, y).0[0] > 0).count() as u32
    };

    let best_x = |center: i64| -> (u32, u32) {
        let lo = (center - pad_x).max(0);
        let hi = (center + pad_x).min(iw as i64 - 1);
        let mut best = (center.clamp(0, iw as i64 - 1) as u32, 0u32);
        for x in lo..=hi {
            let d = col_density(x as u32);
            if d > best.1 {
                best = (x as u32, d);
            }
        }
        best
    };
    let best_y = |center: i64| -> (u32, u32) {
        let lo = (center - pad_y).max(0);
        let hi = (center + pad_y).min(ih as i64 - 1);
        let mut best = (center.clamp(0, ih as i64 - 1) as u32, 0u32);
        for y in lo..=hi {
            let d = row_density(y as u32);
            if d > best.1 {
                best = (y as u32, d);
            }
        }
        best
    };

    let (left, dl) = best_x(rough.x as i64);
    let (right, dr) = best_x((rough.x + rough.w) as i64);
    let (top, dt) = best_y(rough.y as i64);
    let (bottom, db) = best_y((rough.y + rough.h) as i64);

    let (sx, sw) = if right > left { (left, right - left) } else { (rough.x, rough.w) };
    let (sy, sh) = if bottom > top { (top, bottom - top) } else { (rough.y, rough.h) };

    // Edge alignment: fraction of the four borders that found *any* edge.
    let found = [dl, dr, dt, db].iter().filter(|&&d| d > 0).count() as f64;
    let alignment = found / 4.0;

    (PixelRect { x: sx, y: sy, w: sw.max(1), h: sh.max(1) }, alignment)
}

/// Detect tiled-pane columns via a vertical projection profile: dark, uniform
/// gutters between bright content columns. Returns `(columns, rows=1)` when a
/// grid is found.
pub fn detect_gutters(gray: &GrayImage) -> Option<(u32, u32)> {
    let (w, h) = (gray.width() as usize, gray.height() as usize);
    if w < 8 || h < 1 {
        return None;
    }
    // Mean luma per column.
    let mut col_mean = vec![0f64; w];
    for (x, item) in col_mean.iter_mut().enumerate() {
        let mut sum = 0u64;
        for y in 0..h {
            sum += gray.get_pixel(x as u32, y as u32).0[0] as u64;
        }
        *item = sum as f64 / h as f64;
    }
    let global_mean = col_mean.iter().sum::<f64>() / w as f64;
    let threshold = global_mean * 0.5;

    // Contiguous interior runs of below-threshold columns = gutters.
    let mut gutters = 0u32;
    let mut in_gutter = false;
    for (x, &m) in col_mean.iter().enumerate() {
        let dark = m < threshold;
        if dark && !in_gutter {
            // Ignore gutters touching the very edges (border, not a divider).
            if x > 0 && x < w - 1 {
                gutters += 1;
            }
            in_gutter = true;
        } else if !dark {
            in_gutter = false;
        }
    }
    if gutters >= 1 {
        Some((gutters + 1, 1))
    } else {
        None
    }
}

/// Find a hand-drawn red marker's bounding box (R high, G/B low) in a BGRA frame.
pub fn find_red_marker(bgra: &[u8], w: usize, h: usize, stride: usize) -> Option<PixelRect> {
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (usize::MAX, usize::MAX, 0usize, 0usize);
    let mut found = false;
    for y in 0..h {
        for x in 0..w {
            let i = y * stride + x * 4;
            if i + 2 >= bgra.len() {
                continue;
            }
            let (b, g, r) = (bgra[i], bgra[i + 1], bgra[i + 2]);
            if r > 150 && g < 100 && b < 100 {
                found = true;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }
    if !found {
        return None;
    }
    Some(PixelRect {
        x: min_x as u32,
        y: min_y as u32,
        w: (max_x - min_x + 1) as u32,
        h: (max_y - min_y + 1) as u32,
    })
}

/// "Redline Overlay" diagnostic: the Canny edges drawn green, the snapped rect
/// drawn red, so the VLM can supervise the CV. Saved as a PNG at `out`.
pub fn write_redline_overlay(
    gray: &GrayImage,
    snapped: PixelRect,
    out: &std::path::Path,
) -> Result<(), TargetError> {
    let edges = imageproc::edges::canny(gray, 30.0, 90.0);
    let (w, h) = (gray.width(), gray.height());
    let mut rgb = RgbImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let lum = gray.get_pixel(x, y).0[0];
            let mut px = [lum / 3, lum / 3, lum / 3];
            if edges.get_pixel(x, y).0[0] > 0 {
                px = [0, 255, 0]; // green = edge found
            }
            rgb.put_pixel(x, y, image::Rgb(px));
        }
    }
    // Red border around the snapped rect.
    for x in snapped.x..(snapped.x + snapped.w).min(w) {
        rgb.put_pixel(x, snapped.y.min(h - 1), image::Rgb([255, 0, 0]));
        rgb.put_pixel(x, (snapped.y + snapped.h - 1).min(h - 1), image::Rgb([255, 0, 0]));
    }
    for y in snapped.y..(snapped.y + snapped.h).min(h) {
        rgb.put_pixel(snapped.x.min(w - 1), y, image::Rgb([255, 0, 0]));
        rgb.put_pixel((snapped.x + snapped.w - 1).min(w - 1), y, image::Rgb([255, 0, 0]));
    }
    rgb.save(out).map_err(|e| TargetError::Measure(format!("overlay save failed: {e}")))
}

/// Load an image file (PNG/JPEG/…) into a tightly-packed BGRA buffer + dims —
/// used to measure a stream frame that was captured to disk as a PNG.
pub fn load_image_as_bgra(path: &std::path::Path) -> Result<(Vec<u8>, usize, usize), TargetError> {
    let img = image::open(path)
        .map_err(|e| TargetError::Measure(format!("open {}: {e}", path.display())))?;
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width() as usize, rgba.height() as usize);
    let mut bgra = Vec::with_capacity(w * h * 4);
    for p in rgba.pixels() {
        let [r, g, b, a] = p.0;
        bgra.extend_from_slice(&[b, g, r, a]);
    }
    Ok((bgra, w, h))
}

/// Zoom-then-Snap end to end: map the rough normalized region to pixels, snap to
/// edges, detect any grid, and return a [`MeasurementResult`].
pub fn measure(
    bgra: &[u8],
    full_w: usize,
    full_h: usize,
    stride: usize,
    rough: NormRect,
) -> Result<MeasurementResult, TargetError> {
    if !rough.is_valid() {
        return Err(TargetError::InvalidRegion(format!("{rough:?}")));
    }
    let gray = bgra_to_gray(bgra, full_w, full_h, stride)?;
    let rough_px = norm_to_pixel(rough, (full_w as u32, full_h as u32), (0, 0));
    let (snapped, alignment) = snap_to_edges(&gray, rough_px, 0.10);
    let detected_grid = detect_gutters(&gray);
    let snapped_norm = pixel_to_norm(snapped, (full_w as u32, full_h as u32), (0, 0));
    let aspect_ratio = if snapped.h > 0 { snapped.w as f64 / snapped.h as f64 } else { 0.0 };

    Ok(MeasurementResult {
        snapped_rect: snapped_norm,
        aspect_ratio,
        confidence: alignment,
        detected_grid,
        edge_alignment: alignment,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a BGRA frame from an `(x,y) -> (b,g,r)` closure (stride = w*4).
    fn frame(w: usize, h: usize, f: impl Fn(usize, usize) -> (u8, u8, u8)) -> Vec<u8> {
        let mut v = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let (b, g, r) = f(x, y);
                let i = (y * w + x) * 4;
                v[i] = b;
                v[i + 1] = g;
                v[i + 2] = r;
                v[i + 3] = 255;
            }
        }
        v
    }

    #[test]
    fn snaps_rough_box_to_a_bordered_rectangle() {
        // White field with a black-bordered rectangle from x=20..60, y=10..50.
        let (w, h) = (80usize, 60usize);
        let buf = frame(w, h, |x, y| {
            let on_v = (x == 20 || x == 60) && (10..=50).contains(&y);
            let on_h = (y == 10 || y == 50) && (20..=60).contains(&x);
            if on_v || on_h { (0, 0, 0) } else { (255, 255, 255) }
        });
        let gray = bgra_to_gray(&buf, w, h, w * 4).unwrap();
        // Rough box a few px off from the true borders.
        let (snapped, alignment) = snap_to_edges(&gray, PixelRect { x: 17, y: 13, w: 46, h: 34 }, 0.20);
        assert!((snapped.x as i64 - 20).abs() <= 2, "left snapped to ~20, got {}", snapped.x);
        assert!(((snapped.x + snapped.w) as i64 - 60).abs() <= 2, "right ~60");
        assert!(alignment > 0.0);
    }

    #[test]
    fn detects_four_columns_via_gutters() {
        // 4 bright columns separated by 3 dark gutters.
        let (w, h) = (80usize, 20usize);
        let buf = frame(w, h, |x, _| {
            // gutters at x in {19,20, 39,40, 59,60}
            let gutter = (19..=20).contains(&x) || (39..=40).contains(&x) || (59..=60).contains(&x);
            if gutter { (0, 0, 0) } else { (255, 255, 255) }
        });
        let gray = bgra_to_gray(&buf, w, h, w * 4).unwrap();
        assert_eq!(detect_gutters(&gray), Some((4, 1)));
    }

    #[test]
    fn finds_red_marker_bbox() {
        let (w, h) = (40usize, 30usize);
        let buf = frame(w, h, |x, y| {
            if (5..=15).contains(&x) && (8..=20).contains(&y) {
                (10, 10, 230) // BGRA red: high R, low G/B
            } else {
                (200, 200, 200)
            }
        });
        let bbox = find_red_marker(&buf, w, h, w * 4).unwrap();
        assert_eq!(bbox, PixelRect { x: 5, y: 8, w: 11, h: 13 });
    }

    #[test]
    fn measure_end_to_end_returns_result() {
        let (w, h) = (80usize, 60usize);
        let buf = frame(w, h, |x, y| {
            let on = (x == 20 || x == 60) && (10..=50).contains(&y)
                || (y == 10 || y == 50) && (20..=60).contains(&x);
            if on { (0, 0, 0) } else { (255, 255, 255) }
        });
        let rough = NormRect::new(0.22, 0.18, 0.5, 0.6);
        let r = measure(&buf, w, h, w * 4, rough).unwrap();
        assert!(r.snapped_rect.is_valid());
        assert!(r.aspect_ratio > 0.0);
        assert!(r.edge_alignment >= 0.0 && r.edge_alignment <= 1.0);
    }
}
