//! The window-consumed source: one named window, not the whole screen.
//!
//! Composed over an INNER source rather than bound to a display: a window on a
//! screen and a window inside a captured stream crop identically, so the
//! cropping logic should not care which it is. This is D014-1 applied one level
//! down — a new inner kind needs no change here.

use super::{Availability, CaptureSource, SourceError, SourceFrame, SourceIdentity};
use crate::regions::Region;
use crate::target::model::PixelRect;

/// Where a named window is, right now.
///
/// The three states exist because collapsing them is a real failure: a
/// minimised window read as `Gone` stops capture on a window the user will
/// restore in a minute, and a closed window read as `Minimised` retries forever
/// (FR-113).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowState {
    /// On screen, at this rectangle.
    Visible(PixelRect),
    /// Managed but not showing pixels — minimised, or on another workspace.
    Minimised,
    /// No longer managed: closed or quit.
    Gone,
}

/// Finds a named window. Injectable so the loop's behaviour is testable
/// without a window manager — every state below is otherwise unreachable in a
/// test, which is how they go unverified (013/R36).
pub trait WindowLocator: Send {
    fn locate(&self, label: &str) -> WindowState;
}

/// The real locator: the X11 WM provider the region cascade already uses.
pub struct WmLocator;

impl WindowLocator for WmLocator {
    fn locate(&self, label: &str) -> WindowState {
        let windows = match crate::regions::providers::wm::WmProvider::window_states() {
            Ok(w) => w,
            // The WM could not be asked. That is NOT evidence the window is
            // gone — treating it as `Gone` would retire a live source on a
            // transient X error, permanently.
            Err(_) => return WindowState::Minimised,
        };
        for r in windows {
            if r.label.as_deref().is_some_and(|l| l.contains(label)) {
                // The EWMH state, not the geometry: X11 keeps a minimised
                // window's last rectangle and `_NET_CLIENT_LIST` keeps the
                // window (measured live, 2026-08-29 — geometry unchanged
                // 184x69 while `_NET_WM_STATE_HIDDEN` was set). Judging by
                // bbox alone reported every minimised window Visible, and the
                // source then cropped the screen at the stale rectangle —
                // recording whatever was UNDERNEATH it (FR-114), with
                // `Minimised` unreachable outside the test harness.
                if !r.showing {
                    return WindowState::Minimised;
                }
                // A zero-area window is managed but showing nothing, which is
                // what a minimised window looks like through this API.
                if r.bbox.w == 0 || r.bbox.h == 0 {
                    return WindowState::Minimised;
                }
                return WindowState::Visible(r.bbox);
            }
        }
        WindowState::Gone
    }
}

/// One named window, cropped out of an inner source's frames.
pub struct WindowSource {
    inner: Box<dyn CaptureSource>,
    locator: Box<dyn WindowLocator>,
    label: String,
    ordinal: u32,
    state: Availability,
}

impl WindowSource {
    /// Watch the window whose title or class contains `label`.
    pub fn new(
        inner: Box<dyn CaptureSource>,
        locator: Box<dyn WindowLocator>,
        label: impl Into<String>,
        ordinal: u32,
    ) -> Self {
        Self {
            inner,
            locator,
            label: label.into(),
            ordinal,
            state: Availability::Available,
        }
    }

    /// The window's current rectangle, or the availability that explains why
    /// there is none.
    fn rect(&self) -> Result<PixelRect, Availability> {
        match self.locator.locate(&self.label) {
            WindowState::Visible(r) => Ok(r),
            WindowState::Minimised => Err(Availability::Occluded),
            WindowState::Gone => Err(Availability::Ended),
        }
    }
}

impl CaptureSource for WindowSource {
    fn next_frame(&mut self) -> Result<SourceFrame, SourceError> {
        let rect = match self.rect() {
            Ok(r) => r,
            Err(a) => {
                self.state = a;
                return Err(SourceError::new(format!(
                    "window {:?} is {a:?}",
                    self.label
                )));
            }
        };
        let frame = self.inner.next_frame().inspect_err(|_| {
            // The window is there; the INNER source failed. Retryable, and not
            // the window's fault — reporting Ended here would retire a live
            // window because a screen grab hiccuped.
            self.state = Availability::Occluded;
        })?;
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
                // The window's rectangle does not fit the frame — it moved
                // partly off-screen, or onto another display. Retryable.
                self.state = Availability::Occluded;
                Err(SourceError::new(format!("cropping window {:?}: {e}", self.label)))
            }
        }
    }

    fn regions_for(&self, frame: &SourceFrame) -> Option<Vec<Region>> {
        let rect = self.rect().ok()?;
        let inner = self.inner.regions_for(frame)?;
        // Clip to the window and translate to WINDOW-LOCAL coordinates: the
        // frame handed downstream is the crop, so a region in screen
        // coordinates would address the wrong pixels of it. `clip_regions_to`
        // also draws the D014-3 line — inner regions with NO overlap at all
        // yield `None` (this source cannot answer for its crop), never
        // `Some(vec![])` (a claim the cascade looked here and found nothing).
        super::clip_regions_to(inner, rect)
    }

    fn availability(&self) -> Availability {
        // Ask the locator, so a window that closed between frames is seen
        // without waiting for the next failed capture.
        match self.rect() {
            Ok(_) => self.state,
            Err(a) => a,
        }
    }

    fn identity(&self) -> SourceIdentity {
        // The LABEL, never the rectangle: a window dragged or resized is the
        // same window, and position is not identity (013/R30).
        SourceIdentity::new("window", self.label.clone())
    }

    fn ordinal(&self) -> u32 {
        self.ordinal
    }
}
