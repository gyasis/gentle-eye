//! Primitive 6 — **locate**: where the screen is in a frame, and whether its
//! corners are actually visible.
//!
//! Find the emissive screen by TEXTURE, not brightness: Laplacian magnitude,
//! blurred, Otsu-thresholded, morphologically closed, then the largest blobs'
//! convex hulls approximated down to a convex 4-gon. Brightness fails on a
//! dark-themed editor filmed in a dark room — Otsu locks onto the white laptop
//! body and the reflections, not the screen — while text is dense
//! high-frequency energy and a desk and a bezel are smooth, so the Laplacian
//! separates them cleanly.
//!
//! Feature-gated on `tracking` (opencv). The default build compiles the types,
//! [`Quad`]'s ordering and validation, and the corner-visibility measurement,
//! and returns [`LocateError::NotCompiled`] from the entry points — no system
//! libraries, per the repo rule.
//!
//! # It answers; it does not act
//!
//! [`locate`] never rectifies. A prior implementation both detected AND
//! applied the warp, and silently accepted a quad with a DUPLICATE corner —
//! returning a flat grey slab as success. Splitting the question ("where is the
//! screen?") from the action ("warp it") is what makes that failure impossible
//! to repeat: [`rectify`] is a separate call the CALLER makes with a quad, and
//! it still refuses one that [`Quad::validate`] rejects.
//!
//! # What the report carries that a warp would hide
//!
//! - **Which corners are clipped** ([`LocateReport::clipped`]). A corner on or
//!   beyond the frame boundary means the screen extends past the frame and
//!   only a PARTIAL correction is possible. Measured on both reference
//!   recordings: 4 of 4 corners clipped — filling the frame with the screen
//!   maximises pixel density, which is what made small code readable, and
//!   destroys the corners rectification needs. That is a recording decision
//!   the tool cannot make; it can only report it.
//! - **How the quad was found** ([`LocateReport::found`]). A minimum-area
//!   bounding box is NOT the screen; it is what you get when no convex 4-gon
//!   was findable, and it is typed as [`Found::BoundingBox`] rather than
//!   described in prose, so a caller can branch on it.
//! - **Not finding a screen is a stated answer** — [`LocateError::NoScreen`]
//!   names why — never an empty one.
//!
//! # Every threshold is the caller's
//!
//! [`LocateOpts`] carries every knob the detector consults, with a documented
//! default: the area floor, the closing kernel, the polygon-approximation
//! epsilon ladder, and the boundary margin that decides "clipped".
//! [`RectifyOpts`] carries the warp's minimum output side. A default is a
//! starting point the caller can see; a constant buried in the module is a
//! decision the caller cannot.

use serde::Serialize;

/// A screen quadrilateral in pixel coordinates, ordered TL, TR, BR, BL.
///
/// Serialises as four `[x, y]` pairs in that order.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Quad(pub [(f64, f64); 4]);

impl Quad {
    /// Order four arbitrary corners as TL, TR, BR, BL.
    ///
    /// The input is an unordered SET of corners: whatever order they arrive in
    /// (including a bow-tie order) is ignored. Corners are sorted by angle around
    /// their centroid, which is a permutation by construction — every input point
    /// appears exactly once — and the cycle is rotated so the top-left-most point
    /// (smallest `x+y`) comes first. In image coordinates (y down) increasing
    /// angle is clockwise, so the cycle reads TL, TR, BR, BL for any rotation.
    ///
    /// An earlier version picked each slot by an independent argmin/argmax over
    /// `x+y` and `y-x`. That is NOT a permutation: one vertex could win two slots
    /// while another vanished, and the duplicate-corner quad then rectified to a
    /// flat grey slab reported as success. It also mislabelled past ~45° of
    /// rotation.
    ///
    /// Fails with [`LocateError::BadQuad`] when the four points cannot form a
    /// convex quadrilateral: a repeated corner, three collinear corners, or one
    /// corner inside the triangle of the other three.
    pub fn ordered(pts: [(f64, f64); 4]) -> Result<Self, LocateError> {
        if pts.iter().any(|p| !p.0.is_finite() || !p.1.is_finite()) {
            return Err(LocateError::BadQuad("non-finite corner coordinate".into()));
        }
        let cx = pts.iter().map(|p| p.0).sum::<f64>() / 4.0;
        let cy = pts.iter().map(|p| p.1).sum::<f64>() / 4.0;
        let angle = |p: &(f64, f64)| (p.1 - cy).atan2(p.0 - cx);
        let mut v = pts;
        v.sort_by(|a, b| angle(a).partial_cmp(&angle(b)).unwrap_or(std::cmp::Ordering::Equal));
        let start = (0..4)
            .min_by(|&a, &b| {
                (v[a].0 + v[a].1)
                    .partial_cmp(&(v[b].0 + v[b].1))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or(0);
        let q = Quad([v[start], v[(start + 1) % 4], v[(start + 2) % 4], v[(start + 3) % 4]]);
        q.validate()?;
        Ok(q)
    }

    /// Check that the stored corners are four distinct points forming a convex,
    /// non-self-intersecting quadrilateral in clockwise (TL, TR, BR, BL) order.
    ///
    /// `Quad` is a public tuple struct, so a caller can build one by hand;
    /// [`rectify`] runs this so a bad quad is refused instead of warped into a
    /// blank or mirrored image.
    pub fn validate(&self) -> Result<(), LocateError> {
        let p = &self.0;
        if p.iter().any(|c| !c.0.is_finite() || !c.1.is_finite()) {
            return Err(LocateError::BadQuad("non-finite corner coordinate".into()));
        }
        for i in 0..4 {
            for j in (i + 1)..4 {
                if (p[i].0 - p[j].0).abs() < 1e-6 && (p[i].1 - p[j].1).abs() < 1e-6 {
                    return Err(LocateError::BadQuad(format!(
                        "corners {i} and {j} coincide at ({:.1}, {:.1})",
                        p[i].0, p[i].1
                    )));
                }
            }
        }
        // Every turn must be in the same direction and non-zero. For a 4-gon that
        // is exactly "convex and simple"; a bow-tie has mixed signs, a reflex or
        // collinear corner has a non-positive one. Positive = clockwise in image
        // coordinates (y down), which is the TL, TR, BR, BL orientation `rectify`
        // maps onto its destination rectangle.
        for i in 0..4 {
            let (a, b, c) = (p[i], p[(i + 1) % 4], p[(i + 2) % 4]);
            let cross = (b.0 - a.0) * (c.1 - b.1) - (b.1 - a.1) * (c.0 - b.0);
            if cross <= 0.0 {
                return Err(LocateError::BadQuad(format!(
                    "corners are not a convex clockwise quadrilateral (turn at corner {} is {cross:.1})",
                    (i + 1) % 4
                )));
            }
        }
        Ok(())
    }
}

/// One corner of a [`Quad`], named by its slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomRight,
    BottomLeft,
}

impl Corner {
    /// The four corners in [`Quad`] order: TL, TR, BR, BL.
    pub const ALL: [Corner; 4] = [Corner::TopLeft, Corner::TopRight, Corner::BottomRight, Corner::BottomLeft];
}

/// How the quad was found. Typed, so a caller branches on it instead of
/// reading prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Found {
    /// The convex hull of a texture blob approximated to exactly four corners
    /// that form a convex polygon. It is the outline of the largest TEXTURED
    /// region, which is the screen only when the screen is textured to its
    /// edges. Measured on reference recording 1 at 1324 s: a white video-call
    /// tile on the right of the screen is smooth, so the quad ends where the
    /// code panel ends and reports that corner as visible while the screen
    /// itself runs off the frame. Read `clipped` alongside this, and know
    /// what the frame shows.
    ConvexQuad,
    /// No convex 4-gon was findable at any epsilon in the ladder, so this is
    /// the minimum-area rotated bounding box of the largest texture blob.
    /// **It is not the screen** — it is the smallest rectangle around the text
    /// mass — and it must not be treated as a real detection. On both
    /// reference recordings this is what the detector returned.
    BoundingBox,
}

