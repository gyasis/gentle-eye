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
/// material is a misreading, which is why no threshold lives in this module:
/// D015-3 consumes the score but leaves the floor to the caller, per the
/// D015-7 principle that a judgement varying with the content cannot be a
/// constant in a binary.
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

    /// VARIANCE of the Laplacian, not its mean square — the subtraction of
    /// E[x]² is load-bearing. A parabolic ramp has a CONSTANT Laplacian (here
    /// exactly -2 at every interior pixel): smooth shading, no detail. Variance
    /// scores it 0; E[x²] would score it 4. A mutation that drops the mean
    /// subtraction survives every other test in this module — this one kills it.
    #[test]
    fn smooth_curvature_is_not_detail() {
        let (w, h) = (16, 3);
        let mut parabola = vec![0u8; w * h];
        for y in 0..h {
            for x in 0..w {
                parabola[y * w + x] = (x * x) as u8; // 0..=225, no overflow
            }
        }
        assert_eq!(
            sharpness(&parabola, w, h),
            0.0,
            "a constant second derivative has zero variance; a nonzero score \
             here means the mean subtraction has been dropped"
        );
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

/// One frame kept from a recording.
///
/// "Kept" means it survived near-duplicate suppression at the CALLER's
/// threshold. The rows deliberately do not describe what was dropped: the
/// caller set the threshold and can re-run to see more, and a list of
/// near-duplicates nobody asked for is noise (D015-7).
#[derive(Debug, Clone, PartialEq)]
pub struct FrameRow {
    /// Position in the kept sequence, from 0.
    pub index: usize,
    /// Where the frame was written.
    pub path: std::path::PathBuf,
    /// Focus measure. **Comparable within this recording only** — see
    /// [`sharpness`].
    pub sharpness: f64,
}

/// How aggressively to drop near-duplicate frames before anything is paid for.
///
/// A knob, never a constant. Measured on one 15-second recording (research.md
/// M1): the same clip kept 285, 138 or 2 frames across these knob positions,
/// because scrolling text genuinely changes every frame while slides do not.
/// How much of a recording is new is a property of the MATERIAL, so the tool
/// must not choose (D015-7). M1's counts were measured against the full frame
/// rate; here the filter sees the sampled stream — see `Dedup::filter` for
/// why the knob's ordering transfers and the exact counts do not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Dedup {
    /// Keep every frame at the requested rate. For material where any loss
    /// matters more than the cost of reading it.
    None,
    /// Drop only near-identical frames — any visible change is kept. Suits
    /// slides and scrolling text alike: measured on a four-slide fixture,
    /// this keeps exactly one frame per distinct screen.
    Gentle,
    /// The default. Suits mixed material. On the same four-slide fixture it
    /// also keeps one frame per distinct screen.
    #[default]
    Medium,
    /// Collapse to almost nothing. A transition that changes only PART of the
    /// screen — a slide's title line, a changed figure — still counts as a
    /// near-duplicate at these thresholds and is DROPPED. Measured: a fixture
    /// with four distinct slides collapses to ONE frame, and M1's clip kept
    /// 2 of 325. Suits material where only wholesale screen changes matter;
    /// it does NOT suit slides you want one frame of each.
    Aggressive,
}

impl Dedup {
    /// The ffmpeg `mpdecimate` argument, or `None` to skip the filter.
    ///
    /// The thresholds are the knob positions measured in M1, not invented
    /// here (`Gentle` is mpdecimate's own default — M1's "285 of 325" row).
    /// One caveat travels with the citation: M1 ran mpdecimate on the
    /// FULL-RATE stream, while this chain feeds it the `fps`-sampled one, so
    /// consecutive frames are farther apart in time and differ MORE. Static
    /// content is unaffected (identical frames are identical at any rate);
    /// moving content is dropped the same or less (measured: medium kept 71%
    /// of a scrolling clip at full rate, 75% of the same clip sampled at
    /// 4 fps). The ORDERING of the knob transfers; M1's exact counts do not.
    fn filter(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Gentle => Some("mpdecimate=hi=64*12:lo=64*5:frac=0.33"),
            Self::Medium => Some("mpdecimate=hi=64*48:lo=64*24:frac=0.5"),
            Self::Aggressive => Some("mpdecimate=hi=64*200:lo=64*100:frac=0.7"),
        }
    }

    /// Parse a caller's spelling. Unknown values are an ERROR, never a silent
    /// fallback to the default — a caller who typed `agressive` asked for
    /// aggression and must not quietly get `medium`.
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "none" => Ok(Self::None),
            "gentle" => Ok(Self::Gentle),
            "medium" => Ok(Self::Medium),
            "aggressive" => Ok(Self::Aggressive),
            other => Err(format!(
                "unknown dedup {other:?} — use none, gentle, medium or aggressive"
            )),
        }
    }
}

