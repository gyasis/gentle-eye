//! Dayflow run lifecycle: displays, idle, off/on, interval, liveness.
//!
//! A [`DayflowRun`] ties the pieces together — the [`Sampler`] that decides what
//! is worth keeping, the [`WindowController`] that decides when a window ends,
//! and the [`IdleTracker`] that decides when to stop entirely — and exposes the
//! result as [`DayflowLiveness`].
//!
//! It owns no threads and no clock. Every entry point takes `now`, so a whole
//! day can be simulated deterministically in a test: an eight-hour run with
//! pauses, an interval change and a display unplug costs microseconds and yields
//! exactly the same decisions it would make live.

use chrono::{DateTime, NaiveDate, Utc};
use uuid::Uuid;

use crate::config::{DayflowConfig, DayflowIntent};
use crate::dayflow::errors::DayflowError;
use crate::dayflow::idle::{IdleTracker, IdleTransition};
use crate::dayflow::models::{DayflowLiveness, DayflowMode};
use crate::dayflow::window::{ClosedWindow, PauseCause, WindowController};

/// What happened when capture was asked to start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartOutcome {
    /// A new session began.
    Started,
    /// Capture was turned back on and REJOINED the same day's session.
    Rejoined,
}

/// One dayflow run.
#[derive(Debug)]
pub struct DayflowRun {
    session_id: Uuid,
    day: NaiveDate,
    mode: DayflowMode,
    intent: DayflowIntent,
    displays: Vec<u32>,
    windows: WindowController,
    idle: IdleTracker,
    stopped: bool,
    /// Windows closed so far — the liveness evidence, counted as they close.
    chunks_written: u64,
    last_chunk_at: Option<DateTime<Utc>>,
    last_summary_at: Option<DateTime<Utc>>,
}

impl DayflowRun {
    /// Begin a run over `displays` (already resolved from a
    /// [`crate::config::DisplaySelection`]).
    ///
    /// Fails on an empty display set: a run that captures nothing would report
    /// healthy while recording an empty day, which is the exact false-green this
    /// feature exists to prevent.
    pub fn start(
        cfg: &DayflowConfig,
        mode: DayflowMode,
        displays: Vec<u32>,
        now: DateTime<Utc>,
    ) -> Result<Self, DayflowError> {
        cfg.validate().map_err(|e| DayflowError::Invalid(e.to_string()))?;
        if displays.is_empty() {
            return Err(DayflowError::Invalid(
                "no displays selected — a run capturing nothing would report healthy \
                 while recording an empty day"
                    .into(),
            ));
        }
        Ok(Self {
            session_id: Uuid::new_v4(),
            day: now.date_naive(),
            mode,
            intent: cfg.intent,
            displays,
            windows: WindowController::new(cfg.segment_duration()),
            idle: IdleTracker::new(cfg.idle.clone()),
            stopped: false,
            chunks_written: 0,
            last_chunk_at: None,
            last_summary_at: None,
        })
    }

    /// This run's session id.
    pub fn session_id(&self) -> Uuid {
        self.session_id
    }

    /// The calendar day this run belongs to.
    pub fn day(&self) -> NaiveDate {
        self.day
    }

    /// Session vs daemon.
    pub fn mode(&self) -> DayflowMode {
        self.mode
    }

    /// What this run is for.
    pub fn intent(&self) -> DayflowIntent {
        self.intent
    }

    /// Displays currently being captured.
    pub fn displays(&self) -> &[u32] {
        &self.displays
    }

    /// The sampling interval a caller should use for this run's mode.
    pub fn sampling_interval(&self, cfg: &DayflowConfig) -> std::time::Duration {
        cfg.sampling.interval_for(self.mode)
    }

    /// Record that a sample was taken on `display_id`.
    ///
    /// Returns any window that closed as a result. Windows are counted here, so
    /// the liveness evidence advances only when something was actually produced.
    pub fn on_sample(&mut self, display_id: u32, now: DateTime<Utc>) -> Option<ClosedWindow> {
        if self.stopped || !self.displays.contains(&display_id) {
            return None;
        }
        let closed = self.windows.on_sample(display_id, now);
        if let Some(w) = &closed {
            self.note_closed(w);
        }
        closed
    }

