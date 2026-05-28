//! SQLite row conversion utilities for gentle-eye models.
//!
//! This module provides traits and implementations for converting
//! between Rust domain types and SQLite row representations.

use chrono::{DateTime, Utc};
use rusqlite::{Row, Result as SqliteResult, ToSql};
use std::path::PathBuf;
use uuid::Uuid;

/// Trait for converting a SQLite row to a domain type.
pub trait FromRow: Sized {
    /// Convert a SQLite row to the implementing type.
    fn from_row(row: &Row) -> SqliteResult<Self>;
}

/// Trait for converting a domain type to SQLite parameters.
pub trait ToRow {
    /// Return the column names for INSERT statements.
    fn column_names() -> &'static [&'static str];

    /// Return the values as a boxed slice for parameterized queries.
    fn to_params(&self) -> Vec<Box<dyn ToSql>>;
}

/// Recording status enumeration matching the data model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingStatus {
    Recording,
    Finalizing,
    Completed,
    Error,
    Cancelled,
}

impl RecordingStatus {
    /// Convert status to string for SQLite storage.
    pub fn as_str(&self) -> &'static str {
        match self {
            RecordingStatus::Recording => "recording",
            RecordingStatus::Finalizing => "finalizing",
            RecordingStatus::Completed => "completed",
            RecordingStatus::Error => "error",
            RecordingStatus::Cancelled => "cancelled",
        }
    }

    /// Parse status from SQLite string.
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "recording" => Some(RecordingStatus::Recording),
            "finalizing" => Some(RecordingStatus::Finalizing),
            "completed" => Some(RecordingStatus::Completed),
            "error" => Some(RecordingStatus::Error),
            "cancelled" => Some(RecordingStatus::Cancelled),
            _ => None,
        }
    }
}

/// Encoder mode enumeration matching the data model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncoderMode {
    FileBased,
    InMemoryPipe,
}

impl EncoderMode {
    /// Convert encoder mode to string for SQLite storage.
    pub fn as_str(&self) -> &'static str {
        match self {
            EncoderMode::FileBased => "file_based",
            EncoderMode::InMemoryPipe => "in_memory_pipe",
        }
    }

    /// Parse encoder mode from SQLite string.
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "file_based" => Some(EncoderMode::FileBased),
            "in_memory_pipe" => Some(EncoderMode::InMemoryPipe),
            _ => None,
        }
    }
}

/// Recording row representation for SQLite operations.
///
/// This struct maps directly to the recordings table schema.
#[derive(Debug, Clone)]
pub struct RecordingRow {
    pub id: Uuid,
    pub status: RecordingStatus,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    pub duration_ms: Option<i64>,
    pub file_path: Option<PathBuf>,
    pub fps: i32,
    pub width: i32,
    pub height: i32,
    pub file_size_bytes: Option<i64>,
    pub error_message: Option<String>,
    pub display_name: Option<String>,
    pub encoder_mode: EncoderMode,
    pub created_at: Option<DateTime<Utc>>,
    pub max_duration_seconds: Option<i64>,
    pub output_dir: Option<String>,
}

impl FromRow for RecordingRow {
    fn from_row(row: &Row) -> SqliteResult<Self> {
        let id_str: String = row.get("id")?;
        let id = Uuid::parse_str(&id_str).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(e),
            )
        })?;

        let status_str: String = row.get("status")?;
        let status = RecordingStatus::from_str(&status_str).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                1,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("Invalid recording status: {}", status_str),
                )),
            )
        })?;

        let start_time_str: String = row.get("start_time")?;
        let start_time = DateTime::parse_from_rfc3339(&start_time_str)
            .map(|dt| dt.with_timezone(&Utc))
            .map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    2,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;

        let end_time: Option<DateTime<Utc>> = row
            .get::<_, Option<String>>("end_time")?
            .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
            .map(|dt| dt.with_timezone(&Utc));

        let file_path: Option<PathBuf> = row
            .get::<_, Option<String>>("file_path")?
            .map(PathBuf::from);

        let encoder_mode_str: String = row.get("encoder_mode")?;
        let encoder_mode = EncoderMode::from_str(&encoder_mode_str).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                12,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("Invalid encoder mode: {}", encoder_mode_str),
                )),
            )
        })?;

        let created_at: Option<DateTime<Utc>> = row
            .get::<_, Option<String>>("created_at")?
            .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
            .map(|dt| dt.with_timezone(&Utc));

        Ok(RecordingRow {
            id,
            status,
            start_time,
            end_time,
            duration_ms: row.get("duration_ms")?,
            file_path,
            fps: row.get("fps")?,
            width: row.get("width")?,
            height: row.get("height")?,
            file_size_bytes: row.get("file_size_bytes")?,
            error_message: row.get("error_message")?,
            display_name: row.get("display_name")?,
            encoder_mode,
            created_at,
            max_duration_seconds: row.get("max_duration_seconds")?,
            output_dir: row.get("output_dir")?,
        })
    }
}