/// Extract a recording's frames, scoring each for sharpness.
///
/// # No cap, deliberately
///
/// `analysis::ocr::ocr_video` caps extraction at 20 frames, which silently
/// truncates any material longer than that — the actual blocker for a
/// lesson-length recording. Nothing here caps: a caller who wants fewer frames
/// lowers the rate or raises the dedup, both of which are honest choices they
/// made, rather than a limit they never saw.
///
/// # The rows describe THIS run
///
/// Frames land in `out_dir` as `f_NNNNN.png`. That exact pattern is cleared
/// before extraction and is the only pattern read back afterwards, so a
/// re-run into the same directory — at a different threshold, say — reports
/// what IT kept, not what a previous run left behind, and a caller's
/// unrelated images in `out_dir` are never scored as frames.
///
/// # Errors
///
/// A missing or failing `ffmpeg` is an error naming what happened. It is NEVER
/// an empty frame list — "the recording has no frames" and "I could not look"
/// are different facts, and a caller that cannot tell them apart will report the
/// wrong one.
/// The files this module's ffmpeg invocation writes: `f_NNNNN.png`, sorted.
///
/// Matching OUR exact pattern — not "any .png" — is what keeps a caller's
/// unrelated image in the output directory from being scored and returned as
/// if it were a frame of the recording.
fn read_frame_files(out_dir: &std::path::Path) -> Result<Vec<std::path::PathBuf>, String> {
    let is_ours = |p: &std::path::Path| {
        p.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| {
                // At least 5 digits: ffmpeg widens %05d past 99999 frames.
                n.strip_prefix("f_")
                    .and_then(|n| n.strip_suffix(".png"))
                    .is_some_and(|d| d.len() >= 5 && d.bytes().all(|b| b.is_ascii_digit()))
            })
    };
    let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(out_dir)
        .map_err(|e| format!("cannot read {}: {e}", out_dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| is_ours(p))
        .collect();
    paths.sort();
    Ok(paths)
}

pub fn extract_frames(
    video: &std::path::Path,
    fps: f64,
    dedup: Dedup,
    out_dir: &std::path::Path,
) -> Result<Vec<FrameRow>, String> {
    if fps <= 0.0 || !fps.is_finite() {
        return Err(format!("fps must be positive and finite, got {fps}"));
    }
    std::fs::create_dir_all(out_dir)
        .map_err(|e| format!("cannot create {}: {e}", out_dir.display()))?;

    // The rows must describe THIS run. The frames are collected by reading the
    // directory back, so a leftover f_NNNNN.png from a previous run — say, a
    // re-run at a harder threshold into the same directory — would be reported
    // as if this run had kept it, silently inflating the count. Clear our own
    // pattern first; anything else in the directory is not ours to touch.
    for stale in read_frame_files(out_dir)? {
        std::fs::remove_file(&stale)
            .map_err(|e| format!("cannot clear stale frame {}: {e}", stale.display()))?;
    }

    // Dedup runs BEFORE anything downstream is paid for. The existing
    // ocr_video dedups only after reading every frame — the expensive order
    // that T018 repairs. The threshold itself is the caller's (D015-7).
    // ORDER MATTERS, and getting it wrong is silent. `mpdecimate,fps=N` drops
    // duplicates and then the `fps` filter RESAMPLES them back up to hit the
    // requested rate — undoing the deduplication that was just performed, with
    // no error and an unchanged frame count. Sample first, then drop duplicates
    // from what was sampled.
    let mut vf = format!("fps={fps}");
    if let Some(f) = dedup.filter() {
        vf.push(',');
        vf.push_str(f);
    }

    let pattern = out_dir.join("f_%05d.png");
    let out = std::process::Command::new("ffmpeg")
        .args([
            "-v", "error",
            "-i", &video.to_string_lossy(),
            "-vf", &vf,
            // mpdecimate is now LAST in the chain, so its output is a
            // variable-rate stream. vfr pins the muxer to pass that through
            // unresampled. Measured on ffmpeg 4.4: the image-sequence muxer
            // already defaults to this, so the flag changes nothing today —
            // it makes the intent explicit instead of leaning on a muxer
            // default that is not ours to rely on.
            "-vsync", "vfr",
            &pattern.to_string_lossy(),
            "-y",
        ])
        .output()
        .map_err(|e| {
            format!(
                "ffmpeg could not be run ({e}) — it is required for frame extraction. \
                 Install it (e.g. `apt install ffmpeg`) and retry."
            )
        })?;
    if !out.status.success() {
        return Err(format!(
            "ffmpeg failed on {}: {}",
            video.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }

    let paths = read_frame_files(out_dir)?;

    paths
        .into_iter()
        .enumerate()
        .map(|(index, path)| {
            let sharpness = sharpness_of_file(&path)?;
            Ok(FrameRow { index, path, sharpness })
        })
        .collect()
}