    /// Feed the idle detector, applying pause or resume if it transitions.
    ///
    /// `idle_for` is the raw reading (`None` = could not tell, treated as
    /// active); `since_last` is the time since the previous tick.
    pub fn tick_idle(
        &mut self,
        idle_for: Option<std::time::Duration>,
        since_last: std::time::Duration,
        now: DateTime<Utc>,
    ) -> Vec<ClosedWindow> {
        if self.stopped {
            return Vec::new();
        }
        match self.idle.observe(idle_for, since_last) {
            Some(IdleTransition::WentIdle) => self.pause(PauseCause::Idle, now),
            Some(IdleTransition::BecameActive) => {
                self.windows.resume(now);
                Vec::new()
            }
            None => Vec::new(),
        }
    }

    /// Turn capture OFF. The in-progress windows close and are accounted for.
    pub fn turn_off(&mut self, now: DateTime<Utc>) -> Vec<ClosedWindow> {
        self.pause(PauseCause::UserOff, now)
    }

    /// Turn capture back ON.
    ///
    /// Rejoins THIS day's session (FR-033). Returns `Rejoined` when the same
    /// calendar day, and `Err` on a different one — a new day is a new session,
    /// and silently continuing yesterday's would put today's entries on the
    /// wrong day.
    pub fn turn_on(&mut self, now: DateTime<Utc>) -> Result<StartOutcome, DayflowError> {
        if now.date_naive() != self.day {
            return Err(DayflowError::Invalid(format!(
                "this run belongs to {}; {} is a different day and needs a new session",
                self.day,
                now.date_naive()
            )));
        }
        self.windows.resume(now);
        self.stopped = false;
        Ok(StartOutcome::Rejoined)
    }

    /// Change the segment interval, effective from the next window (FR-035).
    pub fn set_interval(
        &mut self,
        interval: std::time::Duration,
        now: DateTime<Utc>,
    ) -> Vec<ClosedWindow> {
        let closed = self.windows.set_interval(interval, now);
        for w in &closed {
            self.note_closed(w);
        }
        closed
    }

    /// Stop adding a display to the run (unplugged), closing its window.
    pub fn remove_display(&mut self, display_id: u32, now: DateTime<Utc>) -> Option<ClosedWindow> {
        self.displays.retain(|d| *d != display_id);
        let closed = self.windows.remove_display(display_id, now);
        if let Some(w) = &closed {
            self.note_closed(w);
        }
        closed
    }

    /// Note that a window was summarised, advancing that evidence.
    pub fn note_summarized(&mut self, at: DateTime<Utc>) {
        self.last_summary_at = Some(at);
    }

    /// Stop the run, closing every open window (FR-005).
    pub fn stop(&mut self, now: DateTime<Utc>) -> Vec<ClosedWindow> {
        let closed = self.windows.stop(now);
        for w in &closed {
            self.note_closed(w);
        }
        self.stopped = true;
        closed
    }

    /// Current liveness, derived from what this run has PRODUCED.
    pub fn liveness(&self, now: DateTime<Utc>) -> DayflowLiveness {
        DayflowLiveness::assess(
            now,
            self.chunks_written,
            self.last_chunk_at,
            self.last_summary_at,
            u32::try_from(self.windows.interval().as_secs()).unwrap_or(u32::MAX),
            u32::try_from(self.displays.len()).unwrap_or(u32::MAX),
            self.windows.pause_cause(),
            self.stopped,
        )
    }

    fn pause(&mut self, cause: PauseCause, now: DateTime<Utc>) -> Vec<ClosedWindow> {
        let closed = self.windows.pause(cause, now);
        for w in &closed {
            self.note_closed(w);
        }
        closed
    }

