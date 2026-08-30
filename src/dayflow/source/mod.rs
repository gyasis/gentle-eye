//! Capture sources — where a Dayflow sample comes from.
//!
//! A source is either an **input taken** (a stream, a capture card, a camera —
//! content that may never be rendered on this machine's screen) or a **display
//! consumed** (a screen, a named window, a target region). The two are co-equal
//! kinds, not a primary and a special case.
//!
//! # Why a trait and not an enum (D014-1)
//!
//! The loop must never match on the source kind. A new kind is added by writing
//! an implementor and **nothing else** — if adding one requires editing the
//! loop, the contract in `contracts/capture-source.md` has been broken. An enum
//! would put every kind in one exhaustive `match` inside the driver, so each new
//! kind would edit the driver by construction.

pub mod display;
pub mod input;
pub mod target;
pub mod window;

pub use display::DisplaySource;
pub use input::{FfmpegGrabber, FrameGrabber, InputSource};
pub use target::NamedTargetSource;
pub use window::{WindowLocator, WindowSource, WindowState};

use crate::dayflow::sampler::RawFrame;
use crate::dayflow::window::PauseCause;
use crate::regions::Region;

/// What a surface asked to capture.
///
/// One type shared by the CLI, MCP and HTTP so the three cannot drift: a
/// session started as a window on one surface must be the same session read
/// from another (FR-115). Parsing lives here, not three times over.
///
/// This is DATA, not sources: building a source touches the window manager,
/// opens a display, or shells out to ffmpeg, and platform capture handles are
/// thread-affine (D014-10) — so the spec crosses to the capture thread and the
/// sources are built there.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SourceSpec {
    /// Whole displays, by index. The default, and what Dayflow did before
    /// sources existed.
    Displays { indices: Vec<u32> },
    /// One named window, matched on title or class.
    Window { label: String },
    /// A persisted named target.
    Target { name: String },
    /// A stream or capture-device URL.
    Input { url: String },
}

impl SourceSpec {
    /// The ordinals this spec will occupy — the `display_id` position on every
    /// sample and segment it produces (D014-2).
    pub fn ordinals(&self) -> Vec<u32> {
        match self {
            Self::Displays { indices } => indices.clone(),
            // A single non-display source is ordinal 0. It is NOT display 0:
            // the field means "which source", and a window session has one.
            Self::Window { .. } | Self::Target { .. } | Self::Input { .. } => vec![0],
        }
    }

    /// A short label for logs and errors.
    pub fn describe(&self) -> String {
        match self {
            Self::Displays { indices } => format!("displays {indices:?}"),
            Self::Window { label } => format!("window {label:?}"),
            Self::Target { name } => format!("target {name:?}"),
            Self::Input { url } => format!("input {url:?}"),
        }
    }

    /// Parse the surface arguments into exactly one spec.
    ///
    /// Refuses more than one kind rather than picking a winner: a caller that
    /// passed both `--window` and `--input` has two different intentions and
    /// silently honouring one records the wrong thing all day.
    pub fn parse(
        displays: Option<Vec<u32>>,
        window: Option<String>,
        target: Option<String>,
        input: Option<String>,
    ) -> Result<Self, String> {
        let mut chosen: Vec<Self> = Vec::new();
        if let Some(label) = window.filter(|s| !s.trim().is_empty()) {
            chosen.push(Self::Window { label });
        }
        if let Some(name) = target.filter(|s| !s.trim().is_empty()) {
            chosen.push(Self::Target { name });
        }
        if let Some(url) = input.filter(|s| !s.trim().is_empty()) {
            chosen.push(Self::Input { url });
        }
        if let Some(indices) = displays.filter(|d| !d.is_empty()) {
            chosen.push(Self::Displays { indices });
        }
        match chosen.len() {
            0 => Ok(Self::Displays { indices: Vec::new() }),
            1 => Ok(chosen.remove(0)),
            _ => Err(format!(
                "choose ONE source: got {}. A session records one subject; \
                 honouring one of two would record the wrong thing all day.",
                chosen
                    .iter()
                    .map(Self::describe)
                    .collect::<Vec<_>>()
                    .join(" and ")
            )),
        }
    }
}

