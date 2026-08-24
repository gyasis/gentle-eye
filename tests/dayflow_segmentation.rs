//! Dayflow capture probes — the Wave 1 findings, codified as durable tests.
//!
//! Both are `#[ignore]`d: they need live hardware (a real `ffmpeg`, a real X
//! server with real displays), so they are not part of the default gate. Run
//! them deliberately:
//!
//! ```text
//! cargo test --test dayflow_segmentation -- --ignored --nocapture
//! ```

use std::io::Write;
use std::process::{Command, Stdio};

/// Ask `ffprobe` for a media file's duration in seconds.
fn probe_duration(path: &std::path::Path) -> f64 {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0"])
        .arg(path)
        .output()
        .expect("ffprobe must be installed to run this probe");
    String::from_utf8_lossy(&out.stdout).trim().parse().unwrap_or(-1.0)
}

/// **T004** — the ffmpeg segment muxer must cut on EXACT wall-clock boundaries at
/// the fractional frame rates dayflow actually uses.
///
/// This is the schedule-critical assumption: every later wave consumes these
/// segment files, and their recorded time ranges are only trustworthy if the cuts
/// land where we asked. `-segment_time` alone cuts at the next keyframe, which
/// drifts; `-force_key_frames` is what pins the boundary. Both are required.
///
/// The argument vector asserted here is the one measured working on ffmpeg 4.4.2
/// at 0.2 fps, 0.5 fps and 1920x1080 (research R1).
#[test]
#[ignore = "live: requires the ffmpeg binary"]
fn segment_muxer_holds_exact_boundaries_at_fractional_fps() {
    const W: usize = 320;
    const H: usize = 180;
    const FPS: &str = "0.5";
    const SEG_SECONDS: u32 = 4;
    const FRAMES: usize = 6; // 0.5 fps * 4s = 2 frames/segment -> 3 segments

    let dir = tempfile::tempdir().expect("tempdir");
    let manifest = dir.path().join("manifest.csv");
    let pattern = dir.path().join("chunk_%04d.mp4");
    let force_kf = format!("expr:gte(t,n_forced*{SEG_SECONDS})");

    let mut child = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error"])
        .args(["-f", "rawvideo", "-pix_fmt", "bgra"])
        .args(["-s", &format!("{W}x{H}"), "-framerate", FPS, "-i", "-"])
        .args(["-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p"])
        .args(["-force_key_frames", &force_kf])
        .args(["-f", "segment", "-segment_time", &SEG_SECONDS.to_string()])
        .args(["-reset_timestamps", "1", "-segment_list_type", "csv"])
        .arg("-segment_list")
        .arg(&manifest)
        .arg(&pattern)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("ffmpeg must be installed to run this probe");

    {
        let stdin = child.stdin.as_mut().expect("ffmpeg stdin");
        for i in 0..FRAMES {
            // vary each frame so x264 cannot collapse the stream to nothing
            let v = ((i * 37) % 256) as u8;
            let frame: Vec<u8> = (0..W * H)
                .flat_map(|_| [v, v.wrapping_mul(3), v.wrapping_mul(5), 255])
                .collect();
            stdin.write_all(&frame).expect("write raw frame");
        }
    } // drop stdin -> EOF, so ffmpeg finalises the last container

    let status = child.wait().expect("ffmpeg wait");
    assert!(status.success(), "ffmpeg exited with {status:?}");

    let mut chunks: Vec<_> = std::fs::read_dir(dir.path())
        .expect("read tempdir")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "mp4"))
        .collect();
    chunks.sort();

    assert_eq!(
        chunks.len(),
        3,
        "expected exactly 3 segments from {FRAMES} frames at {FPS} fps with \
         {SEG_SECONDS}s segments, got {}: {chunks:?}",
        chunks.len()
    );

    for c in &chunks {
        let bytes = std::fs::metadata(c).expect("stat segment").len();
        assert!(bytes > 0, "segment {c:?} is EMPTY — the false-green case");
        let d = probe_duration(c);
        assert!(
            (d - f64::from(SEG_SECONDS)).abs() < 0.05,
            "segment {c:?} duration {d}s is not the requested {SEG_SECONDS}s — \
             boundaries drifted, so -force_key_frames is not taking effect"
        );
    }

    // The manifest is the liveness artifact FR-006 reads: it is written BY ffmpeg
    // as each segment closes, so it is evidence independent of our own bookkeeping.
    let m = std::fs::read_to_string(&manifest).expect("segment manifest must exist");
    let lines: Vec<&str> = m.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), chunks.len(), "manifest must carry one line per closed segment:\n{m}");
    for (i, line) in lines.iter().enumerate() {
        let cols: Vec<&str> = line.split(',').collect();
        assert_eq!(cols.len(), 3, "manifest line {i} should be name,start,end: {line}");
        let (start, end): (f64, f64) = (cols[1].parse().unwrap(), cols[2].parse().unwrap());
        assert!(end > start, "manifest line {i} has non-increasing range: {line}");
        let expected_start = f64::from(SEG_SECONDS) * i as f64;
        assert!(
            (start - expected_start).abs() < 0.05,
            "segment {i} starts at {start}s, expected {expected_start}s — ranges are not contiguous"
        );
    }
}

/// **T006** — one `scrap::Capturer` per display must be able to exist AND yield a
/// frame *concurrently*, because dayflow runs one capture pipeline per display
/// (FR-029) rather than compositing them into a single oversized frame.
///
/// If this fails, the design is not wrong but the scheduling is: the fallback is
/// round-robin capture across displays within one interval. Record which applies
/// before building the multi-pipeline supervisor.
#[test]
#[ignore = "live: requires a real X server with attached displays"]
fn concurrent_capturers_across_all_displays() {
    use scrap::{Capturer, Display};

    let displays = Display::all().expect("enumerate displays");
    assert!(!displays.is_empty(), "no displays enumerated");
    let n = displays.len();
    eprintln!("enumerated {n} display(s)");

    // Hold EVERY capturer open at once — that is the property under test.
    let mut capturers: Vec<Capturer> = Vec::new();
    for (i, d) in displays.into_iter().enumerate() {
        let (w, h) = (d.width(), d.height());
        match Capturer::new(d) {
            Ok(c) => {
                eprintln!("  display {i}: capturer OPEN at {w}x{h}");
                capturers.push(c);
            }
            Err(e) => panic!(
                "display {i} ({w}x{h}) could not open a capturer while {} other(s) were \
                 already open: {e}. Concurrent capture is unavailable — use the \
                 round-robin fallback and record it in research R3.",
                capturers.len()
            ),
        }
    }
    assert_eq!(capturers.len(), n, "not every display yielded a concurrent capturer");

    // Now prove they actually PRODUCE pixels while all are open. scrap returns
    // WouldBlock until a frame is ready, so retry briefly rather than flaking.
    for (i, cap) in capturers.iter_mut().enumerate() {
        let (w, h) = (cap.width(), cap.height());
        let mut got = None;
        for _ in 0..120 {
            match cap.frame() {
                Ok(f) => {
                    got = Some(f.len());
                    break;
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
                Err(e) => panic!("display {i} frame error: {e}"),
            }
        }
        let len = got.unwrap_or_else(|| {
            panic!("display {i} ({w}x{h}) never produced a frame within 3s while all {n} capturers were open")
        });
        assert!(len >= w * h, "display {i} frame is {len} bytes, smaller than {w}x{h}");
        eprintln!("  display {i}: frame OK, {len} bytes ({w}x{h})");
    }
}
