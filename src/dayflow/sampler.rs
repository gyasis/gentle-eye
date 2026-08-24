//! Periodic frame sampling — dayflow's capture primitive.
//!
//! Dayflow SAMPLES; it does not stream video (D9). This module takes one frame
//! at a time, decides whether it is worth keeping, and writes the ones that are.
//! It owns no capture loop and no encoder: frames are handed in, which keeps the
//! decision logic deterministic and testable without a screen.
//!
//! # The ladder
//!
//! Per sample, cheapest first — the shape borrowed from videolocr (research R13):
//!
//! 1. **Downscale to a gate buffer** — greyscale, [`crate::config::DeltaConfig::gate_width`]
//!    px wide. Cheap, and the only thing compared.
//! 2. **Content gate** — [`crate::dayflow::gate`] decides changed / unchanged /
//!    blank, using whichever strategy is configured.
//! 3. **Persist** — only a frame that survives the gate is written as a PNG.
//!
//! # Storage
//!
//! PNG, losslessly. JPEG was rejected: lossy artefacts on small text are exactly
//! what degrades OCR, and legibility failure is SILENT — a slightly mangled
//! transcript reads as plausible. Lifetime is decided by the run's intent
//! ([`crate::config::DayflowIntent::discards_stills_after_summary`]): Activity
//! discards a still once its window is summarised, Content keeps it until the
//! material has been extracted.
//!
//! # Fail open
//!
//! Every uncertain path KEEPS the frame. Dayflow cannot re-capture yesterday, so
//! a gate that errs toward dropping turns any bug into silent data loss.

use std::path::{Path, PathBuf};

use chrono::{DateTime, SecondsFormat, Utc};

use crate::config::DeltaConfig;
use crate::dayflow::errors::DayflowError;
use crate::dayflow::gate::{self, GateVerdict};

/// One raw frame handed to the sampler, in the BGRA layout `scrap` produces.
#[derive(Debug, Clone, Copy)]
pub struct RawFrame<'a> {
    /// Pixel data, 4 bytes per pixel (B, G, R, A).
    pub bgra: &'a [u8],
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
}

impl RawFrame<'_> {
    /// Whether the buffer is large enough for the stated dimensions.
    fn is_well_formed(&self) -> bool {
        self.width > 0
            && self.height > 0
            && self.bgra.len() >= (self.width as usize) * (self.height as usize) * 4
    }
}

/// What happened to one sample. Recorded whether or not the frame was kept —
/// a skip is a fact, and an absent row cannot distinguish "nothing changed" from
/// "the sampler died".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SampleRecord {
    /// Which display the frame came from.
    pub display_id: u32,
    /// The window this sample belongs to.
    pub sequence: u64,
    /// When it was taken, to millisecond precision.
    pub taken_at: DateTime<Utc>,
    /// The gate's verdict.
    pub verdict: GateVerdict,
    /// Where the PNG was written, if it was kept.
    pub path: Option<PathBuf>,
}

impl SampleRecord {
    /// Whether this sample should be handed to perception.
    pub fn perceived(&self) -> bool {
        self.verdict.should_perceive()
    }

    /// The reason recorded in `dayflow_samples.skip_reason`, if skipped.
    pub fn skip_reason(&self) -> Option<&'static str> {
        self.verdict.skip_reason()
    }

    /// Millisecond-precision timestamp, matching the ledger's text format.
    ///
    /// Millisecond rather than second precision because `dayflow_samples` keys
    /// on `(session, display, sequence, taken_at)`: at second resolution two
    /// samples in the same second would violate the primary key.
    pub fn taken_at_key(&self) -> String {
        self.taken_at.to_rfc3339_opts(SecondsFormat::Millis, true)
    }
}

/// Per-display sampler state: the previous gate buffer to compare against.
#[derive(Debug, Default)]
struct DisplayState {
    prev_gate: Option<Vec<u8>>,
}

/// Samples frames, gates them, and writes the survivors.
#[derive(Debug)]
pub struct Sampler {
    delta: DeltaConfig,
    displays: std::collections::HashMap<u32, DisplayState>,
}

impl Sampler {
    /// A sampler using the given delta-gate configuration.
    pub fn new(delta: DeltaConfig) -> Self {
        Self {
            delta,
            displays: std::collections::HashMap::new(),
        }
    }

