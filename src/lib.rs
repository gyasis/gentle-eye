//! # gentle-eye
//!
//! MCP server (rmcp, JSON-RPC over stdio) for screen recording and vision
//! analysis. Modules sit behind trait interfaces (see [`contracts`]): the MCP
//! handler ([`mcp`]) dispatches to the capture, storage, and analysis
//! implementations.
//!
//! Every module is `pub` so the in-package binary (`src/bin/gentle-eye.rs`)
//! and the integration tests (separate crates) can consume them. The `-D warnings`
//! gate is enforced by `.cargo/config.toml`, not a source-level `#![deny]`.
//!
//! ## Using gentle-eye as a library
//!
//! The [`api`] module is the easy entry point — add this crate as a dependency
//! and call one function:
//!
//! ```no_run
//! use gentle_eye::{record, read_text_video};
//! use std::time::Duration;
//! # async fn demo() -> anyhow::Result<()> {
//! let rec = record(0, Duration::from_secs(10), 2, "/tmp/recordings").await?;
//! if let Some(path) = &rec.file_path {
//!     println!("text on screen:\n{}", read_text_video(path)?);
//! }
//! # Ok(())
//! # }
//! ```

// Accepted idioms in recovered code (2026-05-28): inherent `from_str` on the
// status enums (not the std `FromStr` trait) and a couple of hand-written
// `Default` impls. Documented allow rather than churning recovered source.
#![allow(clippy::should_implement_trait, clippy::derivable_impls, clippy::field_reassign_with_default)]

pub mod analysis;
pub mod api;
pub mod capture;
pub mod config;
pub mod contracts;
pub mod mcp;
pub mod models;
pub mod security;
pub mod startup;
pub mod storage;

// Convenience re-exports for library consumers — the one-call facade plus the
// types those calls return/accept, so `use gentle_eye::{record, VisionConfig};`
// is all most callers need.
pub use analysis::ocr::{ocr_image as read_text_image, ocr_video as read_text_video};
pub use api::{analyze, analyze_video_range, record};
pub use contracts::traits::{AnalysisResult, Recording, RecordingConfig, TimeRange, VisionConfig};
