//! Recording/analysis persistence manager over rusqlite + [`PathValidator`].
//!
//! Implements the `contracts::traits::StorageManager` trait (T076). Domain
//! [`Recording`] values are mapped to/from the SQLite-facing [`RecordingRow`]
//! (the two layers use different status/encoder enums on purpose — the DB layer
//! tracks a `Finalizing` state the domain collapses into `Recording`).
//!
//! Authored 2026-05-28 from the trait contract + recovered struct shape
//! (`Arc<Mutex<Connection>>` + `PathValidator` + base_dir) — the recovered
//! source for this file was captured panic-log junk.

use crate::contracts::errors::StorageError;
use crate::contracts::traits::{
    EncoderMode as DomainEncoderMode, Recording, RecordingConfig, RecordingList,
    RecordingStatus as DomainStatus, StorageManager as StorageManagerTrait,
};
use crate::security::PathValidator;
use crate::storage::database::{init_database, init_in_memory};
use crate::storage::metadata::{
    EncoderMode as RowEncoderMode, FromRow, RecordingRow, RecordingStatus as RowStatus,
};
use async_trait::async_trait;
use chrono::{Duration, Utc};
use rusqlite::{Connection, OptionalExtension};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// Persists recordings (and their analysis metadata) in SQLite, and constrains
/// file inputs to the storage directory via [`PathValidator`].
pub struct StorageManager {
    conn: Arc<Mutex<Connection>>,
    base_dir: PathBuf,
    path_validator: PathValidator,
}

impl StorageManager {
    /// Open the on-disk database under `base_dir` (creating the directory and
    /// `gentle-eye.db` if needed).
    pub fn new(base_dir: impl Into<PathBuf>) -> Result<Self, StorageError> {
        let base_dir = base_dir.into();
        std::fs::create_dir_all(&base_dir)?;
        let conn = init_database(&base_dir.join("gentle-eye.db"))?;
        Ok(Self::from_parts(conn, base_dir))
    }

    /// Construct with an in-memory database (used by tests).
    pub fn in_memory(base_dir: impl Into<PathBuf>) -> Result<Self, StorageError> {
        let conn = init_in_memory()?;
        Ok(Self::from_parts(conn, base_dir.into()))
    }

    fn from_parts(conn: Connection, base_dir: PathBuf) -> Self {
        let path_validator = PathValidator::new(base_dir.clone());
        Self {
            conn: Arc::new(Mutex::new(conn)),
            base_dir,
            path_validator,
        }
    }

    /// The path validator scoped to this manager's storage directory.
    pub fn path_validator(&self) -> &PathValidator {
        &self.path_validator
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>, StorageError> {
        self.conn
            .lock()
            .map_err(|_| StorageError::DatabaseError("connection mutex poisoned".into()))
    }
}

// ---- domain <-> row mapping ------------------------------------------------

fn domain_status_to_row(s: DomainStatus) -> RowStatus {
    match s {
        DomainStatus::Recording => RowStatus::Recording,
        DomainStatus::Completed => RowStatus::Completed,
        DomainStatus::Cancelled => RowStatus::Cancelled,
        DomainStatus::Failed => RowStatus::Error,
    }
}

fn row_status_to_domain(s: RowStatus) -> DomainStatus {
    match s {
        // The DB-only `Finalizing` state collapses into the domain `Recording`.
        RowStatus::Recording | RowStatus::Finalizing => DomainStatus::Recording,
        RowStatus::Completed => DomainStatus::Completed,
        RowStatus::Cancelled => DomainStatus::Cancelled,
        RowStatus::Error => DomainStatus::Failed,
    }
}

fn domain_encoder_to_row(m: DomainEncoderMode) -> RowEncoderMode {
    match m {
        DomainEncoderMode::Streaming => RowEncoderMode::InMemoryPipe,
        DomainEncoderMode::FileBased => RowEncoderMode::FileBased,
    }
}

fn row_encoder_to_domain(m: RowEncoderMode) -> DomainEncoderMode {
    match m {
        RowEncoderMode::InMemoryPipe => DomainEncoderMode::Streaming,
        RowEncoderMode::FileBased => DomainEncoderMode::FileBased,
    }
}

/// Map the row status strings a domain status filter should match (the domain
/// `Recording` filter spans both the `recording` and `finalizing` DB states).
fn domain_filter_to_row_strs(s: DomainStatus) -> &'static [&'static str] {
    match s {
        DomainStatus::Recording => &["recording", "finalizing"],
        DomainStatus::Completed => &["completed"],
        DomainStatus::Cancelled => &["cancelled"],
        DomainStatus::Failed => &["error"],
    }
}