    /// Observe one frame from `display_id` belonging to window `sequence`.
    ///
    /// Writes a PNG into `dir` when the frame survives the gate. Returns the
    /// record either way.
    pub fn observe(
        &mut self,
        display_id: u32,
        sequence: u64,
        frame: RawFrame<'_>,
        taken_at: DateTime<Utc>,
        dir: &Path,
    ) -> Result<SampleRecord, DayflowError> {
        let gate_buf = if frame.is_well_formed() {
            downscale_gray(frame, self.delta.gate_width)
        } else {
            // Malformed frame: we cannot compare, so we cannot claim it is
            // unchanged. Empty buffer drives the gate to Indeterminate.
            Vec::new()
        };

        let verdict = if self.delta.enabled {
            let state = self.displays.entry(display_id).or_default();
            gate::evaluate(
                state.prev_gate.as_deref(),
                &gate_buf,
                self.delta.strategy,
                self.delta.magnitude_threshold,
                self.delta.proportion_threshold,
                self.delta.pixel_tolerance,
                self.delta.content_std,
            )
        } else {
            // Gate disabled: keep everything, but still record WHY it was kept.
            GateVerdict::Changed
        };

        // Remember this frame for the next comparison even when it was skipped —
        // otherwise a slow drift never accumulates past the threshold and the
        // gate never fires again.
        if !gate_buf.is_empty() {
            self.displays.entry(display_id).or_default().prev_gate = Some(gate_buf);
        }

        let path = if verdict.should_perceive() {
            let p = dir.join(sample_filename(display_id, sequence, taken_at));
            write_png(frame, &p)?;
            Some(p)
        } else {
            None
        };

        Ok(SampleRecord {
            display_id,
            sequence,
            taken_at,
            verdict,
            path,
        })
    }

    /// Forget a display's history — used when a display is removed, so a stale
    /// buffer cannot be compared against a different screen later.
    pub fn forget_display(&mut self, display_id: u32) {
        self.displays.remove(&display_id);
    }
}

/// Filename for a sample: sortable, and unique per display and instant.
fn sample_filename(display_id: u32, sequence: u64, taken_at: DateTime<Utc>) -> String {
    format!(
        "d{display_id}_w{sequence:06}_{}.png",
        taken_at.format("%Y%m%dT%H%M%S%3f")
    )
}

/// Downscale a BGRA frame to a greyscale buffer `target_width` px wide.
///
/// Nearest-neighbour on purpose: this buffer is only ever compared against
/// another produced the same way, so sampling fidelity does not matter and speed
/// does — it runs on every frame of every display.
pub fn downscale_gray(frame: RawFrame<'_>, target_width: u32) -> Vec<u8> {
    if !frame.is_well_formed() || target_width == 0 {
        return Vec::new();
    }
    let tw = target_width.min(frame.width).max(1);
    let th = ((u64::from(frame.height) * u64::from(tw)) / u64::from(frame.width)).max(1) as u32;
    let mut out = Vec::with_capacity((tw as usize) * (th as usize));
    for y in 0..th {
        let sy = (u64::from(y) * u64::from(frame.height) / u64::from(th)) as usize;
        for x in 0..tw {
            let sx = (u64::from(x) * u64::from(frame.width) / u64::from(tw)) as usize;
            let i = (sy * frame.width as usize + sx) * 4;
            // BGRA → luma (Rec. 601), integer arithmetic.
            let b = u32::from(frame.bgra[i]);
            let g = u32::from(frame.bgra[i + 1]);
            let r = u32::from(frame.bgra[i + 2]);
            out.push(((r * 299 + g * 587 + b * 114) / 1000) as u8);
        }
    }
    out
}

