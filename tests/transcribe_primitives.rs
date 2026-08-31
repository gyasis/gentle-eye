//! The transcription primitives, against REAL motion blur.
//!
//! # Why the fixture is generated rather than committed
//!
//! T003's DONE line asks for the real M4 frames. Those frames are captures of a
//! DIFFERENT machine — its hostname, its network address, its terminal contents
//! — and this repository is public. Committing them would leak all of it.
//!
//! The intent behind "real, not synthetic" is preserved instead: the blur here
//! comes from real MOTION, captured at a frame rate that cannot freeze it, not
//! from a blur filter applied to a still. A gaussian filter cannot fail the way
//! motion blur does; motion blur of generated content can, because it is the
//! same mechanism.
//!
//! Verified before being relied on: this fixture reproduces a 2.5x sharp:blurred
//! separation, against the 3.6x measured on the real feed (research.md M4). Same
//! direction, same mechanism, no leak.

use gentle_eye::transcribe::frames::sharpness_of_file;

fn ffmpeg_available() -> bool {
    std::process::Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Render text, either still or moving fast enough to blur.
fn render(dir: &std::path::Path, moving: bool, count: u32) -> Vec<std::path::PathBuf> {
    let y = if moving { "h/2-(t*380)" } else { "360" };
    // Two tblend passes average successive frames — which is what a real
    // capture does to fast motion, and is why the blur is motion blur rather
    // than a filtered still.
    let blend = if moving { ",tblend=all_mode=average,tblend=all_mode=average" } else { "" };
    let vf = format!(
        "drawtext=text='THE QUICK BROWN FOX JUMPS OVER THE LAZY DOG 0123456789':\
         fontcolor=white:fontsize=36:x=40:y={y}:box=1:boxcolor=black{blend}"
    );
    let pattern = dir.join(if moving { "m_%03d.png" } else { "s_%03d.png" });
    let out = std::process::Command::new("ffmpeg")
        .args([
            "-v", "error",
            "-f", "lavfi",
            "-i", "color=c=black:s=1280x720:r=24:d=4",
            "-vf", &vf,
            "-frames:v", &count.to_string(),
            &pattern.to_string_lossy(),
            "-y",
        ])
        .output()
        .expect("run ffmpeg");
    assert!(
        out.status.success(),
        "ffmpeg could not render the fixture:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let mut v: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension().is_some_and(|x| x == "png")
                && p.file_name().unwrap().to_string_lossy().starts_with(if moving { "m_" } else { "s_" })
        })
        .collect();
    v.sort();
    v
}

/// MOTION BLUR LOWERS SHARPNESS — the property the whole pipeline rests on.
///
/// M4 measured this on a real feed: sharp frames (1,443–1,458) all read cleanly,
/// blurred frames (396–507) ALL failed. If this direction does not hold, the
/// sharpness gate is worthless and every frame costs a model call.
#[test]
#[ignore = "live: needs ffmpeg to render the motion fixture"]
fn motion_blur_lowers_sharpness_as_measured() {
    assert!(ffmpeg_available(), "\n\nffmpeg is not on PATH — this fixture cannot be rendered.\n");
    let dir = tempfile::tempdir().unwrap();

    let still = render(dir.path(), false, 3);
    let moving = render(dir.path(), true, 24);
    // The window below is moving[10..20]; anything short of 20 frames would
    // panic on the slice with a message that blames the wrong thing.
    assert!(!still.is_empty() && moving.len() >= 20, "the fixture rendered too few frames");

    let s: f64 = still.iter().map(|p| sharpness_of_file(p).unwrap()).sum::<f64>() / still.len() as f64;
    // Skip the first frames: motion has not built up until the text is moving.
    let m: Vec<f64> = moving[10..20].iter().map(|p| sharpness_of_file(p).unwrap()).collect();
    let mb: f64 = m.iter().sum::<f64>() / m.len() as f64;

    eprintln!("[fixture] still={s:.0}  moving={mb:.0}  separation={:.1}x", s / mb);
    assert!(
        s > mb,
        "motion blur must LOWER sharpness. still={s:.0}, moving={mb:.0} — if this \
         fails the sharpness gate cannot predict readability and every frame costs \
         a model call"
    );
    assert!(
        s / mb > 1.8,
        "the separation must be substantial enough to threshold on. Measured on the \
         real feed: 3.6x (M4). Here: {:.1}x",
        s / mb
    );
}

/// A frame that cannot be DECODED is not a frame that scores zero.
///
/// Conflating them would let an unreadable file look like a flat image, and the
/// two want different responses — the same rule as "a failed read must never
/// look like an empty result".
#[test]
fn an_undecodable_file_errors_rather_than_scoring_zero() {
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("not_an_image.png");
    std::fs::write(&bad, b"this is not a PNG").unwrap();
    let err = sharpness_of_file(&bad).expect_err("a non-image must not score");
    assert!(err.contains("cannot decode"), "the error must say what happened: {err}");
}

// ── T004: extraction with no cap ─────────────────────────────────────────────

use gentle_eye::transcribe::frames::{extract_frames, Dedup};

