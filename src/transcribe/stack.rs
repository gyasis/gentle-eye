//! Primitive 4 — **stack**: given N frames of the same screen, the best single image.
//!
//! Register N frames of ONE screen region to each other sub-pixel (ECC), resample
//! each into a `scale`-x grid through its own warp, and combine them (median by
//! default). Handheld jitter gives sub-pixel diversity, and a median of registered
//! frames rejects noise and minority positions that no single frame can.
//!
//! Feature-gated on `tracking` (opencv). The default build compiles the types,
//! the memory estimate and the medoid, and returns [`StackError::NotCompiled`]
//! from the entry points — no system libraries, per the repo rule.
//!
//! # Scores, not a verdict
//!
//! **This module never says whether stacking helped.** An earlier version
//! returned `improved: bool`, and it was measured to be unfalsifiable: it
//! sharpened the stack and compared it against an UNsharpened plain `resize`,
//! and unsharp masking alone inflates variance-of-Laplacian by ~50% while the
//! warp resampler adds another ~35% over `resize` — so 15 IDENTICAL copies of
//! one frame, where stacking can recover nothing, scored HIGHER than a real
//! burst and reported success. The number measured the post-processing, not
//! the stacking. Measured honestly — the stack against the reference frame
//! alone, pushed through the SAME resampler and the SAME post-processing — the
//! reference burst scores 14.6 stacked vs 16.9 single (10-frame median, 3x):
//! about 14% LOWER, because the metric rewards the noise and Lanczos ringing
//! that the median suppresses.
//!
//! So the report carries the SCORES — [`StackReport::sharpness_single`],
//! [`StackReport::sharpness_stacked`], [`StackReport::residual_mad`] — and the
//! judgement is the caller's (`contracts/primitives.md`: "Returns scores, not a
//! verdict. The reject threshold is the caller's"). No margin lives here.
//!
//! Sharpness cannot see the characteristic failure. A stack whose frames have
//! drifted GHOSTS, and a ghosted result scores well on any focus measure. The
//! residual spread does move — measured 1.83 clean / 3.87 ghosted / 6.76 for
//! two frames 24 px apart — which is why `residual_mad` is in the report.
//!
//! `dnn_superres` is bound and could upscale here, but it is deliberately NOT
//! used: learned upscaling INVENTS detail, and on text it produces confident
//! wrong glyphs — the exact failure this module exists to avoid. Stacking
//! cannot fix defocus either: it recovers what is present in SOME frame, never
//! information absent from every frame.
//!
//! # Every threshold is the caller's
//!
//! [`StackOpts`] carries every knob with a documented default — the scale, the
//! combine mode, the motion model, the coherence tolerance
//! ([`StackOpts::max_shift_px`], [`StackOpts::min_response`]) and the memory
//! budget. A default is a starting point the caller can see; a constant buried
//! in the module is a decision the caller cannot.
//!
//! # What is preserved, and why
//!
//! Ported from a Python implementation validated end-to-end on two real
//! recordings. Three of its hard-won properties are kept deliberately and are
//! commented at their call sites, because each produced a *plausible report
//! alongside a wrong image*:
//!   1. the warp is `W . S^-1`, never `S . W . S^-1` (a BLACK image, every
//!      frame reported used);
//!   2. a burst spanning a scroll must be SPLIT before combining, and the
//!      naive consecutive-frame check cannot see a slow scroll;
//!   3. a result that is mostly empty is a geometry bug and is refused.

use serde::Serialize;

/// Index of the 2-D medoid: the point minimising the total Euclidean distance to
/// all the others. Ties go to the lowest index. Empty input returns 0.
///
/// Used by [`coherent_group`] to choose the shift cluster to keep. Unlike a
/// per-axis median, the medoid is always an actual member of the set — a
/// per-axis median can take x from one cluster and y from another and land on a
/// phantom centre that no frame is near.
pub fn medoid_2d(pts: &[(f64, f64)]) -> usize {
    let mut best = 0usize;
    let mut best_cost = f64::INFINITY;
    for (i, a) in pts.iter().enumerate() {
        let cost: f64 = pts.iter().map(|b| ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()).sum();
        if cost < best_cost {
            best_cost = cost;
            best = i;
        }
    }
    best
}

/// How frames are combined once registered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Reduce {
    /// Best signal-to-noise, but any residual misalignment ghosts.
    Mean,
    /// Rejects minority positions. Default: measurably better than the mean
    /// on text (mean 12.6 vs median 14.6 on the reference burst) — though
    /// neither beats the single reference frame through the same pipeline
    /// (16.9); see the module header.
    #[default]
    Median,
}

/// Motion model used to register frames to the reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Motion {
    Euclidean,
    Affine,
    /// A plane under camera motion maps by a homography exactly — correct for a
    /// screen, and the default.
    #[default]
    Homography,
}

/// Default [`StackOpts::max_shift_px`]: 2 px. Sub-pixel jitter from a handheld
/// phone stays well under this; a scroll of even one text line is far over it.
pub const DEFAULT_MAX_SHIFT_PX: f64 = 2.0;

/// Default [`StackOpts::min_response`]: 0.5. A featureless frame (a blank or
/// nearly blank screen) comes back from phase correlation with response ~0,
/// and must not be stacked as if it were aligned.
pub const DEFAULT_MIN_RESPONSE: f64 = 0.5;

/// Default [`StackOpts::memory_budget_bytes`]: 2 GiB.
///
/// Why 2 GiB: this runs on a laptop that is also recording the screen, so a
/// call should stay well inside the free half of a 4 GB machine. Measured
/// (release, this OpenCV build) it admits what the module is for — a screen
/// ROI: 60 frames of 500x220 at 3x is ~0.9 GB, 10 frames of 960x540 at 3x is
/// ~1.5 GB — and refuses what the header already says not to do: two 1080p
/// frames at 3x measured 3.3 GB and used to be attempted anyway, and a
/// 30-frame 1080p burst at 3x would need ~10 GB.
pub const DEFAULT_MEMORY_BUDGET_BYTES: u64 = 2 << 30;

/// Largest accepted [`StackOpts::scale`].
///
/// The measured default is 3, and a burst of a dozen handheld frames carries
/// roughly that much sub-pixel diversity; past it the Lanczos resampler is
/// only interpolating, not resolving. 8 leaves headroom for experiments while
/// keeping every product in [`stack`] far inside `i32` for any frame that fits
/// in memory — `scale = 5_000_000` used to panic on `rw * scale` in debug and
/// wrap to a garbage size in release. This is an overflow bound, not a
/// judgement about content, which is why it is not a knob.
pub const MAX_SCALE: i32 = 8;