/// Default [`LocateOpts::min_area_frac`]: 5% of the frame. Smaller texture
/// blobs — a phone in shot, a keyboard's key legends — are not the screen.
pub const DEFAULT_MIN_AREA_FRAC: f64 = 0.05;

/// Default [`LocateOpts::close_kernel_px`]: 35 px square. Text is texture with
/// gaps — line spacing, margins, an empty panel — and the closing bridges them
/// so a screen becomes ONE blob rather than a constellation of paragraphs.
pub const DEFAULT_CLOSE_KERNEL_PX: i32 = 35;

/// Default [`LocateOpts::epsilon_ladder`]: fractions of the hull perimeter,
/// tried in order. A small epsilon keeps a clean screen's true corners; a
/// larger one is needed when the hull carries a bezel reflection or a cable
/// as extra vertices. The ladder stops at the first convex 4-gon.
pub const DEFAULT_EPSILON_LADDER: [f64; 5] = [0.02, 0.03, 0.05, 0.08, 0.12];

/// Default [`LocateOpts::clip_margin_px`]: 1 px. A hull vertex that the mask
/// pushed against the frame edge lands on pixel 0 or `w - 1` exactly; one
/// pixel of slack absorbs the integer rounding of polygon approximation.
pub const DEFAULT_CLIP_MARGIN_PX: f64 = 1.0;

/// Default [`RectifyOpts::min_side_px`]: 40 px. Below that a "screen" is a
/// smear no reader can use, and a wrong quad can easily produce one.
pub const DEFAULT_MIN_SIDE_PX: i32 = 40;

/// Knobs for [`locate`]. Every threshold the detector consults is here, with a
/// documented default — the caller owns all of them.
#[derive(Debug, Clone, PartialEq)]
pub struct LocateOpts {
    /// Area floor as a fraction of the frame, `0..=1`: a blob smaller than this
    /// is not considered. Default [`DEFAULT_MIN_AREA_FRAC`].
    pub min_area_frac: f64,
    /// Side of the square structuring element for the morphological close, in
    /// pixels, `>= 1`. Default [`DEFAULT_CLOSE_KERNEL_PX`].
    pub close_kernel_px: i32,
    /// Polygon-approximation tolerances as fractions of the hull perimeter,
    /// tried in order until one yields a convex 4-gon. Must be non-empty, each
    /// finite and positive. Default [`DEFAULT_EPSILON_LADDER`].
    pub epsilon_ladder: Vec<f64>,
    /// A corner within this many pixels of a frame edge — or beyond it — is
    /// reported as clipped. Finite and non-negative. Default
    /// [`DEFAULT_CLIP_MARGIN_PX`].
    pub clip_margin_px: f64,
}

impl Default for LocateOpts {
    fn default() -> Self {
        Self {
            min_area_frac: DEFAULT_MIN_AREA_FRAC,
            close_kernel_px: DEFAULT_CLOSE_KERNEL_PX,
            epsilon_ladder: DEFAULT_EPSILON_LADDER.to_vec(),
            clip_margin_px: DEFAULT_CLIP_MARGIN_PX,
        }
    }
}

