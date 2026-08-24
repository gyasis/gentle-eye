//! Data models for dayflow mode.
//!
//! Dayflow continuously records the screen at low fps, splits the recording into
//! 15-minute chunks, summarizes each chunk with a vision provider (Map-Reduce with
//! rolling context), and stores the result as a queryable activity timeline.
//!
//! These types are `serde`-only (matching the rest of `models`/`contracts`); the
//! MCP layer (Wave 7) carries its own `JsonSchema` DTOs with String ids.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A coarse activity classification for a timeline entry.
///
/// Context-aware (what the user was *doing*), not app-name logging —
/// "researching on YouTube" is `Browsing`, not "Chrome".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ActivityCategory {
    /// Writing or editing code.
    Coding,
    /// Reading or writing documentation / prose.
    Docs,
    /// Email, chat, calls.
    Comms,
    /// Web browsing / research.
    Browsing,
    /// In a meeting / call.
    Meeting,
    /// No meaningful activity.
    Idle,
    /// Anything that doesn't fit the above.
    #[default]
    Other,
}

/// One recording mode for a dayflow session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DayflowMode {
    /// Explicit start/stop session with an optional max duration.
    #[default]
    Session,
    /// Long-lived continuous daemon, auto-rolling segments across the day.
    Daemon,
}

/// Lifecycle status of a dayflow session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DayflowStatus {
    /// Not recording.
    #[default]
    Idle,
    /// Actively capturing chunks.
    Recording,
    /// Capture stopped; summarization of remaining chunks in progress.
    Summarizing,
    /// Session finished, timeline complete.
    Stopped,
    /// Session ended in error.
    Error,
}

/// A running (or finished) dayflow session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DayflowSession {
    /// Unique session id.
    pub id: Uuid,
    /// The underlying recording this session drives.
    pub recording_id: Uuid,
    /// When the session started.
    pub started_at: DateTime<Utc>,
    /// When the session ended (None while running).
    pub ended_at: Option<DateTime<Utc>>,
    /// Session vs daemon.
    pub mode: DayflowMode,
    /// Current lifecycle status.
    pub status: DayflowStatus,
}

/// Why a recorder is not producing, when it is not.
///
/// The distinction this enum exists to preserve: **paused**, **off** and
/// **degraded** are three different things that look identical from the outside
/// (no new windows). Collapsing them is what makes a liveness signal useless —
/// an operator who cannot tell a deliberate pause from a fault learns to ignore
/// both, and then a whole day is lost before anyone notices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DayflowHealth {
    /// Running and producing.
    Healthy,
    /// Deliberately paused — idle, locked, or the display asleep. NOT a fault
    /// (FR-032).
    Paused,
    /// The user turned capture off.
    Off,
    /// Running, not paused, and producing NOTHING. This is the fault case.
    Degraded,
    /// The session ended.
    Stopped,
}

impl DayflowHealth {
    /// Whether this state means something is WRONG, as opposed to merely quiet.
    ///
    /// Only [`Degraded`](Self::Degraded) is a fault. A pause and an off switch
    /// are quiet on purpose.
    pub fn is_fault(self) -> bool {
        matches!(self, Self::Degraded)
    }

    /// Whether capture is expected to be producing windows right now.
    pub fn expects_output(self) -> bool {
        matches!(self, Self::Healthy | Self::Degraded)
    }
}

/// Evidence that a recorder is alive, derived from what it PRODUCED.
///
/// Every field here comes from an artifact another process wrote — the segment
/// ledger and the timeline table — never from a boolean the daemon keeps about
/// itself. A daemon asked "are you healthy?" will always say yes; the ledger
/// cannot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DayflowLiveness {
    /// Windows closed and recorded so far.
    pub chunks_written: u64,
    /// End of the most recent recorded window.
    pub last_chunk_at: Option<DateTime<Utc>>,
    /// Timestamp of the most recent timeline entry.
    pub last_summary_at: Option<DateTime<Utc>>,
    /// The interval in force — the unit the staleness window is measured in.
    pub segment_seconds: u32,
    /// How many display pipelines are currently capturing.
    pub displays_active: u32,
    /// The derived state.
    pub health: DayflowHealth,
}