/// Knobs for [`stack`]. Every threshold the algorithm consults is here, with a
/// documented default — the caller owns all of them.
#[derive(Debug, Clone, Copy)]
pub struct StackOpts {
    /// Output resolution multiplier, `1..=MAX_SCALE`. Anything else is refused
    /// with [`StackError::BadScale`] before any work is done. Default 3.
    pub scale: i32,
    pub motion: Motion,
    pub reduce: Reduce,
    /// Drop frames whose content has moved (a scroll mid-burst) before
    /// combining. Default on. With it off every frame is stacked and the
    /// report's [`StackReport::incoherent`] says how many had moved.
    pub coherence: bool,
    /// Coherence tolerance: a frame whose content shift (from the kept
    /// cluster's medoid) exceeds this on either axis, in pixels, is showing
    /// different content. Default [`DEFAULT_MAX_SHIFT_PX`]. Must be finite and
    /// non-negative.
    pub max_shift_px: f64,
    /// Coherence floor: a frame whose phase-correlation response against the
    /// middle frame is below this has too little shared structure to place.
    /// Default [`DEFAULT_MIN_RESPONSE`]. Must be finite.
    pub min_response: f64,
    /// Upper bound on the peak memory this call may commit to, in bytes. The
    /// estimate is [`stack_memory_estimate`]; exceeding it returns
    /// [`StackError::TooMuchMemory`] BEFORE anything is allocated. Default
    /// [`DEFAULT_MEMORY_BUDGET_BYTES`].
    pub memory_budget_bytes: u64,
}

impl Default for StackOpts {
    fn default() -> Self {
        Self {
            scale: 3,
            motion: Motion::default(),
            reduce: Reduce::default(),
            coherence: true,
            max_shift_px: DEFAULT_MAX_SHIFT_PX,
            min_response: DEFAULT_MIN_RESPONSE,
            memory_budget_bytes: DEFAULT_MEMORY_BUDGET_BYTES,
        }
    }
}

/// What happened, as numbers. Serialises (`serde`) so an agent on any harness
/// can read it from a shell. Nothing here is a verdict — see the module header.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StackReport {
    /// Frames handed in.
    pub burst: usize,
    /// Frames that survived the coherence filter AND registered.
    pub used: usize,
    /// Frames that survived the coherence filter but ECC could not register.
    pub failed: usize,
    /// Frames the coherence filter removed. Always 0 with `coherence: false` —
    /// then `incoherent` is the count that moved and were stacked anyway.
    pub dropped_for_movement: usize,
    /// Frames that FAILED the coherence test, whether or not the filter was on:
    /// their shift lay outside [`StackOpts::max_shift_px`] of the cluster
    /// medoid, or their response fell below [`StackOpts::min_response`]. Equals
    /// `dropped_for_movement` with the filter on; with it off this is how many
    /// frames of different content went into the result — expect ghosting,
    /// and read `residual_mad`.
    pub incoherent: usize,
    /// Indices (into the input burst) of the frames that were stacked, i.e. the
    /// coherent cluster the filter chose. Lets the caller see WHICH content won.
    pub kept_frames: Vec<usize>,
    /// Largest content shift of any frame from the kept cluster's medoid, in
    /// pixels (2-D distance). Measured over the WHOLE burst, filter or not.
    pub max_drift_px: f64,
    pub scale: i32,
    /// Sharpness of the reference frame alone, pushed through the SAME
    /// resampler and the SAME post-processing as the stack — the paired
    /// baseline. [`super::frames::sharpness`]: comparable against `sharpness_stacked`
    /// of this same call, not across recordings.
    pub sharpness_single: f64,
    /// Sharpness of the combined result, same measure, same post-processing.
    pub sharpness_stacked: f64,
    /// Ghosting signal, which sharpness cannot see: mean absolute deviation of
    /// every registered frame from the combined result, in 8-bit intensity
    /// units. Near 0 = the frames agree (sensor noise only). High = the
    /// registered frames DISAGREE at the pixel level — residual misregistration
    /// or a scroll that slipped past the coherence filter — and the output is a
    /// blend of different images even if it scores "sharp". Compare against a
    /// known-clean burst rather than an absolute threshold.
    pub residual_mad: f64,
}

/// Working memory [`stack`] needs PER OUTPUT PIXEL on top of the accumulator:
/// the combined result and its 8-bit copy, the blank-check masks, and — by far
/// the largest — the guided filter's per-channel float planes in the
/// post-processing, which run twice (stack and baseline). Measured as
/// `(peak RSS - accumulator) / output pixels` over nine sizes from 0.13 to
/// 18.7 Mpx: 155–200 bytes, converging on ~155 at large outputs. 192 is the
/// conservative end.
pub const POST_BYTES_PER_OUTPUT_PIXEL: u64 = 192;

/// Peak memory [`stack`] will need for `frames` registered frames of `w` x
/// `h` at `scale`, in bytes — what the guard compares against
/// [`StackOpts::memory_budget_bytes`]. Pure arithmetic, so a caller can
/// pre-check a burst without the `tracking` feature.
///
/// Two terms: the accumulator holds every registered frame as a full-size
/// `CV_32FC3` upscale (`w * h * scale^2 * 12` bytes EACH — 60 frames of
/// 500x220 at 3x is 713 MB, and that alone dominated the audit's 903 MB RSS),
/// plus [`POST_BYTES_PER_OUTPUT_PIXEL`] per output pixel, which is what
/// actually dominated for two 1080p frames at 3x (448 MB accumulator, 3.3 GB
/// RSS). An accumulator-only estimate would have admitted that call.
/// Accurate to roughly +-20% against measured RSS. Saturates rather than
/// overflowing on absurd arguments — a saturated estimate is over any budget,
/// so it is refused, never wrapped into something small.
pub fn stack_memory_estimate(frames: usize, w: i32, h: i32, scale: i32) -> u64 {
    let s = u64::from(scale.unsigned_abs());
    let out_px = u64::from(w.unsigned_abs())
        .saturating_mul(s)
        .saturating_mul(u64::from(h.unsigned_abs()))
        .saturating_mul(s);
    (frames as u64)
        .saturating_mul(out_px)
        .saturating_mul(12)
        .saturating_add(out_px.saturating_mul(POST_BYTES_PER_OUTPUT_PIXEL))
}

/// Why a stack call could not produce a trustworthy result. Every variant is a
/// STATED failure: nothing here degrades into a plausible image.
#[derive(Debug, thiserror::Error)]
pub enum StackError {
    #[error("need at least 2 frames to stack, got {0}")]
    TooFewFrames(usize),
    /// Fewer than two frames registered to the reference. One registered frame
    /// upscaled is not a stack, so it is refused rather than reported as one.
    #[error("only {registered} frame(s) registered ({failed} failed); a single frame is not a stack")]
    TooFewRegistered { registered: usize, failed: usize },
    /// The coherence filter found fewer than two frames showing the same
    /// content: the burst spans a scroll or an edit. Stacking it anyway would
    /// ghost, so it is refused. Use a shorter window, widen
    /// `StackOpts::max_shift_px`, or opt out explicitly with
    /// `StackOpts { coherence: false, .. }` — the report's `incoherent` then
    /// says how many frames had moved.
    #[error("only {coherent} of {burst} frames show the same content (max drift {max_drift_px:.1} px) — burst spans a scroll or edit; use a shorter window, or coherence=false to stack anyway")]
    NoCoherentFrames { coherent: usize, burst: usize, max_drift_px: f64 },
    #[error("frames differ in size; crop them to a common ROI first")]
    SizeMismatch,
    /// A frame that is not 8-bit 3-channel BGR (`CV_8UC3`). A 16-bit frame
    /// runs the whole pipeline (ECC is scale-invariant) and then saturates to
    /// an all-WHITE image on the final 8-bit conversion, which the blank guard
    /// — it looks for black — returned as `Ok`; a 4-channel frame died deep in
    /// OpenCV arithmetic with an opaque `StsUnmatchedFormats`.
    #[error("frame {index} is {found}; every frame must be 8-bit 3-channel BGR (CV_8UC3)")]
    UnsupportedFrameType { index: usize, found: String },
    /// `StackOpts::scale` outside `1..=MAX_SCALE`. 0 and negatives used to
    /// surface as OpenCV assertions; a huge value overflowed `i32`.
    #[error("scale must be between 1 and {MAX_SCALE}, got {0}")]
    BadScale(i32),
    /// A coherence tolerance that cannot be compared against: a NaN or
    /// infinite `max_shift_px` / `min_response`, or a negative shift. Refused
    /// up front — a NaN compares false against everything and would drop every
    /// frame, and that would then be reported as "the burst spans a scroll".
    #[error("bad coherence tolerance: {0}")]
    BadTolerance(String),
    /// [`stack_memory_estimate`] for this burst exceeds
    /// [`StackOpts::memory_budget_bytes`]. Refused before anything is allocated.
    #[error(
        "stack would need ~{} MB for {frames} frames at {out_w}x{out_h} (budget {} MB, \
         StackOpts::memory_budget_bytes) — crop to the screen ROI, use fewer frames, or a smaller scale",
        .estimated_bytes >> 20, .budget_bytes >> 20
    )]
    TooMuchMemory { estimated_bytes: u64, budget_bytes: u64, frames: usize, out_w: i64, out_h: i64 },
    /// The geometry guard. A wrong warp yields a black image and an otherwise
    /// healthy-looking report, so an empty result is refused rather than returned.
    #[error("stack is {0:.0}% empty — warp geometry is wrong; refusing a blank result")]
    BlankResult(f64),
    #[error("built without the `tracking` feature; rebuild with --features tracking")]
    NotCompiled,
    #[cfg(feature = "tracking")]
    #[error(transparent)]
    Cv(#[from] opencv::Error),
}