/// Knobs for [`rectify`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RectifyOpts {
    /// Smallest output width or height accepted, in pixels, `>= 1`. A quad
    /// whose rectified size falls below this is refused with
    /// [`LocateError::DegenerateQuad`]. Default [`DEFAULT_MIN_SIDE_PX`].
    pub min_side_px: i32,
}

impl Default for RectifyOpts {
    fn default() -> Self {
        Self { min_side_px: DEFAULT_MIN_SIDE_PX }
    }
}

/// Where the screen is, and what a warp alone would hide. Serialises (`serde`)
/// so an agent on any harness can read it from a shell. Nothing here is a
/// verdict — whether to rectify is the caller's call.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LocateReport {
    /// The screen quadrilateral, TL, TR, BR, BL, in frame pixel coordinates.
    /// With [`Found::BoundingBox`] the corners can lie OUTSIDE the frame.
    pub quad: Quad,
    /// How the quad was found. Branch on this before trusting the corners.
    pub found: Found,
    /// Corners on or beyond the frame boundary, within
    /// [`LocateOpts::clip_margin_px`], in [`Quad`] order. Each clipped corner
    /// is a corner the screen has and the frame does not show; a warp through
    /// it can only be partial. Empty means every corner is inside the frame.
    pub clipped: Vec<Corner>,
    /// Frame width in pixels — the coordinate space `quad` lives in.
    pub frame_width: i32,
    /// Frame height in pixels.
    pub frame_height: i32,
}

/// Why a locate or rectify call could not produce a trustworthy answer. Every
/// variant is a STATED failure: nothing here degrades into a plausible quad.
#[derive(Debug, thiserror::Error)]
pub enum LocateError {
    /// No screen. The reason says which stage came up empty — an empty
    /// texture mask, or no blob above the area floor — so a caller can tell
    /// "nothing textured in shot" from "the floor is too high for this frame".
    #[error("no screen found: {0}")]
    NoScreen(String),
    /// Four points that do not form a convex quadrilateral (a repeated corner,
    /// collinear corners, or a corner inside the others' triangle).
    #[error("bad quad: {0}")]
    BadQuad(String),
    /// The quad rectifies to an output smaller than
    /// [`RectifyOpts::min_side_px`] on a side.
    #[error("degenerate quad -> {width}x{height} (minimum side {min_side_px} px, RectifyOpts::min_side_px)")]
    DegenerateQuad { width: i32, height: i32, min_side_px: i32 },
    /// An option that cannot be applied: a NaN, an out-of-range fraction, an
    /// empty epsilon ladder. Refused up front, before any work, so a bad
    /// option is the same first error in either build.
    #[error("bad option: {0}")]
    BadOption(String),
    /// A frame that is not 8-bit 3-channel BGR (`CV_8UC3`), or is empty. The
    /// grey conversion asserts on anything else, and that used to surface as
    /// an opaque OpenCV error naming color.cpp.
    #[error("frame is {0}; it must be a non-empty 8-bit 3-channel BGR image (CV_8UC3)")]
    UnsupportedFrame(String),
    #[error("built without the `tracking` feature; rebuild with --features tracking")]
    NotCompiled,
    #[cfg(feature = "tracking")]
    #[error(transparent)]
    Cv(#[from] opencv::Error),
}

/// Refuse options that cannot be applied. See [`LocateError::BadOption`].
fn check_locate_opts(opts: &LocateOpts) -> Result<(), LocateError> {
    if !opts.min_area_frac.is_finite() || !(0.0..=1.0).contains(&opts.min_area_frac) {
        return Err(LocateError::BadOption(format!(
            "min_area_frac must be a finite fraction in 0..=1, got {}",
            opts.min_area_frac
        )));
    }
    if opts.close_kernel_px < 1 {
        return Err(LocateError::BadOption(format!(
            "close_kernel_px must be at least 1, got {}",
            opts.close_kernel_px
        )));
    }
    if opts.epsilon_ladder.is_empty() {
        return Err(LocateError::BadOption("epsilon_ladder must not be empty".into()));
    }
    if let Some(e) = opts.epsilon_ladder.iter().find(|e| !e.is_finite() || **e <= 0.0) {
        return Err(LocateError::BadOption(format!(
            "every epsilon_ladder entry must be finite and positive, got {e}"
        )));
    }
    if !opts.clip_margin_px.is_finite() || opts.clip_margin_px < 0.0 {
        return Err(LocateError::BadOption(format!(
            "clip_margin_px must be finite and non-negative, got {}",
            opts.clip_margin_px
        )));
    }
    Ok(())
}

/// Refuse a rectify option that cannot be applied.
fn check_rectify_opts(opts: RectifyOpts) -> Result<(), LocateError> {
    if opts.min_side_px < 1 {
        return Err(LocateError::BadOption(format!(
            "min_side_px must be at least 1, got {}",
            opts.min_side_px
        )));
    }
    Ok(())
}

/// Which corners of `quad` sit on or beyond the boundary of a `width` x
/// `height` frame, within `margin_px`. Pure arithmetic, so a caller can
/// re-ask the question of a quad they already hold without the `tracking`
/// feature.
///
/// The last pixel is `width - 1`, so a corner at `x = width - 1` is ON the
/// edge, not one past it; a corner at `x = width` (which a bounding box can
/// produce) is beyond it. Both are clipped.
pub fn clipped_corners(quad: &Quad, width: i32, height: i32, margin_px: f64) -> Vec<Corner> {
    let (max_x, max_y) = (f64::from(width - 1), f64::from(height - 1));
    Corner::ALL
        .iter()
        .zip(quad.0.iter())
        .filter(|(_, (x, y))| {
            *x <= margin_px || *x >= max_x - margin_px || *y <= margin_px || *y >= max_y - margin_px
        })
        .map(|(c, _)| *c)
        .collect()
}

#[cfg(not(feature = "tracking"))]
mod imp {
    //! Default build: no opencv, no system libraries. The entry points still
    //! exist and compile, and answer with [`LocateError::NotCompiled`] — an
    //! actionable error, never a silently different result. They are generic
    //! over the frame type because `opencv::core::Mat` does not exist here;
    //! they never return `Ok`, so the `Ok` type is `()`. The options (and, for
    //! `rectify`, the quad) are still validated first — pure arithmetic — so a
    //! bad input is the same first error in either build.
    use super::*;