/// Write a BGRA frame to `path` as a PNG, creating parent directories.
fn write_png(frame: RawFrame<'_>, path: &Path) -> Result<(), DayflowError> {
    if !frame.is_well_formed() {
        return Err(DayflowError::Invalid(format!(
            "malformed frame: {}x{} needs {} bytes, got {}",
            frame.width,
            frame.height,
            (frame.width as usize) * (frame.height as usize) * 4,
            frame.bgra.len()
        )));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| DayflowError::Internal(format!("create {}: {e}", parent.display())))?;
    }
    let px = (frame.width as usize) * (frame.height as usize);
    let mut rgb = Vec::with_capacity(px * 3);
    for i in 0..px {
        let o = i * 4;
        rgb.push(frame.bgra[o + 2]);
        rgb.push(frame.bgra[o + 1]);
        rgb.push(frame.bgra[o]);
    }
    let img: image::RgbImage = image::ImageBuffer::from_raw(frame.width, frame.height, rgb)
        .ok_or_else(|| DayflowError::Internal("frame did not fit an image buffer".into()))?;
    img.save(path)
        .map_err(|e| DayflowError::Internal(format!("write {}: {e}", path.display())))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DayflowConfig;

    /// A frame with real variation, so it clears the content-std floor.
    fn frame_bytes(w: u32, h: u32, seed: u8) -> Vec<u8> {
        let mut v = Vec::with_capacity((w * h * 4) as usize);
        for i in 0..(w * h) {
            let n = ((i.wrapping_mul(37) % 251) as u8) ^ seed;
            v.extend_from_slice(&[n, n.wrapping_add(40), n.wrapping_add(80), 255]);
        }
        v
    }

    fn delta() -> DeltaConfig {
        DayflowConfig::default().delta
    }

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_787_500_000 + secs, 0).expect("valid ts")
    }

    #[test]
    fn the_first_sample_is_always_kept_and_written() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Sampler::new(delta());
        let px = frame_bytes(64, 48, 0);
        let rec = s
            .observe(0, 1, RawFrame { bgra: &px, width: 64, height: 48 }, at(0), dir.path())
            .unwrap();

        assert_eq!(rec.verdict, GateVerdict::FirstSight);
        assert!(rec.perceived());
        let p = rec.path.expect("first sight must be written");
        assert!(p.exists(), "PNG must exist on disk");
        assert!(std::fs::metadata(&p).unwrap().len() > 0, "and be non-empty");
        assert_eq!(p.extension().unwrap(), "png", "PNG, not a lossy format");
    }

    #[test]
    fn an_identical_second_sample_is_skipped_and_writes_nothing() {
        // The saving. If this regresses, an idle screen costs full price all day.
        let dir = tempfile::tempdir().unwrap();
        let mut s = Sampler::new(delta());
        let px = frame_bytes(64, 48, 0);
        let f = RawFrame { bgra: &px, width: 64, height: 48 };

        s.observe(0, 1, f, at(0), dir.path()).unwrap();
        let second = s.observe(0, 1, f, at(60), dir.path()).unwrap();

        assert_eq!(second.verdict, GateVerdict::Unchanged);
        assert!(!second.perceived());
        assert_eq!(second.path, None, "an unchanged frame must not be written");
        assert_eq!(second.skip_reason(), Some("unchanged"));

        let written = std::fs::read_dir(dir.path()).unwrap().count();
        assert_eq!(written, 1, "two samples, one file — got {written}");
    }

    #[test]
    fn a_genuinely_different_sample_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Sampler::new(delta());
        let a = frame_bytes(64, 48, 0);
        let b = frame_bytes(64, 48, 0xFF); // every pixel differs substantially

        s.observe(0, 1, RawFrame { bgra: &a, width: 64, height: 48 }, at(0), dir.path())
            .unwrap();
        let rec = s
            .observe(0, 1, RawFrame { bgra: &b, width: 64, height: 48 }, at(60), dir.path())
            .unwrap();

        assert_eq!(rec.verdict, GateVerdict::Changed);
        assert!(rec.path.is_some());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[test]
    fn a_blank_screen_is_skipped_with_its_own_reason() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Sampler::new(delta());
        let blank = vec![90u8; (64 * 48 * 4) as usize]; // uniform → std 0
        let rec = s
            .observe(0, 1, RawFrame { bgra: &blank, width: 64, height: 48 }, at(0), dir.path())
            .unwrap();
        assert_eq!(rec.verdict, GateVerdict::Blank);
        assert_eq!(rec.skip_reason(), Some("blank"));
        assert_eq!(rec.path, None);
        // distinguishable from "unchanged" — the two mean different things
        assert_ne!(rec.skip_reason(), Some("unchanged"));
    }

    #[test]
    fn a_malformed_frame_fails_open_and_is_not_called_unchanged() {
        // Dayflow cannot re-capture yesterday. A frame we could not evaluate must
        // never be recorded as "nothing happened".
        let dir = tempfile::tempdir().unwrap();
        let mut s = Sampler::new(delta());
        let good = frame_bytes(64, 48, 0);
        s.observe(0, 1, RawFrame { bgra: &good, width: 64, height: 48 }, at(0), dir.path())
            .unwrap();

        let truncated = vec![0u8; 16]; // far too small for 64x48
        let rec = s.observe(
            0,
            1,
            RawFrame { bgra: &truncated, width: 64, height: 48 },
            at(60),
            dir.path(),
        );
        // The gate says Indeterminate (perceive), then the PNG write refuses the
        // malformed buffer — an ERROR, which is loud, rather than a silent skip.
        assert!(rec.is_err(), "a malformed frame must fail loudly, not silently skip");
    }

    #[test]
    fn displays_are_gated_independently() {
        // Display 1 showing the same thing must not mark display 0 unchanged.
        let dir = tempfile::tempdir().unwrap();
        let mut s = Sampler::new(delta());
        let px = frame_bytes(64, 48, 0);
        let f = RawFrame { bgra: &px, width: 64, height: 48 };

        let d0 = s.observe(0, 1, f, at(0), dir.path()).unwrap();
        let d1 = s.observe(1, 1, f, at(0), dir.path()).unwrap();
        assert_eq!(d0.verdict, GateVerdict::FirstSight);
        assert_eq!(
            d1.verdict,
            GateVerdict::FirstSight,
            "display 1 has its own history; it must not inherit display 0's"
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[test]
    fn forgetting_a_display_resets_its_history() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Sampler::new(delta());
        let px = frame_bytes(64, 48, 0);
        let f = RawFrame { bgra: &px, width: 64, height: 48 };

        s.observe(0, 1, f, at(0), dir.path()).unwrap();
        assert_eq!(s.observe(0, 1, f, at(60), dir.path()).unwrap().verdict, GateVerdict::Unchanged);

        // A display unplugged and replugged must not compare against a stale
        // buffer that may belong to different content entirely.
        s.forget_display(0);
        assert_eq!(
            s.observe(0, 1, f, at(120), dir.path()).unwrap().verdict,
            GateVerdict::FirstSight
        );
    }

    #[test]
    fn sample_timestamps_are_millisecond_precise() {
        // dayflow_samples keys on (session, display, sequence, taken_at). At
        // second resolution two samples in one second violate the primary key.
        let t = DateTime::from_timestamp_millis(1_787_500_000_123).unwrap();
        let rec = SampleRecord {
            display_id: 0,
            sequence: 1,
            taken_at: t,
            verdict: GateVerdict::Changed,
            path: None,
        };
        let key = rec.taken_at_key();
        assert!(key.contains(".123"), "millisecond precision required, got {key}");
    }

    #[test]
    fn filenames_are_unique_per_display_and_instant_and_sort_by_time() {
        let t0 = DateTime::from_timestamp_millis(1_787_500_000_000).unwrap();
        let t1 = DateTime::from_timestamp_millis(1_787_500_000_500).unwrap();
        let a = sample_filename(0, 1, t0);
        let b = sample_filename(1, 1, t0); // same instant, other display
        let c = sample_filename(0, 1, t1); // same display, later
        assert_ne!(a, b, "displays must not collide");
        assert_ne!(a, c, "instants must not collide");
        assert!(a < c, "names must sort chronologically: {a} !< {c}");
    }

    #[test]
    fn the_gate_buffer_is_a_small_downscale_not_the_full_frame() {
        // The gate runs on every frame of every display; comparing full frames
        // would cost more than the perception it is meant to avoid.
        let px = frame_bytes(1920, 1080, 0);
        let g = downscale_gray(RawFrame { bgra: &px, width: 1920, height: 1080 }, 240);
        assert_eq!(g.len(), 240 * 135, "240px wide, aspect preserved");
        // Compare BYTES — that is what is actually held and diffed. A 1080p BGRA
        // frame is 8,294,400 bytes; the gate buffer is 32,400. Roughly 256x.
        let frame_bytes_len = px.len();
        assert!(
            g.len() * 50 < frame_bytes_len,
            "gate buffer ({} B) must be far smaller than the frame ({} B) — it runs \
             on every frame of every display, so comparing full frames would cost \
             more than the perception it exists to avoid",
            g.len(),
            frame_bytes_len
        );
        // ...and it must scale with the configured width, not the source.
        let wide = downscale_gray(RawFrame { bgra: &px, width: 1920, height: 1080 }, 480);
        assert!(wide.len() > g.len(), "a wider gate yields a larger buffer");
    }

    #[test]
    fn disabling_the_gate_keeps_everything() {
        let dir = tempfile::tempdir().unwrap();
        let mut d = delta();
        d.enabled = false;
        let mut s = Sampler::new(d);
        let px = frame_bytes(64, 48, 0);
        let f = RawFrame { bgra: &px, width: 64, height: 48 };

        s.observe(0, 1, f, at(0), dir.path()).unwrap();
        let second = s.observe(0, 1, f, at(60), dir.path()).unwrap();
        assert_eq!(second.verdict, GateVerdict::Changed, "gate off ⇒ nothing is skipped");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }
}