/// Render a recording of a known length with real motion in it.
fn render_video(path: &std::path::Path, seconds: u32) -> bool {
    std::process::Command::new("ffmpeg")
        .args([
            "-v", "error", "-f", "lavfi",
            "-i", &format!("color=c=black:s=640x360:r=24:d={seconds}"),
            "-vf",
            "drawtext=text='FRAME %{n}':fontcolor=white:fontsize=48:x=40:y=180:box=1:boxcolor=black",
            "-pix_fmt", "yuv420p",
            &path.to_string_lossy(), "-y",
        ])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// THE CAP IS GONE. `analysis::ocr::ocr_video` stops at 20 frames, silently
/// truncating anything longer — the actual blocker for lesson-length material.
///
/// A 30-second recording at 1 fps must yield ~30 rows, not 20.
#[test]
#[ignore = "live: needs ffmpeg"]
fn extraction_is_not_capped_at_twenty_frames() {
    assert!(ffmpeg_available(), "\n\nffmpeg is not on PATH.\n");
    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("thirty.mp4");
    assert!(render_video(&video, 30), "could not render the fixture");

    let rows = extract_frames(&video, 1.0, Dedup::None, &dir.path().join("out"))
        .expect("extraction runs");

    eprintln!("[t004] 30s at 1fps -> {} rows", rows.len());
    assert!(
        rows.len() > 20,
        "extraction returned {} rows — the 20-frame cap is still in force, and any \
         recording longer than 20 frames is being silently truncated",
        rows.len()
    );
    // Indices are dense and ordered; a caller sorts and slices on them.
    for (i, r) in rows.iter().enumerate() {
        assert_eq!(r.index, i, "indices must be dense and in order");
        assert!(r.path.exists(), "row {i} names a file that is not there");
    }
}

/// The dedup threshold CHANGES the result. A parameter that is accepted and
/// ignored is the orphan pattern in miniature — and M1 measured this same clip
/// keeping 285, 138 or 2 frames across settings.
#[test]
#[ignore = "live: needs ffmpeg"]
fn the_dedup_threshold_changes_what_is_kept() {
    assert!(ffmpeg_available(), "\n\nffmpeg is not on PATH.\n");
    let dir = tempfile::tempdir().unwrap();
    // Mostly-static content: a few changes, many identical frames. This is the
    // material where dedup SHOULD collapse hard — a scrolling fixture would
    // keep everything at every setting and prove nothing.
    let video = dir.path().join("static.mp4");
    let ok = std::process::Command::new("ffmpeg")
        .args([
            "-v", "error", "-f", "lavfi",
            "-i", "color=c=black:s=640x360:r=24:d=10",
            "-vf",
            "drawtext=text='SLIDE %{eif\\:floor(t/3)\\:d}':fontcolor=white:fontsize=48:x=40:y=180:box=1:boxcolor=black",
            "-pix_fmt", "yuv420p", &video.to_string_lossy(), "-y",
        ])
        .output().map(|o| o.status.success()).unwrap_or(false);
    assert!(ok, "could not render the static fixture");

    let none = extract_frames(&video, 4.0, Dedup::None, &dir.path().join("a")).unwrap();
    let aggressive = extract_frames(&video, 4.0, Dedup::Aggressive, &dir.path().join("b")).unwrap();

    eprintln!("[t004] none={} aggressive={}", none.len(), aggressive.len());
    assert!(
        aggressive.len() < none.len(),
        "the dedup parameter changed nothing: none={} aggressive={}. A threshold that \
         is accepted and ignored is worse than one that does not exist, because the \
         caller believes they set it",
        none.len(),
        aggressive.len()
    );
}

/// ffmpeg absent is an ERROR NAMING IT — never an empty frame list.
///
/// "The recording has no frames" and "I could not look" are different facts. A
/// caller that cannot tell them apart reports the wrong one.
#[test]
fn a_missing_video_errors_rather_than_returning_no_frames() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("does_not_exist.mp4");
    let err = extract_frames(&missing, 1.0, Dedup::None, &dir.path().join("out"))
        .expect_err("a missing recording must not return an empty list");
    assert!(
        err.contains("ffmpeg"),
        "the error must say what failed, got: {err}"
    );
}

/// A nonsensical rate is refused before ffmpeg is invoked.
#[test]
fn a_non_positive_rate_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(
            extract_frames(std::path::Path::new("x.mp4"), bad, Dedup::None, dir.path()).is_err(),
            "fps={bad} must be refused"
        );
    }
}

/// An unknown dedup spelling is an error, not a silent default. A caller who
/// typed `agressive` asked for aggression; quietly giving them `medium` means
/// they believe a setting is in force that is not.
#[test]
fn an_unknown_dedup_is_refused_not_defaulted() {
    assert_eq!(Dedup::parse("aggressive").unwrap(), Dedup::Aggressive);
    assert_eq!(Dedup::parse("  Medium ").unwrap(), Dedup::Medium);
    let err = Dedup::parse("agressive").expect_err("a typo must not silently default");
    assert!(err.contains("agressive"), "the error must quote what was typed: {err}");
    assert_ne!(Dedup::parse("agressive").ok(), Some(Dedup::default()));
}