    pub fn locate<F>(_frame: &F, opts: &LocateOpts) -> Result<(), LocateError> {
        check_locate_opts(opts)?;
        Err(LocateError::NotCompiled)
    }

    pub fn rectify<F>(_frame: &F, quad: &Quad, opts: RectifyOpts) -> Result<(), LocateError> {
        check_rectify_opts(opts)?;
        quad.validate()?;
        Err(LocateError::NotCompiled)
    }
}

#[cfg(feature = "tracking")]
mod imp {
    use super::*;
    use opencv::core::{self, Mat, Point, Point2f, Scalar, Size, Vector};
    use opencv::{geometry, imgproc, prelude::*};

    /// The frame must be a non-empty 8-bit 3-channel BGR image. Checked BEFORE
    /// any OpenCV call, so the refusal names the frame rather than color.cpp.
    fn check_frame(frame: &Mat) -> Result<(), LocateError> {
        if frame.empty() {
            return Err(LocateError::UnsupportedFrame("empty (0x0)".into()));
        }
        if frame.depth() != core::CV_8U || frame.channels() != 3 {
            return Err(LocateError::UnsupportedFrame(format!(
                "depth {} with {} channel(s)",
                frame.depth(),
                frame.channels()
            )));
        }
        Ok(())
    }

    /// Where the screen is in `frame`, and whether its corners are visible.
    ///
    /// The texture measurement — Laplacian magnitude, blur, Otsu, close,
    /// contours, convex hull, polygon approximation — is the one validated
    /// end-to-end on two real recordings and is unchanged here. The largest
    /// blobs above [`LocateOpts::min_area_frac`] are tried in area order; the
    /// first whose hull approximates to a convex 4-gon at some epsilon in
    /// [`LocateOpts::epsilon_ladder`] is the answer, reported as
    /// [`Found::ConvexQuad`]. If none does, the largest blob's minimum-area
    /// rotated bounding box is reported as [`Found::BoundingBox`] — flagged,
    /// because it is not the screen.
    ///
    /// Never warps. Whether to call [`rectify`] on the result — given which
    /// corners are clipped and how the quad was found — is the caller's.
    ///
    /// # Errors
    ///
    /// [`LocateError::NoScreen`] names the stage that came up empty.
    /// [`LocateError::BadOption`] and [`LocateError::UnsupportedFrame`] are
    /// refused before any work.
    pub fn locate(frame: &Mat, opts: &LocateOpts) -> Result<LocateReport, LocateError> {
        check_locate_opts(opts)?;
        check_frame(frame)?;
        let (w, h) = (frame.cols(), frame.rows());
        let mut gray = Mat::default();
        imgproc::cvt_color_def(frame, &mut gray, imgproc::COLOR_BGR2GRAY)?;

        let mut lap = Mat::default();
        imgproc::laplacian_def(&gray, &mut lap, core::CV_32F)?;
        let mut energy = Mat::default();
        core::abs(&lap)?.to_mat()?.convert_to(&mut energy, core::CV_32F, 1.0, 0.0)?;
        let mut blurred = Mat::default();
        imgproc::gaussian_blur(
            &energy, &mut blurred, Size::new(0, 0), 9.0, 9.0, core::BORDER_DEFAULT, core::AlgorithmHint::ALGO_HINT_DEFAULT,
        )?;
        let mut norm8 = Mat::default();
        core::normalize(
            &blurred, &mut norm8, 0.0, 255.0, core::NORM_MINMAX, core::CV_8U, &core::no_array(),
        )?;

        let mut th = Mat::default();
        imgproc::threshold(&norm8, &mut th, 0.0, 255.0, imgproc::THRESH_BINARY + imgproc::THRESH_OTSU)?;
        let k = imgproc::get_structuring_element(
            imgproc::MORPH_RECT, Size::new(opts.close_kernel_px, opts.close_kernel_px), Point::new(-1, -1),
        )?;
        let mut closed = Mat::default();
        imgproc::morphology_ex(
            &th, &mut closed, imgproc::MORPH_CLOSE, &k, Point::new(-1, -1), 3,
            core::BORDER_CONSTANT, imgproc::morphology_default_border_value()?,
        )?;

        let mut contours: Vector<Vector<Point>> = Vector::new();
        imgproc::find_contours(
            &closed, &mut contours, imgproc::RETR_EXTERNAL, imgproc::CHAIN_APPROX_SIMPLE,
            Point::new(0, 0),
        )?;
        if contours.is_empty() {
            return Err(LocateError::NoScreen(
                "the texture mask is empty — nothing in the frame has text-like high-frequency energy".into(),
            ));
        }

        let frame_area = f64::from(w) * f64::from(h);
        let min_area = opts.min_area_frac * frame_area;
        let mut idx: Vec<(usize, f64)> = Vec::with_capacity(contours.len());
        for (i, c) in contours.iter().enumerate() {
            idx.push((i, geometry::contour_area_def(&c)?));
        }
        idx.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let largest = idx[0];
        if largest.1 < min_area {
            return Err(LocateError::NoScreen(format!(
                "no texture blob covers {:.1}% of the frame (LocateOpts::min_area_frac); the largest covers {:.1}%",
                opts.min_area_frac * 100.0,
                largest.1 / frame_area * 100.0
            )));
        }

        let report = |quad: Quad, found: Found| LocateReport {
            clipped: clipped_corners(&quad, w, h, opts.clip_margin_px),
            quad,
            found,
            frame_width: w,
            frame_height: h,
        };

        for (i, area) in idx.iter().take(6) {
            if *area < min_area {
                break;
            }
            let c = contours.get(*i)?;
            let mut hull: Vector<Point> = Vector::new();
            geometry::convex_hull(&c, &mut hull, true, true)?;
            let peri = geometry::arc_length(&hull, true)?;
            for eps in &opts.epsilon_ladder {
                let mut ap: Vector<Point> = Vector::new();
                geometry::approx_poly_dp(&hull, &mut ap, eps * peri, true)?;
                if ap.len() == 4 && geometry::is_contour_convex(&ap)? {
                    let mut pts = [(0.0, 0.0); 4];
                    for (n, p) in ap.iter().enumerate() {
                        pts[n] = (f64::from(p.x), f64::from(p.y));
                    }
                    return Ok(report(Quad::ordered(pts)?, Found::ConvexQuad));
                }
            }
        }

        // No blob's hull approximates to a convex 4-gon: the text mass has no
        // clean outline in this frame (measured: both reference recordings,
        // where the screen overflowed every edge). The minimum-area rotated
        // box around the largest blob is still WHERE the text is, so it is
        // returned — typed as the fallback it is, never as a detection.
        let c = contours.get(largest.0)?;
        let mut hull: Vector<Point> = Vector::new();
        geometry::convex_hull(&c, &mut hull, true, true)?;
        let rect = geometry::min_area_rect(&hull)?;
        let mut box_pts = [Point2f::default(); 4];
        rect.points(&mut box_pts)?;
        let pts = box_pts.map(|p| (f64::from(p.x), f64::from(p.y)));
        Ok(report(Quad::ordered(pts)?, Found::BoundingBox))
    }