/// Build the sources a spec names.
///
/// MUST run on the capture thread: every branch here opens a platform handle
/// (an X11 connection, a display capturer, an ffmpeg scratch dir) and those are
/// thread-affine (D014-10). Returning them across a thread boundary does not
/// compile, and that is the type system enforcing the rule rather than a
/// comment asking for it.
pub fn build_sources(
    spec: &SourceSpec,
    scratch: &std::path::Path,
) -> Result<Vec<Box<dyn CaptureSource>>, String> {
    match spec {
        SourceSpec::Displays { indices } => {
            // REFUSED rather than enumerated here: the caller has already
            // resolved "every display" into concrete ordinals for the run
            // (`start_with_source` does), and a SECOND enumeration at
            // capture-thread time can disagree with the first — a monitor
            // unplugged in between, or one flaky X query, and the run says
            // displays [0,1,2] while the thread builds a different set. Samples
            // then file under ordinals no run window owns and `on_sample`
            // drops them silently. One enumeration, one truth.
            if indices.is_empty() {
                return Err(
                    "display indices must be resolved before building sources — \
                     the run's ordinals and the built sources must come from ONE \
                     enumeration, or they can silently disagree"
                        .into(),
                );
            }
            let mut out: Vec<Box<dyn CaptureSource>> = Vec::new();
            for i in indices.iter().copied() {
                out.push(Box::new(
                    display::DisplaySource::new(i).map_err(|e| e.detail)?,
                ));
            }
            Ok(out)
        }
        SourceSpec::Window { label } => {
            // A window is cropped out of the display it sits on. Display 0 is
            // the starting point; a window on another monitor is a known limit
            // of the region producer today, recorded in research.md.
            let inner = display::DisplaySource::new(0).map_err(|e| e.detail)?;
            Ok(vec![Box::new(window::WindowSource::new(
                Box::new(inner),
                Box::new(window::WmLocator),
                label.clone(),
                0,
            ))])
        }
        SourceSpec::Target { name } => {
            let store = crate::target::store::TargetStore::load()
                .map_err(|e| format!("loading targets: {e}"))?;
            let t = store
                .list()
                .iter()
                .find(|t| t.name == *name)
                .ok_or_else(|| format!("no target named {name:?}"))?
                .clone();
            let inner: Box<dyn CaptureSource> = match &t.source {
                crate::target::model::TargetSource::Display { index } => {
                    Box::new(display::DisplaySource::new(*index as u32).map_err(|e| e.detail)?)
                }
                crate::target::model::TargetSource::Stream { url } => Box::new(
                    input::InputSource::new(
                        url.clone(),
                        Box::new(input::FfmpegGrabber::new(scratch)),
                        0,
                    ),
                ),
            };
            Ok(vec![Box::new(target::NamedTargetSource::new(inner, &t, 0))])
        }
        SourceSpec::Input { url } => Ok(vec![Box::new(input::InputSource::new(
            url.clone(),
            Box::new(input::FfmpegGrabber::new(scratch)),
            0,
        ))]),
    }
}

/// Whether a source can currently produce frames.
///
/// The three states are not decorative: they decide whether the loop retries.
/// `Ended` is terminal — the contract says a source that has ended is **not**
/// restarted, so conflating it with `Occluded` would spin forever on a window
/// that is genuinely gone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    /// Producing frames.
    Available,
    /// Temporarily unreachable — minimised, covered, or the stream stalled.
    /// Retried on the next tick.
    Occluded,
    /// Gone for good: the window closed, the display was unplugged, the stream
    /// ended. Never retried.
    Ended,
}