impl DayflowLiveness {
    /// How many segment intervals of silence before a running recorder counts as
    /// degraded (SC-006). Two, so one slow window does not raise a false alarm.
    pub const STALE_INTERVALS: u32 = 2;

    /// Derive health from produced artifacts plus the declared lifecycle.
    ///
    /// `paused_cause` and `stopped` describe INTENT; everything else is
    /// evidence. Intent wins only for explaining silence that was asked for —
    /// it can never make a silent recorder look healthy.
    pub fn assess(
        now: DateTime<Utc>,
        chunks_written: u64,
        last_chunk_at: Option<DateTime<Utc>>,
        last_summary_at: Option<DateTime<Utc>>,
        segment_seconds: u32,
        displays_active: u32,
        paused_cause: Option<crate::dayflow::window::PauseCause>,
        stopped: bool,
    ) -> Self {
        use crate::dayflow::window::PauseCause;
        let health = if stopped {
            DayflowHealth::Stopped
        } else {
            match paused_cause {
                Some(PauseCause::UserOff) => DayflowHealth::Off,
                Some(_) => DayflowHealth::Paused,
                None => {
                    let stale_after = i64::from(segment_seconds) * i64::from(Self::STALE_INTERVALS);
                    match last_chunk_at {
                        // Running and producing recently.
                        Some(t) if (now - t).num_seconds() < stale_after => DayflowHealth::Healthy,
                        // Running, nothing recent — the fault case.
                        Some(_) => DayflowHealth::Degraded,
                        // Running and has NEVER produced. Degraded once enough
                        // time has passed that it should have.
                        None => DayflowHealth::Degraded,
                    }
                }
            }
        };
        Self {
            chunks_written,
            last_chunk_at,
            last_summary_at,
            segment_seconds,
            displays_active,
            health,
        }
    }

    /// How long since the last window closed, if ever.
    pub fn silence(&self, now: DateTime<Utc>) -> Option<chrono::Duration> {
        self.last_chunk_at.map(|t| now - t)
    }
}

/// A reference to one on-the-fly recording segment (15-min chunk by default).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkRef {
    /// Monotonic index within the session (0-based).
    pub index: usize,
    /// Path to the chunk video file.
    pub path: std::path::PathBuf,
    /// Wall-clock start of this chunk.
    pub start_wall: DateTime<Utc>,
    /// Wall-clock end of this chunk.
    pub end_wall: DateTime<Utc>,
    /// Which display produced it (FR-029), carried forward onto every entry
    /// derived from this window so a merged timeline stays attributable.
    #[serde(default)]
    pub display_id: u32,
    /// Monotonic WITHIN the session, across sampler restarts.
    ///
    /// [`ChunkRef::index`] is a per-run counter that resets to 0 on every pause,
    /// resume, interval change and display change, so it is not a stable
    /// identity. The durable identity of a window is
    /// `(session_id, display_id, sequence)` — matching the `dayflow_segments`
    /// primary key, never the filename and never `index`.
    #[serde(default)]
    pub sequence: u64,
    /// Whether this window has been summarised.
    ///
    /// The eviction guard (FR-025) reads this: a window that failed
    /// summarisation because a backend was unreachable must be RETRIED, never
    /// reclaimed, or a backend outage becomes silent data loss.
    #[serde(default)]
    pub summarized: bool,
}

/// The structured summary of a single chunk produced by the Map step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkSummary {
    /// Index of the chunk this summarizes.
    pub chunk_index: usize,
    /// Wall-clock start of the chunk.
    pub start_time: DateTime<Utc>,
    /// Wall-clock end of the chunk.
    pub end_time: DateTime<Utc>,
    /// Activity classification.
    pub category: ActivityCategory,
    /// Primary application / surface in focus.
    pub app: String,
    /// Short activity label ("researching gentle-eye retention").
    pub activity: String,
    /// Longer free-text detail.
    pub detail: String,
}