fn recording_to_row(r: &Recording) -> RecordingRow {
    RecordingRow {
        id: r.id,
        status: domain_status_to_row(r.status),
        start_time: r.start_time,
        end_time: r.end_time,
        duration_ms: r.duration_ms.map(|d| d as i64),
        file_path: r.file_path.clone(),
        fps: i32::from(r.config.fps),
        width: 0,
        height: 0,
        file_size_bytes: r.file_size_bytes.map(|b| b as i64),
        error_message: r.error_message.clone(),
        display_name: None,
        encoder_mode: domain_encoder_to_row(r.config.encoder_mode),
        created_at: None,
        max_duration_seconds: r.config.max_duration_seconds.map(|d| d as i64),
        output_dir: Some(r.config.output_dir.to_string_lossy().into_owned()),
    }
}

fn row_to_recording(row: RecordingRow) -> Recording {
    let mut config = RecordingConfig::default();
    config.fps = row.fps.clamp(0, i32::from(u8::MAX)) as u8;
    config.encoder_mode = row_encoder_to_domain(row.encoder_mode);
    config.max_duration_seconds = row.max_duration_seconds.map(|d| d.max(0) as u64);
    if let Some(dir) = row.output_dir.as_deref() {
        config.output_dir = std::path::PathBuf::from(dir);
    }
    Recording {
        id: row.id,
        status: row_status_to_domain(row.status),
        start_time: row.start_time,
        end_time: row.end_time,
        duration_ms: row.duration_ms.map(|d| d.max(0) as u64),
        file_path: row.file_path,
        file_size_bytes: row.file_size_bytes.map(|b| b.max(0) as u64),
        config,
        error_message: row.error_message,
    }
}

// ---- trait impl ------------------------------------------------------------

#[async_trait]
impl StorageManagerTrait for StorageManager {
    fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    fn generate_recording_path(&self, id: Uuid) -> PathBuf {
        self.base_dir.join(format!("{id}.mp4"))
    }

    async fn save_recording(&self, recording: &Recording) -> Result<(), StorageError> {
        let row = recording_to_row(recording);
        let conn = self.lock()?;
        // Upsert: try UPDATE first, INSERT if the row didn't exist yet.
        let updated = conn.execute(RecordingRow::update_sql(), row.to_update_params())?;
        if updated == 0 {
            conn.execute(RecordingRow::insert_sql(), row.to_insert_params())?;
        }
        Ok(())
    }

    async fn load_recording(&self, id: Uuid) -> Result<Recording, StorageError> {
        let conn = self.lock()?;
        let row = conn
            .prepare("SELECT * FROM recordings WHERE id = ?1")?
            .query_row([id.to_string()], RecordingRow::from_row)
            .optional()?;
        match row {
            Some(r) => Ok(row_to_recording(r)),
            None => Err(StorageError::NotFound(id)),
        }
    }

    async fn delete_recording(&self, id: Uuid) -> Result<(), StorageError> {
        let path = self.generate_recording_path(id);
        let conn = self.lock()?;
        let removed = conn.execute("DELETE FROM recordings WHERE id = ?1", [id.to_string()])?;
        if removed == 0 {
            return Err(StorageError::NotFound(id));
        }
        drop(conn);
        // Best-effort removal of the associated video file.
        let _ = std::fs::remove_file(path);
        Ok(())
    }

