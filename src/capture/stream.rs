//! Capture a single frame from a live stream URL (RTMP / RTSP / HTTP / SRT —
//! e.g. an OBS or Blackmagic ATEM output) via FFmpeg. Saves a PNG and reports
//! its dimensions.
//!
//! Reconstructed 2026-05-28: the original `capture_stream_frame` tool was lost in
//! the disaster; only its output type survived in `mcp/server.rs.partial`. The
//! capture logic here is authored against that contract (grab one frame → PNG →
//! probe dimensions). The argument builder and dimension parser are pure +
//! unit-tested; the FFmpeg call needs a reachable stream and is integration-only.

use crate::contracts::errors::RecordingError;
use crate::target::model::PixelRect;
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
    capture_stream_frame_cropped(stream_url, output_dir, None)
}

/// Like [`capture_stream_frame`], but applies an optional `crop` pixel rect via
/// an ffmpeg `crop=` filter (used by an active stream target).
pub fn capture_stream_frame_cropped(
    stream_url: &str,
    output_dir: &Path,
    crop: Option<PixelRect>,
) -> Result<StreamFrame, RecordingError> {
    if stream_url.trim().is_empty() {
        return Err(RecordingError::InvalidConfig("stream_url is empty".to_string()));
    }
    std::fs::create_dir_all(output_dir).map_err(RecordingError::StorageError)?;
    let captured_at = Utc::now().to_rfc3339();
    let out_path = output_dir.join(format!("stream_{}.png", Utc::now().format("%Y%m%dT%H%M%SZ")));

    let args = build_ffmpeg_args(stream_url, &out_path, crop);
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
///
/// Works with `rtmp://`, `rtsp://`, `http(s)://`, `srt://`, etc. `-rtsp_transport
/// tcp` is added only for RTSP (it's an RTSP-only option); a best-effort I/O
/// timeout means a dead stream fails fast instead of hanging.
fn build_ffmpeg_args(stream_url: &str, out: &Path, crop: Option<PixelRect>) -> Vec<String> {
    let mut args = vec![
        "-y".to_string(),
        "-loglevel".to_string(),
        "error".to_string(),
        // ~15s I/O timeout (microseconds) so a dead stream errors, not hangs.
        "-rw_timeout".to_string(),
        "15000000".to_string(),
    ];
    // Force TCP for RTSP (more reliable than default UDP). Irrelevant to
    // rtmp/http/srt, so only add it for rtsp:// to avoid an unused option.
    if stream_url.starts_with("rtsp://") {
        args.extend(["-rtsp_transport".to_string(), "tcp".to_string()]);
    }
    args.extend(["-i".to_string(), stream_url.to_string()]);
    // Active target → crop the frame to its pixel rect (`crop=w:h:x:y`). This is
    // an output-side filter, so it must come after `-i`.
    if let Some(r) = crop {
        args.extend([
            "-vf".to_string(),
            format!("crop={}:{}:{}:{}", r.w, r.h, r.x, r.y),
        ]);
    }
    args.extend([
        "-frames:v".to_string(),
        "1".to_string(),
        out.to_string_lossy().to_string(),
    ]);
    args
}

/// Encode a tightly-packed BGRA buffer to a PNG via ffmpeg's `rawvideo`
/// demuxer — no `image`-crate dependency (Phase-1 dep discipline). Used for the
/// `define_target` confirmation image of a display crop.
pub fn write_bgra_png(
    bgra: &[u8],
    width: u32,
    height: u32,
    out: &Path,
) -> Result<(), RecordingError> {
    use std::io::Write;
    use std::process::Stdio;
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).map_err(RecordingError::StorageError)?;
    }
    let mut child = Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "bgra",
            "-s",
            &format!("{width}x{height}"),
            "-i",
            "-",
        ])
        .arg(out)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| RecordingError::EncoderError(format!("failed to run ffmpeg: {e}")))?;
    {
        // Scope the stdin handle so it's dropped (EOF) before we wait.
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| RecordingError::EncoderError("ffmpeg stdin unavailable".into()))?;
        stdin.write_all(bgra).map_err(RecordingError::StorageError)?;
    }
    let status = child.wait().map_err(RecordingError::StorageError)?;
    if !status.success() {
        return Err(RecordingError::EncoderError("ffmpeg PNG encode failed".into()));
    }
    Ok(())
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
    fn rtsp_args_grab_one_frame_over_tcp() {
        let args = build_ffmpeg_args("rtsp://cam/live", Path::new("/tmp/f.png"), None);
        assert!(args.windows(2).any(|w| w == ["-i", "rtsp://cam/live"]));
        assert!(args.windows(2).any(|w| w == ["-frames:v", "1"]));
        assert!(args.windows(2).any(|w| w == ["-rtsp_transport", "tcp"]));
        assert!(args.windows(2).any(|w| w == ["-rw_timeout", "15000000"]));
        assert_eq!(args.last().unwrap(), "/tmp/f.png");
        // No active target → no crop filter.
        assert!(!args.iter().any(|a| a == "-vf"));
    }

    #[test]
    fn rtmp_args_omit_rtsp_transport() {
        let args = build_ffmpeg_args("rtmp://server/app/streamkey", Path::new("/tmp/f.png"), None);
        assert!(args.windows(2).any(|w| w == ["-i", "rtmp://server/app/streamkey"]));
        assert!(args.windows(2).any(|w| w == ["-frames:v", "1"]));
        // RTSP-only option must NOT be present for RTMP.
        assert!(!args.iter().any(|a| a == "-rtsp_transport"));
    }

    #[test]
    fn active_target_injects_crop_filter() {
        let crop = PixelRect { x: 100, y: 50, w: 640, h: 480 };
        let args = build_ffmpeg_args("rtmp://server/live", Path::new("/tmp/f.png"), Some(crop));
        // `crop=w:h:x:y`, placed after `-i` (output-side filter).
        assert!(args.windows(2).any(|w| w == ["-vf", "crop=640:480:100:50"]));
        let vf = args.iter().position(|a| a == "-vf").unwrap();
        let i = args.iter().position(|a| a == "-i").unwrap();
        assert!(vf > i, "the -vf filter must come after -i");
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