/// Refuse a tolerance that cannot be compared against. See
/// [`StackError::BadTolerance`].
fn check_tolerances(max_shift_px: f64, min_response: f64) -> Result<(), StackError> {
    if !max_shift_px.is_finite() || max_shift_px < 0.0 {
        return Err(StackError::BadTolerance(format!(
            "max_shift_px must be finite and non-negative, got {max_shift_px}"
        )));
    }
    if !min_response.is_finite() {
        return Err(StackError::BadTolerance(format!("min_response must be finite, got {min_response}")));
    }
    Ok(())
}

#[cfg(not(feature = "tracking"))]
mod imp {
    //! Default build: no opencv, no system libraries. The entry points still
    //! exist and compile, and answer with [`StackError::NotCompiled`] — an
    //! actionable error, never a silently different result. They are generic
    //! over the frame type because `opencv::core::Mat` does not exist here;
    //! they never return `Ok`, so the `Ok` type is `()`. The tolerances are
    //! still validated first — pure arithmetic — so a bad option is the same
    //! first error in either build.
    use super::*;

    pub fn stack<F>(_frames: &[F], opts: StackOpts) -> Result<(), StackError> {
        check_tolerances(opts.max_shift_px, opts.min_response)?;
        Err(StackError::NotCompiled)
    }

    pub fn coherent_group<F>(
        _frames: &[F], max_shift_px: f64, min_response: f64,
    ) -> Result<(Vec<usize>, f64), StackError> {
        check_tolerances(max_shift_px, min_response)?;
        Err(StackError::NotCompiled)
    }
}

#[cfg(feature = "tracking")]
mod imp {
    use super::*;
    use crate::transcribe::frames;
    use opencv::core::{self, Mat, Scalar, Size};
    use opencv::{imgproc, prelude::*, video};

    /// [`frames::sharpness`] of a BGR frame — the ONE conversion at the OpenCV
    /// boundary. The measure itself is the pure-Rust one every other primitive
    /// uses, so a stack's scores and a recording's frame scores are the same
    /// number, and the default build needs no second implementation.
    fn sharpness_of(bgr: &Mat) -> Result<f64, StackError> {
        let mut gray = Mat::default();
        imgproc::cvt_color_def(bgr, &mut gray, imgproc::COLOR_BGR2GRAY)?;
        let (w, h) = (gray.cols() as usize, gray.rows() as usize);
        // cvt_color writes a fresh, continuous Mat: one row-major 8-bit plane.
        Ok(frames::sharpness(gray.data_bytes()?, w, h))
    }

    /// Every frame must be 8-bit 3-channel BGR and match the first frame's
    /// dimensions.
    ///
    /// Checked BEFORE any OpenCV call, in one walk over the burst.
    /// `phase_correlate` asserts on a size mismatch, and that used to surface
    /// as an opaque `Cv` error naming phasecorr.cpp instead of the actionable
    /// [`StackError::SizeMismatch`]. `cvt_color` happily accepts a 16-bit
    /// frame, so that one used to run the WHOLE pipeline and come back as a
    /// white image reported `Ok` (see [`StackError::UnsupportedFrameType`]).
    fn check_frames(frames: &[Mat]) -> Result<(), StackError> {
        let Some(first) = frames.first() else { return Ok(()) };
        let (w, h) = (first.cols(), first.rows());
        for (index, f) in frames.iter().enumerate() {
            if f.depth() != core::CV_8U || f.channels() != 3 {
                return Err(StackError::UnsupportedFrameType { index, found: type_name(f) });
            }
            if f.cols() != w || f.rows() != h {
                return Err(StackError::SizeMismatch);
            }
        }
        Ok(())
    }

    /// `CV_16UC3`-style name for an error message.
    fn type_name(m: &Mat) -> String {
        let depth = match m.depth() {
            core::CV_8U => "8U",
            core::CV_8S => "8S",
            core::CV_16U => "16U",
            core::CV_16S => "16S",
            core::CV_32S => "32S",
            core::CV_32F => "32F",
            core::CV_64F => "64F",
            core::CV_16F => "16F",
            other => return format!("depth code {other} with {} channel(s)", m.channels()),
        };
        format!("CV_{depth}C{}", m.channels())
    }

