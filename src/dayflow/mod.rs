//! Dayflow mode — continuous low-fps screen recording turned into a queryable
//! activity timeline.
//!
//! Pipeline: record (low fps, 15-min chunks on the fly) → summarize each chunk
//! with a vision provider (Map-Reduce with rolling context, Gemini by default)
//! → store structured `TimelineEntry` rows → query / `ask_day`. A 3-tier
//! retention policy keeps the timeline permanently while shrinking and evicting
//! the raw video.
//!
//! Module layout (filled in across waves):
//! - [`models`]     — data types (this wave).
//! - [`errors`]     — `DayflowError` (re-exported from the contracts layer).
//! - [`summarizer`] — Rust-native Map-Reduce chunk summarizer (Wave 3).
//! - [`timeline`]   — SQLite-backed timeline store + `ask_day` (Wave 4).
//! - [`engine`]     — session lifecycle + real-time scheduler (Wave 5).
//! - [`daemon`]     — continuous all-day daemon (Wave 5).
//! - [`retention`]  — save → shrink → archive + disk-evict guard (Wave 6).

pub mod chunking;
pub mod daemon;
pub mod engine;
pub mod errors;
pub mod models;
pub mod retention;
pub mod summarizer;
pub mod timeline;

pub use chunking::{chunk_filename, plan_chunks};
pub use errors::DayflowError;
pub use models::{
    ActivityCategory, ChunkRef, ChunkSummary, DayflowMode, DayflowSession, DayflowStatus,
    RollingContext, TimelineEntry,
};
