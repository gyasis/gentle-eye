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