    async fn list_recordings(
        &self,
        limit: usize,
        offset: usize,
        status_filter: Option<DomainStatus>,
    ) -> Result<RecordingList, StorageError> {
        // Build a WHERE clause from hardcoded status strings (no user input ->
        // no injection surface).
        let where_clause = match status_filter {
            Some(s) => {
                let in_list = domain_filter_to_row_strs(s)
                    .iter()
                    .map(|v| format!("'{v}'"))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("WHERE status IN ({in_list})")
            }
            None => String::new(),
        };

        let conn = self.lock()?;
        let total_count: i64 = conn.query_row(
            &format!("SELECT COUNT(*) FROM recordings {where_clause}"),
            [],
            |r| r.get(0),
        )?;

        let sql = format!(
            "SELECT * FROM recordings {where_clause} ORDER BY start_time DESC LIMIT ?1 OFFSET ?2"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map([limit as i64, offset as i64], RecordingRow::from_row)?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(RecordingList {
            recordings: rows.into_iter().map(row_to_recording).collect(),
            total_count: total_count.max(0) as usize,
        })
    }

    async fn storage_used(&self) -> Result<u64, StorageError> {
        let conn = self.lock()?;
        let total: i64 = conn.query_row(
            "SELECT COALESCE(SUM(file_size_bytes), 0) FROM recordings",
            [],
            |r| r.get(0),
        )?;
        Ok(total.max(0) as u64)
    }

    async fn cleanup_old_recordings(&self, max_age_days: u32) -> Result<u32, StorageError> {
        let cutoff = (Utc::now() - Duration::days(i64::from(max_age_days))).to_rfc3339();
        let conn = self.lock()?;
        let removed = conn.execute("DELETE FROM recordings WHERE start_time < ?1", [cutoff])?;
        Ok(removed as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_recording() -> Recording {
        Recording::new(RecordingConfig::default())
    }

    #[tokio::test]
    async fn save_load_round_trip() {
        let mgr = StorageManager::in_memory("/tmp/gentle-eye-test").unwrap();
        let rec = sample_recording();
        mgr.save_recording(&rec).await.unwrap();
        let loaded = mgr.load_recording(rec.id).await.unwrap();
        assert_eq!(loaded.id, rec.id);
        assert_eq!(loaded.status, DomainStatus::Recording);
        assert_eq!(loaded.config.fps, rec.config.fps);
    }

    #[tokio::test]
    async fn config_fields_round_trip() {
        let mgr = StorageManager::in_memory("/tmp/gentle-eye-test").unwrap();
        let config = RecordingConfig {
            fps: 5,
            max_duration_seconds: Some(42),
            output_dir: PathBuf::from("/tmp/custom-rec"),
            ..RecordingConfig::default()
        };
        let rec = Recording::new(config);
        mgr.save_recording(&rec).await.unwrap();
        let loaded = mgr.load_recording(rec.id).await.unwrap();
        assert_eq!(loaded.config.fps, 5);
        assert_eq!(loaded.config.max_duration_seconds, Some(42));
        assert_eq!(loaded.config.output_dir, PathBuf::from("/tmp/custom-rec"));
    }

    #[tokio::test]
    async fn load_missing_is_not_found() {
        let mgr = StorageManager::in_memory("/tmp/gentle-eye-test").unwrap();
        let err = mgr.load_recording(Uuid::new_v4()).await.unwrap_err();
        assert!(matches!(err, StorageError::NotFound(_)));
    }

    #[tokio::test]
    async fn save_is_an_upsert() {
        let mgr = StorageManager::in_memory("/tmp/gentle-eye-test").unwrap();
        let mut rec = sample_recording();
        mgr.save_recording(&rec).await.unwrap();
        rec.status = DomainStatus::Completed;
        rec.file_size_bytes = Some(2048);
        mgr.save_recording(&rec).await.unwrap();
        let loaded = mgr.load_recording(rec.id).await.unwrap();
        assert_eq!(loaded.status, DomainStatus::Completed);
        assert_eq!(loaded.file_size_bytes, Some(2048));
        // Still exactly one row.
        let list = mgr.list_recordings(10, 0, None).await.unwrap();
        assert_eq!(list.total_count, 1);
    }

    #[tokio::test]
    async fn list_filters_and_counts() {
        let mgr = StorageManager::in_memory("/tmp/gentle-eye-test").unwrap();
        for _ in 0..3 {
            mgr.save_recording(&sample_recording()).await.unwrap();
        }
        let mut done = sample_recording();
        done.status = DomainStatus::Completed;
        done.file_size_bytes = Some(1000);
        mgr.save_recording(&done).await.unwrap();

        let all = mgr.list_recordings(10, 0, None).await.unwrap();
        assert_eq!(all.total_count, 4);

        let recording_only = mgr
            .list_recordings(10, 0, Some(DomainStatus::Recording))
            .await
            .unwrap();
        assert_eq!(recording_only.total_count, 3);

        let completed = mgr
            .list_recordings(10, 0, Some(DomainStatus::Completed))
            .await
            .unwrap();
        assert_eq!(completed.total_count, 1);
        assert_eq!(mgr.storage_used().await.unwrap(), 1000);
    }

    #[tokio::test]
    async fn delete_removes_row() {
        let mgr = StorageManager::in_memory("/tmp/gentle-eye-test").unwrap();
        let rec = sample_recording();
        mgr.save_recording(&rec).await.unwrap();
        mgr.delete_recording(rec.id).await.unwrap();
        assert!(matches!(
            mgr.load_recording(rec.id).await.unwrap_err(),
            StorageError::NotFound(_)
        ));
        assert!(matches!(
            mgr.delete_recording(rec.id).await.unwrap_err(),
            StorageError::NotFound(_)
        ));
    }
}
