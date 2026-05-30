//! `preview` — OBS-style preview pane: a LIVE preview of what's being captured
//! (Part 1) and POST-capture review of what was just captured (Part 2).
//!
//! Design (PRD `gentle_eye_preview_pane_2026-05-30`): **supply-chain-minimal**.
//! The default build adds ZERO new crates — it reuses the already-installed
//! `ffplay` (subprocess) and a hand-rolled [`gallery`] server on `std::net`.
//! A pure-Rust window (`winit`+`softbuffer`) is an **opt-in** backend behind the
//! off-by-default `richwindow` feature. No countdown.
//!
//! Backends sit behind the [`renderer::PreviewRenderer`] trait.

pub mod discover;
pub mod errors;
pub mod gallery;
pub mod live;
pub mod player;
pub mod renderer;

pub use discover::{latest_capture, recent_captures, Capture, CaptureKind};
pub use errors::PreviewError;