    /// Split a burst into frames showing the SAME screen content.
    ///
    /// A scroll or edit mid-burst puts the same text at two heights; averaging
    /// that ghosts while every frame still "registers", so the report looks
    /// healthy. Note the naive check misses it: CONSECUTIVE-frame difference
    /// stays tiny through a slow scroll while total drift is large. Phase
    /// correlation of every frame against the middle frame catches it.
    ///
    /// Returns `(kept indices, max drift px)`. Drift is the largest 2-D
    /// distance of any frame's shift from the kept cluster's medoid.
    ///
    /// **Which cluster is kept — the assumption, stated:** the frames whose
    /// shift lies within `max_shift_px` of the 2-D MEDOID shift (the frame
    /// whose shift is closest, in total, to all the others) and whose
    /// correlation response is at least `min_response`. For a burst split into
    /// two clusters that is the LARGER cluster, regardless of which came first
    /// in time; for more than two it is the most central one. Nothing here
    /// privileges "before the scroll": if most of a burst is post-scroll, the
    /// post-scroll content is what gets stacked, and `kept_frames` in the
    /// report says so. An earlier version took a median PER AXIS, which could
    /// land on a phantom centre — x from one cluster, y from another — that no
    /// frame was near.
    ///
    /// The two tolerances are the caller's ([`StackOpts::max_shift_px`],
    /// [`StackOpts::min_response`]); a NaN, infinite or negative one is refused
    /// with [`StackError::BadTolerance`]. An empty burst is refused with
    /// [`StackError::TooFewFrames`] rather than indexing the middle frame of
    /// nothing.
    pub fn coherent_group(
        frames: &[Mat], max_shift_px: f64, min_response: f64,
    ) -> Result<(Vec<usize>, f64), StackError> {
        check_tolerances(max_shift_px, min_response)?;
        if frames.is_empty() {
            return Err(StackError::TooFewFrames(0));
        }
        check_frames(frames)?;
        let mid = frames.len() / 2;
        let to_gray_f64 = |m: &Mat| -> Result<Mat, StackError> {
            let mut g = Mat::default();
            imgproc::cvt_color_def(m, &mut g, imgproc::COLOR_BGR2GRAY)?;
            let mut f = Mat::default();
            g.convert_to(&mut f, core::CV_64F, 1.0, 0.0)?;
            Ok(f)
        };
        let reference = to_gray_f64(&frames[mid])?;
        let mut shifts = Vec::with_capacity(frames.len());
        for f in frames {
            let g = to_gray_f64(f)?;
            let mut resp = 0.0_f64;
            let d = imgproc::phase_correlate(&reference, &g, &core::no_array(), &mut resp)?;
            shifts.push((d.x, d.y, resp));
        }
        // Every shift and response is finite: the inputs are validated 8-bit
        // frames, so their DFTs are finite, and OpenCV's phase correlation
        // guards each division with an epsilon (`divSpectrums` FLT_EPSILON,
        // `weightedCentroid` DBL_EPSILON). A featureless burst therefore comes
        // back as response 0 / shift 0 — filtered out by `min_response` below —
        // not as NaN, so `max_drift_px` can never be NaN either. Asserted here
        // so a future OpenCV that drops a guard fails loudly in tests instead
        // of sorting a NaN shift as `Equal` into the medoid.
        debug_assert!(
            shifts.iter().all(|s| s.0.is_finite() && s.1.is_finite() && s.2.is_finite()),
            "phase correlation returned a non-finite shift or response: {shifts:?}"
        );
        let xy: Vec<(f64, f64)> = shifts.iter().map(|s| (s.0, s.1)).collect();
        let (med_x, med_y) = xy[medoid_2d(&xy)];
        let keep: Vec<usize> = shifts
            .iter()
            .enumerate()
            .filter(|(_, (dx, dy, r))| {
                *r >= min_response && (dx - med_x).abs() <= max_shift_px && (dy - med_y).abs() <= max_shift_px
            })
            .map(|(i, _)| i)
            .collect();
        let drift = xy
            .iter()
            .map(|(x, y)| ((x - med_x).powi(2) + (y - med_y).powi(2)).sqrt())
            .fold(0.0_f64, f64::max);
        Ok((keep, drift))
    }

    /// Resample one frame into the `scale`-x output grid through `warp`, the
    /// reference->frame transform as ECC estimates it. With an identity `warp`
    /// this is a plain upscale of `src` by the same Lanczos kernel — which is
    /// what the paired baseline in [`stack`] uses, so that the ONLY difference
    /// between the two scores is N frames versus one.
    fn upscale_through(
        src: &Mat, warp: &Mat, motion: i32, scale: i32, out_w: i32, out_h: i32,
    ) -> Result<Mat, StackError> {
        // WARP_INVERSE_MAP maps DST -> SRC. dst is the UPSCALED grid, src is
        // the ORIGINAL frame, so the map is `W . S^-1` — NOT `S . W . S^-1`,
        // which maps upscaled->upscaled, samples off the edge, and returns a
        // BLACK image while still reporting every frame as used.
        let rows = warp.rows();
        let s = scale as f32;
        let mut scaled = Mat::zeros(rows, 3, core::CV_32F)?.to_mat()?;
        for r in 0..rows {
            for c in 0..3 {
                let v = *warp.at_2d::<f32>(r, c)?;
                let div = if c < 2 { s } else { 1.0 };
                *scaled.at_2d_mut::<f32>(r, c)? = v / div;
            }
        }
        let mut up = Mat::default();
        let flags = imgproc::INTER_LANCZOS4 | imgproc::WARP_INVERSE_MAP;
        if motion == video::MOTION_HOMOGRAPHY {
            imgproc::warp_perspective(
                src, &mut up, &scaled, Size::new(out_w, out_h), flags,
                core::BORDER_CONSTANT, Scalar::default(), core::AlgorithmHint::ALGO_HINT_DEFAULT,
            )?;
        } else {
            imgproc::warp_affine(
                src, &mut up, &scaled, Size::new(out_w, out_h), flags,
                core::BORDER_CONSTANT, Scalar::default(), core::AlgorithmHint::ALGO_HINT_DEFAULT,
            )?;
        }
        Ok(up)
    }

    /// Edge-preserving cleanup (guided filter) then an unsharp mask.
    ///
    /// Applied to the stack AND to the single-frame baseline it is scored
    /// beside. Unsharp masking alone inflates variance-of-Laplacian by ~50%,
    /// so a baseline that skips it makes the pair of scores meaningless —
    /// 15 identical copies of one frame used to "win" against a bare resize.
    ///
    /// A failing guided filter is an error, not a silent skip. An earlier
    /// version fell back to the unfiltered image, which changed the output
    /// with no signal in the report — the exact substitution the contract
    /// forbids.
    fn post_process(img: &Mat) -> Result<Mat, StackError> {
        let mut filtered = Mat::default();
        opencv::ximgproc::guided_filter_def(img, img, &mut filtered, 2, 25.0)?;
        let mut blur = Mat::default();
        imgproc::gaussian_blur(
            &filtered, &mut blur, Size::new(0, 0), 1.2, 1.2, core::BORDER_DEFAULT, core::AlgorithmHint::ALGO_HINT_DEFAULT,
        )?;
        let mut sharp = Mat::default();
        core::add_weighted(&filtered, 1.6, &blur, -0.6, 0.0, &mut sharp, -1)?;
        Ok(sharp)
    }

