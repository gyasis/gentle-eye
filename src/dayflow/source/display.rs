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

    /// Keep only the regions the cascade can attribute to display `index`,
    /// or admit that it cannot answer.
    ///
    /// - Regions tagged for this display are kept: the cascade ran and these
    ///   are its findings. An EMPTY keep with an empty input stays `Some(vec![])`
    ///   — the cascade ran and found nothing anywhere.
    /// - When the cascade produced regions but NONE are attributable to this
    ///   display, the answer is `None`, not `Some(vec![])`. Under the current
    ///   producer gap (`Region::display_id`: the WM provider tags everything
    ///   `0`, root coordinates) this branch fires on every frame for any
    ///   display other than 0 — and `Some(vec![])` would claim "the cascade
    ///   looked at this display and found nothing", hiding the whole-frame
    ///   read from `samples_read_whole` forever (FR-103). `None` is the honest
    ///   report the counter exists to receive. T010/T011 own fixing the
    ///   producer; this keeps the source honest until they do.
    fn select_regions(all: Vec<Region>, index: u32) -> Option<Vec<Region>> {
        let any_elsewhere = all.iter().any(|r| r.display_id != index);
        let mine: Vec<Region> = all.into_iter().filter(|r| r.display_id == index).collect();
        if mine.is_empty() && any_elsewhere {
            return None;
        }
        Some(mine)
    }
}

/// Repack a capture into tightly-packed BGRA.
///
/// The platform capturer may return padded rows (stride > width * 4); the
/// sampler's frame contract is tightly packed, and handing it a padded
/// buffer produces a sheared image that still passes every dimension check.
///
/// Public because it is the ONE packing implementation: `tests/dayflow_live.rs`
/// used to carry its own copy, and the two had already drifted (this one
/// bounds-checks a short final row; the copy did not) — the "duplication
/// relocated, not removed" defect 013 flagged twice (R40).
pub fn tightly_packed(frame: &[u8], width: usize, height: usize) -> Vec<u8> {
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
                    bgra: tightly_packed(&raw, w, h),
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
        // Empty is NOT the same as absent. An empty vec says the cascade ran and
        // found nothing; `None` says it could not answer for this display. Only
        // the latter is a whole-frame read, and conflating them would hide it
        // from `samples_read_whole` (D014-3, FR-103). `select_regions` draws
        // that line — see its doc for the producer gap it defends against.
        Self::select_regions(crate::regions::detect(self.depth), self.index)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::regions::Source;
    use crate::target::model::PixelRect;

    fn region_on(display: u32) -> Region {
        Region::new(
            PixelRect { x: 10, y: 10, w: 100, h: 100 },
            Source::Wm,
            Granularity::Window,
            1.0,
        )
        .on_display(display)
    }

    #[test]
    fn regions_tagged_for_this_display_are_kept_and_others_are_not() {
        let picked = DisplaySource::select_regions(vec![region_on(1), region_on(0)], 1)
            .expect("the cascade answered for display 1");
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].display_id, 1, "only display 1's region survives");
    }

    #[test]
    fn a_cascade_that_found_nothing_anywhere_is_an_empty_answer_not_an_absent_one() {
        // Some(vec![]) — the cascade ran and there was nothing to find. This is
        // NOT counted as a whole-frame read (D014-3).
        assert_eq!(DisplaySource::select_regions(vec![], 3), Some(vec![]));
    }

    #[test]
    fn a_cascade_that_cannot_attribute_anything_to_this_display_says_none() {
        // The producer gap in force today: every region tagged display 0, and
        // the source is display 2. Some(vec![]) would claim "looked here, found
        // nothing" and hide the whole-frame read from samples_read_whole on
        // every non-zero display forever (FR-103). None is the honest answer.
        let got = DisplaySource::select_regions(vec![region_on(0), region_on(0)], 2);
        assert_eq!(got, None, "unattributable regions must not read as 'found nothing here'");
    }
}