    /// Warp a quad to a rectangle (de-keystone). A separate call from
    /// [`locate`], by contract: the detector answers, the caller decides.
    ///
    /// LIMIT: this cannot recover corners that are outside the frame. Handheld
    /// footage usually clips the screen at an edge — see
    /// [`LocateReport::clipped`] — in which case the correction is only
    /// partial, and a [`Found::BoundingBox`] quad straightens the text mass,
    /// not the screen.
    ///
    /// # Errors
    ///
    /// [`LocateError::BadQuad`] for a quad [`Quad::validate`] rejects — a
    /// duplicate or mis-ordered corner warps to a flat slab or a mirror image
    /// that still passes the size floor, so it is refused here rather than
    /// returned. [`LocateError::DegenerateQuad`] when a rectified side would
    /// be under [`RectifyOpts::min_side_px`].
    pub fn rectify(frame: &Mat, quad: &Quad, opts: RectifyOpts) -> Result<Mat, LocateError> {
        check_rectify_opts(opts)?;
        quad.validate()?;
        let [tl, tr, br, bl] = quad.0;
        let dist = |a: (f64, f64), b: (f64, f64)| ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
        let out_w = dist(br, bl).max(dist(tr, tl)) as i32;
        let out_h = dist(tr, br).max(dist(tl, bl)) as i32;
        if out_w < opts.min_side_px || out_h < opts.min_side_px {
            return Err(LocateError::DegenerateQuad {
                width: out_w,
                height: out_h,
                min_side_px: opts.min_side_px,
            });
        }
        let src = Vector::<Point2f>::from_iter(
            quad.0.iter().map(|p| Point2f::new(p.0 as f32, p.1 as f32)),
        );
        let dst = Vector::<Point2f>::from_iter([
            Point2f::new(0.0, 0.0),
            Point2f::new((out_w - 1) as f32, 0.0),
            Point2f::new((out_w - 1) as f32, (out_h - 1) as f32),
            Point2f::new(0.0, (out_h - 1) as f32),
        ]);
        let m = geometry::get_perspective_transform(&src, &dst, core::DECOMP_LU)?;
        let mut out = Mat::default();
        imgproc::warp_perspective(
            frame, &mut out, &m, Size::new(out_w, out_h), imgproc::INTER_LANCZOS4,
            core::BORDER_CONSTANT, Scalar::default(), core::AlgorithmHint::ALGO_HINT_DEFAULT,
        )?;
        Ok(out)
    }
}

pub use imp::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quad_orders_corners_tl_tr_br_bl() {
        // deliberately shuffled
        let q = Quad::ordered([(100.0, 10.0), (0.0, 90.0), (0.0, 0.0), (110.0, 100.0)]).unwrap();
        assert_eq!(q.0[0], (0.0, 0.0), "TL");
        assert_eq!(q.0[1], (100.0, 10.0), "TR");
        assert_eq!(q.0[2], (110.0, 100.0), "BR");
        assert_eq!(q.0[3], (0.0, 90.0), "BL");
    }

    #[test]
    fn quad_ordered_is_a_permutation_never_a_duplicate() {
        // The audit's case: the old argmin/argmax picker returned (358,156) as
        // BOTH TR and BR and dropped (200,219). These are four distinct corners
        // of a perfectly valid convex quad, so they must all survive.
        let input = [(0.0, 0.0), (358.0, 156.0), (200.0, 219.0), (0.0, 219.0)];
        let q = Quad::ordered(input).unwrap();
        assert_eq!(q.0, [(0.0, 0.0), (358.0, 156.0), (200.0, 219.0), (0.0, 219.0)]);
        for p in &input {
            assert_eq!(q.0.iter().filter(|c| *c == p).count(), 1, "{p:?} appears exactly once");
        }
    }

