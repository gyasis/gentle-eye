//! Frames of a recording, each with a sharpness score.
//!
//! # Why sharpness is worth measuring
//!
//! Blur PREDICTS unreadability, and it costs no model call to detect. Measured
//! 2026-08-31 against a live capture feed (research.md M4): frames scoring
//! 1,443–1,458 all read cleanly, while frames scoring 396–507 **all** failed —
//! every one, no exceptions. The blur was motion blur from scrolling.
//!
//! So the cheapest measurement available decides where the most expensive stage
//! spends its time. It also inverts the usual reason to raise a frame rate: a
//! higher rate is not for capturing more CONTENT, it is for capturing a SHARP
//! INSTANCE of the same content.

/// The sharpness of an 8-bit greyscale image, as the variance of its Laplacian.
///
/// Higher is sharper. Blur suppresses high-frequency detail, so the second
/// derivative flattens and its variance collapses.
///
/// # This number is comparable WITHIN a recording, not across recordings
///
/// It is a focus measure, not a calibrated scale: it moves with resolution,
/// contrast and content. A caller compares frames of the same source and picks
/// the best of them. Treating it as an absolute threshold across different
/// material is a misreading, which is why no threshold lives in this module
/// (D015-1 — the judgement belongs to the caller).
///
/// Returns `0.0` for an image too small to have an interior pixel; a 1×1 or 2×2
/// image has no 3×3 neighbourhood, and zero is the honest answer for "no detail
/// measurable" rather than a panic or a fabricated value.
pub fn sharpness(gray: &[u8], width: usize, height: usize) -> f64 {
    if width < 3 || height < 3 || gray.len() < width * height {
        return 0.0;
    }
    // 4-neighbour Laplacian. Applied over interior pixels only — a border pixel
    // has no full neighbourhood, and padding it would invent detail at the edge.
    let mut sum = 0.0_f64;
    let mut sum_sq = 0.0_f64;
    let mut n = 0.0_f64;
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let i = y * width + x;
            let lap = 4.0 * f64::from(gray[i])
                - f64::from(gray[i - 1])
                - f64::from(gray[i + 1])
                - f64::from(gray[i - width])
                - f64::from(gray[i + width]);
            sum += lap;
            sum_sq += lap * lap;
            n += 1.0;
        }
    }
    if n == 0.0 {
        return 0.0;
    }
    let mean = sum / n;
    (sum_sq / n) - (mean * mean)
}

/// [`sharpness`] for an image file, decoded to greyscale.
///
/// Errors are the caller's to see: a frame that cannot be decoded is not a
/// frame that scores zero. Conflating them would let an unreadable FILE look
/// like an unreadable IMAGE, and the two want different responses.
pub fn sharpness_of_file(path: &std::path::Path) -> Result<f64, String> {
    let img = image::open(path)
        .map_err(|e| format!("cannot decode {}: {e}", path.display()))?
        .to_luma8();
    let (w, h) = (img.width() as usize, img.height() as usize);
    Ok(sharpness(img.as_raw(), w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flat image has no detail, so its Laplacian variance is zero. This is
    /// the one case where zero is a real measurement rather than a refusal.
    #[test]
    fn a_flat_image_has_no_sharpness() {
        let flat = vec![128u8; 32 * 32];
        assert_eq!(sharpness(&flat, 32, 32), 0.0);
    }

    /// Detail raises the score. Without this, a function returning a constant
    /// would pass the flat-image test.
    #[test]
    fn detail_scores_above_flatness() {
        let flat = vec![128u8; 32 * 32];
        let mut checker = vec![0u8; 32 * 32];
        for (i, px) in checker.iter_mut().enumerate() {
            *px = if (i / 32 + i % 32) % 2 == 0 { 0 } else { 255 };
        }
        assert!(
            sharpness(&checker, 32, 32) > sharpness(&flat, 32, 32),
            "a detailed image must score above a flat one"
        );
    }

    /// BLUR LOWERS THE SCORE — the property M4 rests on.
    ///
    /// The blur here is a box average, which is what motion blur does to an
    /// edge: it spreads it across neighbouring pixels. The live test
    /// (`tests/transcribe_primitives.rs`) uses REAL motion blur from real
    /// motion; this unit test pins the direction without needing ffmpeg.
    #[test]
    fn blur_lowers_the_score() {
        let w = 64;
        let h = 64;
        let mut sharp_img = vec![0u8; w * h];
        for (i, px) in sharp_img.iter_mut().enumerate() {
            *px = if (i % w) % 4 < 2 { 0 } else { 255 };
        }
        // Box-average the columns: the same thing motion does to a vertical edge.
        let mut blurred = sharp_img.clone();
        for y in 0..h {
            for x in 2..w - 2 {
                let s: u32 = (x - 2..=x + 2).map(|k| u32::from(sharp_img[y * w + k])).sum();
                blurred[y * w + x] = (s / 5) as u8;
            }
        }
        let a = sharpness(&sharp_img, w, h);
        let b = sharpness(&blurred, w, h);
        assert!(
            b < a,
            "blur must LOWER sharpness — this is the property M4 rests on \
             (measured: sharp 1443-1458 all read cleanly, blurred 396-507 all failed). \
             sharp={a}, blurred={b}"
        );
        assert!(a / b > 2.0, "the separation must be substantial, got {:.1}x", a / b);
    }

    /// An image with no interior pixel scores zero rather than panicking.
    #[test]
    fn an_image_too_small_to_measure_scores_zero() {
        assert_eq!(sharpness(&[1, 2, 3, 4], 2, 2), 0.0);
        assert_eq!(sharpness(&[], 0, 0), 0.0);
        // Short buffer: honest zero, not an out-of-bounds read.
        assert_eq!(sharpness(&[1, 2, 3], 10, 10), 0.0);
    }
}
