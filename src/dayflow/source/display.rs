//! The display-consumed source: a whole screen.
//!
//! This is the kind Dayflow had before the abstraction existed, expressed
//! through [`CaptureSource`] with no change to what it produces. It wraps the
//! same `capture::screen::ScreenCapturer` path the live test drives, so an
//! existing session keeps its sample filenames, its `display_id` values and its
//! regions (D014-2: the trait's `ordinal` occupies the `display_id` position
//! that a display index already filled).

use super::{Availability, CaptureSource, SourceError, SourceFrame, SourceIdentity};
use crate::capture::screen::ScreenCapturer;
use crate::regions::{Granularity, Region};

/// A whole display, sampled through the platform screen capturer.
pub struct DisplaySource {
    index: u32,
    capturer: ScreenCapturer,
    /// How deep to ask the region cascade. `Window` is the cheap default: the
    /// perception budget is derived from regions per sample, so a deeper
    /// granularity here silently raises the cost of every segment.
    depth: Granularity,
    /// Set once a capture fails in a way that cannot recover.
    ended: bool,
    /// Set when the last capture attempt failed but may recover.
    occluded: bool,
    timeout: std::time::Duration,
}

impl DisplaySource {
    /// Open display `index` for capture.
    pub fn new(index: u32) -> Result<Self, SourceError> {
        let capturer = ScreenCapturer::new(index as usize)
            .map_err(|e| SourceError::new(format!("cannot open display {index}: {e}")))?;
        Ok(Self {
            index,
            capturer,
            depth: Granularity::Window,
            ended: false,
            occluded: false,
            timeout: std::time::Duration::from_secs(2),
        })
    }

    /// Ask the cascade to this depth instead of `Window`.
    pub fn with_depth(mut self, depth: Granularity) -> Self {
        self.depth = depth;
        self
    }

    /// Repack a capture into tightly-packed BGRA.
    ///
    /// The platform capturer may return padded rows (stride > width * 4); the
    /// sampler's frame contract is tightly packed, and handing it a padded
    /// buffer produces a sheared image that still passes every dimension check.
    fn tightly_packed(frame: &[u8], width: usize, height: usize) -> Vec<u8> {
        let stride = frame.len().checked_div(height).unwrap_or(width * 4);
        let row = width * 4;
        if stride == row {
            return frame.to_vec();
        }
        let mut out = Vec::with_capacity(row * height);
        for y in 0..height {
            let start = y * stride;
            if start + row > frame.len() {
                break;
            }
            out.extend_from_slice(&frame[start..start + row]);
        }
        out
    }
}

impl CaptureSource for DisplaySource {
    fn next_frame(&mut self) -> Result<SourceFrame, SourceError> {
        if self.ended {
            return Err(SourceError::new(format!("display {} has ended", self.index)));
        }
        let (w, h) = (self.capturer.width(), self.capturer.height());
        match self.capturer.capture_frame(self.timeout) {
            Ok(raw) => {
                self.occluded = false;
                Ok(SourceFrame {
                    bgra: Self::tightly_packed(&raw, w, h),
                    width: w as u32,
                    height: h as u32,
                })
            }
            Err(e) => {
                // A failed grab is treated as occlusion, not as the end. A
                // display is only Ended when it is gone from the system, which
                // this call cannot distinguish — and guessing Ended would stop
                // retrying a screen that was merely asleep.
                self.occluded = true;
                Err(SourceError::new(format!("display {}: {e}", self.index)))
            }
        }
    }

    fn regions_for(&self, _frame: &SourceFrame) -> Option<Vec<Region>> {
        let regions: Vec<Region> = crate::regions::detect(self.depth)
            .into_iter()
            .filter(|r| r.display_id == self.index)
            .collect();
        // Empty is NOT the same as absent. An empty vec says the cascade ran and
        // found nothing; `None` says there was no cascade to ask. Only the
        // latter is a whole-frame read, and conflating them would hide it from
        // `samples_read_whole` (D014-3, FR-103).
        Some(regions)
    }

    fn availability(&self) -> Availability {
        if self.ended {
            Availability::Ended
        } else if self.occluded {
            Availability::Occluded
        } else {
            Availability::Available
        }
    }

    fn identity(&self) -> SourceIdentity {
        SourceIdentity::new("display", self.index.to_string())
    }

    fn ordinal(&self) -> u32 {
        self.index
    }
}
