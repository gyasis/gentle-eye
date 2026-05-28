//! Screen frame capture via `scrap::Capturer`.
//!
//! Thin wrapper that enumerates displays, owns a `scrap::Capturer`, and yields
//! BGRA frames (handling scrap's `WouldBlock` "frame not ready yet" signal).
//! Actual capture requires a display server, so only the pure error-mapping is
//! unit-tested here; live capture is integration-tested.
//!
//! Authored 2026-05-28 against the scrap 0.5 API (`Display::all/primary`,
//! `Capturer::new`, `capturer.frame() -> io::Result<Frame>`) — the recovered
//! source was a junk partial.

use crate::contracts::errors::RecordingError;
use scrap::{Capturer, Display};
use std::io::ErrorKind;
use std::time::{Duration, Instant};

/// Owns a scrap capturer for one display and produces BGRA frames.
pub struct ScreenCapturer {
    capturer: Capturer,
    width: usize,
    height: usize,
}

impl ScreenCapturer {
    /// Capture the display at `display_index` (0 = first enumerated display).
    pub fn new(display_index: usize) -> Result<Self, RecordingError> {
        let display = Display::all()
            .map_err(map_io)?
            .into_iter()
            .nth(display_index)
            .ok_or(RecordingError::NoDisplay)?;
        Self::from_display(display)
    }

    /// Capture the primary display.
    pub fn primary() -> Result<Self, RecordingError> {
        Self::from_display(Display::primary().map_err(map_io)?)
    }

    fn from_display(display: Display) -> Result<Self, RecordingError> {
        let capturer = Capturer::new(display).map_err(map_io)?;
        let width = capturer.width();
        let height = capturer.height();
        Ok(Self {
            capturer,
            width,
            height,
        })
    }

    /// Display width in pixels.
    pub fn width(&self) -> usize {
        self.width
    }

    /// Display height in pixels.
    pub fn height(&self) -> usize {
        self.height
    }

    /// Capture one BGRA frame, polling past scrap's `WouldBlock` up to `timeout`.
    ///
    /// Note: scrap may return rows padded to a platform-specific stride; callers
    /// that need exactly `width*height*4` bytes should account for that. On X11
    /// the buffer is tightly packed.
    pub fn capture_frame(&mut self, timeout: Duration) -> Result<Vec<u8>, RecordingError> {
        let deadline = Instant::now() + timeout;
        loop {
            match self.capturer.frame() {
                Ok(frame) => return Ok(frame.to_vec()),
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return Err(RecordingError::Internal(
                            "screen frame capture timed out".to_string(),
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(e) => return Err(map_io(e)),
            }
        }
    }
}

/// Map a scrap/OS I/O error onto the right [`RecordingError`].
fn map_io(e: std::io::Error) -> RecordingError {
    match e.kind() {
        ErrorKind::PermissionDenied => RecordingError::PermissionDenied,
        ErrorKind::NotFound => RecordingError::NoDisplay,
        _ => RecordingError::Internal(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_permission_denied() {
        let e = std::io::Error::from(ErrorKind::PermissionDenied);
        assert!(matches!(map_io(e), RecordingError::PermissionDenied));
    }

    #[test]
    fn maps_not_found_to_no_display() {
        let e = std::io::Error::from(ErrorKind::NotFound);
        assert!(matches!(map_io(e), RecordingError::NoDisplay));
    }

    #[test]
    fn maps_other_to_internal() {
        let e = std::io::Error::other("boom");
        assert!(matches!(map_io(e), RecordingError::Internal(_)));
    }
}