/// The rolling context threaded between chunks in the Map-Reduce summarizer
/// (videolocr's `CONTEXT SUMMARY FOR NEXT CHUNK`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RollingContext {
    /// A compact summary of everything seen up to (but not including) the next chunk.
    pub summary: String,
}

impl RollingContext {
    /// Whether any prior context has accumulated.
    pub fn is_empty(&self) -> bool {
        self.summary.is_empty()
    }
}

/// A single entry in the queryable activity timeline (persisted in SQLite,
/// Wave 4). Column-aligned with the `timeline_entries` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineEntry {
    /// Unique entry id (primary key).
    pub id: Uuid,
    /// The recording / session this entry belongs to.
    pub recording_id: Uuid,
    /// Start of the time range this entry covers.
    pub start_time: DateTime<Utc>,
    /// End of the time range this entry covers.
    pub end_time: DateTime<Utc>,
    /// Activity classification.
    pub category: ActivityCategory,
    /// Primary application / surface.
    pub app: String,
    /// Short activity label.
    pub activity: String,
    /// Human-readable summary of the activity in this range.
    pub summary: String,
}

#[cfg(test)]
mod chunk_identity_tests {
    use super::*;

    #[test]
    fn chunkref_still_loads_data_written_before_these_fields_existed() {
        // Real: deleting any `serde(default)` fails this.
        let old = r#"{"index":2,"path":"/tmp/c.mp4",
            "start_wall":"2026-08-24T09:00:00Z","end_wall":"2026-08-24T09:15:00Z"}"#;
        let c: ChunkRef = serde_json::from_str(old).expect("old manifest must still parse");
        assert_eq!(c.index, 2);
        assert_eq!(c.display_id, 0);
        assert_eq!(c.sequence, 0);
        assert!(!c.summarized);
    }

    // NOTE: `sequence` being the durable identity across a sampler restart is a
    // property of the SAMPLER (T010), which does not exist yet. Tests asserting
    // it on hand-built structs would only compare literals the test itself
    // supplied — they would pass against any implementation, including a broken
    // one — so they are deliberately not written here. The behaviour is covered
    // by T010's `> DONE:` criterion, and `plan_chunks` below is tested where it
    // actually assigns the field.
}