    #[test]
    fn quad_ordered_handles_a_50_degree_rotation() {
        // A 200x100 rectangle rotated 50° about (300,300). Generated in
        // clockwise (image-coords) order TL,TR,BR,BL, then fed in shuffled.
        let th = 50.0_f64.to_radians();
        let rot = |x: f64, y: f64| (300.0 + x * th.cos() - y * th.sin(), 300.0 + x * th.sin() + y * th.cos());
        let cw = [rot(-100.0, -50.0), rot(100.0, -50.0), rot(100.0, 50.0), rot(-100.0, 50.0)];
        let q = Quad::ordered([cw[2], cw[0], cw[3], cw[1]]).unwrap();
        // First corner is the top-left-most (smallest x+y) ...
        let sums: Vec<f64> = cw.iter().map(|p| p.0 + p.1).collect();
        let min_i = (0..4).min_by(|&a, &b| sums[a].partial_cmp(&sums[b]).unwrap()).unwrap();
        assert_eq!(q.0[0], cw[min_i], "TL is the top-left-most corner");
        // ... and the rest follow the original clockwise cycle from there.
        for k in 0..4 {
            assert_eq!(q.0[k], cw[(min_i + k) % 4], "corner {k} keeps the clockwise cycle");
        }
        q.validate().unwrap();
    }

    #[test]
    fn quad_ordered_untangles_a_bow_tie_input_order() {
        // The input is an unordered SET; a bow-tie is a property of an ORDER, so
        // the same four points come back as the rectangle they actually form.
        let q = Quad::ordered([(0.0, 0.0), (100.0, 100.0), (100.0, 0.0), (0.0, 100.0)]).unwrap();
        assert_eq!(q.0, [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]);
    }

    #[test]
    fn quad_ordered_rejects_shapes_that_are_not_convex_quads() {
        // a corner repeated
        assert!(matches!(
            Quad::ordered([(0.0, 0.0), (0.0, 0.0), (100.0, 0.0), (100.0, 100.0)]),
            Err(LocateError::BadQuad(_))
        ));
        // three collinear corners
        assert!(matches!(
            Quad::ordered([(0.0, 0.0), (50.0, 0.0), (100.0, 0.0), (100.0, 100.0)]),
            Err(LocateError::BadQuad(_))
        ));
        // one corner inside the triangle of the other three: no convex order exists
        assert!(matches!(
            Quad::ordered([(0.0, 0.0), (100.0, 0.0), (50.0, 100.0), (50.0, 30.0)]),
            Err(LocateError::BadQuad(_))
        ));
        // a hand-built bow-tie ORDER is rejected by validate (it is self-intersecting)
        assert!(matches!(
            Quad([(0.0, 0.0), (100.0, 100.0), (100.0, 0.0), (0.0, 100.0)]).validate(),
            Err(LocateError::BadQuad(_))
        ));
        // ... and so is a counter-clockwise order, which rectify would mirror
        assert!(matches!(
            Quad([(0.0, 0.0), (0.0, 100.0), (100.0, 100.0), (100.0, 0.0)]).validate(),
            Err(LocateError::BadQuad(_))
        ));
        // ... and a non-finite coordinate, which no comparison can place
        assert!(matches!(
            Quad::ordered([(0.0, 0.0), (f64::NAN, 0.0), (100.0, 100.0), (0.0, 100.0)]),
            Err(LocateError::BadQuad(_))
        ));
    }

    #[test]
    fn defaults_are_the_validated_constants() {
        // The thresholds that used to be literals in the detector, now visible
        // knobs with the same values — the move to the caller changed no default.
        let o = LocateOpts::default();
        assert_eq!(o.min_area_frac, 0.05, "5% area floor");
        assert_eq!(o.close_kernel_px, 35, "35x35 closing kernel");
        assert_eq!(o.epsilon_ladder, vec![0.02, 0.03, 0.05, 0.08, 0.12], "epsilon ladder");
        assert_eq!(RectifyOpts::default().min_side_px, 40, "40 px minimum output side");
        check_locate_opts(&o).unwrap();
        check_rectify_opts(RectifyOpts::default()).unwrap();
    }

    #[test]
    fn an_option_that_cannot_be_applied_is_refused() {
        let bad = [
            LocateOpts { min_area_frac: f64::NAN, ..Default::default() },
            LocateOpts { min_area_frac: 1.5, ..Default::default() },
            LocateOpts { min_area_frac: -0.1, ..Default::default() },
            LocateOpts { close_kernel_px: 0, ..Default::default() },
            LocateOpts { epsilon_ladder: vec![], ..Default::default() },
            LocateOpts { epsilon_ladder: vec![0.02, 0.0], ..Default::default() },
            LocateOpts { epsilon_ladder: vec![f64::INFINITY], ..Default::default() },
            LocateOpts { clip_margin_px: -1.0, ..Default::default() },
            LocateOpts { clip_margin_px: f64::NAN, ..Default::default() },
        ];
        for o in &bad {
            assert!(matches!(check_locate_opts(o), Err(LocateError::BadOption(_))), "{o:?} must be refused");
        }
        assert!(matches!(check_rectify_opts(RectifyOpts { min_side_px: 0 }), Err(LocateError::BadOption(_))));
        // The edges of the legal range are legal.
        check_locate_opts(&LocateOpts { min_area_frac: 0.0, clip_margin_px: 0.0, ..Default::default() }).unwrap();
        check_locate_opts(&LocateOpts { min_area_frac: 1.0, close_kernel_px: 1, ..Default::default() }).unwrap();
        check_rectify_opts(RectifyOpts { min_side_px: 1 }).unwrap();
    }

