//! On-screen text extraction (OCR).
//!
//! Backend: the system `tesseract` CLI (already present, no extra crate, same
//! subprocess pattern as the FFmpeg calls). This is deliberately a single
//! `ocr_image` seam so the backend can be swapped to a pure-Rust engine (e.g.
//! `ocrs`) later without touching callers.
//!
//! Used two ways: a standalone `read-text` capability, and to AUGMENT the local
//! (Ollama) vision path — OCR carries the exact text so the small local model
//! only needs one frame for visual context (cheap + accurate). Gemini reads text
//! natively and does not use this.

use crate::contracts::errors::VisionError;
use std::path::Path;
use std::process::Command;

/// Whether the `tesseract` binary is available on PATH.
pub fn ocr_available() -> bool {
    Command::new("tesseract")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Extract text from a single image via tesseract. Returns the recognized text
/// (may be empty if the image has no text).
pub fn ocr_image(path: &Path) -> Result<String, VisionError> {
    let output = Command::new("tesseract")
        .args([&*path.to_string_lossy(), "stdout"])
        .output()
        .map_err(|e| VisionError::Unavailable(format!("tesseract unavailable: {e}")))?;
    if !output.status.success() {
        return Err(VisionError::Unavailable(format!(
            "tesseract failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// OCR several images and merge their text, dropping consecutive near-duplicate
/// blocks (a static screen yields the same text every frame). Best-effort: a
/// frame that fails to OCR is skipped rather than aborting.
pub fn ocr_images_combined(paths: &[std::path::PathBuf]) -> String {
    let mut blocks: Vec<String> = Vec::new();
    for path in paths {
        let Ok(text) = ocr_image(path) else { continue };
        let trimmed = text.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Skip if ~identical to the previous block (static content across frames).
        if blocks
            .last()
            .is_some_and(|prev| text_similarity(prev, trimmed) > 0.85)
        {
            continue;
        }
        blocks.push(trimmed.to_string());
    }
    blocks.join("\n---\n")
}

/// Cheap Jaccard similarity over whitespace tokens, in `[0.0, 1.0]`.
fn text_similarity(a: &str, b: &str) -> f64 {
    use std::collections::HashSet;
    let sa: HashSet<&str> = a.split_whitespace().collect();
    let sb: HashSet<&str> = b.split_whitespace().collect();
    if sa.is_empty() && sb.is_empty() {
        return 1.0;
    }
    let inter = sa.intersection(&sb).count() as f64;
    let union = sa.union(&sb).count() as f64;
    if union == 0.0 {
        0.0
    } else {
        inter / union
    }
}

/// OCR a video by sampling frames (1 fps, capped) and merging their text.
/// Frames are kept at higher resolution than the VL path — OCR accuracy benefits
/// from resolution, whereas the vision model benefits from smaller images.
pub fn ocr_video(path: &Path) -> Result<String, VisionError> {
    let dir = tempfile::tempdir().map_err(|e| VisionError::Unavailable(e.to_string()))?;
    let pattern = dir.path().join("ocr_%04d.png");
    let output = Command::new("ffmpeg")
        .args([
            "-y",
            "-i",
            &*path.to_string_lossy(),
            "-vf",
            "fps=1,scale=1600:1600:force_original_aspect_ratio=decrease",
            "-frames:v",
            "20",
            &*pattern.to_string_lossy(),
        ])
        .output()
        .map_err(|e| VisionError::Unavailable(format!("ffmpeg unavailable: {e}")))?;
    if !output.status.success() {
        return Err(VisionError::Unavailable(format!(
            "ffmpeg frame extraction failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let mut frames: Vec<std::path::PathBuf> = std::fs::read_dir(dir.path())
        .map_err(|e| VisionError::Unavailable(e.to_string()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "png"))
        .collect();
    frames.sort();
    Ok(ocr_images_combined(&frames))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn similarity_bounds() {
        assert_eq!(text_similarity("a b c", "a b c"), 1.0);
        assert_eq!(text_similarity("a b c", "x y z"), 0.0);
        assert!(text_similarity("a b c d", "a b c x") > 0.5);
        assert_eq!(text_similarity("", ""), 1.0);
    }

    #[test]
    #[ignore = "live: requires tesseract + /tmp/ge_test.png"]
    fn ocr_reads_known_image() {
        let text = ocr_image(Path::new("/tmp/ge_test.png")).unwrap();
        assert!(text.to_uppercase().contains("GENTLE"));
    }
}
