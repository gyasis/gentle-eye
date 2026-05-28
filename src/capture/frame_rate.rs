//! Frame-rate control for the capture loop.
//!
//! A [`FrameRateController`] gates the capture loop so frames are emitted at (at
//! most) the configured FPS, regardless of how fast the loop spins. The logic is
//! clock-injectable and fully unit-tested; no I/O.
//!
//! Authored 2026-05-28 from the `RecordingConfig.fps` contract + PRD §Capture
//! (fps 1-30) — the recovered source was junk.

use std::time::{Duration, Instant};

/// Clamp range for capture FPS (PRD: 1-30).
const MIN_FPS: u32 = 1;
const MAX_FPS: u32 = 30;

/// Throttles a capture loop to a target frame rate.
#[derive(Debug, Clone)]
pub struct FrameRateController {
    fps: u32,
    interval: Duration,
    next_capture: Instant,
}

impl FrameRateController {
    /// Create a controller for `fps` (clamped to 1-30), starting now.
    pub fn new(fps: u32) -> Self {
        Self::starting_at(fps, Instant::now())
    }

    /// Clock-injectable constructor (used by tests).
    pub fn starting_at(fps: u32, now: Instant) -> Self {
        let fps = fps.clamp(MIN_FPS, MAX_FPS);
        Self {
            fps,
            interval: Duration::from_secs_f64(1.0 / f64::from(fps)),
            next_capture: now,
        }
    }

    /// The effective (clamped) frames-per-second.
    pub fn fps(&self) -> u32 {
        self.fps
    }

    /// The target interval between captured frames.
    pub fn interval(&self) -> Duration {
        self.interval
    }

    /// Whether a frame should be captured at `now`. When it returns `true`, the
    /// next deadline advances by one interval (catching up if the loop stalled).
    pub fn should_capture(&mut self, now: Instant) -> bool {
        if now >= self.next_capture {
            // Advance to the next future deadline (skip missed slots to avoid
            // a burst after a stall).
            self.next_capture += self.interval;
            if self.next_capture <= now {
                self.next_capture = now + self.interval;
            }
            true
        } else {
            false
        }
    }

    /// How long to sleep until the next capture deadline (zero if already due).
    pub fn time_until_next(&self, now: Instant) -> Duration {
        self.next_capture.saturating_duration_since(now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_fps_to_valid_range() {
        assert_eq!(FrameRateController::new(0).fps(), MIN_FPS);
        assert_eq!(FrameRateController::new(1000).fps(), MAX_FPS);
        assert_eq!(FrameRateController::new(5).fps(), 5);
    }

    #[test]
    fn interval_matches_fps() {
        assert_eq!(FrameRateController::new(1).interval(), Duration::from_secs(1));
        assert_eq!(
            FrameRateController::new(10).interval(),
            Duration::from_millis(100)
        );
    }

    #[test]
    fn gates_on_deadline() {
        let t0 = Instant::now();
        let mut c = FrameRateController::starting_at(1, t0); // 1s interval
        assert!(c.should_capture(t0)); // first slot due immediately
        assert!(!c.should_capture(t0 + Duration::from_millis(500))); // too soon
        assert!(c.should_capture(t0 + Duration::from_millis(1000))); // next slot
    }

    #[test]
    fn skips_missed_slots_after_stall() {
        let t0 = Instant::now();
        let mut c = FrameRateController::starting_at(10, t0); // 100ms interval
        assert!(c.should_capture(t0));
        // Loop stalled for 5 intervals; we still only fire once and re-anchor.
        let stalled = t0 + Duration::from_millis(500);
        assert!(c.should_capture(stalled));
        assert!(!c.should_capture(stalled + Duration::from_millis(50)));
        assert!(c.should_capture(stalled + Duration::from_millis(100)));
    }
}
