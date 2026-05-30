//! PV1 (T412) — discovery feeds the player arg-builder end to end.

use gentle_eye::preview::discover::{classify, latest_capture, CaptureKind};
use gentle_eye::preview::player::{ffplay_args, PlaybackOpts};
use std::io::Write;

#[test]
fn latest_capture_feeds_player_args() {
    let d = tempfile::tempdir().unwrap();
    std::fs::File::create(d.path().join("old.png")).unwrap().write_all(b"x").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::File::create(d.path().join("new.mp4")).unwrap().write_all(b"x").unwrap();

    // No FILE arg → latest capture is the newer video.
    let latest = latest_capture(d.path()).unwrap().unwrap();
    assert_eq!(latest.path.file_name().unwrap(), "new.mp4");
    assert_eq!(latest.kind, CaptureKind::Video);

    // That capture builds a sane ffplay command (video → plays once, exits).
    let kind = classify(&latest.path).unwrap();
    let args = ffplay_args(&latest.path, kind, &PlaybackOpts::default());
    assert!(args.contains(&"-autoexit".to_string()));
    assert_eq!(args.last().unwrap(), &latest.path.to_string_lossy().to_string());
}
