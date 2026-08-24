//! Idle detection, and the hysteresis that turns it into pause/resume decisions.
//!
//! # What signal, and what NOT to use
//!
//! Idle comes from the X11 MIT-SCREEN-SAVER extension's
//! `ms_since_user_input` counter, verified monotonic on this host (T005).
//!
//! It deliberately does **not** use that extension's `state` field. Probed on
//! GNOME/X11 that field returns **3** — outside the documented `Off`/`On`/
//! `Disabled` range — the classic X screensaver is disabled, and no locker
//! daemon runs. Lock detection built on it would have silently never fired.
//! Lock-based pausing was descoped in any case; the idle threshold is the
//! primary and sufficient trigger.
//!
//! # Degrade to recording, never to silence
//!
//! Detection sits behind [`IdleDetector`] so a host without a backend (Wayland,
//! macOS, a headless box) uses [`NeverIdle`] and simply keeps recording. That
//! direction is deliberate: a detector that fails toward "idle" would pause a
//! working day and lose it, and dayflow cannot re-capture yesterday.

use std::time::Duration;

use crate::config::IdleConfig;

/// A source of "how long since the user last did anything".
pub trait IdleDetector: Send + Sync {
    /// Time since the last user input, or `None` if it cannot be determined.
    ///
    /// `None` means UNKNOWN, not zero, and callers must treat it as
    /// "keep recording" rather than as activity.
    fn idle_for(&self) -> Option<Duration>;
}

/// The fallback for a platform with no supported backend: never idle, so
/// capture simply continues.
#[derive(Debug, Clone, Copy, Default)]
pub struct NeverIdle;

impl IdleDetector for NeverIdle {
    fn idle_for(&self) -> Option<Duration> {
        Some(Duration::ZERO)
    }
}

/// Reads the X11 MIT-SCREEN-SAVER idle counter.
#[cfg(target_os = "linux")]
pub struct X11IdleDetector {
    conn: x11rb::rust_connection::RustConnection,
    root: u32,
}

#[cfg(target_os = "linux")]
impl std::fmt::Debug for X11IdleDetector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("X11IdleDetector").field("root", &self.root).finish()
    }
}

#[cfg(target_os = "linux")]
impl X11IdleDetector {
    /// Connect to the X server named by `$DISPLAY`.
    ///
    /// Returns `None` when there is no display or the extension is unavailable,
    /// so the caller can fall back to [`NeverIdle`] rather than fail a recording.
    pub fn new() -> Option<Self> {
        let (conn, screen_num) = x11rb::connect(None).ok()?;
        let root = {
            use x11rb::connection::Connection;
            conn.setup().roots.get(screen_num)?.root
        };
        let det = Self { conn, root };
        // Prove the extension actually answers before claiming this backend.
        det.idle_for().map(|_| det)
    }
}

#[cfg(target_os = "linux")]
impl IdleDetector for X11IdleDetector {
    fn idle_for(&self) -> Option<Duration> {
        use x11rb::protocol::screensaver::ConnectionExt;
        let reply = self.conn.screensaver_query_info(self.root).ok()?.reply().ok()?;
        Some(Duration::from_millis(u64::from(reply.ms_since_user_input)))
    }
}

/// Whether capture should be running, from the tracker's point of view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    /// The user is present; capture.
    Active,
    /// The user has been away past the threshold; pause.
    Idle,
}

/// A change worth acting on. `None` from [`IdleTracker::observe`] means the
/// state is unchanged, including while a transition is still dwelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleTransition {
    /// Crossed into idle and dwelled: pause capture.
    WentIdle,
    /// Returned to activity and dwelled: resume capture.
    BecameActive,
}

/// Turns a raw idle duration into debounced pause/resume decisions.
///
/// Hysteresis applies to BOTH directions. Without it, an idle reading that
/// hovers around the threshold flips the recorder repeatedly and shreds the day
/// into a burst of tiny windows.
#[derive(Debug)]
pub struct IdleTracker {
    cfg: IdleConfig,
    state: Activity,
    /// The opposite condition currently being observed, if any.
    pending: Option<Activity>,
    /// How long `pending` has held SINCE IT WAS FIRST SEEN.
    pending_for: Duration,
}

impl IdleTracker {
    /// A tracker starting from [`Activity::Active`].
    pub fn new(cfg: IdleConfig) -> Self {
        Self { cfg, state: Activity::Active, pending: None, pending_for: Duration::ZERO }
    }

    /// Current state.
    pub fn state(&self) -> Activity {
        self.state
    }

