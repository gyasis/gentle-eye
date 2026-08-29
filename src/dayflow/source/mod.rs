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

pub use display::DisplaySource;

use crate::dayflow::sampler::RawFrame;
use crate::dayflow::window::PauseCause;
use crate::regions::Region;

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

    /// A stable 64-bit id, written to disk and compared across runs.
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
