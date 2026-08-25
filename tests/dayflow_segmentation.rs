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

// ─── T020: window/sample integration over a whole run ───────────────────────
//
// These drive the real types end to end at a simulated clock. They are NOT
// `#[ignore]`d: no hardware is involved, and the edge cases they cover — clock
// discontinuity, hot-plug, non-uniform windows, pause gaps — are exactly the
// ones that only appear when the pieces run together.

mod integration {
    use chrono::{DateTime, Duration as ChronoDuration, Utc};
    use gentle_eye::config::{DayflowConfig, DeltaConfig};
    use gentle_eye::dayflow::engine::DayflowRun;
    use gentle_eye::dayflow::models::{DayflowHealth, DayflowMode};
    use gentle_eye::dayflow::sampler::{RawFrame, Sampler};
    use gentle_eye::dayflow::window::{CloseReason, PauseCause};
    use std::time::Duration;

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_787_500_000 + secs, 0).expect("valid timestamp")
    }

    fn cfg() -> DayflowConfig {
        let mut c = DayflowConfig::default();
        c.segment_seconds = 600;
        c
    }

    fn frame(w: u32, h: u32, seed: u8) -> Vec<u8> {
        let mut v = Vec::with_capacity((w * h * 4) as usize);
        for i in 0..(w * h) {
            let n = ((i.wrapping_mul(37) % 251) as u8) ^ seed;
            v.extend_from_slice(&[n, n.wrapping_add(40), n.wrapping_add(80), 255]);
        }
        v
    }

    #[test]
    fn windows_are_contiguous_and_non_overlapping_across_a_run() {
        let mut r = DayflowRun::start(&cfg(), DayflowMode::Daemon, vec![0], at(0)).unwrap();
        let mut closed = Vec::new();
        for step in 0..30 {
            closed.extend(r.on_sample(0, at(step * 180)));
        }
        closed.extend(r.stop(at(30 * 180)));

        assert!(closed.len() >= 4, "a 90-minute run must yield several windows");
        for pair in closed.windows(2) {
            assert!(
                pair[0].end_wall <= pair[1].start_wall,
                "windows must not overlap: {:?} then {:?}",
                pair[0],
                pair[1]
            );
            assert!(pair[0].sequence < pair[1].sequence, "sequence must advance");
        }
    }

    #[test]
    fn a_pause_leaves_a_visible_gap_rather_than_a_stretched_window() {
        // The failure this prevents: a window spanning the pause would claim the
        // user was working through it.
        let mut r = DayflowRun::start(&cfg(), DayflowMode::Daemon, vec![0], at(0)).unwrap();
        r.on_sample(0, at(0));
        r.on_sample(0, at(120));
        let before = r.turn_off(at(300)).pop().expect("the open window closes");

        r.turn_on(at(7_200)).unwrap();
        r.on_sample(0, at(7_200));
        let after = r.on_sample(0, at(7_800)).expect("a new window after resume");

        let gap = after.start_wall - before.end_wall;
        assert!(gap > ChronoDuration::minutes(100), "the gap is real: {gap:?}");
        assert!(
            after.duration() <= ChronoDuration::seconds(600),
            "no window may stretch across the gap"
        );
        let recorded = r.pauses_seen();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].cause, PauseCause::UserOff);
        assert_eq!(recorded[0].to, Some(at(7_200)), "the gap is a recorded fact");
    }

    #[test]
    fn a_backwards_clock_step_cannot_produce_a_negative_window() {
        // DST or a manual clock change. A window whose end precedes its start is
        // corruption, and every duration computation downstream inherits it.
        let mut r = DayflowRun::start(&cfg(), DayflowMode::Daemon, vec![0], at(0)).unwrap();
        r.on_sample(0, at(1_000));
        r.on_sample(0, at(1_060));

        // clock jumps BACKWARDS an hour, then the run is stopped
        let closed = r.stop(at(1_000 - 3_600));
        for w in &closed {
            assert!(
                w.duration().num_seconds() <= 0 || w.end_wall >= w.start_wall,
                "a backwards clock must not silently yield a plausible-looking window: {w:?}"
            );
        }
        // whatever it recorded, liveness must not read Healthy off a future stamp
        let l = r.liveness(at(1_060));
        assert_eq!(l.health, DayflowHealth::Stopped);
    }

    #[test]
    fn hot_plugging_a_display_mid_run_strands_nothing() {
        let mut r = DayflowRun::start(&cfg(), DayflowMode::Daemon, vec![0, 1], at(0)).unwrap();
        for step in 0..3 {
            r.on_sample(0, at(step * 200));
            r.on_sample(1, at(step * 200));
        }
        let orphaned = r.remove_display(1, at(700)).expect("its window must close");
        assert_eq!(orphaned.display_id, 1);
        assert_eq!(orphaned.reason, CloseReason::DisplayRemoved);
        assert_eq!(orphaned.sample_count, 3, "its samples are accounted for, not lost");

        // display 0 continues untouched
        assert_eq!(r.displays(), &[0]);
        r.on_sample(0, at(700));
        assert!(r.on_sample(0, at(1_400)).is_some());
        // and a sample from the unplugged display is ignored, not recorded
        assert!(r.on_sample(1, at(1_500)).is_none());
    }

    #[test]
    fn the_sampler_and_window_controller_agree_on_what_was_produced() {
        // Cross-checks the two halves: every sample the sampler KEPT should be
        // reflected in the window's count, and the skipped ones too.
        let dir = tempfile::tempdir().unwrap();
        let mut s = Sampler::new(DeltaConfig::default());
        let mut r = DayflowRun::start(&cfg(), DayflowMode::Daemon, vec![0], at(0)).unwrap();

        let a = frame(64, 48, 0);
        let b = frame(64, 48, 0xFF);
        let mut kept = 0usize;
        let mut total = 0usize;
        let mut closed = Vec::new();
        for step in 0..6 {
            // Pairs — A, A, B, B, A, A — so every SECOND frame is a repeat and
            // the gate has something to skip. Alternating single frames would
            // mean every sample genuinely changed, and the gate keeping all of
            // them would be correct rather than a bug.
            let px = if (step / 2) % 2 == 0 { &a } else { &b };
            let t = at(step * 120);
            let rec = s
                .observe(0, 0, RawFrame { bgra: px, width: 64, height: 48 }, t, dir.path())
                .unwrap();
            if rec.path.is_some() {
                kept += 1;
            }
            total += 1;
            // 6 samples at 120s spans 720s, which CROSSES the 600s boundary — so
            // this run legitimately produces more than one window. Collect them
            // all rather than assuming.
            closed.extend(r.on_sample(0, t));
        }
        closed.extend(r.stop(at(6 * 120)));

        let counted: usize = closed.iter().map(|w| w.sample_count as usize).sum();
        assert!(closed.len() >= 2, "720s at a 600s interval spans two windows");
        assert_eq!(counted, total, "the windows together count EVERY sample");
        assert!(kept < total, "the gate must have skipped at least one");
        let files = std::fs::read_dir(dir.path()).unwrap().count();
        assert_eq!(files, kept, "only kept samples are written to disk");
    }

    #[test]
    fn a_day_of_windows_has_no_duration_that_could_be_computed_from_a_count() {
        // Guards the invariant that makes every downstream aggregation honest.
        let mut r = DayflowRun::start(&cfg(), DayflowMode::Daemon, vec![0], at(0)).unwrap();
        let mut closed = Vec::new();
        for step in 0..20 {
            closed.extend(r.on_sample(0, at(step * 200)));
        }
        closed.extend(r.set_interval(Duration::from_secs(1_800), at(4_100)));
        for step in 21..40 {
            closed.extend(r.on_sample(0, at(step * 200)));
        }
        closed.extend(r.stop(at(40 * 200)));

        let mut durs: Vec<i64> = closed.iter().map(|w| w.duration().num_seconds()).collect();
        durs.sort_unstable();
        durs.dedup();
        assert!(
            durs.len() > 1,
            "a day with an interval change must contain windows of differing length: {durs:?}"
        );
        let total: i64 = closed.iter().map(|w| w.duration().num_seconds()).sum();
        let naive = closed.len() as i64 * 600;
        assert_ne!(total, naive, "count x configured interval must NOT equal the truth");
    }
}
