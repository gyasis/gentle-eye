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
//!
//! # Every row carries WHEN it was taken
//!
//! A kept frame's `index` counts the frames that SURVIVED deduplication, so
//! under any [`Dedup`] but `None` it says nothing about time — `index / fps` is
//! plausible, wrong, and undetectable from the rows. The timestamp on each
//! [`FrameRow`] is ffmpeg's own presentation time for that frame, read back
//! from the extraction; see [`extract_frames`] for how it is tied to the file.

use serde::Serialize;

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
///
/// Serialises flat — `{index, timestamp_s, path, sharpness}` — so an agent
/// reading the rows from a shell sees the fields under these names.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FrameRow {
    /// Position in the kept sequence, from 0. **Not a clock**: after
    /// deduplication it counts survivors, so `index / fps` is wrong for any
    /// [`Dedup`] but `None`. Use `timestamp_s`.
    pub index: usize,
    /// When the frame was taken, in seconds from the start of the recording:
    /// the presentation timestamp ffmpeg reported for THIS frame as it wrote
    /// it, not a value inferred from `index`. Finite, non-negative, strictly
    /// increasing down the rows. Precision is ffmpeg's — six significant
    /// figures, so millisecond-scale for anything shorter than a few hours.
    pub timestamp_s: f64,
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
/// # Where the timestamp comes from
///
/// ffmpeg is asked to report every frame that reaches the END of the filter
/// chain — after `fps` and after dedup, so exactly the frames it then writes —
/// into a small file beside the frames (the `metadata=print` filter; `showinfo`
/// would do the same but logs at `info`, which `-v error` suppresses). Both the
/// report and the written files are one entry per surviving frame, in stream
/// order, so line *i* is file *i*. That pairing is only sound if the counts
/// agree, so **a count mismatch is a stated error and returns no rows** — a
/// truncating zip would silently drop the tail of a recording, which is the
/// silent failure this module exists to refuse. The report is removed once
/// read; only the frames stay in `out_dir`.
///
/// # Errors
///
/// A missing or failing `ffmpeg` is an error naming what happened. It is NEVER
/// an empty frame list — "the recording has no frames" and "I could not look"
/// are different facts, and a caller that cannot tell them apart will report the
/// wrong one. A timestamp report that does not match the written frames is an
/// error for the same reason.
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

/// Where ffmpeg's per-frame timestamp report lands inside `out_dir`. Named so
/// that [`read_frame_files`] can never mistake it for a frame.
const TIMESTAMPS_FILE: &str = "timestamps.txt";