    /// The measured fact from both reference recordings: a screen that fills
    /// the frame has every corner on the boundary, and the report must say so.
    #[test]
    fn a_screen_filling_the_frame_has_all_four_corners_clipped() {
        let (w, h) = (1080, 1920);
        let full = Quad([(0.0, 0.0), (1079.0, 0.0), (1079.0, 1919.0), (0.0, 1919.0)]);
        assert_eq!(clipped_corners(&full, w, h, 0.0), Corner::ALL.to_vec());
        // A bounding-box fallback can put corners BEYOND the frame; that is
        // clipped too, not "inside".
        let beyond = Quad([(-12.0, -3.0), (1100.0, -8.0), (1105.0, 1930.0), (-9.0, 1925.0)]);
        assert_eq!(clipped_corners(&beyond, w, h, 1.0), Corner::ALL.to_vec());
    }

    #[test]
    fn corner_clipping_names_exactly_the_corners_on_the_boundary() {
        let (w, h) = (640, 480);
        let inside = Quad([(20.0, 20.0), (600.0, 25.0), (610.0, 450.0), (15.0, 440.0)]);
        assert!(clipped_corners(&inside, w, h, 1.0).is_empty(), "every corner is inside the frame");
        // The right edge is at x = 639. The top-right corner sits on it; the
        // bottom-right corner sits 2 px inside, which a 1 px margin leaves alone.
        let right_edge = Quad([(20.0, 20.0), (639.0, 25.0), (637.0, 450.0), (15.0, 440.0)]);
        assert_eq!(clipped_corners(&right_edge, w, h, 1.0), vec![Corner::TopRight]);
        // The margin is the caller's: at 3 px the bottom-right corner is clipped too.
        assert_eq!(clipped_corners(&right_edge, w, h, 3.0), vec![Corner::TopRight, Corner::BottomRight]);
        // Bottom-left on the bottom edge (y = 479).
        let bottom = Quad([(20.0, 20.0), (600.0, 25.0), (610.0, 450.0), (15.0, 479.0)]);
        assert_eq!(clipped_corners(&bottom, w, h, 1.0), vec![Corner::BottomLeft]);
    }

    /// The contract's machine-readable requirement: the report serialises with
    /// its field names, the quad as four `[x, y]` pairs, and the two facts a
    /// warp would hide — `found` and `clipped` — as typed values, not prose.
    #[test]
    fn the_report_is_machine_readable_and_types_the_fallback() {
        let rep = LocateReport {
            quad: Quad([(0.0, 0.0), (1079.0, 0.0), (1079.0, 1919.0), (0.0, 1919.0)]),
            found: Found::BoundingBox,
            clipped: Corner::ALL.to_vec(),
            frame_width: 1080,
            frame_height: 1920,
        };
        let v = serde_json::to_value(&rep).unwrap();
        let obj = v.as_object().unwrap();
        for key in ["quad", "found", "clipped", "frame_width", "frame_height"] {
            assert!(obj.contains_key(key), "missing {key}: {v}");
        }
        assert_eq!(obj.len(), 5, "unexpected extra field: {v}");
        assert_eq!(v["found"], "BoundingBox");
        assert_eq!(v["clipped"], serde_json::json!(["TopLeft", "TopRight", "BottomRight", "BottomLeft"]));
        assert_eq!(v["quad"][2], serde_json::json!([1079.0, 1919.0]));
        assert_eq!(v["frame_width"], 1080);
    }