impl RecordingRow {
    /// Create SQL INSERT statement for a recording.
    pub fn insert_sql() -> &'static str {
        r#"
        INSERT INTO recordings (
            id, status, start_time, end_time, duration_ms, file_path,
            fps, width, height, file_size_bytes, error_message,
            display_name, encoder_mode, max_duration_seconds, output_dir
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15
        )
        "#
    }

    /// Create SQL UPDATE statement for a recording.
    pub fn update_sql() -> &'static str {
        r#"
        UPDATE recordings SET
            status = ?2,
            end_time = ?3,
            duration_ms = ?4,
            file_path = ?5,
            file_size_bytes = ?6,
            error_message = ?7
        WHERE id = ?1
        "#
    }

    /// Convert to INSERT parameters.
    pub fn to_insert_params(&self) -> impl rusqlite::Params {
        (
            self.id.to_string(),
            self.status.as_str().to_string(),
            self.start_time.to_rfc3339(),
            self.end_time.map(|dt| dt.to_rfc3339()),
            self.duration_ms,
            self.file_path.as_ref().map(|p| p.to_string_lossy().to_string()),
            self.fps,
            self.width,
            self.height,
            self.file_size_bytes,
            self.error_message.clone(),
            self.display_name.clone(),
            self.encoder_mode.as_str().to_string(),
            self.max_duration_seconds,
            self.output_dir.clone(),
        )
    }

    /// Convert to UPDATE parameters.
    pub fn to_update_params(&self) -> impl rusqlite::Params {
        (
            self.id.to_string(),
            self.status.as_str().to_string(),
            self.end_time.map(|dt| dt.to_rfc3339()),
            self.duration_ms,
            self.file_path.as_ref().map(|p| p.to_string_lossy().to_string()),
            self.file_size_bytes,
            self.error_message.clone(),
        )
    }
}

/// Analysis request row representation for SQLite operations.
///
/// This struct maps directly to the analysis_requests table schema.
#[derive(Debug, Clone)]
pub struct AnalysisRequestRow {
    pub id: Uuid,
    pub recording_id: Option<Uuid>,
    pub video_path: PathBuf,
    pub prompt: String,
    pub provider: String,
    pub timestamp: DateTime<Utc>,
    pub timeframe_start: Option<f64>,
    pub timeframe_end: Option<f64>,
}

impl FromRow for AnalysisRequestRow {
    fn from_row(row: &Row) -> SqliteResult<Self> {
        let id_str: String = row.get("id")?;
        let id = Uuid::parse_str(&id_str).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(e),
            )
        })?;

        let recording_id: Option<Uuid> = row
            .get::<_, Option<String>>("recording_id")?
            .and_then(|s| Uuid::parse_str(&s).ok());

        let video_path: String = row.get("video_path")?;

        let timestamp_str: String = row.get("timestamp")?;
        let timestamp = DateTime::parse_from_rfc3339(&timestamp_str)
            .map(|dt| dt.with_timezone(&Utc))
            .map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    5,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;

        Ok(AnalysisRequestRow {
            id,
            recording_id,
            video_path: PathBuf::from(video_path),
            prompt: row.get("prompt")?,
            provider: row.get("provider")?,
            timestamp,
            timeframe_start: row.get("timeframe_start")?,
            timeframe_end: row.get("timeframe_end")?,
        })
    }
}

