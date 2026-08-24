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
    /// When this run last started producing — start, or the most recent resume.
    producing_since: DateTime<Utc>,
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
            producing_since: now,
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
                // Only lift an AUTOMATIC pause. Resuming unconditionally means a
                // mouse movement silently un-does a deliberate off switch.
                if self.windows.resume_if_automatic(now) {
                    // Restart the staleness clock: a just-resumed recorder has
                    // not had time to produce, and must not read as a fault.
                    self.producing_since = now;
                }
                Vec::new()
            }
            None => Vec::new(),
        }
    }

    /// Turn capture OFF. The in-progress windows close and are accounted for.
    ///
    /// A no-op on a stopped run: recording a pause that can never close would
    /// leave a permanently open interval in a finished run's ledger.
    pub fn turn_off(&mut self, now: DateTime<Utc>) -> Vec<ClosedWindow> {
        if self.stopped {
            return Vec::new();
        }
        self.pause(PauseCause::UserOff, now)
    }

    /// Turn capture back ON.
    ///
    /// Rejoins THIS day's session (FR-033). Returns `Rejoined` when the same
    /// calendar day, and `Err` on a different one — a new day is a new session,
    /// and silently continuing yesterday's would put today's entries on the
    /// wrong day.
    pub fn turn_on(&mut self, now: DateTime<Utc>) -> Result<StartOutcome, DayflowError> {
        if self.stopped {
            return Err(DayflowError::Invalid(
                "this run has stopped; start a new one rather than resurrecting it".into(),
            ));
        }
        if now.date_naive() != self.day {
            return Err(DayflowError::Invalid(format!(
                "this run belongs to {}; {} is a different day and needs a new session",
                self.day,
                now.date_naive()
            )));
        }
        // Only restart the staleness clock if this call ACTUALLY lifted a pause.
        //
        // Resetting unconditionally makes `turn_on` a health-launderer: an
        // idempotent "ensure capture is on" caller would flip a genuinely
        // Degraded run back to Healthy on every invocation and mask a dead
        // sampler indefinitely. Turning on something already on is a no-op, and
        // a no-op must not change what the evidence says.
        if self.windows.pause_cause().is_some() {
            self.windows.resume(now);
            self.producing_since = now;
        }
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

    /// Pause intervals recorded by this run.
    pub fn pauses_seen(&self) -> &[crate::dayflow::window::PauseInterval] {
        self.windows.pauses()
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
        let capturing = if self.stopped || self.windows.pause_cause().is_some() {
            // S3: "currently capturing" must mean it. A paused or stopped run is
            // capturing nothing, whatever remains selected.
            0
        } else {
            u32::try_from(self.displays.len()).unwrap_or(u32::MAX)
        };
        DayflowLiveness::assess(crate::dayflow::models::LivenessInput {
            now,
            chunks_written: self.chunks_written,
            last_chunk_at: self.last_chunk_at,
            last_summary_at: self.last_summary_at,
            segment_seconds: u32::try_from(self.windows.interval().as_secs())
                .unwrap_or(u32::MAX),
            displays_active: capturing,
            paused_cause: self.windows.pause_cause(),
            stopped: self.stopped,
            producing_since: self.producing_since,
        })
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
        // Evidence of production is when a sample was last actually TAKEN, not
        // when the window happened to close.
        //
        // A window closed by a pause, an interval change or a stop ends at `now`
        // whatever the sampler was doing. Using `end_wall` here let a dead
        // sampler read Healthy the instant any of those fired — closing a stale
        // window is bookkeeping, not output.
        let Some(produced_at) = w.last_sample_at else {
            return; // a window containing no samples is not evidence of anything
        };
        self.last_chunk_at = Some(match self.last_chunk_at {
            Some(prev) if prev > produced_at => prev,
            _ => produced_at,
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

    /// Drive the tracker until the idle state settles, at a realistic cadence.
    fn settle_idle(r: &mut DayflowRun, idle_secs: u64, from: i64) -> Vec<ClosedWindow> {
        let d = std::time::Duration::from_secs(60);
        let mut out = Vec::new();
        for i in 0..4 {
            out.extend(r.tick_idle(
                Some(std::time::Duration::from_secs(idle_secs)),
                d,
                at(from + i * 60),
            ));
        }
        out
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

        let closed = settle_idle(&mut r, 400, 310);
        assert_eq!(closed.len(), 1, "the open window closes on pause");
        assert_eq!(r.liveness(at(550)).health, DayflowHealth::Paused);

        // no samples counted while paused
        assert!(r.on_sample(0, at(400)).is_none());

        // return to activity
        settle_idle(&mut r, 0, 560);
        assert_eq!(r.liveness(at(800)).health, DayflowHealth::Healthy);
        r.on_sample(0, at(900));
        let w = r.on_sample(0, at(1500)).expect("a fresh window after resume");
        assert_eq!(w.start_wall, at(900), "resume starts a new window, no splice");
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
    fn activity_must_not_resume_capture_after_a_deliberate_off() {
        // F1, ordering A: off first, then the user walks away and comes back.
        // Resuming on a mouse movement would override an explicit instruction to
        // stop recording.
        let mut r = run(vec![0]);
        r.on_sample(0, at(0));
        r.turn_off(at(100));
        assert_eq!(r.liveness(at(100)).health, DayflowHealth::Off);

        // go idle while off, then return to activity
        settle_idle(&mut r, 400, 300);
        settle_idle(&mut r, 0, 600);

        assert_eq!(
            r.liveness(at(900)).health,
            DayflowHealth::Off,
            "activity must NOT lift a deliberate off"
        );
        assert!(r.on_sample(0, at(700)).is_none(), "and no samples are taken");
    }

    #[test]
    fn turning_off_while_idle_paused_actually_takes_effect() {
        // F1, ordering B: idle-pause first, THEN off. Previously the pause call
        // returned early, the cause stayed Idle, and the next activity resumed —
        // the off did nothing at all.
        let mut r = run(vec![0]);
        r.on_sample(0, at(0));
        settle_idle(&mut r, 400, 300);
        assert_eq!(r.liveness(at(550)).health, DayflowHealth::Paused);

        r.turn_off(at(600));
        assert_eq!(
            r.liveness(at(600)).health,
            DayflowHealth::Off,
            "the off must override the idle pause, not be swallowed by it"
        );
        // The DURABLE record must agree with the live state. Updating only one
        // of them leaves the ledger saying Idle while liveness says Off — and a
        // gap's recorded cause is what a later reader has to trust.
        assert_eq!(
            r.pauses_seen().last().expect("a pause was recorded").cause,
            crate::dayflow::window::PauseCause::UserOff,
            "the recorded pause interval must be upgraded too, not just live state"
        );

        // activity now must NOT resume it
        settle_idle(&mut r, 0, 700);
        assert_eq!(r.liveness(at(1000)).health, DayflowHealth::Off);
    }

    #[test]
    fn turning_on_again_after_a_deliberate_off_works() {
        // The guard must block ACTIVITY, not the user.
        let mut r = run(vec![0]);
        r.on_sample(0, at(0));
        r.turn_off(at(100));
        assert_eq!(r.turn_on(at(200)).unwrap(), StartOutcome::Rejoined);
        assert_eq!(r.liveness(at(200)).health, DayflowHealth::Healthy);
        r.on_sample(0, at(200));
        assert!(r.on_sample(0, at(800)).is_some(), "capture works again");
    }

    #[test]
    fn turning_on_an_already_running_run_cannot_launder_a_degraded_state() {
        // The mutation-surviving hole: turn_on unconditionally reset the
        // staleness clock, so an idempotent "ensure capture is on" caller would
        // flip a genuinely Degraded run back to Healthy on EVERY call and mask a
        // dead sampler forever.
        let mut r = run(vec![0]);
        r.on_sample(0, at(0));
        r.on_sample(0, at(600)); // one window closes

        // sampler dies; silence past two intervals
        assert_eq!(r.liveness(at(3_000)).health, DayflowHealth::Degraded);

        // "ensure it's on" — must NOT change the evidence
        assert_eq!(r.turn_on(at(3_000)).unwrap(), StartOutcome::Rejoined);
        assert_eq!(
            r.liveness(at(3_000)).health,
            DayflowHealth::Degraded,
            "turning on something already on is a no-op; it must not launder health"
        );
        // ...and repeating it must not help either
        for t in [3_100, 3_200, 3_300] {
            r.turn_on(at(t)).unwrap();
            assert_eq!(
                r.liveness(at(t)).health,
                DayflowHealth::Degraded,
                "repeated ensure-on must not mask a dead sampler"
            );
        }
    }

    #[test]
    fn closing_a_stale_window_is_not_evidence_of_production() {
        // The root cause behind the laundering family: an interval change (or a
        // pause, or a stop) closes whatever window is open with end_wall = now.
        // Treating that as output let a DEAD sampler read Healthy the instant
        // any of them fired.
        let mut r = run(vec![0]);
        r.on_sample(0, at(0));
        r.on_sample(0, at(60)); // ...then the sampler dies

        assert_eq!(r.liveness(at(3_000)).health, DayflowHealth::Degraded);

        // A config change closes the stale window — bookkeeping, not production.
        // Note the new interval must not WIDEN the staleness tolerance, or the
        // run is legitimately healthy for a longer silence and the test would be
        // measuring the wrong thing. 300s ⇒ stale after 600s.
        r.set_interval(std::time::Duration::from_secs(300), at(3_000));
        assert_eq!(
            r.liveness(at(3_000)).health,
            DayflowHealth::Degraded,
            "closing a stale window must NOT resurrect a dead sampler"
        );
        assert_eq!(
            r.liveness(at(3_000)).last_chunk_at,
            Some(at(60)),
            "the evidence timestamp is the last SAMPLE, not the window close"
        );
    }

    #[test]
    fn turning_on_after_a_long_deliberate_off_reads_healthy() {
        // Guards the clock-reset half that survived mutation: without it a user
        // turning capture back on after a long off instantly reads Degraded,
        // violating FR-032.
        let mut r = run(vec![0]);
        r.on_sample(0, at(0));
        r.on_sample(0, at(600));
        r.turn_off(at(700));
        assert_eq!(r.liveness(at(700)).health, DayflowHealth::Off);

        r.turn_on(at(20_000)).unwrap();
        assert_eq!(
            r.liveness(at(20_000)).health,
            DayflowHealth::Healthy,
            "turning it back on must give it time to produce, not fault instantly"
        );
        // ...and it still degrades if it then produces nothing
        assert_eq!(r.liveness(at(22_000)).health, DayflowHealth::Degraded);
    }

    #[test]
    fn stopping_a_paused_run_leaves_no_open_gap_in_its_ledger() {
        let mut r = run(vec![0]);
        r.on_sample(0, at(0));
        settle_idle(&mut r, 400, 300);
        assert!(r.pauses_seen().last().unwrap().to.is_none(), "open while paused");
        r.stop(at(2_000));
        assert!(
            r.pauses_seen().iter().all(|p| p.to.is_some()),
            "a finished run must not carry a pause that never closes"
        );
    }

    #[test]
    fn turning_off_a_stopped_run_records_no_open_pause() {
        let mut r = run(vec![0]);
        r.on_sample(0, at(0));
        r.stop(at(100));
        let before = r.pauses_seen().len();
        r.turn_off(at(200));
        assert_eq!(
            r.pauses_seen().len(),
            before,
            "a stopped run must not gain a pause interval that can never close"
        );
    }

    #[test]
    fn unplugging_a_display_decrements_what_is_capturing() {
        // Coverage the day-sim lost when S3 changed its stopped-state assertion.
        let mut r = run(vec![0, 1]);
        r.on_sample(0, at(0));
        r.on_sample(1, at(0));
        assert_eq!(r.liveness(at(0)).displays_active, 2);
        r.remove_display(1, at(120));
        assert_eq!(
            r.liveness(at(120)).displays_active,
            1,
            "while STILL RUNNING, an unplug must decrement the count"
        );
    }

    #[test]
    fn a_stopped_run_cannot_be_resurrected() {
        let mut r = run(vec![0]);
        r.on_sample(0, at(0));
        r.stop(at(100));
        let err = r.turn_on(at(200)).unwrap_err();
        assert!(format!("{err}").contains("stopped"), "got: {err}");
        assert_eq!(r.liveness(at(200)).health, DayflowHealth::Stopped);
    }

    #[test]
    fn a_fresh_run_is_not_reported_as_a_fault() {
        // F2: before this, a brand-new run read Degraded until its first window.
        let r = run(vec![0]);
        let l = r.liveness(at(0));
        assert_eq!(l.health, DayflowHealth::Healthy);
        assert_eq!(l.chunks_written, 0);
        assert!(!l.health.is_fault(), "a run that just started is not broken");
    }

    #[test]
    fn resuming_from_a_long_pause_is_not_reported_as_a_fault() {
        // F2: last_chunk_at is hours stale after a long idle stretch. Measuring
        // from it alone reports Degraded the instant capture resumes (FR-032).
        let mut r = run(vec![0]);
        r.on_sample(0, at(0));
        settle_idle(&mut r, 400, 300);

        // ...hours pass...
        settle_idle(&mut r, 0, 20_000);

        let l = r.liveness(at(20_300));
        assert_eq!(l.health, DayflowHealth::Healthy, "just resumed — give it time to produce");
        assert!(l.last_chunk_at.unwrap() < at(1_000), "its history really is stale");
    }

    #[test]
    fn displays_active_reports_what_is_actually_capturing() {
        // S3: "currently capturing" must mean it.
        let mut r = run(vec![0, 1]);
        r.on_sample(0, at(0));
        assert_eq!(r.liveness(at(0)).displays_active, 2);
        r.turn_off(at(100));
        assert_eq!(r.liveness(at(100)).displays_active, 0, "off ⇒ capturing nothing");
        r.turn_on(at(200)).unwrap();
        assert_eq!(r.liveness(at(200)).displays_active, 2);
        r.stop(at(300));
        assert_eq!(r.liveness(at(300)).displays_active, 0, "stopped ⇒ capturing nothing");
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
        // F6: assert the SCENARIO actually happened, not just that invariants
        // held. Without these the test passes even if tick_idle, set_interval
        // and remove_display were all no-ops — which is exactly the interaction
        // coverage it is supposed to provide.
        assert!(
            !r.pauses_seen().is_empty(),
            "the mid-morning idle stretch must have produced a recorded pause"
        );
        assert!(
            windows.iter().any(|w| w.reason == crate::dayflow::window::CloseReason::Paused),
            "a window must have closed BECAUSE of the pause"
        );
        assert!(
            windows.iter().any(|w| w.duration().num_seconds() > 600),
            "after the interval change windows must exceed the original 600s"
        );
        let last_d1 = windows
            .iter()
            .filter(|w| w.display_id == 1)
            .map(|w| w.end_wall)
            .max()
            .expect("display 1 produced windows before being unplugged");
        assert!(
            last_d1 <= at(120 * 180),
            "display 1 must produce NOTHING after it was unplugged at step 120"
        );
        assert!(
            windows.iter().any(|w| w.display_id == 0 && w.end_wall > at(120 * 180)),
            "display 0 must keep going after display 1 was removed"
        );

        // and the evidence agrees with what was produced
        let l = r.liveness(at(160 * 180));
        assert_eq!(l.chunks_written as usize, windows.len());
        assert_eq!(l.health, DayflowHealth::Stopped);
        assert_eq!(l.displays_active, 0, "a STOPPED run captures nothing (S3)");
    }
}