    /// Register frames sub-pixel and resample into a `scale`-x grid.
    ///
    /// CROP TO THE SCREEN ROI BEFORE CALLING. Registering on a whole frame fits
    /// the desk and bezel — a different depth plane — and misaligns the screen.
    ///
    /// Refuses (rather than quietly degrading) when fewer than two frames show
    /// the same content ([`StackError::NoCoherentFrames`]) or fewer than two
    /// register ([`StackError::TooFewRegistered`]). With `coherence: false`
    /// every frame is stacked, and the report's `incoherent` says how many had
    /// moved.
    pub fn stack(frames: &[Mat], opts: StackOpts) -> Result<(Mat, StackReport), StackError> {
        if frames.len() < 2 {
            return Err(StackError::TooFewFrames(frames.len()));
        }
        check_frames(frames)?;
        if !(1..=MAX_SCALE).contains(&opts.scale) {
            return Err(StackError::BadScale(opts.scale));
        }
        let burst = frames.len();

        // Always measure coherence — it is cheap next to ECC — so the report can
        // never claim `dropped_for_movement: 0` beside a large drift without
        // saying why. An earlier version silently stacked the whole burst when
        // the filter kept <2 frames, with dropped=0 and drift=23px in one report.
        let (keep, drift) = coherent_group(frames, opts.max_shift_px, opts.min_response)?;
        let incoherent = burst - keep.len();
        let kept_frames: Vec<usize> = if opts.coherence {
            if keep.len() < 2 {
                return Err(StackError::NoCoherentFrames {
                    coherent: keep.len(),
                    burst,
                    max_drift_px: drift,
                });
            }
            keep
        } else {
            (0..burst).collect()
        };
        let dropped = burst - kept_frames.len();
        let work: Vec<Mat> = kept_frames.iter().map(|i| frames[*i].clone()).collect();

        let motion = match opts.motion {
            Motion::Euclidean => video::MOTION_EUCLIDEAN,
            Motion::Affine => video::MOTION_AFFINE,
            Motion::Homography => video::MOTION_HOMOGRAPHY,
        };
        let mid = work.len() / 2;
        let reference = work[mid].clone();
        let (rw, rh) = (reference.cols(), reference.rows());
        // i64 so the products cannot overflow for any `i32` frame and any legal
        // `scale`; the memory guard bounds them long before `i32` would.
        let out_w64 = i64::from(rw) * i64::from(opts.scale);
        let out_h64 = i64::from(rh) * i64::from(opts.scale);
        let estimated_bytes = stack_memory_estimate(work.len(), rw, rh, opts.scale);
        let too_much = || StackError::TooMuchMemory {
            estimated_bytes,
            budget_bytes: opts.memory_budget_bytes,
            frames: work.len(),
            out_w: out_w64,
            out_h: out_h64,
        };
        if estimated_bytes > opts.memory_budget_bytes {
            return Err(too_much());
        }
        // Cannot fail once the budget check passed for any sane budget (the
        // pixel count then sits far below i32::MAX), but a library must not
        // panic on it.
        let out_w = i32::try_from(out_w64).map_err(|_| too_much())?;
        let out_h = i32::try_from(out_h64).map_err(|_| too_much())?;

        let mut ref_g = Mat::default();
        imgproc::cvt_color_def(&reference, &mut ref_g, imgproc::COLOR_BGR2GRAY)?;
        let mut ref_f = Mat::default();
        ref_g.convert_to(&mut ref_f, core::CV_32F, 1.0 / 255.0, 0.0)?;

        let rows = if motion == video::MOTION_HOMOGRAPHY { 3 } else { 2 };
        let mut acc: Vec<Mat> = Vec::with_capacity(work.len());
        let (mut used, mut failed) = (0usize, 0usize);

        for f in &work {
            let mut g = Mat::default();
            imgproc::cvt_color_def(f, &mut g, imgproc::COLOR_BGR2GRAY)?;
            let mut gf = Mat::default();
            g.convert_to(&mut gf, core::CV_32F, 1.0 / 255.0, 0.0)?;

            let mut warp = Mat::eye(rows, 3, core::CV_32F)?.to_mat()?;
            let crit = core::TermCriteria::new(
                core::TermCriteria_EPS + core::TermCriteria_COUNT, 200, 1e-6,
            )?;
            if video::find_transform_ecc(
                &ref_f, &gf, &mut warp, motion, crit, &core::no_array(), 5,
            )
            .is_err()
            {
                failed += 1;
                continue;
            }

            let up = upscale_through(f, &warp, motion, opts.scale, out_w, out_h)?;
            let mut upf = Mat::default();
            up.convert_to(&mut upf, core::CV_32F, 1.0, 0.0)?;
            acc.push(upf);
            used += 1;
        }
        if used < 2 {
            return Err(StackError::TooFewRegistered { registered: used, failed });
        }

        let combined = match opts.reduce {
            Reduce::Mean => {
                let mut sum = Mat::zeros(out_h, out_w, core::CV_32FC3)?.to_mat()?;
                for m in &acc {
                    let mut t = Mat::default();
                    core::add(&sum, m, &mut t, &core::no_array(), -1)?;
                    sum = t;
                }
                let mut avg = Mat::default();
                sum.convert_to(&mut avg, core::CV_32F, 1.0 / acc.len() as f64, 0.0)?;
                avg
            }
            Reduce::Median => median_of(&acc, out_w, out_h)?,
        };

        // Ghosting signal: how far each registered frame sits from the result.
        // Sharpness cannot detect ghosting — a burst spanning a scroll, stacked
        // with coherence off, scores SHARPER than the clean stack — this can.
        let mut mad_sum = 0.0_f64;
        for m in &acc {
            let mut d = Mat::default();
            core::absdiff(m, &combined, &mut d)?;
            let mean = core::mean_def(&d)?;
            mad_sum += (mean[0] + mean[1] + mean[2]) / 3.0;
        }
        let residual_mad = mad_sum / acc.len() as f64;

        let mut out = Mat::default();
        combined.convert_to(&mut out, core::CV_8U, 1.0, 0.0)?;

        // A geometry bug yields a plausible report and a BLANK image. Refuse it.
        let mut g8 = Mat::default();
        imgproc::cvt_color_def(&out, &mut g8, imgproc::COLOR_BGR2GRAY)?;
        let mut mask = Mat::default();
        imgproc::threshold(&g8, &mut mask, 8.0, 255.0, imgproc::THRESH_BINARY)?;
        let nonblack = f64::from(core::count_non_zero(&mask)?) / (f64::from(out_w) * f64::from(out_h));
        if nonblack < 0.30 {
            return Err(StackError::BlankResult(100.0 * (1.0 - nonblack)));
        }

        let sharp = post_process(&out)?;

        // The paired baseline: the reference frame alone, pushed through the
        // SAME resampler (identity warp) and the SAME post-processing, so the
        // only variable between the two scores is N frames versus one.
        // Comparing against a bare `resize` measured the resampler and the
        // unsharp mask instead: 15 identical copies of one frame scored 16.7
        // against a resize at 8.0. Both numbers go in the report; whether the
        // difference means anything is the caller's call, and there is no
        // margin here to make it for them.
        let identity = Mat::eye(rows, 3, core::CV_32F)?.to_mat()?;
        let base_up = upscale_through(&reference, &identity, motion, opts.scale, out_w, out_h)?;
        let base = post_process(&base_up)?;
        let (sharpness_single, sharpness_stacked) = (sharpness_of(&base)?, sharpness_of(&sharp)?);

        Ok((
            sharp,
            StackReport {
                burst,
                used,
                failed,
                dropped_for_movement: dropped,
                incoherent,
                kept_frames,
                max_drift_px: drift,
                scale: opts.scale,
                sharpness_single,
                sharpness_stacked,
                residual_mad,
            },
        ))
    }

    /// Per-pixel median across the stack. Rejects minority positions, which is
    /// why it beats the mean on text.
    ///
    /// Walks each row as a slice (`at_row`) rather than one bounds-and-type-
    /// checked `at_2d` per pixel per frame per channel. Same values, same
    /// sort, same middle element — only the access pattern changed, and the
    /// output is byte-identical (`median_row_walk_matches_the_per_pixel_definition`
    /// pins it). 60 frames at 1500x660 in release: 13.8 s -> 7.7 s for the
    /// whole `stack` call (the mean path is 4.6 s, so the median itself went
    /// from ~9 s to ~3 s).
    pub(super) fn median_of(acc: &[Mat], w: i32, h: i32) -> Result<Mat, StackError> {
        let n = acc.len();
        let mut out = Mat::zeros(h, w, core::CV_32FC3)?.to_mat()?;
        let mut buf = vec![0.0_f32; n];
        let mut rows: Vec<&[core::Vec3f]> = Vec::with_capacity(n);
        for y in 0..h {
            rows.clear();
            for m in acc {
                rows.push(m.at_row::<core::Vec3f>(y)?);
            }
            for (x, px) in out.at_row_mut::<core::Vec3f>(y)?.iter_mut().enumerate() {
                for ch in 0..3 {
                    for (b, row) in buf.iter_mut().zip(&rows) {
                        *b = row[x][ch];
                    }
                    buf.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                    px[ch] = if n % 2 == 1 { buf[n / 2] } else { (buf[n / 2 - 1] + buf[n / 2]) / 2.0 };
                }
            }
        }
        Ok(out)
    }
}