impl AnalysisRequestRow {
    /// Create SQL INSERT statement for an analysis request.
    pub fn insert_sql() -> &'static str {
        r#"
        INSERT INTO analysis_requests (
            id, recording_id, video_path, prompt, provider,
            timestamp, timeframe_start, timeframe_end
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8
        )
        "#
    }

    /// Convert to INSERT parameters.
    pub fn to_insert_params(&self) -> impl rusqlite::Params {
        (
            self.id.to_string(),
            self.recording_id.map(|id| id.to_string()),
            self.video_path.to_string_lossy().to_string(),
            self.prompt.clone(),
            self.provider.clone(),
            self.timestamp.to_rfc3339(),
            self.timeframe_start,
            self.timeframe_end,
        )
    }
}

/// Analysis result row representation for SQLite operations.
///
/// This struct maps directly to the analysis_results table schema.
#[derive(Debug, Clone)]
pub struct AnalysisResultRow {
    pub id: Uuid,
    pub request_id: Uuid,
    pub analysis_text: String,
    pub model_used: String,
    pub token_count: Option<i32>,
    pub processing_time_ms: i64,
    pub timestamp: DateTime<Utc>,
    pub success: bool,
    pub error_message: Option<String>,
}

impl FromRow for AnalysisResultRow {
    fn from_row(row: &Row) -> SqliteResult<Self> {
        let id_str: String = row.get("id")?;
        let id = Uuid::parse_str(&id_str).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(e),
            )
        })?;

        let request_id_str: String = row.get("request_id")?;
        let request_id = Uuid::parse_str(&request_id_str).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                1,
                rusqlite::types::Type::Text,
                Box::new(e),
            )
        })?;

        let timestamp_str: String = row.get("timestamp")?;
        let timestamp = DateTime::parse_from_rfc3339(&timestamp_str)
            .map(|dt| dt.with_timezone(&Utc))
            .map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    6,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;

        let success_int: i32 = row.get("success")?;

        Ok(AnalysisResultRow {
            id,
            request_id,
            analysis_text: row.get("analysis_text")?,
            model_used: row.get("model_used")?,
            token_count: row.get("token_count")?,
            processing_time_ms: row.get("processing_time_ms")?,
            timestamp,
            success: success_int != 0,
            error_message: row.get("error_message")?,
        })
    }
}

impl AnalysisResultRow {
    /// Create SQL INSERT statement for an analysis result.
    pub fn insert_sql() -> &'static str {
        r#"
        INSERT INTO analysis_results (
            id, request_id, analysis_text, model_used, token_count,
            processing_time_ms, timestamp, success, error_message
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9
        )
        "#
    }

    /// Convert to INSERT parameters.
    pub fn to_insert_params(&self) -> impl rusqlite::Params {
        (
            self.id.to_string(),
            self.request_id.to_string(),
            self.analysis_text.clone(),
            self.model_used.clone(),
            self.token_count,
            self.processing_time_ms,
            self.timestamp.to_rfc3339(),
            if self.success { 1i32 } else { 0i32 },
            self.error_message.clone(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recording_status_round_trip() {
        let statuses = [
            RecordingStatus::Recording,
            RecordingStatus::Finalizing,
            RecordingStatus::Completed,
            RecordingStatus::Error,
            RecordingStatus::Cancelled,
        ];

        for status in statuses {
            let s = status.as_str();
            let parsed = RecordingStatus::from_str(s).unwrap();
            assert_eq!(status, parsed);
        }
    }

    #[test]
    fn test_encoder_mode_round_trip() {
        let modes = [EncoderMode::FileBased, EncoderMode::InMemoryPipe];

        for mode in modes {
            let s = mode.as_str();
            let parsed = EncoderMode::from_str(s).unwrap();
            assert_eq!(mode, parsed);
        }
    }

    #[test]
    fn test_recording_status_case_insensitive() {
        assert_eq!(
            RecordingStatus::from_str("RECORDING"),
            Some(RecordingStatus::Recording)
        );
        assert_eq!(
            RecordingStatus::from_str("Recording"),
            Some(RecordingStatus::Recording)
        );
        assert_eq!(
            RecordingStatus::from_str("recording"),
            Some(RecordingStatus::Recording)
        );
    }

    #[test]
    fn test_invalid_status_returns_none() {
        assert_eq!(RecordingStatus::from_str("invalid"), None);
        assert_eq!(RecordingStatus::from_str(""), None);
    }

    #[test]
    fn test_invalid_encoder_mode_returns_none() {
        assert_eq!(EncoderMode::from_str("invalid"), None);
        assert_eq!(EncoderMode::from_str(""), None);
    }
}
