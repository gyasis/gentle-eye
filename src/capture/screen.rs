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
        self.try_frame(timeout)?.ok_or_else(|| {
            RecordingError::Internal("screen frame capture timed out".to_string())
        })
    }

    /// Like [`capture_frame`](Self::capture_frame), but "no new frame before
    /// `timeout`" is `Ok(None)` rather than an error.
    ///
    /// On macOS scrap sits on a display stream that only delivers a frame when
    /// the screen CHANGES, so a still screen yields `WouldBlock` indefinitely:
    /// there `None` means "unchanged", not "failed". Measured on a Mac Studio
    /// (2026-09-26): with a spinner animating, the median gap between frames was
    /// 11 ms, but gaps over 200 ms still happened 3 times in 20 s.
    pub fn try_frame(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>, RecordingError> {
        let deadline = Instant::now() + timeout;
        loop {
            match self.capturer.frame() {
                Ok(frame) => return Ok(Some(frame.to_vec())),
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return Ok(None);
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(e) => return Err(map_io(e)),
            }
        }
    }
}

/// How long a recording waits for its FIRST frame. Missing it is a real failure
/// (no permission, a black or asleep display), not a still screen.
pub const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(2);

/// Holds the most recent frame so a recorder can repeat it while the screen is
/// unchanged. Only the first frame is mandatory.
#[derive(Default)]
pub struct FrameHold {
    last: Option<Vec<u8>>,
    repeats: u64,
}

impl FrameHold {
    /// Feed the result of [`ScreenCapturer::try_frame`]; get the frame to encode.
    pub fn next(&mut self, fresh: Option<Vec<u8>>) -> Result<&[u8], RecordingError> {
        match fresh {
            Some(f) => self.last = Some(f),
            None if self.last.is_some() => self.repeats += 1,
            None => {
                return Err(RecordingError::Internal(format!(
                    "no screen frame within {}s: check Screen Recording permission for the app running gentle-eye",
                    FIRST_FRAME_TIMEOUT.as_secs()
                )))
            }
        }
        Ok(self.last.as_deref().unwrap_or_default())
    }

    /// True until the first frame has arrived.
    pub fn is_empty(&self) -> bool {
        self.last.is_none()
    }

    /// Frames that re-used the previous image because the screen had not changed.
    pub fn repeats(&self) -> u64 {
        self.repeats
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
    fn frame_hold_requires_a_first_frame() {
        let mut h = FrameHold::default();
        assert!(h.is_empty());
        assert!(matches!(h.next(None), Err(RecordingError::Internal(_))));
    }

    #[test]
    fn frame_hold_repeats_last_frame_while_screen_is_unchanged() {
        let mut h = FrameHold::default();
        assert_eq!(h.next(Some(vec![1, 2])).unwrap(), &[1, 2]);
        assert_eq!(h.next(None).unwrap(), &[1, 2]);
        assert_eq!(h.next(None).unwrap(), &[1, 2]);
        assert_eq!(h.repeats(), 2);
        assert_eq!(h.next(Some(vec![3])).unwrap(), &[3]);
        assert_eq!(h.repeats(), 2);
    }

    #[test]
    fn maps_other_to_internal() {
        let e = std::io::Error::other("boom");
        assert!(matches!(map_io(e), RecordingError::Internal(_)));
    }
}
