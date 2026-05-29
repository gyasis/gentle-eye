use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ActivityCategory {
    Coding,
    Docs,
    Comms,
    Browsing,
    Meeting,
    Idle,
    #[default]
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DayflowMode {
    #[default]
    Session,
    Daemon,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DayflowStatus {
    #[default]
    Idle,
    Recording,
    Summarizing,
    Stopped,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DayflowSession {
    pub id: Uuid,
    pub recording_id: Uuid,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub mode: DayflowMode,
    pub status: DayflowStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkRef {
    pub index: usize,
    pub path: PathBuf,
    pub start_wall: DateTime<Utc>,
    pub end_wall: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkSummary {
    pub chunk_index: usize,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub category: ActivityCategory,
    pub app: String,
    pub activity: String,
    pub detail: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RollingContext {
    pub summary: String,
}

impl RollingContext {
    pub fn is_empty(&self) -> bool {
        self.summary.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineEntry {
    pub id: Uuid,
    pub recording_id: Uuid,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub category: ActivityCategory,
    pub app: String,
    pub activity: String,
    pub summary: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;

    #[test]
    fn test_activity_category_serialization() {
        let categories = vec![
            ActivityCategory::Coding,
            ActivityCategory::Docs,
            ActivityCategory::Comms,
            ActivityCategory::Browsing,
            ActivityCategory::Meeting,
            ActivityCategory::Idle,
            ActivityCategory::Other,
        ];

        for category in categories {
            let serialized = serde_json::to_string(&category).unwrap();
            let deserialized: ActivityCategory = serde_json::from_str(&serialized).unwrap();
            assert_eq!(category, deserialized);
        }
    }

    #[test]
    fn test_dayflow_mode_serialization() {
        let modes = vec![DayflowMode::Session, DayflowMode::Daemon];
        for mode in modes {
            let serialized = serde_json::to_string(&mode).unwrap();
            let deserialized: DayflowMode = serde_json::from_str(&serialized).unwrap();
            assert_eq!(mode, deserialized);
        }
    }

    #[test]
    fn test_dayflow_status_serialization() {
        let statuses = vec![
            DayflowStatus::Idle,
            DayflowStatus::Recording,
            DayflowStatus::Summarizing,
            DayflowStatus::Stopped,
            DayflowStatus::Error,
        ];

        for status in statuses {
            let serialized = serde_json::to_string(&status).unwrap();
            let deserialized: DayflowStatus = serde_json::from_str(&serialized).unwrap();
            assert_eq!(status, deserialized);
        }
    }

    #[test]
    fn test_dayflow_session_serialization() {
        let session = DayflowSession {
            id: Uuid::new_v4(),
            recording_id: Uuid::new_v4(),
            started_at: Utc::now(),
            ended_at: Some(Utc::now()),
            mode: DayflowMode::Session,
            status: DayflowStatus::Recording,
        };

        let serialized = serde_json::to_string(&session).unwrap();
        let deserialized: DayflowSession = serde_json::from_str(&serialized).unwrap();
        assert_eq!(session.id, deserialized.id);
        assert_eq!(session.recording_id, deserialized.recording_id);
        assert_eq!(session.started_at, deserialized.started_at);
        assert_eq!(session.ended_at, deserialized.ended_at);
        assert_eq!(session.mode, deserialized.mode);
        assert_eq!(session.status, deserialized.status);
    }

    #[test]
    fn test_chunk_ref_serialization() {
        let chunk_ref = ChunkRef {
            index: 42,
            path: PathBuf::from("/tmp/test.mp4"),
            start_wall: Utc::now(),
            end_wall: Utc::now(),
        };

        let serialized = serde_json::to_string(&chunk_ref).unwrap();
        let deserialized: ChunkRef = serde_json::from_str(&serialized).unwrap();
        assert_eq!(chunk_ref.index, deserialized.index);
        assert_eq!(chunk_ref.path, deserialized.path);
        assert_eq!(chunk_ref.start_wall, deserialized.start_wall);
        assert_eq!(chunk_ref.end_wall, deserialized.end_wall);
    }

    #[test]
    fn test_chunk_summary_serialization() {
        let summary = ChunkSummary {
            chunk_index: 10,
            start_time: Utc::now(),
            end_time: Utc::now(),
            category: ActivityCategory::Coding,
            app: "VSCode".to_string(),
            activity: "writing code".to_string(),
            detail: "working on Rust project".to_string(),
        };

        let serialized = serde_json::to_string(&summary).unwrap();
        let deserialized: ChunkSummary = serde_json::from_str(&serialized).unwrap();
        assert_eq!(summary.chunk_index, deserialized.chunk_index);
        assert_eq!(summary.start_time, deserialized.start_time);
        assert_eq!(summary.end_time, deserialized.end_time);
        assert_eq!(summary.category, deserialized.category);
        assert_eq!(summary.app, deserialized.app);
        assert_eq!(summary.activity, deserialized.activity);
        assert_eq!(summary.detail, deserialized.detail);
    }

    #[test]
    fn test_rolling_context_serialization() {
        let context = RollingContext {
            summary: "test summary".to_string(),
        };

        let serialized = serde_json::to_string(&context).unwrap();
        let deserialized: RollingContext = serde_json::from_str(&serialized).unwrap();
        assert_eq!(context.summary, deserialized.summary);
        assert!(!deserialized.is_empty());

        let empty_context = RollingContext::default();
        let serialized_empty = serde_json::to_string(&empty_context).unwrap();
        let deserialized_empty: RollingContext = serde_json::from_str(&serialized_empty).unwrap();
        assert!(deserialized_empty.is_empty());
    }

    #[test]
    fn test_timeline_entry_serialization() {
        let entry = TimelineEntry {
            id: Uuid::new_v4(),
            recording_id: Uuid::new_v4(),
            start_time: Utc::now(),
            end_time: Utc::now(),
            category: ActivityCategory::Browsing,
            app: "Chrome".to_string(),
            activity: "web browsing".to_string(),
            summary: "researching documentation".to_string(),
        };

        let serialized = serde_json::to_string(&entry).unwrap();
        let deserialized: TimelineEntry = serde_json::from_str(&serialized).unwrap();
        assert_eq!(entry.id, deserialized.id);
        assert_eq!(entry.recording_id, deserialized.recording_id);
        assert_eq!(entry.start_time, deserialized.start_time);
        assert_eq!(entry.end_time, deserialized.end_time);
        assert_eq!(entry.category, deserialized.category);
        assert_eq!(entry.app, deserialized.app);
        assert_eq!(entry.activity, deserialized.activity);
        assert_eq!(entry.summary, deserialized.summary);
    }

    #[test]
    fn test_default_values() {
        let default_category = ActivityCategory::default();
        assert_eq!(default_category, ActivityCategory::Other);

        let default_mode = DayflowMode::default();
        assert_eq!(default_mode, DayflowMode::Session);

        let default_status = DayflowStatus::default();
        assert_eq!(default_status, DayflowStatus::Idle);

        let default_context = RollingContext::default();
        assert!(default_context.is_empty());
    }

    #[test]
    fn test_uuid_generation() {
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        assert_ne!(id1, id2);
    }
}