/// The timestamps in ffmpeg's `metadata=print` report, one per surviving
/// frame, checked against the number of files actually written.
///
/// The report is one header line per frame —
/// `frame:N    pts:P       pts_time:T` — followed by that frame's metadata
/// lines. `N` is ffmpeg's own count of frames through the filter, from 0, so
/// it must equal the line's position; `T` is the presentation time in
/// seconds. Anything else — a count that differs from `files`, a missing or
/// unparsable time, a time that is negative or not finite, a time that does
/// not increase — is refused with a message saying which. None of these may
/// be smoothed over: each would put a plausible, wrong time on a row.
fn parse_timestamps(report: &str, files: usize) -> Result<Vec<f64>, String> {
    let mut out: Vec<f64> = Vec::new();
    for line in report.lines().filter(|l| l.starts_with("frame:")) {
        let position = out.len();
        let mut fields = line.split_whitespace();
        let n: usize = fields
            .next()
            .and_then(|f| f.strip_prefix("frame:"))
            .and_then(|n| n.parse().ok())
            .ok_or_else(|| format!("ffmpeg timestamp report: unreadable frame number in {line:?}"))?;
        if n != position {
            return Err(format!(
                "ffmpeg timestamp report: frame {n} reported at position {position} — \
                 the report is not one line per frame in order, so it cannot be paired with the files"
            ));
        }
        let t: f64 = fields
            .find_map(|f| f.strip_prefix("pts_time:"))
            .and_then(|t| t.parse().ok())
            .ok_or_else(|| format!("ffmpeg timestamp report: no readable pts_time in {line:?}"))?;
        if !t.is_finite() || t < 0.0 {
            return Err(format!("ffmpeg timestamp report: frame {n} has an impossible time {t}"));
        }
        if let Some(&prev) = out.last() {
            if t <= prev {
                return Err(format!(
                    "ffmpeg timestamp report: frame {n} at {t}s does not come after frame {} at {prev}s",
                    n - 1
                ));
            }
        }
        out.push(t);
    }
    if out.len() != files {
        return Err(format!(
            "ffmpeg wrote {files} frame files but reported {} timestamps — they cannot be paired, \
             so no rows are returned rather than rows with guessed times",
            out.len()
        ));
    }
    Ok(out)
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
    let report_path = out_dir.join(TIMESTAMPS_FILE);
    if report_path.exists() {
        std::fs::remove_file(&report_path)
            .map_err(|e| format!("cannot clear stale {}: {e}", report_path.display()))?;
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
    // LAST in the chain, so it sees exactly the frames that survived and are
    // about to be written: `metadata=print` writes one `frame:N ... pts_time:T`
    // line per frame to the report file. ffmpeg 4.4's `metadata` filter skips
    // a frame that carries no metadata at all (a decoded video frame usually
    // has none) — measured: 80 files, 0 lines — so a key is added just before
    // the print and deleted just after, leaving the PNGs untouched.
    vf.push_str(",metadata=mode=add:key=kept:value=1,metadata=mode=print:file=");
    vf.push_str(TIMESTAMPS_FILE);
    vf.push_str(",metadata=mode=delete:key=kept");

    // ffmpeg runs INSIDE out_dir so that the report file and the frame
    // pattern are bare names: a path inside a filtergraph string has to be
    // escaped at two levels (`:` `,` `\` `'`), and a caller's directory name
    // is not ours to get that right for. The input path is made absolute so
    // the change of directory cannot re-resolve a relative one.
    let video = std::path::absolute(video)
        .map_err(|e| format!("cannot resolve {}: {e}", video.display()))?;
    let out = std::process::Command::new("ffmpeg")
        .current_dir(out_dir)
        .args([
            "-v", "error",
            "-i", &video.to_string_lossy(),
            "-vf", &vf,
            // mpdecimate is the last filter that touches timing (only the
            // metadata bookkeeping follows it), so the stream reaching the
            // muxer is variable-rate. vfr pins the muxer to pass that through
            // unresampled. Measured on ffmpeg 4.4: the image-sequence muxer
            // already defaults to this, so the flag changes nothing today —
            // it makes the intent explicit instead of leaning on a muxer
            // default that is not ours to rely on.
            "-vsync", "vfr",
            "f_%05d.png",
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
    let report = std::fs::read_to_string(&report_path).map_err(|e| {
        format!(
            "ffmpeg succeeded but left no timestamp report at {}: {e} — \
             without it the frames cannot be timed, so no rows are returned",
            report_path.display()
        )
    })?;
    std::fs::remove_file(&report_path)
        .map_err(|e| format!("cannot remove {}: {e}", report_path.display()))?;
    // One timestamp per written file, or a stated error — never a zip that
    // stops at the shorter side.
    let timestamps = parse_timestamps(&report, paths.len())?;

    paths
        .into_iter()
        .zip(timestamps)
        .enumerate()
        .map(|(index, (path, timestamp_s))| {
            let sharpness = sharpness_of_file(&path)?;
            Ok(FrameRow { index, timestamp_s, path, sharpness })
        })
        .collect()
}

#[cfg(test)]
mod timestamp_tests {
    use super::*;

    /// The report exactly as ffmpeg 4.4 writes it (measured on a real
    /// recording): a header per frame, then the frame's metadata lines. After
    /// dedup the `pts` column jumps — that gap is the whole point.
    const REPORT: &str = "frame:0    pts:0       pts_time:0\nkept=1\n\
                          frame:1    pts:1       pts_time:0.25\nkept=1\n\
                          frame:2    pts:2       pts_time:0.5\nkept=1\n\
                          frame:3    pts:117     pts_time:29.25\nkept=1\n\
                          frame:4    pts:119     pts_time:29.75\nkept=1\n";

    #[test]
    fn one_timestamp_per_header_line_in_order() {
        assert_eq!(parse_timestamps(REPORT, 5).unwrap(), vec![0.0, 0.25, 0.5, 29.25, 29.75]);
        assert_eq!(parse_timestamps("", 0).unwrap(), Vec::<f64>::new(), "no frames, no report, no rows");
    }

    /// The guard the contract's "stated failure, never a silent one" rule
    /// demands. Five timestamps against six files means a frame has no time;
    /// against four, a time has no frame. Either way the pairing is unknown,
    /// so the answer is an error naming both counts — never the first four
    /// rows with the tail quietly gone.
    #[test]
    fn a_count_mismatch_is_a_stated_error_never_a_truncating_zip() {
        for files in [4, 6] {
            let err = parse_timestamps(REPORT, files).unwrap_err();
            assert!(
                err.contains(&format!("{files} frame files")) && err.contains("5 timestamps"),
                "the error must name both counts, got: {err}"
            );
        }
    }

    /// A malformed report is refused rather than read around: a missing time,
    /// an impossible time, a time that does not advance, or a frame counter
    /// that does not match the line's position. Each would otherwise end as a
    /// plausible wrong timestamp on a row.
    #[test]
    fn a_malformed_report_is_refused_not_repaired() {
        let cases = [
            ("frame:0    pts:0       pts_time:0\nframe:1    pts:4\n", "no readable pts_time"),
            ("frame:0    pts:0       pts_time:nan\n", "impossible time"),
            ("frame:0    pts:0       pts_time:-1\n", "impossible time"),
            ("frame:0    pts:0       pts_time:1\nframe:1    pts:4       pts_time:1\n", "does not come after"),
            ("frame:0    pts:0       pts_time:0\nframe:5    pts:4       pts_time:1\n", "reported at position 1"),
            ("frame:x    pts:0       pts_time:0\n", "unreadable frame number"),
        ];
        for (report, expect) in cases {
            let files = report.matches("frame:").count();
            let err = parse_timestamps(report, files).unwrap_err();
            assert!(err.contains(expect), "for {report:?} expected {expect:?}, got: {err}");
        }
    }

    /// THE CRUX, against real ffmpeg: under dedup the timestamps are the
    /// times the screen actually changed, not `index / fps`.
    ///
    /// The fixture's text changes at 1, 4, 5 and 9 seconds — deliberately
    /// irregular. At 4 fps with medium dedup the kept frames must sit at those
    /// moments (plus the first frame), so their spacing is 1, 3, 1, 4 — while
    /// `index / fps` would say 0, 0.25, 0.5, 0.75, 1.0. Uniform spacing here
    /// would mean the wrong thing is being measured.
    #[test]
    #[ignore = "live: needs ffmpeg"]
    fn under_dedup_timestamps_are_when_the_screen_changed_not_index_over_fps() {
        let dir = tempfile::tempdir().unwrap();
        let video = dir.path().join("steps.mp4");
        let rendered = std::process::Command::new("ffmpeg")
            .args([
                "-v", "error", "-f", "lavfi",
                "-i", "color=c=black:s=640x360:r=24:d=12",
                "-vf",
                "drawtext=text='STEP %{eif\\:gte(t\\,1)+gte(t\\,4)+gte(t\\,5)+gte(t\\,9)\\:d}':\
                 fontcolor=white:fontsize=48:x=40:y=180:box=1:boxcolor=black",
                "-pix_fmt", "yuv420p",
                &video.to_string_lossy(), "-y",
            ])
            .output()
            .expect("run ffmpeg");
        assert!(rendered.status.success(), "fixture: {}", String::from_utf8_lossy(&rendered.stderr));

        let rows = extract_frames(&video, 4.0, Dedup::Medium, &dir.path().join("out")).unwrap();
        let times: Vec<f64> = rows.iter().map(|r| r.timestamp_s).collect();
        eprintln!("[timestamps] kept {} frames at {times:?}", rows.len());
        assert_eq!(times, vec![0.0, 1.0, 4.0, 5.0, 9.0], "the kept frames are the moments the text changed");
        let by_index: Vec<f64> = rows.iter().map(|r| r.index as f64 / 4.0).collect();
        assert_ne!(times, by_index, "index / fps is exactly the wrong answer this field exists to replace");
        assert!(
            !dir.path().join("out").join(TIMESTAMPS_FILE).exists(),
            "the report is read and removed; only frames stay in out_dir"
        );
    }
}
