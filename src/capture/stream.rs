//! Capture a single frame from a live stream URL (RTSP / HTTP / SRT — e.g. a
//! Blackmagic ATEM output) via FFmpeg. Saves a PNG and reports its dimensions.
//!
//! Reconstructed 2026-05-28: the original `capture_stream_frame` tool was lost in
//! the disaster; only its output type survived in `mcp/server.rs.partial`. The
//! capture logic here is authored against that contract (grab one frame → PNG →
//! probe dimensions). The argument builder and dimension parser are pure +
//! unit-tested; the FFmpeg call needs a reachable stream and is integration-only.

use crate::contracts::errors::RecordingError;
use chrono::Utc;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Metadata for a frame grabbed from a stream.
#[derive(Debug, Clone)]
pub struct StreamFrame {
    pub file_path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub file_size_bytes: u64,
    pub stream_url: String,
    /// ISO 8601 capture time.
    pub captured_at: String,
}

/// Grab one frame from `stream_url` into `output_dir` as a PNG.
pub fn capture_stream_frame(
    stream_url: &str,
    output_dir: &Path,
) -> Result<StreamFrame, RecordingError> {
    if stream_url.trim().is_empty() {
        return Err(RecordingError::InvalidConfig("stream_url is empty".to_string()));
    }
    std::fs::create_dir_all(output_dir).map_err(RecordingError::StorageError)?;
    let captured_at = Utc::now().to_rfc3339();
    let out_path = output_dir.join(format!("stream_{}.png", Utc::now().format("%Y%m%dT%H%M%SZ")));

    let args = build_ffmpeg_args(stream_url, &out_path);
    let output = Command::new("ffmpeg")
        .args(&args)
        .output()
        .map_err(|e| RecordingError::EncoderError(format!("failed to run ffmpeg: {e}")))?;
    if !output.status.success() {
        return Err(RecordingError::EncoderError(format!(
            "stream capture failed: {}",
            String::from_utf8_lossy(&output.stderr)
                .lines()
                .last()
                .unwrap_or("ffmpeg exited non-zero")
        )));
    }

    let (width, height) = probe_dimensions(&out_path).unwrap_or((0, 0));
    let file_size_bytes = std::fs::metadata(&out_path).map(|m| m.len()).unwrap_or(0);
    Ok(StreamFrame {
        file_path: out_path,
        width,
        height,
        file_size_bytes,
        stream_url: stream_url.to_string(),
        captured_at,
    })
}

/// Build the ffmpeg args to grab a single frame from a stream into `out`.
fn build_ffmpeg_args(stream_url: &str, out: &Path) -> Vec<String> {
    vec![
        "-y".to_string(),
        "-loglevel".to_string(),
        "error".to_string(),
        // Prefer TCP for RTSP (more reliable than the default UDP); ignored by
        // other protocols.
        "-rtsp_transport".to_string(),
        "tcp".to_string(),
        "-i".to_string(),
        stream_url.to_string(),
        "-frames:v".to_string(),
        "1".to_string(),
        out.to_string_lossy().to_string(),
    ]
}

/// Probe a PNG's pixel dimensions via ffprobe.
fn probe_dimensions(path: &Path) -> Option<(u32, u32)> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height",
            "-of",
            "csv=p=0:s=x",
            &path.to_string_lossy(),
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_dimensions(&String::from_utf8_lossy(&output.stdout))
}

/// Parse ffprobe `width x height` CSV output (e.g. `1920x1080`).
fn parse_dimensions(s: &str) -> Option<(u32, u32)> {
    let (w, h) = s.trim().split_once('x')?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ffmpeg_args_grab_one_frame() {
        let args = build_ffmpeg_args("rtsp://cam/live", Path::new("/tmp/f.png"));
        assert!(args.windows(2).any(|w| w == ["-i", "rtsp://cam/live"]));
        assert!(args.windows(2).any(|w| w == ["-frames:v", "1"]));
        assert!(args.windows(2).any(|w| w == ["-rtsp_transport", "tcp"]));
        assert_eq!(args.last().unwrap(), "/tmp/f.png");
    }

    #[test]
    fn parses_ffprobe_dimensions() {
        assert_eq!(parse_dimensions("1920x1080\n"), Some((1920, 1080)));
        assert_eq!(parse_dimensions("  640x480 "), Some((640, 480)));
        assert_eq!(parse_dimensions("garbage"), None);
        assert_eq!(parse_dimensions(""), None);
    }

    #[test]
    fn empty_url_is_rejected() {
        let err = capture_stream_frame("  ", Path::new("/tmp")).unwrap_err();
        assert!(matches!(err, RecordingError::InvalidConfig(_)));
    }
}