    /// Feed a reading.
    ///
    /// `idle_for` is what the detector reported; `since_last` is how long since
    /// the previous call, used to accumulate dwell. `None` for `idle_for` means
    /// the detector could not tell — treated as ACTIVE, so an unreadable signal
    /// keeps recording instead of silently pausing the day.
    pub fn observe(
        &mut self,
        idle_for: Option<Duration>,
        since_last: Duration,
    ) -> Option<IdleTransition> {
        if !self.cfg.enabled {
            // Disabled: never pause, and never accumulate a pending transition.
            // There is no "release a held pause" branch, because `enabled` is
            // fixed for a tracker's lifetime and `state` can only become Idle on
            // the enabled path — that combination is unreachable.
            self.pending = None;
            self.pending_for = Duration::ZERO;
            return None;
        }

        let threshold = Duration::from_secs(u64::from(self.cfg.threshold_seconds));
        let dwell = Duration::from_secs(u64::from(self.cfg.hysteresis_seconds));

        // Unknown reads as active — degrade to recording, never to silence.
        let looks_idle = idle_for.is_some_and(|d| d >= threshold);

        let wants = if looks_idle { Activity::Idle } else { Activity::Active };
        if wants == self.state {
            self.pending = None;
            self.pending_for = Duration::ZERO;
            return None;
        }

        // Dwell accumulates only from readings AFTER the flip was first seen.
        //
        // Adding `since_last` on the first sighting would credit time spent
        // under the OPPOSITE condition toward the dwell, which makes the whole
        // debounce collapse whenever the poll period is at least as long as the
        // dwell: every transition would then fire on its first observation.
        // Dayflow polls minutes apart, so that is the normal case, not an edge.
        if self.pending != Some(wants) {
            self.pending = Some(wants);
            self.pending_for = Duration::ZERO;
            return None;
        }
        self.pending_for = self.pending_for.saturating_add(since_last);
        if self.pending_for < dwell {
            return None; // still dwelling
        }

        self.pending = None;
        self.pending_for = Duration::ZERO;
        self.state = wants;
        Some(match wants {
            Activity::Idle => IdleTransition::WentIdle,
            Activity::Active => IdleTransition::BecameActive,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DayflowConfig;

    fn cfg() -> IdleConfig {
        DayflowConfig::default().idle // 300s threshold, 30s hysteresis
    }

    const TICK: Duration = Duration::from_secs(60);

    #[test]
    fn a_never_idle_backend_keeps_recording() {
        // The fallback for an unsupported platform must not pause anything.
        let d = NeverIdle;
        let mut t = IdleTracker::new(cfg());
        for _ in 0..100 {
            assert_eq!(t.observe(d.idle_for(), TICK), None);
        }
        assert_eq!(t.state(), Activity::Active);
    }

    #[test]
    fn an_unreadable_signal_degrades_to_recording_not_to_pausing() {
        // `None` means the detector could not tell. Treating that as idle would
        // pause a working day over a transient failure, and yesterday cannot be
        // re-captured.
        let mut t = IdleTracker::new(cfg());
        for _ in 0..100 {
            assert_eq!(t.observe(None, TICK), None, "unknown must never trigger a pause");
        }
        assert_eq!(t.state(), Activity::Active);
    }

    #[test]
    fn crossing_the_threshold_pauses_only_after_the_dwell() {
        let mut t = IdleTracker::new(cfg()); // 300s threshold, 30s dwell
        assert_eq!(t.observe(Some(Duration::from_secs(299)), TICK), None, "under threshold");

        // First reading past the threshold only STARTS the dwell — the time
        // before it was spent active and must not be credited.
        let first = t.observe(Some(Duration::from_secs(301)), Duration::from_secs(10));
        assert_eq!(first, None, "the flip is only just observed");
        assert_eq!(t.state(), Activity::Active);

        let second = t.observe(Some(Duration::from_secs(310)), Duration::from_secs(20));
        assert_eq!(second, None, "20s of dwell is under the 30s requirement");

        // ...and it fires once the dwell is genuinely satisfied.
        let then = t.observe(Some(Duration::from_secs(320)), Duration::from_secs(15));
        assert_eq!(then, Some(IdleTransition::WentIdle));
        assert_eq!(t.state(), Activity::Idle);
    }

    #[test]
    fn returning_to_activity_resumes_after_its_own_dwell() {
        let mut t = IdleTracker::new(cfg());
        t.observe(Some(Duration::from_secs(400)), TICK);
        t.observe(Some(Duration::from_secs(400)), TICK).unwrap();
        assert_eq!(t.state(), Activity::Idle);

        let first = t.observe(Some(Duration::ZERO), Duration::from_secs(5));
        assert_eq!(first, None, "resume dwells too — a stray event should not resume");

        let second = t.observe(Some(Duration::ZERO), Duration::from_secs(10));
        assert_eq!(second, None, "still under the dwell");

        let then = t.observe(Some(Duration::ZERO), Duration::from_secs(30));
        assert_eq!(then, Some(IdleTransition::BecameActive));
        assert_eq!(t.state(), Activity::Active);
    }

    #[test]
    fn flapping_around_the_threshold_does_not_thrash_the_recorder() {
        // THE reason hysteresis exists: without it this alternation would emit a
        // transition on every reading and shred the day into tiny windows.
        let mut t = IdleTracker::new(cfg());
        let mut transitions = 0;
        for i in 0..40 {
            let idle = if i % 2 == 0 { 299 } else { 301 };
            if t
                .observe(Some(Duration::from_secs(idle)), Duration::from_secs(5))
                .is_some()
            {
                transitions += 1;
            }
        }
        assert_eq!(transitions, 0, "flapping must produce NO transitions, got {transitions}");
        assert_eq!(t.state(), Activity::Active);
    }

    #[test]
    fn a_brief_return_to_activity_cancels_a_pending_pause() {
        let mut t = IdleTracker::new(cfg());
        assert_eq!(t.observe(Some(Duration::from_secs(310)), Duration::from_secs(20)), None);
        // user touches the keyboard — pending pause must be abandoned
        assert_eq!(t.observe(Some(Duration::from_secs(1)), Duration::from_secs(5)), None);
        // ...and the dwell restarts rather than resuming where it left off
        assert_eq!(
            t.observe(Some(Duration::from_secs(310)), Duration::from_secs(20)),
            None,
            "20s of fresh dwell is still under 30s"
        );
        assert_eq!(t.state(), Activity::Active);
    }

    #[test]
    fn disabling_idle_detection_never_pauses() {
        // Named for what it actually tests. There is no "release a held pause"
        // branch, because that state is unreachable: `enabled` is fixed for a
        // tracker's lifetime and `state` only becomes Idle on the enabled path.
        let mut c = cfg();
        c.enabled = false;
        let mut t = IdleTracker::new(c);
        for _ in 0..50 {
            assert_eq!(t.observe(Some(Duration::from_secs(9_999)), TICK), None);
        }
        assert_eq!(t.state(), Activity::Active, "disabled must never pause");
    }

    #[test]
    fn hysteresis_holds_at_realistic_poll_rates_not_just_fast_ones() {
        // F4. Dayflow polls MINUTES apart, far longer than the 30s dwell. If the
        // dwell credited time spent under the opposite condition, every
        // transition would fire on its first observation and the debounce would
        // be a no-op in normal operation — while a fast-tick test still passed.
        let mut t = IdleTracker::new(cfg()); // 300s threshold, 30s dwell
        let slow = Duration::from_secs(180); // the day-simulation cadence

        // first sighting of idle must NOT fire, however long the gap before it
        assert_eq!(
            t.observe(Some(Duration::from_secs(400)), slow),
            None,
            "a 180s gap must not be credited toward the dwell"
        );
        assert_eq!(t.state(), Activity::Active);
        // the NEXT reading, still idle, satisfies the dwell
        assert_eq!(
            t.observe(Some(Duration::from_secs(600)), slow),
            Some(IdleTransition::WentIdle)
        );

        // and the same in the resume direction
        assert_eq!(t.observe(Some(Duration::ZERO), slow), None, "resume dwells too");
        assert_eq!(
            t.observe(Some(Duration::ZERO), slow),
            Some(IdleTransition::BecameActive)
        );
    }

    #[test]
    fn flapping_at_a_slow_cadence_also_produces_no_transitions() {
        // The original flap test used 5s ticks and could not see the bug above.
        let mut t = IdleTracker::new(cfg());
        let mut transitions = 0;
        for i in 0..40 {
            let idle = if i % 2 == 0 { 299 } else { 301 };
            if t.observe(Some(Duration::from_secs(idle)), Duration::from_secs(180)).is_some() {
                transitions += 1;
            }
        }
        assert_eq!(transitions, 0, "slow-cadence flapping must also be absorbed");
    }

    #[test]
    #[ignore = "live: requires a real X11 DISPLAY"]
    fn x11_backend_reports_a_monotonic_counter() {
        let d = X11IdleDetector::new().expect("an X display must be available for this probe");
        let a = d.idle_for().expect("counter must read");
        std::thread::sleep(Duration::from_millis(1200));
        let b = d.idle_for().expect("counter must read");
        assert!(b > a, "idle counter must advance: {a:?} -> {b:?}");
    }
}
