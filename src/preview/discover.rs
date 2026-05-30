//! Capture discovery — find recent images/videos under the recordings dir.
//!
//! Used by `preview` (latest capture) and `--gallery` (recent list). Top-level
//! scan, classified by extension, newest-first.

use super::errors::PreviewError;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// A capture is either a still image or a video.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureKind {
    Image,
    Video,
}

/// A discovered capture file.
#[derive(Debug, Clone)]
pub struct Capture {
    pub path: PathBuf,
    pub kind: CaptureKind,
    pub modified: SystemTime,
}

/// Classify a path as an image/video capture by extension, or `None`.
pub fn classify(path: &Path) -> Option<CaptureKind> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "webp" | "bmp" | "gif" => Some(CaptureKind::Image),
        "mp4" | "mkv" | "mov" | "webm" | "avi" | "m4v" => Some(CaptureKind::Video),
        _ => None,
    }
}

/// Recent captures under `root` (top-level), newest-first, capped at `limit`.
/// A missing/unreadable dir yields an empty list (not an error).
pub fn recent_captures(root: &Path, limit: usize) -> Result<Vec<Capture>, PreviewError> {
    let mut out = Vec::new();
    let rd = match std::fs::read_dir(root) {
        Ok(rd) => rd,
        Err(_) => return Ok(out),
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if let Some(kind) = classify(&path) {
            let modified = entry
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            out.push(Capture { path, kind, modified });
        }
    }
    out.sort_by_key(|c| std::cmp::Reverse(c.modified));
    out.truncate(limit);
    Ok(out)
}

/// The single most-recent capture under `root`, if any.
pub fn latest_capture(root: &Path) -> Result<Option<Capture>, PreviewError> {
    Ok(recent_captures(root, 1)?.into_iter().next())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn touch(dir: &Path, name: &str) {
        let mut f = std::fs::File::create(dir.join(name)).unwrap();
        f.write_all(b"x").unwrap();
    }

    #[test]
    fn classifies_by_extension() {
        assert_eq!(classify(Path::new("a.png")), Some(CaptureKind::Image));
        assert_eq!(classify(Path::new("a.MP4")), Some(CaptureKind::Video));
        assert_eq!(classify(Path::new("a.txt")), None);
        assert_eq!(classify(Path::new("noext")), None);
    }

    #[test]
    fn recent_excludes_nonmedia_and_caps_limit() {
        let d = tempfile::tempdir().unwrap();
        touch(d.path(), "a.png");
        touch(d.path(), "b.mp4");
        touch(d.path(), "notes.txt");
        let all = recent_captures(d.path(), 10).unwrap();
        assert_eq!(all.len(), 2, "txt excluded");
        let capped = recent_captures(d.path(), 1).unwrap();
        assert_eq!(capped.len(), 1, "limit honored");
    }

    #[test]
    fn newest_first() {
        let d = tempfile::tempdir().unwrap();
        touch(d.path(), "old.png");
        std::thread::sleep(std::time::Duration::from_millis(20));
        touch(d.path(), "new.mp4");
        let r = recent_captures(d.path(), 10).unwrap();
        assert_eq!(r[0].path.file_name().unwrap(), "new.mp4");
        assert_eq!(r[0].kind, CaptureKind::Video);
    }

    #[test]
    fn missing_dir_is_empty_not_error() {
        let r = recent_captures(Path::new("/nope/does/not/exist"), 5).unwrap();
        assert!(r.is_empty());
        assert!(latest_capture(Path::new("/nope")).unwrap().is_none());
    }
}
