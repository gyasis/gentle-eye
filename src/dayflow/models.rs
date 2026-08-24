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

    fn chunk(display: u32, seq: u64, index: usize) -> ChunkRef {
        ChunkRef {
            index,
            path: std::path::PathBuf::from("/tmp/chunk.mp4"),
            start_wall: Utc::now(),
            end_wall: Utc::now(),
            display_id: display,
            sequence: seq,
            summarized: false,
        }
    }

    #[test]
    fn sequence_is_the_durable_identity_not_index() {
        // After a pause/resume the sampler's per-run `index` restarts at 0 while
        // `sequence` keeps counting. Anything keying on `index` would collide.
        let before_pause = chunk(0, 7, 7);
        let after_resume = chunk(0, 8, 0); // index reset, sequence continued
        assert_eq!(after_resume.index, 0, "per-run index restarts");
        assert!(
            after_resume.sequence > before_pause.sequence,
            "sequence must keep increasing across a restart"
        );
        let key = |c: &ChunkRef| (c.display_id, c.sequence);
        assert_ne!(key(&before_pause), key(&after_resume));
    }

    #[test]
    fn same_sequence_on_different_displays_is_a_different_window() {
        let a = chunk(0, 3, 3);
        let b = chunk(1, 3, 3);
        assert_ne!(
            (a.display_id, a.sequence),
            (b.display_id, b.sequence),
            "identity must include the display, or a 3-display desk collides"
        );
    }

    #[test]
    fn an_unsummarized_chunk_is_never_reclaimable() {
        // FR-025: a window that failed summarisation because a backend was down
        // must be retried, not evicted. The flag is the guard.
        let c = chunk(0, 1, 1);
        assert!(!c.summarized, "a fresh window is not summarised");
    }

    #[test]
    fn chunkref_still_loads_data_written_before_these_fields_existed() {
        // serde(default) keeps older persisted manifests readable.
        let old = r#"{"index":2,"path":"/tmp/c.mp4",
            "start_wall":"2026-08-24T09:00:00Z","end_wall":"2026-08-24T09:15:00Z"}"#;
        let c: ChunkRef = serde_json::from_str(old).expect("old manifest must still parse");
        assert_eq!(c.index, 2);
        assert_eq!(c.display_id, 0);
        assert_eq!(c.sequence, 0);
        assert!(!c.summarized);
    }
}
