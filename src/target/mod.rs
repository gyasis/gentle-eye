//! `target` — agent-driven region-of-interest (crop) on a capture source.
//!
//! A *target* is an OBS-style crop on a display or stream. The agent (a VLM)
//! decides *what* to focus on and passes a rough region in **normalized 0–1
//! coordinates**; gentle-eye maps it to pixels, crops every captured frame to
//! it, and (Phase 2) snaps the box to real edges with pure-Rust CV.
//!
//! Design (PRD `gentle_eye_target_feature_2026-05-29`): **Vision-First,
//! CV-Second** — the VLM is the brain (semantics), the `geometry`/`measure`
//! code is the caliper (precision). Phases:
//!   - P1 crop primitive: [`model`] + [`geometry`] + [`store`] + [`crop`]
//!   - P2 measurement:    [`measure`] (`imageproc`)
//!   - P3 tracking:       `track` (feature-gated `opencv`, deferred)

pub mod crop;
pub mod errors;
pub mod geometry;
pub mod measure;
pub mod model;
pub mod store;
pub mod track;

pub use errors::TargetError;
pub use model::{NormRect, PixelRect, Target, TargetSource};
