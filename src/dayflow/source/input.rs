//! The input-taken source: a stream or capture device.
//!
//! The co-equal half of the pair. Content here may never have been rendered on
//! this machine's screen — a capture card, an IP camera, an encoder feed — so
//! nothing about it can assume a display, a window manager, or a local desktop.
//!
//! This is the source kind that proves the abstraction: if the loop still works
//! against an input it never rendered, `CaptureSource` really is the seam
//! (FR-111a).

use std::path::PathBuf;

use super::{Availability, CaptureSource, SourceError, SourceFrame, SourceIdentity};
use crate::regions::Region;

/// Grabs one frame from an input URL. Injectable because the real
/// implementation shells out to ffmpeg against a live stream — untestable
/// without one, which is how a source kind ends up shipped unverified.
pub trait FrameGrabber: Send {
    /// Grab one frame as tightly-packed BGRA.
    fn grab(&mut self, url: &str) -> Result<SourceFrame, SourceError>;
}

/// The real grabber: one ffmpeg frame per call, decoded back to BGRA.
pub struct FfmpegGrabber {
    scratch: PathBuf,
}

impl FfmpegGrabber {
    /// Write intermediate frames under `scratch`.
    pub fn new(scratch: impl Into<PathBuf>) -> Self {
        Self { scratch: scratch.into() }
    }
}

impl FrameGrabber for FfmpegGrabber {
    fn grab(&mut self, url: &str) -> Result<SourceFrame, SourceError> {
        let shot = crate::capture::stream::capture_stream_frame(url, &self.scratch)
            .map_err(|e| SourceError::new(format!("stream grab failed: {e}")))?;
        let frame = decode_bgra(&shot.file_path);
        // The intermediate PNG has served its purpose; leaving one per tick
        // would grow without bound beside the samples that retention manages.
        // Removed on the error path too — a frame that failed to decode is
        // still a file on disk.
        let _ = std::fs::remove_file(&shot.file_path);
        frame
    }
}

/// Decode a grabbed image file to the tightly-packed BGRA the sampler expects.
///
/// Factored out of [`FfmpegGrabber::grab`] so the byte-order conversion is
/// testable without a live stream: the sampler's contract is BGRA
/// (`downscale_gray` reads `[o]=B, [o+1]=G, [o+2]=R`), and a channel-swapped
/// frame still gates, crops and summarises — it just describes the wrong
/// colours, silently. This function is the only place that guarantee is made.
fn decode_bgra(path: &std::path::Path) -> Result<SourceFrame, SourceError> {
    let img = image::open(path)
        .map_err(|e| SourceError::new(format!("decoding the grabbed frame: {e}")))?
        .to_rgba8();
    let (w, h) = (img.width(), img.height());
    // RGBA -> BGRA: swap the R and B bytes of every pixel.
    let mut bgra = img.into_raw();
    for px in bgra.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    Ok(SourceFrame { bgra, width: w, height: h })
}

/// A stream or capture device as a Dayflow source.
pub struct InputSource {
    url: String,
    grabber: Box<dyn FrameGrabber>,
    ordinal: u32,
    consecutive_failures: u32,
    give_up_after: u32,
}

impl InputSource {
    /// Watch `url`, grabbing frames with `grabber`.
    pub fn new(url: impl Into<String>, grabber: Box<dyn FrameGrabber>, ordinal: u32) -> Self {
        Self {
            url: url.into(),
            grabber,
            ordinal,
            consecutive_failures: 0,
            give_up_after: 10,
        }
    }

    /// How many consecutive failures before the input is treated as ended.
    pub fn with_give_up_after(mut self, n: u32) -> Self {
        self.give_up_after = n;
        self
    }
}

impl CaptureSource for InputSource {
    fn next_frame(&mut self) -> Result<SourceFrame, SourceError> {
        match self.grabber.grab(&self.url) {
            Ok(f) => {
                self.consecutive_failures = 0;
                Ok(f)
            }
            Err(e) => {
                self.consecutive_failures = self.consecutive_failures.saturating_add(1);
                Err(e)
            }
        }
    }

    fn regions_for(&self, _frame: &SourceFrame) -> Option<Vec<Region>> {
        // HONESTLY None. There is no window manager to ask about content that
        // was never on this desktop, and no accessibility tree behind a video
        // feed. Synthesising a whole-frame region here would be
        // indistinguishable from a real detection and would hide the
        // whole-frame read from `samples_read_whole` — the degradation would
        // become invisible exactly where it is guaranteed to happen
        // (contract, D014-3, FR-103).
        None
    }

    fn availability(&self) -> Availability {
        // A failed grab is NOT proof a stream is finished: an encoder restart,
        // a flapping network or a camera waking up all look identical to one
        // ffmpeg failure. So a failure is Occluded and retried.
        //
        // But retrying forever is its own bug — a permanently dead URL would
        // spend an ffmpeg invocation every tick for the rest of the day and
        // report a source that is "temporarily" unavailable until midnight.
        // The threshold is a stated heuristic, not a fact about the stream,
        // and it is the reason `give_up_after` is a knob rather than a
        // constant.
        if self.consecutive_failures >= self.give_up_after {
            Availability::Ended
        } else if self.consecutive_failures > 0 {
            Availability::Occluded
        } else {
            Availability::Available
        }
    }

    fn identity(&self) -> SourceIdentity {
        // The URL. A stream that reconnects to the same address is the same
        // input, and a day's record must not split across a reconnect.
        SourceIdentity::new("input", self.url.clone())
    }

    fn ordinal(&self) -> u32 {
        self.ordinal
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The decode path must deliver BGRA — the sampler reads `[0]=B, [1]=G,
    /// [2]=R` (`downscale_gray`, `write_png`). A pure-red pixel that comes back
    /// `[255, 0, 0, 255]` means the swap was skipped and every colour the
    /// summariser ever describes is wrong, silently: nothing downstream can
    /// detect it, so this test is the only guard.
    #[test]
    fn decoded_frames_are_bgra_not_rgba() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("known.png");
        // Left pixel pure red, right pixel pure blue — the two colours the
        // swap confuses. Green survives any 0<->2 mistake, so it pins lane 1.
        let img = image::RgbaImage::from_fn(2, 1, |x, _| {
            if x == 0 { image::Rgba([255, 0, 0, 255]) } else { image::Rgba([0, 0, 255, 255]) }
        });
        img.save(&path).unwrap();

        let frame = decode_bgra(&path).expect("decodes");
        assert_eq!((frame.width, frame.height), (2, 1));
        assert_eq!(
            frame.bgra,
            vec![
                0, 0, 255, 255, // red pixel as B,G,R,A
                255, 0, 0, 255, // blue pixel as B,G,R,A
            ],
            "the decode must emit BGRA — an RGBA frame here describes the wrong \
             colours for the rest of the pipeline, silently"
        );
    }

    /// A file that is not an image is an error, not a panic and not a frame.
    #[test]
    fn a_garbage_grab_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("not-an-image.png");
        std::fs::write(&path, b"ffmpeg wrote half a header and died").unwrap();
        let err = decode_bgra(&path).expect_err("garbage must not decode");
        assert!(err.detail.contains("decoding"), "the error names the stage: {}", err.detail);
    }
}
