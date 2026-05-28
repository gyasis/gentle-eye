//! Screen capture: display enumeration, frame capture (scrap), encoding (FFmpeg),
//! and memory-pressure monitoring.
//!
//! Note (2026-05-28): the recovered `capture/mod.rs` was a 192 KB cross-project
//! junk blob (Python filenames + matplotlib docs + agent notes — NOT capture code;
//! the REVIEW_REPORT mis-classified it by size). The real capture logic lives in
//! the submodules below; this root is a clean Structuralist declaration. The junk
//! blob is preserved at `mod.rs.raw.<epoch>`.

pub mod display;
pub mod encoder;
pub mod frame_rate;
pub mod memory;
pub mod screen;
pub mod service;
pub mod stream;

pub use encoder::PipeEncoder;
pub use frame_rate::FrameRateController;
pub use memory::{MemoryConfig, MemoryMonitor, MemoryPressure, MemoryStats};
pub use screen::ScreenCapturer;
pub use service::CaptureService;
pub use stream::{capture_stream_frame, StreamFrame};
