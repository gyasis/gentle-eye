//! Phase 3 (DEFERRED): real-time region tracking — follow a moving window.
//!
//! Per PRD §3, `opencv` is deferred: its cost is the **system/build dependency**
//! (`libopencv-dev`, C++ linkage, cross-platform brittleness), and static
//! screens need no tracking. So the default build ships only the trait + a no-op
//! tracker; the opencv-backed implementation lives behind the off-by-default
//! `tracking` cargo feature (`cargo build --features tracking`, needs
//! `libopencv-dev`). Until there's real motion to follow, nothing here is wired
//! into the capture path.

use super::model::PixelRect;

/// Tracks a region of interest across frames.
pub trait RegionTracker {
    /// Initialize the tracker on the first frame with the region to follow.
    fn init(&mut self, frame_bgra: &[u8], width: u32, height: u32, region: PixelRect);
    /// Update with a new frame; returns the region's new position if still
    /// tracked, or `None` if the target was lost.
    fn update(&mut self, frame_bgra: &[u8], width: u32, height: u32) -> Option<PixelRect>;
}

/// The default tracker compiled when the `tracking` feature is OFF: it holds the
/// last region unchanged (a static screen never moves — PRD §3). This lets the
/// rest of the codebase depend on [`RegionTracker`] without pulling opencv.
#[derive(Debug, Default)]
pub struct NoopTracker {
    last: Option<PixelRect>,
}

impl RegionTracker for NoopTracker {
    fn init(&mut self, _frame_bgra: &[u8], _width: u32, _height: u32, region: PixelRect) {
        self.last = Some(region);
    }
    fn update(&mut self, _frame_bgra: &[u8], _width: u32, _height: u32) -> Option<PixelRect> {
        self.last
    }
}

/// opencv-backed tracker — present only with `--features tracking`.
#[cfg(feature = "tracking")]
mod opencv_impl {
    //! Requires `libopencv-dev`. Deferred per PRD §3 — left as the seam where a
    //! CSRT/KCF tracker would be wired when real motion-tracking is needed.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_tracker_holds_region() {
        let mut t = NoopTracker::default();
        let r = PixelRect { x: 10, y: 20, w: 100, h: 80 };
        t.init(&[], 0, 0, r);
        assert_eq!(t.update(&[], 0, 0), Some(r));
    }
}
