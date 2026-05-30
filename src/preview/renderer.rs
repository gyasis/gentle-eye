//! PV4 — pluggable preview renderer.
//!
//! Default backend = **ffplay** (subprocess, zero new crates). An opt-in
//! pure-Rust window backend (`winit`+`softbuffer`) lives behind the
//! off-by-default `richwindow` feature — its 75-crate tree is NOT built unless
//! requested (supply-chain default; same pattern as opencv `tracking`). An
//! opencv-highgui backend can be reused under `--features tracking` (see
//! `docs/PREVIEW.md`); it is never added for preview alone.

use super::discover::CaptureKind;
use super::errors::PreviewError;
use super::player::{open_with_player, PlaybackOpts};
use std::path::Path;

/// A backend that can render a captured file for preview.
pub trait PreviewRenderer {
    fn show_file(
        &self,
        path: &Path,
        kind: CaptureKind,
        opts: &PlaybackOpts,
    ) -> Result<(), PreviewError>;
}

/// Default renderer: ffplay subprocess (reuses ffmpeg; OS-open fallback). 0 crates.
#[derive(Debug, Default, Clone, Copy)]
pub struct FfplayRenderer;

impl PreviewRenderer for FfplayRenderer {
    fn show_file(
        &self,
        path: &Path,
        kind: CaptureKind,
        opts: &PlaybackOpts,
    ) -> Result<(), PreviewError> {
        open_with_player(path, kind, opts).map(|_| ())
    }
}

/// Opt-in pure-Rust window backend (winit + softbuffer) — present ONLY with
/// `--features richwindow` (NOT built by default). This is the seam where
/// agent-controlled multi-monitor placement + frame-blitting land.
#[cfg(feature = "richwindow")]
pub mod rich {
    //! winit + softbuffer backend. Requires `--features richwindow` (+ the 75-crate
    //! tree). Intentionally a minimal scaffold — the default build never compiles
    //! this, keeping the default trust surface at zero new crates.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ffplay_renderer_errors_on_missing_file() {
        let r = FfplayRenderer;
        let e = r.show_file(Path::new("/no/such.png"), CaptureKind::Image, &PlaybackOpts::default());
        assert!(e.is_err());
    }
}