pub use imp::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn medoid_is_a_member_of_the_larger_cluster() {
        // Two clusters: 4 frames post-scroll at y≈24, 3 pre-scroll at y≈0. The
        // larger cluster wins regardless of temporal order.
        let pts = [(0.0, 0.1), (0.1, 0.0), (0.0, -0.1), (0.0, 24.0), (0.1, 24.1), (-0.1, 23.9), (0.0, 24.2)];
        let m = medoid_2d(&pts);
        assert!(m >= 3, "medoid {m} should be in the post-scroll (larger) cluster");
        // Per-axis medians of these four land on (10,10) — a phantom centre no
        // frame is near. The medoid is always an actual member.
        let phantom = [(0.0, 10.0), (0.0, 10.0), (10.0, 0.0), (10.0, 0.0)];
        let m = medoid_2d(&phantom);
        assert!(phantom.contains(&phantom[m]));
        assert_eq!(medoid_2d(&[]), 0);
    }

    #[test]
    fn defaults_are_the_measured_best() {
        let o = StackOpts::default();
        assert_eq!(o.reduce, Reduce::Median, "median beat mean on text");
        assert_eq!(o.motion, Motion::Homography, "a screen is a plane");
        assert!(o.coherence, "a scroll mid-burst must be split, not averaged");
        // The thresholds that used to be module constants, now visible knobs
        // with the same values — the move to the caller changed no default.
        assert_eq!(o.max_shift_px, 2.0);
        assert_eq!(o.min_response, 0.5);
        assert_eq!(o.memory_budget_bytes, 2 << 30);
    }

    /// The contract's machine-readable requirement: the report serialises with
    /// its field names, and the two verdict fields an audit found unfalsifiable
    /// (`improved`, `warning`) are GONE, not renamed.
    #[test]
    fn the_report_is_machine_readable_and_carries_no_verdict() {
        let rep = StackReport {
            burst: 15,
            used: 10,
            failed: 0,
            dropped_for_movement: 5,
            incoherent: 5,
            kept_frames: (0..10).collect(),
            max_drift_px: 24.3,
            scale: 3,
            sharpness_single: 16.9,
            sharpness_stacked: 14.6,
            residual_mad: 1.83,
        };
        let v = serde_json::to_value(&rep).unwrap();
        let obj = v.as_object().unwrap();
        for key in [
            "burst", "used", "failed", "dropped_for_movement", "incoherent", "kept_frames",
            "max_drift_px", "scale", "sharpness_single", "sharpness_stacked", "residual_mad",
        ] {
            assert!(obj.contains_key(key), "missing {key}: {v}");
        }
        assert_eq!(obj.len(), 11, "unexpected extra field: {v}");
        assert!(!obj.contains_key("improved") && !obj.contains_key("warning"), "{v}");
        assert_eq!(v["used"], 10);
        assert_eq!(v["kept_frames"][9], 9);
    }

    #[test]
    fn memory_estimate_is_accumulator_plus_post_processing() {
        // The reference burst: 10 frames of 500x220 at 3x -> 1500x660 output.
        let out_px = 1500 * 660;
        assert_eq!(stack_memory_estimate(10, 500, 220, 3), 10 * out_px * 12 + out_px * POST_BYTES_PER_OUTPUT_PIXEL);
        assert!(stack_memory_estimate(60, 500, 220, 3) < DEFAULT_MEMORY_BUDGET_BYTES, "60-frame ROI burst is admitted");
        assert!(stack_memory_estimate(2, 1920, 1080, 3) > DEFAULT_MEMORY_BUDGET_BYTES, "2 x 1080p at 3x measured 3.3 GB");
        // Absurd arguments saturate (and are therefore refused) instead of
        // wrapping to a small number that would pass the guard.
        assert_eq!(stack_memory_estimate(usize::MAX, i32::MAX, i32::MAX, MAX_SCALE), u64::MAX);
        assert_eq!(stack_memory_estimate(2, i32::MIN, i32::MIN, i32::MIN), u64::MAX);
    }

    #[test]
    fn a_tolerance_that_cannot_be_compared_is_refused() {
        for (shift, resp) in [(f64::NAN, 0.5), (f64::INFINITY, 0.5), (-1.0, 0.5), (2.0, f64::NAN), (2.0, f64::NEG_INFINITY)] {
            assert!(
                matches!(check_tolerances(shift, resp), Err(StackError::BadTolerance(_))),
                "({shift}, {resp}) must be refused"
            );
        }
        check_tolerances(0.0, -1.0).unwrap(); // zero shift and a negative floor are legal, if strict
        check_tolerances(2.0, 0.5).unwrap();
    }

    /// Without the feature the entry points are a STATED failure naming the
    /// fix, never an empty result.
    #[cfg(not(feature = "tracking"))]
    #[test]
    fn stack_is_a_stated_failure_without_the_feature() {
        assert!(matches!(stack::<u8>(&[], StackOpts::default()), Err(StackError::NotCompiled)));
        assert!(matches!(coherent_group::<u8>(&[], 2.0, 0.5), Err(StackError::NotCompiled)));
        assert!(StackError::NotCompiled.to_string().contains("--features tracking"));
    }

    #[cfg(feature = "tracking")]
    #[test]
    fn size_mismatch_is_reported_before_any_opencv_work() {
        use opencv::core::{Mat, Scalar, CV_8UC3};
        // 10x10 vs 12x12. phase_correlate asserts on these, and that used to
        // escape as an opaque `Cv` error because coherent_group ran before the
        // size check in the registration loop.
        let a = Mat::new_rows_cols_with_default(10, 10, CV_8UC3, Scalar::all(0.0)).unwrap();
        let b = Mat::new_rows_cols_with_default(12, 12, CV_8UC3, Scalar::all(0.0)).unwrap();
        let frames = [a, b];
        assert!(matches!(stack(&frames, StackOpts::default()), Err(StackError::SizeMismatch)));
        // pub and callable on its own, so it must refuse the same way.
        assert!(matches!(coherent_group(&frames, 2.0, 0.5), Err(StackError::SizeMismatch)));
    }

    /// A deterministic 3-channel noise texture: dense high-frequency content so
    /// phase correlation and ECC both have something to lock onto.
    #[cfg(feature = "tracking")]
    fn noise_frame(w: i32, h: i32, shift_y: i32) -> opencv::core::Mat {
        use opencv::core::{Mat, Scalar, Vec3b, CV_8UC3};
        use opencv::prelude::*;
        let mut m = Mat::new_rows_cols_with_default(h, w, CV_8UC3, Scalar::all(0.0)).unwrap();
        let mut seed = 0x9E37_79B9_u32;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed >> 24) as u8
        };
        let src: Vec<u8> = (0..(w * h * 3)).map(|_| next()).collect();
        for y in 0..h {
            let sy = (y - shift_y).rem_euclid(h);
            for x in 0..w {
                let i = ((sy * w + x) * 3) as usize;
                *m.at_2d_mut::<Vec3b>(y, x).unwrap() = Vec3b::from([src[i], src[i + 1], src[i + 2]]);
            }
        }
        m
    }

    #[cfg(feature = "tracking")]
    #[test]
    fn identical_frames_score_equal_and_agree() {
        // F1: stacking N copies of one frame can recover nothing. The paired
        // scores must say so — stacked equals single — and the frames must
        // agree perfectly (no ghosting). No verdict is drawn; the numbers are
        // the answer.
        let f = noise_frame(96, 64, 0);
        let frames = [f.clone(), f.clone(), f];
        let (_, rep) = stack(&frames, StackOpts { scale: 2, ..Default::default() }).unwrap();
        assert!((rep.sharpness_stacked - rep.sharpness_single).abs() < 1e-6, "{rep:?}");
        assert!(rep.sharpness_single > 0.0, "a textured frame has measurable detail: {rep:?}");
        assert_eq!(rep.residual_mad, 0.0);
        assert_eq!(rep.kept_frames, vec![0, 1, 2]);
        assert_eq!(rep.dropped_for_movement, 0);
        assert_eq!(rep.incoherent, 0);
    }

    #[cfg(feature = "tracking")]
    #[test]
    fn a_burst_spanning_a_scroll_is_refused_not_silently_stacked() {
        // F3: two frames 20px apart share no content. The old code discarded the
        // coherence result, stacked both, and reported dropped=0 next to a 20px
        // drift. Now it refuses — and with coherence OFF it stacks but the
        // report COUNTS the frames that had moved.
        let frames = [noise_frame(96, 64, 0), noise_frame(96, 64, 20)];
        match stack(&frames, StackOpts::default()) {
            Err(StackError::NoCoherentFrames { coherent, burst, max_drift_px }) => {
                assert!(coherent < 2);
                assert_eq!(burst, 2);
                assert!(max_drift_px > 15.0, "drift {max_drift_px}");
            }
            other => panic!("expected NoCoherentFrames, got {other:?}"),
        }
        let off = StackOpts { coherence: false, scale: 2, ..Default::default() };
        match stack(&frames, off) {
            Ok((_, rep)) => {
                assert_eq!(rep.dropped_for_movement, 0, "nothing is dropped with the filter off");
                assert!(rep.incoherent >= 1, "the moved frame is counted, not hidden: {rep:?}");
                assert!(rep.max_drift_px > 15.0, "{rep:?}");
                assert!(rep.residual_mad > 10.0, "two unrelated frames must disagree: {rep:?}");
            }
            // ECC may legitimately refuse to register two unrelated noise
            // fields; that is an honest refusal, not a silent stack.
            Err(StackError::TooFewRegistered { .. }) => {}
            other => panic!("unexpected: {other:?}"),
        }
    }

    /// The coherence tolerances are the CALLER's: the same burst is coherent
    /// or not depending on the knob, and the knob is honoured.
    #[cfg(feature = "tracking")]
    #[test]
    fn the_coherence_tolerances_are_the_callers() {
        let apart = [noise_frame(96, 64, 0), noise_frame(96, 64, 20)];
        // At the default 2 px the 20 px pair splits ...
        let (keep, drift) = coherent_group(&apart, DEFAULT_MAX_SHIFT_PX, DEFAULT_MIN_RESPONSE).unwrap();
        assert!(keep.len() < 2, "{keep:?}");
        assert!(drift > 15.0, "{drift}");
        // ... and at 25 px the same pair is one cluster. Drift is a property
        // of the burst, not of the tolerance, so it does not move.
        let (keep, drift2) = coherent_group(&apart, 25.0, DEFAULT_MIN_RESPONSE).unwrap();
        assert_eq!(keep, vec![0, 1], "a wide enough tolerance keeps both");
        assert_eq!(drift, drift2);
        // An impossible response floor (phase correlation never exceeds 1)
        // rejects even identical frames — through `stack`, as a stated refusal.
        let f = noise_frame(96, 64, 0);
        let same = [f.clone(), f.clone(), f];
        let strict = StackOpts { min_response: 1.5, scale: 2, ..Default::default() };
        match stack(&same, strict) {
            Err(StackError::NoCoherentFrames { coherent: 0, burst: 3, .. }) => {}
            other => panic!("expected NoCoherentFrames with nothing kept, got {other:?}"),
        }
        // And a bad tolerance is refused before any frame is looked at.
        assert!(matches!(
            stack(&same, StackOpts { max_shift_px: f64::NAN, ..Default::default() }),
            Err(StackError::BadTolerance(_))
        ));
        assert!(matches!(coherent_group(&same, 2.0, f64::NAN), Err(StackError::BadTolerance(_))));
    }

    #[cfg(feature = "tracking")]
    #[test]
    fn frames_must_be_8bit_3_channel() {
        // F5: a 16-bit burst used to run the whole pipeline and return an
        // all-WHITE image as Ok (the blank guard only looks for black); a BGRA
        // burst died inside OpenCV arithmetic with StsUnmatchedFormats.
        use opencv::core::{Mat, Scalar, CV_16UC3, CV_8UC3, CV_8UC4};
        let ok = Mat::new_rows_cols_with_default(16, 16, CV_8UC3, Scalar::all(0.0)).unwrap();
        let deep = Mat::new_rows_cols_with_default(16, 16, CV_16UC3, Scalar::all(0.0)).unwrap();
        let bgra = Mat::new_rows_cols_with_default(16, 16, CV_8UC4, Scalar::all(0.0)).unwrap();
        match stack(&[deep.clone(), deep], StackOpts::default()) {
            Err(StackError::UnsupportedFrameType { index, found }) => {
                assert_eq!(index, 0);
                assert_eq!(found, "CV_16UC3");
            }
            other => panic!("expected UnsupportedFrameType, got {other:?}"),
        }
        // The offending frame is named, not just "a frame".
        match stack(&[ok.clone(), bgra.clone()], StackOpts::default()) {
            Err(StackError::UnsupportedFrameType { index, found }) => {
                assert_eq!(index, 1);
                assert_eq!(found, "CV_8UC4");
            }
            other => panic!("expected UnsupportedFrameType, got {other:?}"),
        }
        assert!(matches!(
            coherent_group(&[ok, bgra], 2.0, 0.5),
            Err(StackError::UnsupportedFrameType { index: 1, .. })
        ));
    }

    #[cfg(feature = "tracking")]
    #[test]
    fn scale_is_validated_before_any_work() {
        // F7: 5_000_000 used to panic on `rw * scale`; 0 and -1 surfaced as
        // OpenCV assertions from deep inside the pipeline.
        use opencv::prelude::*;
        let frames = [noise_frame(32, 32, 0), noise_frame(32, 32, 0)];
        for bad in [0, -1, MAX_SCALE + 1, 5_000_000, i32::MAX, i32::MIN] {
            match stack(&frames, StackOpts { scale: bad, ..Default::default() }) {
                Err(StackError::BadScale(s)) => assert_eq!(s, bad),
                other => panic!("scale {bad}: expected BadScale, got {other:?}"),
            }
        }
        let (img, rep) = stack(&frames, StackOpts { scale: MAX_SCALE, ..Default::default() }).unwrap();
        assert_eq!((img.cols(), img.rows()), (32 * MAX_SCALE, 32 * MAX_SCALE));
        assert_eq!(rep.scale, MAX_SCALE);
    }

    #[cfg(feature = "tracking")]
    #[test]
    fn an_oversized_stack_is_refused_before_allocating() {
        // F6b: two flat 1000x700 frames at 8x -> 8000x5600 output. Nothing is
        // allocated, so the test is instant even though the call would need
        // ~9.7 GB. Flat frames fail the coherence test, so the filter is off
        // to reach the guard with every frame counted.
        use opencv::core::{Mat, Scalar, CV_8UC3};
        let f = Mat::new_rows_cols_with_default(700, 1000, CV_8UC3, Scalar::all(90.0)).unwrap();
        let frames = [f.clone(), f];
        let opts = StackOpts { scale: MAX_SCALE, coherence: false, ..Default::default() };
        match stack(&frames, opts) {
            Err(StackError::TooMuchMemory { estimated_bytes, budget_bytes, frames: n, out_w, out_h }) => {
                assert_eq!(estimated_bytes, stack_memory_estimate(2, 1000, 700, MAX_SCALE));
                assert!(estimated_bytes > 9 << 30, "{estimated_bytes}");
                assert_eq!(budget_bytes, DEFAULT_MEMORY_BUDGET_BYTES);
                assert_eq!((n, out_w, out_h), (2, 8000, 5600));
            }
            other => panic!("expected TooMuchMemory, got {other:?}"),
        }
    }

    /// The budget is the CALLER's: a burst the default admits is refused under
    /// a tighter one, and the refusal names the budget that was actually set.
    #[cfg(feature = "tracking")]
    #[test]
    fn the_memory_budget_is_the_callers() {
        let frames = [noise_frame(32, 32, 0), noise_frame(32, 32, 0)];
        let tight = StackOpts { scale: 2, memory_budget_bytes: 1, ..Default::default() };
        match stack(&frames, tight) {
            Err(StackError::TooMuchMemory { estimated_bytes, budget_bytes, frames: n, .. }) => {
                assert_eq!(budget_bytes, 1);
                assert_eq!(estimated_bytes, stack_memory_estimate(2, 32, 32, 2));
                assert_eq!(n, 2);
            }
            other => panic!("expected TooMuchMemory, got {other:?}"),
        }
        stack(&frames, StackOpts { scale: 2, ..Default::default() }).unwrap();
    }

    #[cfg(feature = "tracking")]
    #[test]
    fn coherent_group_refuses_an_empty_burst() {
        // F8: it is pub and used to index frames[0] of nothing.
        assert!(matches!(coherent_group(&[], 2.0, 0.5), Err(StackError::TooFewFrames(0))));
        assert!(matches!(stack(&[], StackOpts::default()), Err(StackError::TooFewFrames(0))));
    }

    #[cfg(feature = "tracking")]
    #[test]
    fn median_row_walk_matches_the_per_pixel_definition() {
        // F6a: `median_of` walks rows as slices. It must be the SAME function
        // as the per-pixel `at_2d` definition, bit for bit — checked for an odd
        // and an even stack, with negatives, ties and zeros in the data.
        use opencv::core::{Mat, Scalar, Vec3f, CV_32FC3};
        use opencv::prelude::*;
        let (w, h) = (13, 7);
        let mut seed = 0x2545_F491_u32;
        let mut frame = || {
            let mut m = Mat::new_rows_cols_with_default(h, w, CV_32FC3, Scalar::all(0.0)).unwrap();
            for y in 0..h {
                for x in 0..w {
                    let mut v = [0.0_f32; 3];
                    for c in &mut v {
                        seed ^= seed << 13;
                        seed ^= seed >> 17;
                        seed ^= seed << 5;
                        // small integer-ish values so ties and exact zeros occur
                        *c = f32::from((seed >> 28) as i8) - 4.0;
                    }
                    *m.at_2d_mut::<Vec3f>(y, x).unwrap() = Vec3f::from(v);
                }
            }
            m
        };
        let naive = |acc: &[Mat]| -> Mat {
            let n = acc.len();
            let mut out = Mat::new_rows_cols_with_default(h, w, CV_32FC3, Scalar::all(0.0)).unwrap();
            let mut buf = vec![0.0_f32; n];
            for y in 0..h {
                for x in 0..w {
                    for ch in 0..3 {
                        for (i, m) in acc.iter().enumerate() {
                            buf[i] = m.at_2d::<Vec3f>(y, x).unwrap()[ch];
                        }
                        buf.sort_by(|a, b| a.partial_cmp(b).unwrap());
                        out.at_2d_mut::<Vec3f>(y, x).unwrap()[ch] =
                            if n % 2 == 1 { buf[n / 2] } else { (buf[n / 2 - 1] + buf[n / 2]) / 2.0 };
                    }
                }
            }
            out
        };
        for n in [2usize, 4, 5] {
            let acc: Vec<Mat> = (0..n).map(|_| frame()).collect();
            let fast = imp::median_of(&acc, w, h).unwrap();
            let slow = naive(&acc);
            for y in 0..h {
                for x in 0..w {
                    let (a, b) = (fast.at_2d::<Vec3f>(y, x).unwrap(), slow.at_2d::<Vec3f>(y, x).unwrap());
                    for ch in 0..3 {
                        assert_eq!(a[ch].to_bits(), b[ch].to_bits(), "n={n} ({x},{y}) ch{ch}: {} vs {}", a[ch], b[ch]);
                    }
                }
            }
        }
    }

    #[cfg(feature = "tracking")]
    #[test]
    fn degenerate_bursts_keep_max_drift_finite() {
        // F9 (not reproduced): the audit hypothesised a NaN shift/response from
        // phase correlation sorting as `Equal` into the medoid and reaching the
        // report as `max_drift_px: NaN`. Featureless and mostly-featureless
        // bursts come back with response 0 (every OpenCV division on that path
        // is epsilon-guarded), so the filter drops them and drift stays finite.
        // Pin that, so a NaN can never reach a report silently.
        use opencv::core::{Mat, Scalar, CV_8UC3};
        let flat = |v: f64| Mat::new_rows_cols_with_default(64, 96, CV_8UC3, Scalar::all(v)).unwrap();
        let tex = noise_frame(96, 64, 0);
        let bursts: Vec<Vec<Mat>> = vec![
            vec![flat(0.0), flat(0.0), flat(0.0)],
            vec![flat(128.0), flat(128.0), tex.clone(), flat(128.0), flat(128.0)],
            vec![tex.clone(), flat(0.0)],
            vec![flat(0.0), tex],
        ];
        for b in &bursts {
            let (keep, drift) = coherent_group(b, 2.0, 0.5).unwrap();
            assert!(drift.is_finite(), "drift {drift}");
            assert!(keep.len() < 2, "featureless frames must not pass the response floor: {keep:?}");
            match stack(b, StackOpts { scale: 2, ..Default::default() }) {
                Err(StackError::NoCoherentFrames { max_drift_px, .. }) => assert!(max_drift_px.is_finite()),
                other => panic!("expected NoCoherentFrames, got {other:?}"),
            }
            match stack(b, StackOpts { scale: 2, coherence: false, ..Default::default() }) {
                Ok((_, rep)) => assert!(rep.max_drift_px.is_finite(), "{rep:?}"),
                Err(StackError::TooFewRegistered { .. }) => {}
                other => panic!("unexpected: {other:?}"),
            }
        }
    }
}
