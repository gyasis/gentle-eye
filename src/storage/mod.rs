//! Storage and persistence module for gentle-eye.
//!
//! This module handles data persistence including:
//! - SQLite database initialization and management
//! - Recording storage and retrieval
//! - Analysis results caching
//! - Temporary file management
//! - Storage cleanup and maintenance

pub mod database;
pub mod manager;
pub mod metadata;

pub use database::init_database;
pub use manager::StorageManager;
pub use metadata::{
    AnalysisRequestRow, AnalysisResultRow, RecordingRow, FromRow, ToRow,
};
