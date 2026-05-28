//! FFmpeg pipe encoder: raw BGRA frames (stdin) → H.264 MP4.
//!
//! Spawns `ffmpeg` reading `rawvideo` from stdin and muxing to MP4. The argument
//! construction is a pure, unit-tested function; the spawn / write / finalize
//! path requires the `ffmpeg` binary and is integration-tested.
//!
//! Authored 2026-05-28 from PRD §Capture (FFmpeg encoding, `EncoderMode`) — the
//! recovered source was binary garbage.

use crate::contracts::errors::RecordingError;
use std::io::Write;
use std::path::Path;
use std::process::{Child, Command, Stdio};

/// An in-progress FFmpeg encode fed one raw BGRA frame at a time.
pub struct PipeEncoder {
    child: Child,
    width: u32,
    height: u32,
    frames_written: u64,
}

impl PipeEncoder {
    /// Spawn ffmpeg to encode `width`x`height` BGRA frames at `fps` into `output`.
    pub fn start(
        width: u32,
        height: u32,
        fps: u32,
        output: &Path,
    ) -> Result<Self, RecordingError> {
        let args = build_ffmpeg_args(width, height, fps, output);
        let child = Command::new("ffmpeg")
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| RecordingError::EncoderError(format!("failed to spawn ffmpeg: {e}")))?;
        Ok(Self {
            child,
            width,
            height,
            frames_written: 0,
        })
    }

    /// Expected byte length of one BGRA frame.
    pub fn frame_len(&self) -> usize {
        self.width as usize * self.height as usize * 4
    }

    /// Number of frames written so far.
    pub fn frames_written(&self) -> u64 {
        self.frames_written
    }

    /// Write one raw BGRA frame to ffmpeg's stdin.
    pub fn write_frame(&mut self, bgra: &[u8]) -> Result<(), RecordingError> {
        if bgra.len() != self.frame_len() {
            return Err(RecordingError::EncoderError(format!(
                "frame size mismatch: got {} bytes, expected {}",
                bgra.len(),
                self.frame_len()
            )));
        }
        let stdin = self
            .child
            .stdin
            .as_mut()
            .ok_or_else(|| RecordingError::EncoderError("ffmpeg stdin closed".to_string()))?;
        stdin
            .write_all(bgra)
            .map_err(|e| RecordingError::EncoderError(format!("write to ffmpeg failed: {e}")))?;
        self.frames_written += 1;
        Ok(())
    }

    /// Close stdin and wait for ffmpeg to finish muxing the file.
    pub fn finish(mut self) -> Result<(), RecordingError> {
        // Dropping stdin signals EOF to ffmpeg so it can finalize the container.
        drop(self.child.stdin.take());
        let output = self
            .child
            .wait_with_output()
            .map_err(|e| RecordingError::EncoderError(format!("ffmpeg wait failed: {e}")))?;
        if !output.status.success() {
            return Err(RecordingError::EncoderError(
                String::from_utf8_lossy(&output.stderr)
                    .lines()
                    .last()
                    .unwrap_or("ffmpeg exited with a non-zero status")
                    .to_string(),
            ));
        }
        Ok(())
    }
}

/// Build the ffmpeg CLI arguments for a raw-BGRA-stdin → H.264 MP4 encode.
fn build_ffmpeg_args(width: u32, height: u32, fps: u32, output: &Path) -> Vec<String> {
    vec![
        "-y".to_string(),
        // Input: raw video from stdin.
        "-f".to_string(),
        "rawvideo".to_string(),
        "-pixel_format".to_string(),
        "bgra".to_string(),
        "-video_size".to_string(),
        format!("{width}x{height}"),
        "-framerate".to_string(),
        fps.max(1).to_string(),
        "-i".to_string(),
        "-".to_string(),
        // Output: H.264, broadly-compatible pixel format, low-latency preset.
        "-c:v".to_string(),
        "libx264".to_string(),
        "-pix_fmt".to_string(),
        "yuv420p".to_string(),
        "-preset".to_string(),
        "ultrafast".to_string(),
        output.to_string_lossy().to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ffmpeg_args_describe_raw_bgra_input() {
        let args = build_ffmpeg_args(1920, 1080, 5, Path::new("/tmp/out.mp4"));
        assert!(args.windows(2).any(|w| w == ["-f", "rawvideo"]));
        assert!(args.windows(2).any(|w| w == ["-pixel_format", "bgra"]));
        assert!(args.windows(2).any(|w| w == ["-video_size", "1920x1080"]));
        assert!(args.windows(2).any(|w| w == ["-framerate", "5"]));
        assert!(args.windows(2).any(|w| w == ["-c:v", "libx264"]));
        assert_eq!(args.last().unwrap(), "/tmp/out.mp4");
    }

    #[test]
    fn fps_is_floored_to_one() {
        let args = build_ffmpeg_args(640, 480, 0, Path::new("/tmp/x.mp4"));
        assert!(args.windows(2).any(|w| w == ["-framerate", "1"]));
    }
}