    /// Without the feature the entry points are a STATED failure naming the
    /// fix, never an empty result — and a bad option or a bad quad is refused
    /// first, the same as in the full build.
    #[cfg(not(feature = "tracking"))]
    #[test]
    fn locate_is_a_stated_failure_without_the_feature() {
        let q = Quad([(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]);
        assert!(matches!(locate::<u8>(&0, &LocateOpts::default()), Err(LocateError::NotCompiled)));
        assert!(matches!(rectify::<u8>(&0, &q, RectifyOpts::default()), Err(LocateError::NotCompiled)));
        assert!(LocateError::NotCompiled.to_string().contains("--features tracking"));
        let bad_opts = LocateOpts { epsilon_ladder: vec![], ..Default::default() };
        assert!(matches!(locate::<u8>(&0, &bad_opts), Err(LocateError::BadOption(_))));
        let dup = Quad([(0.0, 0.0), (358.0, 156.0), (358.0, 156.0), (0.0, 219.0)]);
        assert!(matches!(rectify::<u8>(&0, &dup, RectifyOpts::default()), Err(LocateError::BadQuad(_))));
    }

    /// A deterministic 3-channel noise texture over `rect` (x, y, w, h) of a
    /// flat mid-grey frame: dense high-frequency energy where a screen full of
    /// text would be, and none where the desk would be.
    #[cfg(feature = "tracking")]
    fn textured_frame(w: i32, h: i32, rect: (i32, i32, i32, i32)) -> opencv::core::Mat {
        use opencv::core::{Mat, Scalar, Vec3b, CV_8UC3};
        use opencv::prelude::*;
        let mut m = Mat::new_rows_cols_with_default(h, w, CV_8UC3, Scalar::all(128.0)).unwrap();
        let mut seed = 0x9E37_79B9_u32;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed >> 24) as u8
        };
        let (rx, ry, rw, rh) = rect;
        for y in ry..(ry + rh).min(h) {
            for x in rx..(rx + rw).min(w) {
                let px = m.at_2d_mut::<Vec3b>(y, x).unwrap();
                let v = next();
                *px = Vec3b::from([v, v, v]);
            }
        }
        m
    }

    #[cfg(feature = "tracking")]
    #[test]
    fn a_textured_rectangle_inside_the_frame_is_a_clean_quad_with_no_clipping() {
        let frame = textured_frame(640, 480, (120, 90, 400, 280));
        let rep = locate(&frame, &LocateOpts::default()).unwrap();
        assert_eq!(rep.found, Found::ConvexQuad, "{rep:?}");
        assert!(rep.clipped.is_empty(), "every corner is well inside the frame: {rep:?}");
        assert_eq!((rep.frame_width, rep.frame_height), (640, 480));
        // The quad is the rectangle, to within the blur/close halo.
        let [tl, tr, br, bl] = rep.quad.0;
        for (got, want) in [(tl, (120.0, 90.0)), (tr, (520.0, 90.0)), (br, (520.0, 370.0)), (bl, (120.0, 370.0))] {
            assert!((got.0 - want.0).abs() < 25.0 && (got.1 - want.1).abs() < 25.0, "corner {got:?} vs {want:?}");
        }
    }

    /// Texture over the whole frame — the reference recordings' framing — has
    /// no visible corner. The report says so, whichever path found the quad.
    #[cfg(feature = "tracking")]
    #[test]
    fn texture_filling_the_frame_reports_all_four_corners_clipped() {
        let frame = textured_frame(640, 480, (0, 0, 640, 480));
        let rep = locate(&frame, &LocateOpts::default()).unwrap();
        assert_eq!(rep.clipped, Corner::ALL.to_vec(), "{rep:?}");
    }

    /// A flat frame has no texture anywhere: a stated failure naming the empty
    /// mask, never an empty or invented quad.
    #[cfg(feature = "tracking")]
    #[test]
    fn a_flat_frame_is_a_stated_failure() {
        let frame = textured_frame(320, 240, (0, 0, 0, 0));
        match locate(&frame, &LocateOpts::default()) {
            Err(LocateError::NoScreen(reason)) => assert!(reason.contains("mask is empty"), "{reason}"),
            other => panic!("expected NoScreen, got {other:?}"),
        }
    }

    /// The area floor is the caller's, and the refusal says which floor and
    /// how far the frame fell short of it.
    #[cfg(feature = "tracking")]
    #[test]
    fn a_blob_under_the_area_floor_is_refused_with_the_numbers() {
        // 40x40 of 640x480 is ~0.3% of the frame, far under the 5% default.
        let frame = textured_frame(640, 480, (300, 220, 40, 40));
        match locate(&frame, &LocateOpts::default()) {
            Err(LocateError::NoScreen(reason)) => {
                assert!(reason.contains("5.0%") && reason.contains("min_area_frac"), "{reason}");
            }
            other => panic!("expected NoScreen, got {other:?}"),
        }
        // Lower the floor and the same frame is found — the knob is real.
        let low = LocateOpts { min_area_frac: 0.001, ..Default::default() };
        assert!(locate(&frame, &low).is_ok());
    }

    #[cfg(feature = "tracking")]
    #[test]
    fn a_frame_that_is_not_bgr8_is_refused_before_any_work() {
        use opencv::core::{Mat, Scalar, CV_16UC3, CV_8UC1};
        let gray = Mat::new_rows_cols_with_default(100, 100, CV_8UC1, Scalar::all(0.0)).unwrap();
        assert!(matches!(locate(&gray, &LocateOpts::default()), Err(LocateError::UnsupportedFrame(_))));
        let deep = Mat::new_rows_cols_with_default(100, 100, CV_16UC3, Scalar::all(0.0)).unwrap();
        assert!(matches!(locate(&deep, &LocateOpts::default()), Err(LocateError::UnsupportedFrame(_))));
        let empty = Mat::default();
        assert!(matches!(locate(&empty, &LocateOpts::default()), Err(LocateError::UnsupportedFrame(_))));
    }

    #[cfg(feature = "tracking")]
    #[test]
    fn rectify_refuses_a_duplicate_corner_quad() {
        // The quad the old `ordered` produced. It used to warp to a grey slab.
        let img = textured_frame(400, 240, (0, 0, 400, 240));
        let bad = Quad([(0.0, 0.0), (358.0, 156.0), (358.0, 156.0), (0.0, 219.0)]);
        assert!(matches!(rectify(&img, &bad, RectifyOpts::default()), Err(LocateError::BadQuad(_))));
    }

    #[cfg(feature = "tracking")]
    #[test]
    fn rectify_warps_a_valid_quad_and_the_size_floor_is_the_callers() {
        use opencv::prelude::*;
        let img = textured_frame(400, 240, (0, 0, 400, 240));
        let q = Quad([(10.0, 10.0), (210.0, 20.0), (200.0, 120.0), (5.0, 110.0)]);
        let out = rectify(&img, &q, RectifyOpts::default()).unwrap();
        assert!(out.cols() >= 200 && out.rows() >= 100, "{}x{}", out.cols(), out.rows());
        // The same quad, but only 30 px tall, is under the 40 px default...
        let small = Quad([(10.0, 10.0), (210.0, 10.0), (210.0, 40.0), (10.0, 40.0)]);
        assert!(matches!(
            rectify(&img, &small, RectifyOpts::default()),
            Err(LocateError::DegenerateQuad { height: 30, min_side_px: 40, .. })
        ));
        // ...and fine at a floor the caller lowered.
        assert!(rectify(&img, &small, RectifyOpts { min_side_px: 20 }).is_ok());
    }
}
