//! Region providers. Each implements [`crate::regions::RegionProvider`] at a
//! (granularity, cost) slot; the cascade resolver (E4) consults them cheapest-first.
//!
//! - [`wm`] — X11 EWMH window rects (E2). Free, exact, all apps.
//! - (later) `atspi`, `segment`, `contrast`, `ocr`, `yolo`, `vlm`.

pub mod atspi;
pub mod contrast;
pub mod wm;