#[cfg(test)]
mod liveness_tests {
    use super::*;
    use crate::dayflow::window::PauseCause;

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_787_500_000 + secs, 0).unwrap()
    }

    const SEG: u32 = 600; // 10-minute windows -> stale after 1200s

    fn assess(
        now_s: i64,
        last_chunk: Option<i64>,
        pause: Option<PauseCause>,
        stopped: bool,
    ) -> DayflowLiveness {
        DayflowLiveness::assess(
            at(now_s),
            if last_chunk.is_some() { 5 } else { 0 },
            last_chunk.map(at),
            None,
            SEG,
            2,
            pause,
            stopped,
        )
    }

    #[test]
    fn a_recorder_producing_recently_is_healthy() {
        let l = assess(1_000, Some(700), None, false);
        assert_eq!(l.health, DayflowHealth::Healthy);
        assert!(!l.health.is_fault());
    }

    #[test]
    fn a_running_recorder_producing_nothing_is_degraded() {
        // THE failure this feature exists to catch: reports alive, writes
        // nothing, and nobody notices until tomorrow.
        let l = assess(5_000, Some(700), None, false);
        assert_eq!(l.health, DayflowHealth::Degraded);
        assert!(l.health.is_fault());
        assert!(l.silence(at(5_000)).unwrap().num_seconds() > 1200);
    }

    #[test]
    fn a_recorder_that_never_produced_anything_is_degraded_not_healthy() {
        // The nastiest variant: it started, claimed success, and has written
        // nothing ever. An implementation defaulting to Healthy would hide it.
        let l = assess(5_000, None, None, false);
        assert_eq!(l.health, DayflowHealth::Degraded);
        assert_eq!(l.chunks_written, 0);
    }

    #[test]
    fn paused_off_and_degraded_are_three_distinguishable_states() {
        // All three look identical from outside — no new windows. If they
        // collapse, an operator learns to ignore the signal entirely.
        let paused = assess(5_000, Some(700), Some(PauseCause::Idle), false);
        let off = assess(5_000, Some(700), Some(PauseCause::UserOff), false);
        let degraded = assess(5_000, Some(700), None, false);

        assert_eq!(paused.health, DayflowHealth::Paused);
        assert_eq!(off.health, DayflowHealth::Off);
        assert_eq!(degraded.health, DayflowHealth::Degraded);

        let all = [paused.health, off.health, degraded.health];
        for (i, a) in all.iter().enumerate() {
            for b in all.iter().skip(i + 1) {
                assert_ne!(a, b, "{a:?} and {b:?} must not be the same state");
            }
        }
    }

    #[test]
    fn a_deliberate_pause_is_never_reported_as_a_fault() {
        // FR-032. Identical silence to the degraded case — only the cause differs.
        for cause in [PauseCause::Idle, PauseCause::Locked, PauseCause::DisplaySleep] {
            let l = assess(99_999, Some(700), Some(cause), false);
            assert!(
                !l.health.is_fault(),
                "{cause:?} paused for a long time is still not a fault"
            );
            assert!(!l.health.expects_output(), "a paused recorder is not expected to produce");
        }
    }

    #[test]
    fn staleness_is_measured_in_segment_intervals_not_fixed_minutes() {
        // SC-006. The same 25-minute silence is a fault at a 10-minute interval
        // and perfectly normal at a 30-minute one.
        let silence_secs = 1_500;
        let short = DayflowLiveness::assess(
            at(silence_secs), 3, Some(at(0)), None, 600, 1, None, false,
        );
        let long = DayflowLiveness::assess(
            at(silence_secs), 3, Some(at(0)), None, 1800, 1, None, false,
        );
        assert_eq!(short.health, DayflowHealth::Degraded, "1500s > 2x600s");
        assert_eq!(long.health, DayflowHealth::Healthy, "1500s < 2x1800s");
    }

    #[test]
    fn stopping_is_not_a_fault_however_long_the_silence() {
        let l = assess(999_999, Some(700), None, true);
        assert_eq!(l.health, DayflowHealth::Stopped);
        assert!(!l.health.is_fault());
        assert!(!l.health.expects_output());
    }

    #[test]
    fn intent_can_explain_silence_but_can_never_manufacture_health() {
        // The property that keeps this honest: no combination of declared
        // lifecycle turns a silent recorder into a Healthy one.
        for pause in [
            None,
            Some(PauseCause::Idle),
            Some(PauseCause::Locked),
            Some(PauseCause::DisplaySleep),
            Some(PauseCause::UserOff),
        ] {
            for stopped in [true, false] {
                let l = assess(999_999, Some(0), pause, stopped);
                assert_ne!(
                    l.health,
                    DayflowHealth::Healthy,
                    "silence of 999999s must never read Healthy (pause={pause:?}, stopped={stopped})"
                );
            }
        }
    }

    #[test]
    fn liveness_round_trips_so_a_caller_gets_every_field() {
        let l = assess(1_000, Some(700), None, false);
        let back: DayflowLiveness =
            serde_json::from_str(&serde_json::to_string(&l).unwrap()).unwrap();
        assert_eq!(back, l);
        // a caller must be able to tell the states apart from the payload alone
        let json = serde_json::to_string(&l).unwrap();
        assert!(json.contains("\"health\""), "health must be in the payload: {json}");
    }
}
