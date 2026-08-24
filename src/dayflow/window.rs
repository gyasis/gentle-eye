//! Sample-window lifecycle: when a window opens, closes, and what it recorded.
//!
//! A dayflow "window" is a span of wall-clock time containing the samples taken
//! during it (D9) — not a continuously encoded video chunk. This module decides
//! when one ends and hands back what it actually contained.
//!
//! # Why this is not in `capture::service`
//!
//! `capture::service` is gentle-eye's GENERAL recording service, shared with
//! real-time video recording. Dayflow's window rules — a 5-minute floor, an
//! intent, pause-on-idle, mid-day interval changes — are properties of this
//! feature, not of screen recording. Growing them inside the shared service
//! would make every recording carry dayflow's constraints. The controller lives
//! here and USES capture instead.
//!
//! # Nothing derives duration from configuration
//!
//! A day may contain windows of different lengths: the interval is changeable
//! mid-day (FR-035), a pause truncates whichever window was open, and stopping
//! closes a partial one. Every closed window therefore carries its REAL start
//! and end, and no consumer may reconstruct a duration by multiplying a count by
//! the configured interval.

use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, Utc};

/// Why capture paused. Kept distinct from a fault: a pause is a recorded fact
/// with a cause, and must never be reported as a degraded recorder (FR-032).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PauseCause {
    /// The user went idle.
    Idle,
    /// The screen was locked.
    Locked,
    /// The display went to sleep.
    DisplaySleep,
    /// The user turned capture off.
    UserOff,
}

/// A recorded pause interval. `to` is `None` while still paused.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PauseInterval {
    /// When capture stopped.
    pub from: DateTime<Utc>,
    /// When it resumed, if it has.
    pub to: Option<DateTime<Utc>>,
    /// Why it stopped.
    pub cause: PauseCause,
}

/// A window that has ended, with what it actually contained.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosedWindow {
    /// Which display.
    pub display_id: u32,
    /// Monotonic within the session, across pauses and interval changes.
    pub sequence: u64,
    /// REAL start — never derived from the configured interval.
    pub start_wall: DateTime<Utc>,
    /// REAL end.
    pub end_wall: DateTime<Utc>,
    /// How many samples were taken during it, including skipped ones.
    pub sample_count: u32,
    /// Why it ended.
    pub reason: CloseReason,
}

impl ClosedWindow {
    /// The window's actual duration.
    pub fn duration(&self) -> chrono::Duration {
        self.end_wall - self.start_wall
    }
}

/// Why a window ended. A boundary close is routine; the others are all shorter
/// than the configured interval, which is exactly why durations must be read and
/// not computed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReason {
    /// The configured interval elapsed.
    Boundary,
    /// Capture paused mid-window.
    Paused,
    /// The session or daemon stopped.
    Stopped,
    /// The segment interval changed.
    IntervalChanged,
    /// The display went away.
    DisplayRemoved,
}

/// A window currently accumulating samples.
#[derive(Debug, Clone)]
struct OpenWindow {
    sequence: u64,
    start_wall: DateTime<Utc>,
    sample_count: u32,
}

/// Owns window lifecycle for every captured display.
#[derive(Debug)]
pub struct WindowController {
    interval: Duration,
    open: HashMap<u32, OpenWindow>,
    next_sequence: HashMap<u32, u64>,
    paused: Option<PauseInterval>,
    pauses: Vec<PauseInterval>,
}

