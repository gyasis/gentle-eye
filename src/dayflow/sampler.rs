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

use crate::config::{DeltaConfig, DropPolicy};
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

/// Why a sample could not be captured for its interval.
///
/// A drop is NOT a skip. A skip is the gate doing its job — nothing changed, so
/// nothing needed keeping. A drop means the interval's frame was WANTED and
/// could not be obtained, which is missing data and must be visible as such.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DropReason {
    /// The frame buffer did not match its stated dimensions — a truncated or
    /// half-written capture.
    MalformedFrame,
    /// The frame was good but could not be written to disk.
    WriteFailed,
}

impl DropReason {
    /// Short stable label, for the ledger and for status payloads.
    pub fn label(self) -> &'static str {
        match self {
            Self::MalformedFrame => "malformed_frame",
            Self::WriteFailed => "write_failed",
        }
    }
}

/// A frame that was wanted for an interval and could not be obtained.
///
/// Recorded rather than raised, because a single bad frame must not end an
/// all-day recording — but it must never be invisible either.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SampleDrop {
    /// Which display.
    pub display_id: u32,
    /// The window the interval belonged to.
    pub sequence: u64,
    /// The interval's timestamp.
    pub at: DateTime<Utc>,
    /// Why it was dropped.
    pub reason: DropReason,
    /// How many acquisition attempts were made for this interval.
    pub attempts: u32,
    /// Whether a later attempt recovered a usable frame for the same interval.
    pub recovered: bool,
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
    /// Why this interval produced no frame, if it did not.
    ///
    /// `None` covers both "kept" and "deliberately skipped"; the two are
    /// distinguished by [`SampleRecord::verdict`]. A `Some` here means data is
    /// MISSING, which is a different thing entirely.
    pub drop: Option<DropReason>,
    /// Acquisition attempts made for this interval.
    pub attempts: u32,
}

impl SampleRecord {
    /// Whether this sample should be handed to perception.
    pub fn perceived(&self) -> bool {
        self.drop.is_none() && self.verdict.should_perceive()
    }

    /// Whether this interval lost data, as opposed to deliberately skipping it.
    pub fn dropped(&self) -> bool {
        self.drop.is_some()
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

/// One interval's worth of sampling context.
///
/// A struct rather than a long argument list, for the same reason
/// [`crate::dayflow::models::LivenessInput`] is one: eight positional parameters
/// is a swap the compiler cannot catch. This is the second time that lint fired
/// in this feature.
#[derive(Debug, Clone, Copy)]
pub struct SampleRequest<'a> {
    /// Which display.
    pub display_id: u32,
    /// The window this interval belongs to.
    pub sequence: u64,
    /// The captured frame.
    pub frame: RawFrame<'a>,
    /// The interval's instant.
    pub taken_at: DateTime<Utc>,
    /// Where a kept frame is written.
    pub dir: &'a Path,
    /// How many acquisition attempts are permitted.
    pub max_attempts: u32,
}

/// Samples frames, gates them, and writes the survivors.
#[derive(Debug)]
pub struct Sampler {
    delta: DeltaConfig,
    displays: std::collections::HashMap<u32, DisplayState>,
    drops: Vec<SampleDrop>,
}

impl Sampler {
    /// A sampler using the given delta-gate configuration.
    pub fn new(delta: DeltaConfig) -> Self {
        Self {
            delta,
            displays: std::collections::HashMap::new(),
            drops: Vec::new(),
        }
    }

    /// Every interval whose frame could not be obtained.
    ///
    /// This is the surface a status payload or a timeline view renders: dropped
    /// frames must be visible and countable, not inferred from a gap.
    pub fn drops(&self) -> &[SampleDrop] {
        &self.drops
    }

    /// How many intervals lost data.
    pub fn dropped_count(&self) -> usize {
        self.drops.len()
    }

    /// Drops not recovered by a later attempt — the ones that are truly missing.
    pub fn unrecovered_drops(&self) -> impl Iterator<Item = &SampleDrop> {
        self.drops.iter().filter(|d| !d.recovered)
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
        self.observe_with_reacquire(
            SampleRequest { display_id, sequence, frame, taken_at, dir, max_attempts: 1 },
            |_| None,
        )
    }

