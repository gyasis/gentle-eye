//! A named target as a capture source.
//!
//! A target is a persisted, normalised region-of-interest over a display or a
//! stream — the "watch this one panel" primitive gentle-eye already has. As a
//! source it is an inner source plus a fixed crop, so a QA session can point
//! Dayflow at one region and get a day's record of only that.

use super::{Availability, CaptureSource, SourceError, SourceFrame, SourceIdentity};
use crate::regions::Region;
use crate::target::geometry::norm_to_pixel;
use crate::target::model::{NormRect, PixelRect, Target};

/// A named target, cropped out of an inner source's frames.
pub struct NamedTargetSource {
    inner: Box<dyn CaptureSource>,
    name: String,
    region: NormRect,
    ordinal: u32,
    state: Availability,
}

impl NamedTargetSource {
    /// Watch `target`, taking frames from `inner`.
    ///
    /// The caller supplies the inner source because a target names WHERE it
    /// came from (`TargetSource::Display` / `Stream`) but does not own the
    /// capture — and resolving it here would make this type know about every
    /// source kind, which is the enum shape D014-1 rejects.
    pub fn new(inner: Box<dyn CaptureSource>, target: &Target, ordinal: u32) -> Self {
        Self {
            inner,
            name: target.name.clone(),
            region: target.region,
            ordinal,
            state: Availability::Available,
        }
    }

    /// The target's rectangle in this frame's pixels.
    ///
    /// Resolved PER FRAME from the normalised region rather than cached: the
    /// whole point of storing a target normalised is that it survives a
    /// resolution change, and a cached pixel rect would silently address the
    /// wrong area after one.
    fn rect_for(&self, frame: &SourceFrame) -> PixelRect {
        norm_to_pixel(self.region, (frame.width, frame.height), (0, 0))
    }
}

impl CaptureSource for NamedTargetSource {
    fn next_frame(&mut self) -> Result<SourceFrame, SourceError> {
        let frame = self.inner.next_frame().inspect_err(|_| {
            self.state = self.inner.availability();
        })?;
        let rect = self.rect_for(&frame);
        let stride = frame.width as usize * 4;
        match crate::target::crop::crop_bgra(
            &frame.bgra,
            frame.width as usize,
            frame.height as usize,
            stride,
            rect,
        ) {
            Ok((bgra, w, h)) => {
                self.state = Availability::Available;
                Ok(SourceFrame { bgra, width: w, height: h })
            }
            Err(e) => {
                self.state = Availability::Occluded;
                Err(SourceError::new(format!("cropping target {:?}: {e}", self.name)))
            }
        }
    }

    fn regions_for(&self, frame: &SourceFrame) -> Option<Vec<Region>> {
        let inner = self.inner.regions_for(frame)?;
        let rect = self.rect_for(frame);
        // Clip to the target and translate to TARGET-LOCAL coordinates; see
        // `clip_regions_to` for why partial overlaps are clipped rather than
        // dropped, and why dropping EVERYTHING must answer `None` (D014-3).
        super::clip_regions_to(inner, rect)
    }

    fn availability(&self) -> Availability {
        // A target has no existence of its own: it is a rectangle over an
        // inner source, so the inner source's condition IS the target's.
        //
        // Every non-Available inner state passes through, not just `Ended`:
        // `self.state` is only written inside `next_frame`, so an inner source
        // that went Occluded between frames would otherwise be reported from a
        // stale `Available` until the next failed capture — while
        // `WindowSource::availability` re-asks its locator live. Two sources
        // answering the same trait question with different freshness is how a
        // status surface ends up trusting one and not the other.
        match self.inner.availability() {
            Availability::Available => self.state,
            unavailable => unavailable,
        }
    }

    fn identity(&self) -> SourceIdentity {
        // The target's NAME. Its rectangle can be edited and it remains the
        // same target; a day's record must not split when the user nudges it.
        SourceIdentity::new("target", self.name.clone())
    }

    fn ordinal(&self) -> u32 {
        self.ordinal
    }
}
