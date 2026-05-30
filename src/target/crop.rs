//! Pure-Rust BGRA sub-rectangle crop primitive (Phase 1 basis, TG3).
//!
//! No new dependency: a captured screen frame is a flat BGRA byte buffer
//! (`scrap` may pad each row to a platform `stride`), so cropping is a
//! stride-aware row-by-row copy of the sub-rectangle. Stream crops are done by
//! an ffmpeg `crop=` filter in [`crate::capture::stream`], not here.

use super::geometry::norm_to_pixel;
use super::model::{PixelRect, Target, TargetSource};
use crate::target::errors::TargetError;

/// Crop a BGRA buffer to `rect`, returning a tightly-packed `(buf, w, h)`.
///
/// - `full_w` / `full_h`: the frame's pixel dimensions.
/// - `stride`: bytes per row (`>= full_w * 4`; equals it when un-padded).
/// - `rect`: the sub-rectangle in pixels; must lie within the frame.
pub fn crop_bgra(
    buf: &[u8],
    full_w: usize,
    full_h: usize,
    stride: usize,
    rect: PixelRect,
) -> Result<(Vec<u8>, u32, u32), TargetError> {
    let (x, y, w, h) = (
        rect.x as usize,
        rect.y as usize,
        rect.w as usize,
        rect.h as usize,
    );
    if w == 0 || h == 0 {
        return Err(TargetError::Capture("crop rect has zero area".into()));
    }
    if x + w > full_w || y + h > full_h {
        return Err(TargetError::Capture(format!(
            "crop rect {rect:?} exceeds frame {full_w}x{full_h}"
        )));
    }
    if stride < full_w * 4 {
        return Err(TargetError::Capture(format!(
            "stride {stride} < row bytes {}",
            full_w * 4
        )));
    }
    if buf.len() < stride * full_h {
        return Err(TargetError::Capture(format!(
            "buffer {} < stride*height {}",
            buf.len(),
            stride * full_h
        )));
    }

    let row_bytes = w * 4;
    let mut out = Vec::with_capacity(row_bytes * h);
    for row in y..y + h {
        let start = row * stride + x * 4;
        out.extend_from_slice(&buf[start..start + row_bytes]);
    }
    Ok((out, w as u32, h as u32))
}

/// Apply the active target to a captured **screen** frame.
///
/// - `Some(target)` whose source is a `Display` → crop to its region (the
///   normalized rect is relative to this display's own buffer, so the global
///   offset is `(0, 0)` here).
/// - `None`, or a target whose source is a `Stream` → byte-identical
///   pass-through at full size (a stream target doesn't crop screen frames).
pub fn crop_frame_for_target(
    buf: &[u8],
    full_w: usize,
    full_h: usize,
    stride: usize,
    target: Option<&Target>,
) -> Result<(Vec<u8>, u32, u32), TargetError> {
    match target {
        Some(t) if matches!(t.source, TargetSource::Display { .. }) => {
            let rect = norm_to_pixel(t.region, (full_w as u32, full_h as u32), (0, 0));
            crop_bgra(buf, full_w, full_h, stride, rect)
        }
        _ => Ok((buf.to_vec(), full_w as u32, full_h as u32)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::model::NormRect;

    /// A `w×h` BGRA buffer where pixel (x,y) = bytes [x, y, 0, 255].
    fn make_frame(w: usize, h: usize, stride: usize) -> Vec<u8> {
        let mut v = vec![0u8; stride * h];
        for y in 0..h {
            for x in 0..w {
                let i = y * stride + x * 4;
                v[i] = x as u8;
                v[i + 1] = y as u8;
                v[i + 2] = 0;
                v[i + 3] = 255;
            }
        }
        v
    }

    #[test]
    fn crops_2x2_from_4x4_exact_bytes() {
        let frame = make_frame(4, 4, 16);
        let (out, w, h) = crop_bgra(&frame, 4, 4, 16, PixelRect { x: 1, y: 1, w: 2, h: 2 }).unwrap();
        assert_eq!((w, h), (2, 2));
        // Top-left of the crop is pixel (1,1).
        assert_eq!(&out[0..4], &[1, 1, 0, 255]);
        // Next pixel right is (2,1).
        assert_eq!(&out[4..8], &[2, 1, 0, 255]);
        // Second row first pixel is (1,2).
        assert_eq!(&out[8..12], &[1, 2, 0, 255]);
        assert_eq!(out.len(), 2 * 2 * 4);
    }

    #[test]
    fn handles_row_stride_padding() {
        // 2×2 image with 4 bytes of row padding (stride 12 vs 8 packed).
        let frame = make_frame(2, 2, 12);
        let (out, w, h) = crop_bgra(&frame, 2, 2, 12, PixelRect { x: 0, y: 0, w: 2, h: 2 }).unwrap();
        assert_eq!((w, h), (2, 2));
        assert_eq!(out.len(), 16); // tightly packed, padding stripped
        assert_eq!(&out[0..4], &[0, 0, 0, 255]); // pixel (0,0)
        assert_eq!(&out[8..12], &[0, 1, 0, 255]); // pixel (0,1) — padding skipped
    }

    #[test]
    fn out_of_bounds_errors() {
        let frame = make_frame(4, 4, 16);
        assert!(crop_bgra(&frame, 4, 4, 16, PixelRect { x: 3, y: 0, w: 2, h: 1 }).is_err());
        assert!(crop_bgra(&frame, 4, 4, 16, PixelRect { x: 0, y: 0, w: 0, h: 1 }).is_err());
    }

    #[test]
    fn display_target_crops_right_half() {
        let frame = make_frame(4, 2, 16);
        let t = Target::new(
            "right",
            TargetSource::Display { index: 0 },
            NormRect::new(0.5, 0.0, 0.5, 1.0),
        );
        let (out, w, h) = crop_frame_for_target(&frame, 4, 2, 16, Some(&t)).unwrap();
        assert_eq!((w, h), (2, 2));
        assert_eq!(&out[0..4], &[2, 0, 0, 255]); // first cropped pixel is (2,0)
    }

    #[test]
    fn no_target_is_byte_identical_passthrough() {
        let frame = make_frame(3, 3, 12); // un-padded
        let (out, w, h) = crop_frame_for_target(&frame, 3, 3, 12, None).unwrap();
        assert_eq!((w, h), (3, 3));
        assert_eq!(out, frame);
    }

    #[test]
    fn stream_target_does_not_crop_screen_frame() {
        let frame = make_frame(3, 3, 12);
        let t = Target::new(
            "cam",
            TargetSource::Stream { url: "rtsp://x".into() },
            NormRect::new(0.0, 0.0, 0.5, 0.5),
        );
        let (out, w, h) = crop_frame_for_target(&frame, 3, 3, 12, Some(&t)).unwrap();
        assert_eq!((w, h), (3, 3)); // pass-through
        assert_eq!(out, frame);
    }
}