    /// Observe a frame, and on a bad one try to RE-ACQUIRE a good frame for the
    /// same interval before giving up.
    ///
    /// `acquire` is called with the attempt number (2, 3, …) and returns a fresh
    /// buffer for the SAME interval, or `None` if it cannot. Recovering a frame
    /// for the interval is always better than recording a hole — the timeline is
    /// the artifact, and a missing minute cannot be captured again later.
    ///
    /// A frame that still cannot be obtained after `max_attempts` is recorded as
    /// a [`SampleDrop`] and logged at WARN. It is deliberately NOT an error: a
    /// single bad frame must not end an all-day recording. It must equally never
    /// be silent — a drop is missing data, not a skip.
    pub fn observe_with_reacquire<F>(
        &mut self,
        req: SampleRequest<'_>,
        mut acquire: F,
    ) -> Result<SampleRecord, DayflowError>
    where
        F: FnMut(u32) -> Option<Vec<u8>>,
    {
        let SampleRequest { display_id, sequence, frame, taken_at, dir, max_attempts } = req;
        let attempts_allowed = max_attempts.max(1);
        let (width, height) = (frame.width, frame.height);
        let mut owned: Option<Vec<u8>> = None;
        let mut attempts = 0u32;
        let mut last_reason = DropReason::MalformedFrame;

        loop {
            attempts += 1;
            let current = match owned.as_deref() {
                Some(bytes) => RawFrame { bgra: bytes, width, height },
                None => frame,
            };

            if current.is_well_formed() {
                match self.evaluate_and_store(display_id, sequence, current, taken_at, dir) {
                    Ok(rec) => {
                        if attempts > 1 {
                            // A retry succeeded — record that the interval was at
                            // risk and was recovered, so the anomaly stays visible.
                            self.drops.push(SampleDrop {
                                display_id,
                                sequence,
                                at: taken_at,
                                reason: last_reason,
                                attempts,
                                recovered: true,
                            });
                            tracing::warn!(
                                display_id,
                                sequence,
                                attempts,
                                reason = last_reason.label(),
                                "dayflow: recovered a frame for the interval after a bad capture"
                            );
                        }
                        return Ok(SampleRecord { attempts, drop: None, ..rec });
                    }
                    Err(e) => {
                        last_reason = DropReason::WriteFailed;
                        tracing::warn!(
                            display_id,
                            sequence,
                            attempt = attempts,
                            error = %e,
                            "dayflow: could not write the sample for this interval"
                        );
                    }
                }
            } else {
                last_reason = DropReason::MalformedFrame;
                tracing::warn!(
                    display_id,
                    sequence,
                    attempt = attempts,
                    expected_bytes = (width as usize) * (height as usize) * 4,
                    got_bytes = current.bgra.len(),
                    "dayflow: malformed frame — a truncated or half-written capture"
                );
            }

            if attempts >= attempts_allowed {
                break;
            }
            match acquire(attempts + 1) {
                Some(next) => owned = Some(next),
                None => break, // no way to re-acquire; stop rather than spin
            }
        }

        // Every attempt failed. Record the hole LOUDLY — this is missing data.
        self.drops.push(SampleDrop {
            display_id,
            sequence,
            at: taken_at,
            reason: last_reason,
            attempts,
            recovered: false,
        });
        tracing::warn!(
            display_id,
            sequence,
            attempts,
            reason = last_reason.label(),
            total_dropped = self.drops.len(),
            "dayflow: DROPPED a frame for this interval — investigate; the minute cannot be recaptured"
        );
        if self.delta.on_drop == DropPolicy::Fail {
            // Development posture: stop, so the cause is investigated rather
            // than accumulating quietly in the ledger. The drop is recorded
            // either way — the policy only decides whether the run continues.
            return Err(DayflowError::Invalid(format!(
                "dropped frame on display {display_id} at {taken_at} after {attempts} attempt(s): \
                 {} — set dayflow.delta.on_drop = \"record\" to keep recording through this",
                last_reason.label()
            )));
        }
        Ok(SampleRecord {
            display_id,
            sequence,
            taken_at,
            verdict: GateVerdict::Indeterminate,
            path: None,
            drop: Some(last_reason),
            attempts,
        })
    }

    /// Gate a well-formed frame and persist it if it survives.
    fn evaluate_and_store(
        &mut self,
        display_id: u32,
        sequence: u64,
        frame: RawFrame<'_>,
        taken_at: DateTime<Utc>,
        dir: &Path,
    ) -> Result<SampleRecord, DayflowError> {
        let gate_buf = downscale_gray(frame, self.delta.gate_width);

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
            GateVerdict::Changed
        };