impl WindowController {
    /// A controller using `interval` as the initial window length.
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            open: HashMap::new(),
            next_sequence: HashMap::new(),
            paused: None,
            pauses: Vec::new(),
        }
    }

    /// The interval currently in force.
    pub fn interval(&self) -> Duration {
        self.interval
    }

    /// Whether capture is currently paused, and why.
    pub fn pause_cause(&self) -> Option<PauseCause> {
        self.paused.as_ref().map(|p| p.cause)
    }

    /// Every pause recorded so far, including one still open.
    pub fn pauses(&self) -> &[PauseInterval] {
        &self.pauses
    }

    /// Record a sample against `display_id` at `now`.
    ///
    /// Returns the window that just CLOSED, if this sample crossed a boundary.
    /// The sample is always counted against the window it belongs to — the new
    /// one when a boundary was crossed.
    ///
    /// While paused this is a no-op returning `None`: no samples are taken, so
    /// none may be counted.
    pub fn on_sample(&mut self, display_id: u32, now: DateTime<Utc>) -> Option<ClosedWindow> {
        if self.paused.is_some() {
            return None;
        }
        let elapsed_exceeded = self
            .open
            .get(&display_id)
            .map(|w| (now - w.start_wall).to_std().unwrap_or_default() >= self.interval)
            .unwrap_or(false);

        let closed = if elapsed_exceeded {
            self.close(display_id, now, CloseReason::Boundary)
        } else {
            None
        };
        self.open_if_absent(display_id, now).sample_count += 1;
        closed
    }

    /// Pause capture, closing whatever window was open on each display.
    ///
    /// The in-progress window is CLOSED and accounted for, never discarded — a
    /// truncated window is real data, and dropping it loses the minutes before
    /// the user stepped away.
    pub fn pause(&mut self, cause: PauseCause, now: DateTime<Utc>) -> Vec<ClosedWindow> {
        if self.paused.is_some() {
            return Vec::new();
        }
        let closed = self.close_all(now, CloseReason::Paused);
        let p = PauseInterval { from: now, to: None, cause };
        self.pauses.push(p.clone());
        self.paused = Some(p);
        closed
    }

    /// Resume capture. The next sample opens a fresh window — resume never
    /// splices across the gap into a window claiming continuous activity.
    pub fn resume(&mut self, now: DateTime<Utc>) {
        if self.paused.take().is_some() {
            // Close the recorded interval too — `pauses` is the durable record,
            // and an interval left open reads as "still paused" forever.
            if let Some(last) = self.pauses.last_mut() {
                last.to = Some(now);
            }
        }
    }

    /// Change the segment interval.
    ///
    /// Takes effect from the NEXT window: open windows are closed at `now` with
    /// their real duration, and no already-recorded window is re-timed (FR-035).
    pub fn set_interval(&mut self, interval: Duration, now: DateTime<Utc>) -> Vec<ClosedWindow> {
        if interval == self.interval {
            return Vec::new();
        }
        let closed = self.close_all(now, CloseReason::IntervalChanged);
        self.interval = interval;
        closed
    }

    /// A display went away: close its window so its samples are not stranded.
    pub fn remove_display(&mut self, display_id: u32, now: DateTime<Utc>) -> Option<ClosedWindow> {
        self.close(display_id, now, CloseReason::DisplayRemoved)
    }

    /// Stop: close every open window (FR-005).
    pub fn stop(&mut self, now: DateTime<Utc>) -> Vec<ClosedWindow> {
        self.close_all(now, CloseReason::Stopped)
    }

    fn open_if_absent(&mut self, display_id: u32, now: DateTime<Utc>) -> &mut OpenWindow {
        let next = self.next_sequence.entry(display_id).or_insert(0);
        self.open.entry(display_id).or_insert_with(|| {
            let w = OpenWindow { sequence: *next, start_wall: now, sample_count: 0 };
            *next += 1;
            w
        })
    }

    fn close(
        &mut self,
        display_id: u32,
        now: DateTime<Utc>,
        reason: CloseReason,
    ) -> Option<ClosedWindow> {
        self.open.remove(&display_id).map(|w| ClosedWindow {
            display_id,
            sequence: w.sequence,
            start_wall: w.start_wall,
            end_wall: now,
            sample_count: w.sample_count,
            reason,
        })
    }

    fn close_all(&mut self, now: DateTime<Utc>, reason: CloseReason) -> Vec<ClosedWindow> {
        let mut ids: Vec<u32> = self.open.keys().copied().collect();
        ids.sort_unstable();
        ids.into_iter()
            .filter_map(|d| self.close(d, now, reason))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_787_500_000 + secs, 0).expect("valid ts")
    }

    /// 10-minute windows, well inside the permitted 5-minute-to-1-hour range.
    fn ctl() -> WindowController {
        WindowController::new(Duration::from_secs(600))
    }

    #[test]
    fn a_window_closes_on_the_boundary_and_reports_its_real_span() {
        let mut c = ctl();
        c.on_sample(0, at(0));
        assert!(c.on_sample(0, at(300)).is_none(), "mid-window, nothing closes");

        let closed = c.on_sample(0, at(600)).expect("boundary must close a window");
        assert_eq!(closed.sequence, 0);
        assert_eq!(closed.start_wall, at(0));
        assert_eq!(closed.end_wall, at(600));
        assert_eq!(closed.reason, CloseReason::Boundary);
        assert_eq!(closed.duration().num_seconds(), 600);
        assert_eq!(closed.sample_count, 2, "the two samples before the boundary");
    }

    #[test]
    fn a_day_may_contain_windows_of_different_lengths() {
        // The reason nothing may multiply a count by the configured interval.
        let mut c = ctl();
        c.on_sample(0, at(0));
        let full = c.on_sample(0, at(600)).unwrap();

        // interval changed mid-day -> the open window closes SHORT
        let short = c.set_interval(Duration::from_secs(1800), at(700));
        assert_eq!(short.len(), 1);
        assert_eq!(short[0].duration().num_seconds(), 100, "a 100s window is legitimate");
        assert_eq!(short[0].reason, CloseReason::IntervalChanged);

        // and the next window uses the NEW interval
        c.on_sample(0, at(700));
        assert!(c.on_sample(0, at(2000)).is_none(), "1300s < the new 1800s interval");
        let long = c.on_sample(0, at(2500)).expect("closes at the new boundary");

        assert_eq!(full.duration().num_seconds(), 600);
        assert_eq!(long.duration().num_seconds(), 1800);
        assert_ne!(
            full.duration(),
            long.duration(),
            "two windows in one day with different real durations"
        );
    }

    #[test]
    fn changing_the_interval_never_retimes_an_already_closed_window() {
        // FR-035. The closed window's recorded span must be untouched.
        let mut c = ctl();
        c.on_sample(0, at(0));
        let before = c.on_sample(0, at(600)).unwrap();
        let recorded = (before.start_wall, before.end_wall, before.sequence);

        c.set_interval(Duration::from_secs(3600), at(700));

        assert_eq!(
            (before.start_wall, before.end_wall, before.sequence),
            recorded,
            "an already-closed window is immutable"
        );
    }

    #[test]
    fn pausing_closes_the_open_window_rather_than_discarding_it() {
        // The minutes before the user stepped away are real data.
        let mut c = ctl();
        c.on_sample(0, at(0));
        c.on_sample(0, at(120));

        let closed = c.pause(PauseCause::Idle, at(200));
        assert_eq!(closed.len(), 1);
        assert_eq!(closed[0].reason, CloseReason::Paused);
        assert_eq!(closed[0].end_wall, at(200), "closed AT the pause, not at the boundary");
        assert_eq!(closed[0].sample_count, 2, "its samples are not lost");
        assert_eq!(c.pause_cause(), Some(PauseCause::Idle));
    }

    #[test]
    fn no_samples_are_counted_while_paused() {
        let mut c = ctl();
        c.on_sample(0, at(0));
        c.pause(PauseCause::Locked, at(100));

        assert!(c.on_sample(0, at(200)).is_none());
        assert!(c.on_sample(0, at(900)).is_none(), "not even across a boundary");

        c.resume(at(1000));
        c.on_sample(0, at(1000));
        let w = c.on_sample(0, at(1600)).expect("a fresh window after resume");
        assert_eq!(w.start_wall, at(1000), "resume starts a NEW window at the resume instant");
        assert_eq!(w.sample_count, 1, "only the post-resume sample counted");
    }

    #[test]
    fn resume_never_splices_across_the_gap() {
        // A window spanning the pause would claim continuous activity that did
        // not happen.
        let mut c = ctl();
        c.on_sample(0, at(0));
        let before = c.pause(PauseCause::Idle, at(100)).pop().unwrap();
        c.resume(at(5000));
        c.on_sample(0, at(5000));
        let after = c.on_sample(0, at(5600)).unwrap();

        assert!(before.end_wall <= after.start_wall, "windows must not overlap the gap");
        assert_ne!(before.sequence, after.sequence);
        assert!(after.start_wall > before.end_wall, "there IS a gap, and it is visible");
    }

    #[test]
    fn a_pause_is_recorded_with_its_cause_and_closed_on_resume() {
        // A gap must be a recorded FACT, not an absence of rows — otherwise an
        // idle pause and a crash are indistinguishable.
        let mut c = ctl();
        c.on_sample(0, at(0));
        c.pause(PauseCause::DisplaySleep, at(100));

        assert_eq!(c.pauses().len(), 1);
        assert_eq!(c.pauses()[0].cause, PauseCause::DisplaySleep);
        assert_eq!(c.pauses()[0].from, at(100));
        assert_eq!(c.pauses()[0].to, None, "open while still paused");

        c.resume(at(500));
        assert_eq!(c.pauses()[0].to, Some(at(500)), "closed on resume");
        assert_eq!(c.pause_cause(), None);
    }

    #[test]
    fn stopping_closes_the_in_progress_window() {
        // FR-005: the partial window is accounted for, not discarded.
        let mut c = ctl();
        c.on_sample(0, at(0));
        c.on_sample(1, at(0));
        c.on_sample(0, at(60));

        let closed = c.stop(at(90));
        assert_eq!(closed.len(), 2, "every display's open window closes");
        assert!(closed.iter().all(|w| w.reason == CloseReason::Stopped));
        assert!(closed.iter().all(|w| w.end_wall == at(90)));
        let d0 = closed.iter().find(|w| w.display_id == 0).unwrap();
        assert_eq!(d0.sample_count, 2);
    }

    #[test]
    fn sequence_is_monotonic_per_display_across_pauses_and_interval_changes() {
        // The durable identity. It must not restart when anything interrupts.
        let mut c = ctl();
        let mut seqs = Vec::new();

        c.on_sample(0, at(0));
        seqs.push(c.on_sample(0, at(600)).unwrap().sequence);

        c.on_sample(0, at(600));
        seqs.extend(c.pause(PauseCause::Idle, at(700)).iter().map(|w| w.sequence));
        c.resume(at(800));

        c.on_sample(0, at(800));
        seqs.extend(c.set_interval(Duration::from_secs(900), at(900)).iter().map(|w| w.sequence));

        c.on_sample(0, at(900));
        seqs.push(c.stop(at(1000)).pop().unwrap().sequence);

        assert_eq!(seqs, vec![0, 1, 2, 3], "sequence never restarts: {seqs:?}");
    }

    #[test]
    fn displays_keep_independent_windows_and_sequences() {
        let mut c = ctl();
        c.on_sample(0, at(0));
        c.on_sample(1, at(300)); // display 1 starts later

        assert!(c.on_sample(1, at(600)).is_none(), "display 1 is only 300s in");
        let d0 = c.on_sample(0, at(600)).expect("display 0 hits its boundary");
        assert_eq!(d0.display_id, 0);

        let d1 = c.on_sample(1, at(900)).expect("display 1 hits its own boundary later");
        assert_eq!(d1.display_id, 1);
        assert_eq!(d1.start_wall, at(300), "its own start, not display 0's");
        assert_eq!(d0.sequence, 0);
        assert_eq!(d1.sequence, 0, "sequences are per display");
    }

    #[test]
    fn removing_a_display_closes_its_window_so_samples_are_not_stranded() {
        let mut c = ctl();
        c.on_sample(0, at(0));
        c.on_sample(1, at(0));
        let closed = c.remove_display(1, at(120)).expect("its window must close");
        assert_eq!(closed.display_id, 1);
        assert_eq!(closed.reason, CloseReason::DisplayRemoved);
        assert_eq!(closed.sample_count, 1);
        // display 0 is unaffected
        assert!(c.on_sample(0, at(600)).is_some());
    }

    #[test]
    fn pausing_twice_does_not_double_record() {
        let mut c = ctl();
        c.on_sample(0, at(0));
        c.pause(PauseCause::Idle, at(100));
        let second = c.pause(PauseCause::Locked, at(150));
        assert!(second.is_empty(), "already paused — nothing further to close");
        assert_eq!(c.pauses().len(), 1, "one pause, not two");
        assert_eq!(c.pause_cause(), Some(PauseCause::Idle), "the original cause stands");
    }
}
