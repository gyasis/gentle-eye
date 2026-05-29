use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum ActivityCategory {
    Coding,
    Docs,
    Comms,
    Browsing,
    Meeting,
    Idle,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TimelineEntry {
    pub recording_id: Uuid,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub category: ActivityCategory,
    pub app: String,
    pub activity: String,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChunkSummary {
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub category: ActivityCategory,
    pub app: String,
    pub activity: String,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RollingContext {
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
    pub entries: Vec<TimelineEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DayflowSession {
    pub session_id: Uuid,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    pub entries: Vec<TimelineEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DayflowStatus {
    pub is_active: bool,
    pub current_session: Option<DayflowSession>,
    pub last_updated: DateTime<Utc>,
}