impl Availability {
    /// The gap cause to record when a frame could not be taken, or `None` when
    /// no gap is warranted.
    ///
    /// `Available` maps to `None` on purpose. A source that is available did not
    /// fail for availability reasons, so writing a gap for it would manufacture
    /// a recorded fact that never happened — and a gap is read as a deliberate,
    /// explained pause (see `timeline::Gap`).
    pub fn gap_cause(self) -> Option<PauseCause> {
        match self {
            Self::Available => None,
            Self::Occluded => Some(PauseCause::SourceOccluded),
            Self::Ended => Some(PauseCause::SourceEnded),
        }
    }

    /// Whether the loop should ask this source again on the next tick.
    pub fn retryable(self) -> bool {
        !matches!(self, Self::Ended)
    }
}

/// The durable name of a source, stable across movement.
///
/// A window dragged to another monitor, or reopened, is the **same** source.
/// Position is therefore deliberately absent from the hash (013/R30): including
/// it would make every drag a new identity and split a day's work in two.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceIdentity {
    /// Which kind produced this — `"display"`, `"window"`, `"target"`,
    /// `"input"`. Part of the hash so two kinds cannot collide on one key.
    pub kind: &'static str,
    /// The kind's own stable name for this source: a display's serial, a
    /// window's class, a stream URL.
    pub key: String,
}

impl SourceIdentity {
    /// A new identity.
    pub fn new(kind: &'static str, key: impl Into<String>) -> Self {
        Self { kind, key: key.into() }
    }

    /// A stable 64-bit id, safe to write to disk and compare across runs.
    ///
    /// Nothing persists it yet — the pinned algorithm exists so that when a row
    /// does store it, a toolchain upgrade cannot silently rebind every source.
    ///
    /// FNV-1a with pinned constants, matching `regions::Region::identity`, and
    /// for the same reason: `DefaultHasher` is explicitly not stable across
    /// toolchain releases, which is fine for a `HashMap` and fatal for an id
    /// that is persisted — an upgrade would silently rebind every stored source
    /// and pre-upgrade rows would stop matching, with no error anywhere
    /// (013/R31).
    pub fn hash(&self) -> u64 {
        const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
        const PRIME: u64 = 0x0000_0100_0000_01b3;
        let mut h = OFFSET;
        let mut feed = |bytes: &[u8]| {
            for b in bytes {
                h ^= u64::from(*b);
                h = h.wrapping_mul(PRIME);
            }
        };
        feed(self.kind.as_bytes());
        // Separator: without it ("win","dow") and ("window","") would hash
        // identically. 0xff cannot occur in UTF-8, so no key can forge it.
        feed(&[0xff]);
        feed(self.key.as_bytes());
        h
    }
}

/// Clip `regions` to `rect` and translate the survivors into RECT-LOCAL
/// coordinates, for a source that hands downstream a crop of its inner frame.
///
/// Shared by the window and target sources — one implementation, because two
/// copies of the same filter is how they drift apart untested (013/R40).
///
/// # Clipped, not gated on containment
///
/// A region PARTIALLY overlapping the crop is kept as its intersection rather
/// than dropped. Containment-only filtering silently discarded any pane that
/// extends past the crop's edge by a pixel — WM frame geometry and detected
/// pane boxes routinely disagree by a border's width — and every pixel of the
/// intersection genuinely exists in the cropped frame. Confinement (FR-114)
/// holds either way: a clip can never exceed the crop.
///
/// # The empty answer is two different answers (D014-3, the W5 rule)
///
/// - inner produced NOTHING → `Some(vec![])`: the cascade ran and found
///   nothing, which is an answer, and is not a whole-frame read.
/// - inner produced regions but NONE intersect the crop → `None`: nothing the
///   cascade said is attributable to this crop, so this source cannot answer.
///   `Some(vec![])` here would claim "looked, found nothing" and hide the
///   whole-frame read from `samples_read_whole` — the exact defect the W5 gate
///   fixed in `DisplaySource::select_regions`, reintroduced one seam up.
pub(crate) fn clip_regions_to(
    regions: Vec<Region>,
    rect: crate::target::model::PixelRect,
) -> Option<Vec<Region>> {
    let had_input = !regions.is_empty();
    let kept: Vec<Region> = regions
        .into_iter()
        .filter_map(|mut r| {
            let x0 = r.bbox.x.max(rect.x);
            let y0 = r.bbox.y.max(rect.y);
            let x1 = (r.bbox.x + r.bbox.w).min(rect.x + rect.w);
            let y1 = (r.bbox.y + r.bbox.h).min(rect.y + rect.h);
            if x1 <= x0 || y1 <= y0 {
                return None; // no overlap at all
            }
            r.bbox = crate::target::model::PixelRect {
                x: x0 - rect.x,
                y: y0 - rect.y,
                w: x1 - x0,
                h: y1 - y0,
            };
            Some(r)
        })
        .collect();
    if kept.is_empty() && had_input {
        return None;
    }
    Some(kept)
}

