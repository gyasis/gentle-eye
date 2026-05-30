//! PV1 — open a capture (image or video) in a player.
//!
//! Default renderer: **ffplay** (ships with ffmpeg — zero new crates), with an
//! OS-opener (`xdg-open`/`open`) fallback when ffplay is absent. Playback params
//! (loop once/forever, show-N-seconds-then-autoclose) map to ffplay flags.

use super::discover::CaptureKind;
use super::errors::PreviewError;
use std::path::Path;
use std::process::Command;

/// How the preview should loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopMode {
    Once,
    Forever,
}

/// Playback parameters for the preview.
#[derive(Debug, Clone, Default)]
pub struct PlaybackOpts {
    pub loop_mode: Option<LoopMode>,
    /// Show for N seconds, then auto-close.
    pub autoclose_secs: Option<u64>,
}

/// Build the ffplay argument vector for `path`/`kind` under `opts`.
///
/// - `autoclose_secs` → `-t N -autoexit` (works for image *and* video).
/// - else loop `Forever` → `-loop 0`; `Once`/default → `-autoexit` for video
///   (an image with no autoclose simply stays open until dismissed).
pub fn ffplay_args(path: &Path, kind: CaptureKind, opts: &PlaybackOpts) -> Vec<String> {
    let mut a: Vec<String> = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-window_title".into(),
        "gentle-eye preview".into(),
    ];
    if let Some(secs) = opts.autoclose_secs {
        a.push("-t".into());
        a.push(secs.to_string());
        a.push("-autoexit".into());
    } else {
        match opts.loop_mode {
            Some(LoopMode::Forever) => {
                a.push("-loop".into());
                a.push("0".into());
            }
            Some(LoopMode::Once) | None => {
                if kind == CaptureKind::Video {
                    a.push("-autoexit".into());
                }
            }
        }
    }
    a.push(path.to_string_lossy().into_owned());
    a
}

/// Open `path` in ffplay; fall back to the OS opener if ffplay isn't available.
/// Returns which backend handled it. Non-blocking (the window outlives the CLI).
pub fn open_with_player(
    path: &Path,
    kind: CaptureKind,
    opts: &PlaybackOpts,
) -> Result<&'static str, PreviewError> {
    if !path.exists() {
        return Err(PreviewError::NotFound(path.display().to_string()));
    }
    let args = ffplay_args(path, kind, opts);
    match Command::new("ffplay").args(&args).spawn() {
        Ok(_) => Ok("ffplay"),
        Err(_) => {
            os_open(path)?;
            Ok("os-open")
        }
    }
}

/// Open `path` in the OS default viewer (`open` on macOS, `xdg-open` elsewhere).
fn os_open(path: &Path) -> Result<(), PreviewError> {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    Command::new(opener)
        .arg(path)
        .spawn()
        .map_err(|e| PreviewError::Spawn(format!("{opener}: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn has_pair(a: &[String], k: &str, v: &str) -> bool {
        a.windows(2).any(|w| w[0] == k && w[1] == v)
    }

    #[test]
    fn video_default_plays_once_and_exits() {
        let a = ffplay_args(Path::new("/x/clip.mp4"), CaptureKind::Video, &PlaybackOpts::default());
        assert!(a.contains(&"-autoexit".to_string()));
        assert!(!a.contains(&"-loop".to_string()));
        assert_eq!(a.last().unwrap(), "/x/clip.mp4");
        assert!(has_pair(&a, "-window_title", "gentle-eye preview"));
    }

    #[test]
    fn video_forever_loops() {
        let o = PlaybackOpts { loop_mode: Some(LoopMode::Forever), autoclose_secs: None };
        let a = ffplay_args(Path::new("/x/clip.mp4"), CaptureKind::Video, &o);
        assert!(has_pair(&a, "-loop", "0"));
        assert!(!a.contains(&"-autoexit".to_string()));
    }

    #[test]
    fn image_default_stays_open() {
        let a = ffplay_args(Path::new("/x/shot.png"), CaptureKind::Image, &PlaybackOpts::default());
        assert!(!a.contains(&"-autoexit".to_string()), "image with no autoclose stays open");
        assert!(!a.contains(&"-loop".to_string()));
    }

    #[test]
    fn autoclose_sets_duration_and_exit() {
        let o = PlaybackOpts { loop_mode: None, autoclose_secs: Some(5) };
        let a = ffplay_args(Path::new("/x/shot.png"), CaptureKind::Image, &o);
        assert!(has_pair(&a, "-t", "5"));
        assert!(a.contains(&"-autoexit".to_string()));
    }

    #[test]
    fn missing_file_errors() {
        let err = open_with_player(Path::new("/no/such.png"), CaptureKind::Image, &PlaybackOpts::default()).unwrap_err();
        assert!(matches!(err, PreviewError::NotFound(_)));
    }
}