        let path = if verdict.should_perceive() {
            let p = dir.join(sample_filename(display_id, sequence, taken_at));
            write_png(frame, &p)?;
            Some(p)
        } else {
            None
        };

        // Only remember the frame once it is safely handled: updating the
        // comparison buffer before a failed write would make the NEXT frame look
        // unchanged against a frame that was never stored.
        if !gate_buf.is_empty() {
            self.displays.entry(display_id).or_default().prev_gate = Some(gate_buf);
        }

        Ok(SampleRecord {
            display_id,
            sequence,
            taken_at,
            verdict,
            path,
            drop: None,
            attempts: 1,
        })
    }

    /// Forget a display's history — used when a display is removed, so a stale
    /// buffer cannot be compared against a different screen later.
    pub fn forget_display(&mut self, display_id: u32) {
        self.displays.remove(&display_id);
    }
}

/// The filename prefix every sample of one window shares.
///
/// Public because the summarizer resolves a window's samples by it: a private
/// convention that two modules both depend on is a convention that drifts.
pub fn sample_prefix(display_id: u32, sequence: u64) -> String {
    format!("d{display_id}_w{sequence:06}_")
}

/// Filename for a sample: sortable, and unique per display and instant.
fn sample_filename(display_id: u32, sequence: u64, taken_at: DateTime<Utc>) -> String {
    format!(
        "{}{}.png",
        sample_prefix(display_id, sequence),
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

    /// Production posture: record the drop and keep going.
    fn delta_recording() -> DeltaConfig {
        let mut d = delta();
        d.on_drop = crate::config::DropPolicy::Record;
        d
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
    fn a_malformed_frame_is_recorded_as_a_drop_not_a_skip() {
        // A half-written capture is MISSING DATA for that interval. It must not
        // be an error that ends the day, and it must never be filed as a skip —
        // a skip means the gate worked, a drop means a minute is gone and cannot
        // be recaptured.
        let dir = tempfile::tempdir().unwrap();
        let mut s = Sampler::new(delta_recording());
        let good = frame_bytes(64, 48, 0);
        s.observe(0, 1, RawFrame { bgra: &good, width: 64, height: 48 }, at(0), dir.path())
            .unwrap();

        let truncated = vec![0u8; 16];
        let rec = s
            .observe(0, 1, RawFrame { bgra: &truncated, width: 64, height: 48 }, at(60), dir.path())
            .expect("a bad frame must not end the recording");

        assert!(rec.dropped(), "it must be recorded as a drop");
        assert_eq!(rec.drop, Some(DropReason::MalformedFrame));
        assert!(!rec.perceived(), "a dropped interval has nothing to perceive");
        assert_eq!(rec.path, None);
        assert_ne!(rec.skip_reason(), Some("unchanged"), "a drop is NOT a skip");

        assert_eq!(s.dropped_count(), 1, "the hole must be countable");
        assert_eq!(s.unrecovered_drops().count(), 1);
        let d = &s.drops()[0];
        assert_eq!(d.at, at(60), "the drop names the interval it lost");
        assert!(!d.recovered);
    }

    #[test]
    fn the_development_default_stops_on_a_dropped_frame() {
        // While the feature is being built a drop should STOP us and get fixed.
        // A hole quietly recorded in a ledger is easy to scroll past; a run that
        // halts is not.
        let dir = tempfile::tempdir().unwrap();
        let mut s = Sampler::new(delta()); // default policy
        assert_eq!(delta().on_drop, crate::config::DropPolicy::Fail, "dev default");

        let truncated = vec![0u8; 16];
        let err = s
            .observe(0, 1, RawFrame { bgra: &truncated, width: 64, height: 48 }, at(0), dir.path())
            .expect_err("the dev default must surface a drop as an error");
        assert!(format!("{err}").contains("dropped frame"), "got: {err}");

        // ...and the drop is still RECORDED, so the policy only decides whether
        // the run continues — never whether the hole is visible.
        assert_eq!(s.dropped_count(), 1, "a failing policy must not skip recording");
        assert_eq!(s.unrecovered_drops().count(), 1);
    }

    #[test]
    fn switching_to_record_keeps_an_all_day_run_alive() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Sampler::new(delta_recording());
        let truncated = vec![0u8; 16];
        for i in 0..3 {
            let rec = s
                .observe(0, 1, RawFrame { bgra: &truncated, width: 64, height: 48 }, at(i * 60), dir.path())
                .expect("production posture must survive a bad frame");
            assert!(rec.dropped());
        }
        assert_eq!(s.dropped_count(), 3, "every hole is counted");
    }

    #[test]
    fn a_bad_frame_can_be_recovered_by_re_acquiring_for_the_same_interval() {
        // Better than recording a hole: go back and get a good frame for the
        // interval. The timeline is the artifact and the minute is not repeatable.
        let dir = tempfile::tempdir().unwrap();
        let mut s = Sampler::new(delta());
        let good = frame_bytes(64, 48, 7);
        let truncated = vec![0u8; 16];

        let mut calls = 0;
        let rec = s
            .observe_with_reacquire(
                SampleRequest {
                    display_id: 0,
                    sequence: 1,
                    frame: RawFrame { bgra: &truncated, width: 64, height: 48 },
                    taken_at: at(0),
                    dir: dir.path(),
                    max_attempts: 3,
                },
                |attempt| {
                    calls += 1;
                    assert_eq!(attempt, 2, "re-acquire is asked for the NEXT attempt");
                    Some(good.clone())
                },
            )
            .unwrap();

        assert_eq!(calls, 1, "one re-acquisition was enough");
        assert!(!rec.dropped(), "the interval was recovered");
        assert!(rec.path.is_some(), "and a frame was written for it");
        assert_eq!(rec.attempts, 2);

        // ...but the anomaly stays VISIBLE rather than being erased by success
        assert_eq!(s.dropped_count(), 1, "the near-miss is still recorded");
        assert!(s.drops()[0].recovered, "flagged as recovered");
        assert_eq!(s.unrecovered_drops().count(), 0, "no data is actually missing");
    }

    #[test]
    fn re_acquisition_gives_up_rather_than_spinning() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Sampler::new(delta_recording());
        let truncated = vec![0u8; 16];
        let mut calls = 0;
        let rec = s
            .observe_with_reacquire(
                SampleRequest {
                    display_id: 0,
                    sequence: 1,
                    frame: RawFrame { bgra: &truncated, width: 64, height: 48 },
                    taken_at: at(0),
                    dir: dir.path(),
                    max_attempts: 4,
                },
                |_| {
                    calls += 1;
                    Some(vec![0u8; 16]) // still bad
                },
            )
            .unwrap();
        assert_eq!(rec.attempts, 4, "it honours max_attempts");
        assert!(calls <= 4, "and does not spin: {calls} re-acquisitions");
        assert!(rec.dropped());
        assert_eq!(s.unrecovered_drops().count(), 1);
    }

    #[test]
    fn a_capture_source_that_cannot_re_acquire_stops_immediately() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Sampler::new(delta_recording());
        let truncated = vec![0u8; 16];
        let rec = s
            .observe_with_reacquire(
                SampleRequest {
                    display_id: 0,
                    sequence: 1,
                    frame: RawFrame { bgra: &truncated, width: 64, height: 48 },
                    taken_at: at(0),
                    dir: dir.path(),
                    max_attempts: 10,
                },
                |_| None, // the source has nothing to give
            )
            .unwrap();
        assert_eq!(rec.attempts, 1, "no point retrying a source that cannot produce");
        assert!(rec.dropped());
    }

    #[test]
    fn a_drop_does_not_corrupt_the_gate_for_the_next_frame() {
        // The subtle one: if a bad frame updated the comparison buffer, the NEXT
        // good frame would be compared against garbage — or worse, a frame that
        // was never stored would make a real change look unchanged.
        let dir = tempfile::tempdir().unwrap();
        let mut s = Sampler::new(delta_recording());
        let a = frame_bytes(64, 48, 0);

        s.observe(0, 1, RawFrame { bgra: &a, width: 64, height: 48 }, at(0), dir.path())
            .unwrap();
        let truncated = vec![0u8; 16];
        s.observe(0, 1, RawFrame { bgra: &truncated, width: 64, height: 48 }, at(60), dir.path())
            .unwrap();

        // the SAME frame as before the drop must still read as unchanged
        let rec = s
            .observe(0, 1, RawFrame { bgra: &a, width: 64, height: 48 }, at(120), dir.path())
            .unwrap();
        assert_eq!(
            rec.verdict,
            GateVerdict::Unchanged,
            "a dropped frame must not become the comparison baseline"
        );
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
            drop: None,
            attempts: 1,
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