/// One frame taken from a source, owning its pixels.
///
/// Owned rather than borrowed because a source generally decodes into its own
/// buffer; `as_raw` hands the loop the borrowed [`RawFrame`] the sampler wants.
#[derive(Debug, Clone)]
pub struct SourceFrame {
    /// Pixel data, 4 bytes per pixel (B, G, R, A).
    pub bgra: Vec<u8>,
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
}

impl SourceFrame {
    /// Borrow as the sampler's frame type.
    pub fn as_raw(&self) -> RawFrame<'_> {
        RawFrame { bgra: &self.bgra, width: self.width, height: self.height }
    }
}

/// A place Dayflow can take a frame from.
///
/// See `specs/014-dayflow-capture-loop/contracts/capture-source.md`. The loop
/// depends on this trait and never on a concrete kind.
/// # Thread affinity — deliberately NOT `Send`
///
/// Platform capture handles are thread-affine: scrap's X11 capturer holds an
/// `Rc<Server>` and raw pointers, so a `Send` bound here would exclude the
/// display source — the most basic kind there is. The loop therefore owns its
/// sources on the thread that created them, which is what a single ticking
/// driver wants anyway. A source that genuinely needs to cross threads (a
/// network stream with its own reader) owns that channel internally.
pub trait CaptureSource {
    /// Take the next frame, or fail.
    ///
    /// Failure is **not** fatal: the loop asks [`Self::availability`] and
    /// records a gap. A source error must never become a silently dropped
    /// interval — Dayflow cannot re-capture yesterday.
    fn next_frame(&mut self) -> Result<SourceFrame, SourceError>;

    /// The regions detected in this frame, or `None` when this source has no
    /// cascade to ask.
    ///
    /// Returning `None` is the honest answer and is **required** over
    /// synthesising a whole-frame region: a synthetic region is
    /// indistinguishable from a real detection, which would hide the
    /// whole-frame read the `samples_read_whole` counter exists to surface
    /// (FR-103, D014-3).
    fn regions_for(&self, frame: &SourceFrame) -> Option<Vec<Region>>;

    /// Whether frames can currently be taken.
    fn availability(&self) -> Availability;

    /// The durable name of this source, stable across movement.
    fn identity(&self) -> SourceIdentity;

    /// This source's position in the session's source list.
    ///
    /// Occupies the existing `display_id` field on samples and segments
    /// (D014-2): a source ordinal *is* what `display_id` always meant, so no
    /// schema changes and no migration. It must be stable for the session's
    /// lifetime — an ordinal that changed mid-session would split the source's
    /// own segments across two durable keys (013/R34).
    fn ordinal(&self) -> u32;
}

/// Why a frame could not be taken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceError {
    /// Human-readable cause, for logs and status.
    pub detail: String,
}

impl SourceError {
    /// An error with the given detail.
    pub fn new(detail: impl Into<String>) -> Self {
        Self { detail: detail.into() }
    }
}

impl std::fmt::Display for SourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "capture source failed: {}", self.detail)
    }
}

impl std::error::Error for SourceError {}