    fn note_closed(&mut self, w: &ClosedWindow) {
        self.chunks_written += 1;
        self.last_chunk_at = Some(match self.last_chunk_at {
            Some(prev) if prev > w.end_wall => prev,
            _ => w.end_wall,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DisplaySelection;
    use crate::dayflow::models::DayflowHealth;

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_787_500_000 + secs, 0).unwrap()
    }

    fn cfg() -> DayflowConfig {
        let mut c = DayflowConfig::default();
        c.segment_seconds = 600; // 10-minute windows
        c
    }

    fn run(displays: Vec<u32>) -> DayflowRun {
        DayflowRun::start(&cfg(), DayflowMode::Daemon, displays, at(0)).unwrap()
    }

    #[test]
    fn a_run_capturing_no_displays_is_refused() {
        // It would report healthy while recording an empty day.
        let err = DayflowRun::start(&cfg(), DayflowMode::Daemon, vec![], at(0)).unwrap_err();
        assert!(format!("{err}").contains("no displays"), "got: {err}");
    }

    #[test]
    fn an_invalid_interval_is_refused_at_start_not_discovered_later() {
        let mut c = cfg();
        c.segment_seconds = 30; // below the 5-minute floor
        assert!(DayflowRun::start(&c, DayflowMode::Daemon, vec![0], at(0)).is_err());
    }

    #[test]
    fn every_selected_display_gets_its_own_pipeline() {
        // T012: three displays, three independent window streams.
        let mut r = run(vec![0, 1, 2]);
        for d in [0, 1, 2] {
            r.on_sample(d, at(0));
        }
        let mut closed = Vec::new();
        for d in [0, 1, 2] {
            closed.extend(r.on_sample(d, at(600)));
        }
        assert_eq!(closed.len(), 3, "each display closes its own window");
        let mut ds: Vec<u32> = closed.iter().map(|w| w.display_id).collect();
        ds.sort_unstable();
        assert_eq!(ds, vec![0, 1, 2]);
        assert_eq!(r.liveness(at(600)).displays_active, 3);
    }

    #[test]
    fn a_sample_from_an_unselected_display_is_ignored() {
        // A focused session on the portrait screen must not silently record the
        // others.
        let mut r = run(vec![1]);
        assert!(r.on_sample(0, at(0)).is_none());
        r.on_sample(1, at(0));
        assert!(r.on_sample(0, at(600)).is_none(), "display 0 is not in this run");
        assert!(r.on_sample(1, at(600)).is_some());
    }

    #[test]
    fn display_selection_resolves_into_a_run() {
        // The identity selector feeding the engine, end to end.
        use crate::capture::display::DisplayInfo;
        let desk = vec![
            DisplayInfo::new(0, 1920, 1080, true),
            DisplayInfo::new(1, 1080, 2560, false),
            DisplayInfo::new(2, 3440, 1440, false),
        ];
        let picked = DisplaySelection::Named(vec!["portrait".into()])
            .resolve(&desk)
            .expect("portrait must resolve");
        let r = DayflowRun::start(&cfg(), DayflowMode::Session, picked, at(0)).unwrap();
        assert_eq!(r.displays(), &[1], "only the portrait panel is captured");
    }

    #[test]
    fn going_idle_pauses_and_returning_resumes() {
        // T014: the detector wired to the window controller.
        let mut r = run(vec![0]);
        r.on_sample(0, at(0));

        // idle past 300s, dwelling past 30s
        r.tick_idle(Some(std::time::Duration::from_secs(310)), std::time::Duration::from_secs(20), at(310));
        let closed = r.tick_idle(
            Some(std::time::Duration::from_secs(330)),
            std::time::Duration::from_secs(20),
            at(330),
        );
        assert_eq!(closed.len(), 1, "the open window closes on pause");
        assert_eq!(r.liveness(at(330)).health, DayflowHealth::Paused);

        // no samples counted while paused
        assert!(r.on_sample(0, at(400)).is_none());

        // return to activity
        r.tick_idle(Some(std::time::Duration::ZERO), std::time::Duration::from_secs(40), at(500));
        assert_eq!(r.liveness(at(500)).health, DayflowHealth::Healthy);
        r.on_sample(0, at(500));
        let w = r.on_sample(0, at(1100)).expect("a fresh window after resume");
        assert_eq!(w.start_wall, at(500), "resume starts a new window, no splice");
    }

    #[test]
    fn an_unreadable_idle_signal_never_pauses_a_run() {
        let mut r = run(vec![0]);
        r.on_sample(0, at(0));
        for i in 1..50 {
            let closed = r.tick_idle(None, std::time::Duration::from_secs(60), at(i * 60));
            assert!(closed.is_empty(), "unknown idle must never pause");
        }
        assert_ne!(r.liveness(at(3000)).health, DayflowHealth::Paused);
    }

    #[test]
    fn turning_off_and_on_the_same_day_rejoins_the_same_session() {
        // T015 / FR-033.
        let mut r = run(vec![0]);
        let id = r.session_id();
        r.on_sample(0, at(0));

        let closed = r.turn_off(at(300));
        assert_eq!(closed.len(), 1, "the in-progress window is accounted for, not dropped");
        assert_eq!(r.liveness(at(300)).health, DayflowHealth::Off);

        assert_eq!(r.turn_on(at(4000)).unwrap(), StartOutcome::Rejoined);
        assert_eq!(r.session_id(), id, "same day ⇒ same session");
        assert_eq!(r.day(), at(0).date_naive());
    }

    #[test]
    fn turning_on_during_a_different_day_is_refused() {
        // Silently continuing yesterday's session would file today's entries
        // under the wrong day.
        let mut r = run(vec![0]);
        r.turn_off(at(100));
        let tomorrow = at(0) + chrono::Duration::days(1);
        let err = r.turn_on(tomorrow).unwrap_err();
        assert!(format!("{err}").contains("different day"), "got: {err}");
    }

    #[test]
    fn an_off_interval_reads_as_off_not_as_degraded() {
        // Identical silence to a fault; only the cause differs (FR-032).
        let mut r = run(vec![0]);
        r.on_sample(0, at(0));
        r.turn_off(at(100));
        let l = r.liveness(at(100_000));
        assert_eq!(l.health, DayflowHealth::Off);
        assert!(!l.health.is_fault(), "a deliberate off switch is not a fault");
    }

    #[test]
    fn changing_the_interval_takes_effect_at_the_next_window() {
        // T016 / FR-035.
        let mut r = run(vec![0]);
        r.on_sample(0, at(0));
        let first = r.on_sample(0, at(600)).expect("closes at the 10-minute boundary");
        assert_eq!(first.duration().num_seconds(), 600);

        r.on_sample(0, at(600));
        let truncated = r.set_interval(std::time::Duration::from_secs(1800), at(700));
        assert_eq!(truncated.len(), 1);
        assert_eq!(truncated[0].duration().num_seconds(), 100, "closes short, honestly");

        r.on_sample(0, at(700));
        assert!(r.on_sample(0, at(2000)).is_none(), "1300s < the new 1800s");
        let longer = r.on_sample(0, at(2500)).expect("closes at the new boundary");
        assert_eq!(longer.duration().num_seconds(), 1800);
        assert_eq!(
            r.liveness(at(2500)).segment_seconds,
            1800,
            "liveness reports the interval actually in force"
        );
    }

    #[test]
    fn liveness_counts_only_windows_that_actually_closed() {
        // The evidence must track production, not intent.
        let mut r = run(vec![0]);
        assert_eq!(r.liveness(at(0)).chunks_written, 0);
        r.on_sample(0, at(0));
        assert_eq!(r.liveness(at(60)).chunks_written, 0, "an OPEN window is not evidence");
        r.on_sample(0, at(600));
        assert_eq!(r.liveness(at(600)).chunks_written, 1, "only on close");
    }

    #[test]
    fn a_full_simulated_working_day_stays_coherent() {
        // Eight hours with idle pauses, an interval change and an unplug. The
        // point is that the pieces do not disagree with each other.
        let mut r = run(vec![0, 1]);
        let tick = std::time::Duration::from_secs(180);
        let mut windows: Vec<ClosedWindow> = Vec::new();

        for step in 0..160 {
            let t = at(step * 180);
            // idle for one stretch mid-morning
            let idle = if (40..50).contains(&step) {
                Some(std::time::Duration::from_secs(600))
            } else {
                Some(std::time::Duration::ZERO)
            };
            windows.extend(r.tick_idle(idle, tick, t));
            for d in r.displays().to_vec() {
                windows.extend(r.on_sample(d, t));
            }
            if step == 80 {
                windows.extend(r.set_interval(std::time::Duration::from_secs(1800), t));
            }
            if step == 120 {
                windows.extend(r.remove_display(1, t));
            }
        }
        windows.extend(r.stop(at(160 * 180)));

        assert!(!windows.is_empty(), "a day must produce windows");
        // no window may claim a negative or zero span
        assert!(
            windows.iter().all(|w| w.end_wall > w.start_wall),
            "every window must have a positive duration"
        );
        // windows on one display must not overlap
        for d in [0u32, 1] {
            let mut ws: Vec<&ClosedWindow> =
                windows.iter().filter(|w| w.display_id == d).collect();
            ws.sort_by_key(|w| w.start_wall);
            for pair in ws.windows(2) {
                assert!(
                    pair[0].end_wall <= pair[1].start_wall,
                    "display {d}: windows overlap: {:?} then {:?}",
                    pair[0],
                    pair[1]
                );
            }
        }
        // durations are NOT uniform — the whole reason nothing may compute them
        let mut durs: Vec<i64> = windows.iter().map(|w| w.duration().num_seconds()).collect();
        durs.sort_unstable();
        durs.dedup();
        assert!(durs.len() > 1, "a real day yields varied window lengths, got {durs:?}");
        // and the evidence agrees with what was produced
        let l = r.liveness(at(160 * 180));
        assert_eq!(l.chunks_written as usize, windows.len());
        assert_eq!(l.health, DayflowHealth::Stopped);
        assert_eq!(l.displays_active, 1, "display 1 was unplugged");
    }
}
