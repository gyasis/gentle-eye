//! gentle-eye binary — one tool, two front-ends.
//!
//! * `gentle-eye [serve]` — run as an MCP server over stdio (default; no-arg
//!   form preserved so existing MCP-client integrations keep working).
//! * `gentle-eye analyze|record|list|provider-info …` — direct CLI subcommands
//!   that reuse the same library and print JSON to stdout, for agents that
//!   prefer to shell out and parse a result.
//!
//! `serve` reconstructed 2026-05-28; CLI front-end added 2026-05-28 (both share
//! `GentleEyeServer`'s wiring — no duplicated logic).

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use gentle_eye::capture::display::{DisplayConfig, DisplayManager};
use gentle_eye::config::AppConfig;
use gentle_eye::contracts::traits::{RecordingConfig, RecordingStatus, TimeRange};
use gentle_eye::mcp::GentleEyeServer;
use gentle_eye::startup::{validate_startup, StartupError};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;
use tokio::signal;
use tracing::{error, info, warn};
use tracing_subscriber::{fmt, EnvFilter};

const HELP: &str = "\
gentle-eye — screen recording + AI video analysis

USAGE:
  gentle-eye [serve]                                  Run as an MCP server over stdio (default)
  gentle-eye analyze --image PATH  --prompt TEXT [--provider gemini|ollama]
  gentle-eye analyze --video PATH  --prompt TEXT [--start S --end E] [--provider …]
  gentle-eye record  [--duration SECS] [--fps N] [--out FILE.mp4] [--display IDX|LABEL]
  gentle-eye capture-stream --url URL [--out DIR] [--region x,y,w,h]   Grab one frame from a stream (ATEM/RTSP/HTTP); --region crops (normalized 0-1)
  gentle-eye list    [--status all|recording|completed|cancelled|failed] [--limit N]
  gentle-eye read-text --image PATH | --video PATH    Extract on-screen text (OCR) as JSON
  gentle-eye displays                                 List available displays (the catalogue)
  gentle-eye label   --display IDX --name \"left\"      Label a display (persists across runs)
  gentle-eye target add NAME (--display IDX | --stream URL) --region x,y,w,h   Define a crop (normalized 0-1)
  gentle-eye target use NAME                          Make NAME the active target
  gentle-eye target list                              List targets + the active one
  gentle-eye preview [FILE] [--loop once|forever] [--seconds N]   Preview a capture (default: most recent); ffplay/OS-open
  gentle-eye preview --gallery [--port N]             Serve a media gallery (browser; Range video) until idle
  gentle-eye preview --live                           Live preview of the active target via ffplay (default off)
  gentle-eye screenshot --out FILE.png [--display IDX] [--region x,y,w,h | --target NAME]   One-shot screen grab → PNG (optional crop)
  gentle-eye segment --display IDX [--read] [--provider gemini|ollama]   Detect terminal/editor PANELS (column-activity divider analysis); --read reads each via the vision provider
  gentle-eye redpen-list [--limit N]                  List redpen annotation captures (newest first) for the agent to ingest
  gentle-eye redpen-analyze [--image PATH] [--prompt TEXT] [--provider gemini|ollama]   Send a redpen capture (default: latest) + its boxes to vision AI
  gentle-eye provider-info [--provider gemini|ollama]
  gentle-eye help

Env: GENTLE_EYE_PROVIDER, GEMINI_API_KEY/GOOGLE_API_KEY, OLLAMA_HOST, OLLAMA_PORT,
     GENTLE_EYE_DATA, GENTLE_EYE_FPS. CLI subcommands print JSON to stdout.";

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(String::as_str).unwrap_or("serve");
    // CLI subcommands stay quiet (clean JSON on stdout); the server is chatty.
    let level = if cmd == "serve" { "info" } else { "warn" };
    if let Err(e) = setup_logging(level) {
        eprintln!("Warning: failed to set up logging: {e}");
    }
    let rest: &[String] = if args.len() > 2 { &args[2..] } else { &[] };

    let result = match cmd {
        "serve" => run_serve().await,
        "analyze" => run_analyze(rest).await,
        "record" => run_record(rest).await,
        "capture-stream" => run_capture_stream(rest).await,
        "list" => run_list(rest).await,
        "provider-info" => run_provider_info(rest).await,
        "read-text" => run_read_text(rest).await,
        "displays" => run_displays(rest).await,
        "label" => run_label(rest).await,
        "target" => run_target(rest).await,
        "preview" => run_preview(rest).await,
        "screenshot" => run_screenshot(rest).await,
        "segment" => run_segment(rest).await,
        "regions" => run_regions(rest).await,
        "dayflow" => run_dayflow(rest).await,
        "redpen-list" => run_redpen_list(rest).await,
        "redpen-analyze" => run_redpen_analyze(rest).await,
        "help" | "-h" | "--help" => {
            println!("{HELP}");
            Ok(())
        }
        other => Err(anyhow!("unknown command '{other}'\n\n{HELP}")),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn setup_logging(default_level: &str) -> Result<()> {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));
    fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init()
        .map_err(|e| anyhow!("failed to init tracing: {e}"))?;
    Ok(())
}

/// Find a `--flag value` pair in the argument list.
fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

fn parse_status(s: &str) -> Option<RecordingStatus> {
    match s.to_lowercase().as_str() {
        "recording" => Some(RecordingStatus::Recording),
        "completed" => Some(RecordingStatus::Completed),
        "cancelled" => Some(RecordingStatus::Cancelled),
        "error" | "failed" => Some(RecordingStatus::Failed),
        _ => None, // "all" / unknown => no filter
    }
}

// ---- CLI subcommands -------------------------------------------------------

async fn run_analyze(args: &[String]) -> Result<()> {
    if let Some(p) = flag(args, "--provider") {
        std::env::set_var("GENTLE_EYE_PROVIDER", p);
    }
    let prompt = flag(args, "--prompt").ok_or_else(|| anyhow!("--prompt is required"))?;
    let server = GentleEyeServer::new().await?;

    let result = if let Some(video) = flag(args, "--video") {
        let timeframe = match (flag(args, "--start"), flag(args, "--end")) {
            (Some(s), Some(e)) => Some(TimeRange {
                start_seconds: s.parse().context("--start must be a number")?,
                end_seconds: e.parse().context("--end must be a number")?,
            }),
            _ => None,
        };
        server
            .vision()
            .analyze_video(Path::new(video), prompt, timeframe)
            .await?
    } else if let Some(image) = flag(args, "--image") {
        server.vision().analyze_image(Path::new(image), prompt).await?
    } else {
        return Err(anyhow!("provide --image PATH or --video PATH"));
    };

    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

async fn run_record(args: &[String]) -> Result<()> {
    let duration: u64 = flag(args, "--duration")
        .unwrap_or("10")
        .parse()
        .context("--duration must be an integer (seconds)")?;
    let fps: u8 = flag(args, "--fps")
        .unwrap_or("2")
        .parse()
        .context("--fps must be an integer 1-30")?;

    if let Some(selector) = flag(args, "--display") {
        let index = resolve_display(selector)?;
        std::env::set_var("GENTLE_EYE_DISPLAY", index.to_string());
    }
    let server = GentleEyeServer::new().await?;
    let config = RecordingConfig {
        fps,
        max_duration_seconds: Some(duration),
        ..RecordingConfig::default()
    };
    let started = server.recording().start_recording(config).await?;
    let id = started.id;
    info!("recording {id} for {duration}s @ {fps} fps…");

    // Foreground: wait out the duration (the worker auto-stops at max_duration),
    // then stop to finalize and fetch the completed record.
    tokio::time::sleep(Duration::from_secs(duration) + Duration::from_millis(800)).await;
    let mut record = server.recording().stop_recording(id).await?;

    // Honor --out by moving the finalized file to the requested path.
    if let (Some(out), Some(src)) = (flag(args, "--out"), record.file_path.clone()) {
        if std::fs::rename(&src, out).is_ok() {
            record.file_path = Some(PathBuf::from(out));
        }
    }
    println!("{}", serde_json::to_string_pretty(&record)?);
    Ok(())
}

async fn run_capture_stream(args: &[String]) -> Result<()> {
    let url = flag(args, "--url").ok_or_else(|| anyhow!("--url <stream-url> is required"))?;
    let out = flag(args, "--out").unwrap_or("/tmp/gentle-eye/frames");
    // Optional --region x,y,w,h (normalized 0-1) crops the grabbed frame to a
    // sub-region via the same ffmpeg `crop=` filter an active stream target uses.
    let frame = if let Some(region_str) = flag(args, "--region") {
        let region = parse_region(region_str)?;
        if !region.is_valid() {
            return Err(anyhow!("region must lie within 0-1 with positive area"));
        }
        // Probe full-frame resolution → compute pixel rect → capture cropped.
        let full = gentle_eye::capture::stream::capture_stream_frame(url, Path::new(out))?;
        let rect = gentle_eye::target::geometry::norm_to_pixel(region, (full.width, full.height), (0, 0));
        gentle_eye::capture::stream::capture_stream_frame_cropped(url, Path::new(out), Some(rect))?
    } else {
        gentle_eye::capture::stream::capture_stream_frame(url, Path::new(out))?
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "file_path": frame.file_path.to_string_lossy(),
            "width": frame.width,
            "height": frame.height,
            "file_size_bytes": frame.file_size_bytes,
            "stream_url": frame.stream_url,
            "captured_at": frame.captured_at,
        }))?
    );
    Ok(())
}

async fn run_preview(args: &[String]) -> Result<()> {
    use gentle_eye::preview::discover::{classify, latest_capture, CaptureKind};
    use gentle_eye::preview::gallery;
    use gentle_eye::preview::player::{open_with_player, LoopMode, PlaybackOpts};

    // `--gallery`: spin up the zero-dep std::net media gallery (serves until idle).
    if args.iter().any(|a| a == "--gallery") {
        let port: u16 = match flag(args, "--port") {
            Some(p) => p.parse().context("--port must be a number")?,
            None => 8080,
        };
        let root = AppConfig::load()
            .map(|c| c.storage.base_dir)
            .map_err(|e| anyhow!("config error: {e}"))?;
        let listener = gallery::bind(port).map_err(|e| anyhow!("{e}"))?;
        let actual = listener.local_addr()?.port();
        let url = gallery::announce(actual, gallery::is_ssh_session());
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "gallery": url, "root": root.to_string_lossy()
            }))?
        );
        // Blocks until ~5 min idle (or Ctrl-C).
        gallery::serve_listener(listener, root, Duration::from_secs(300)).map_err(|e| anyhow!("{e}"))?;
        return Ok(());
    }

    let opts = PlaybackOpts {
        loop_mode: match flag(args, "--loop") {
            Some("once") => Some(LoopMode::Once),
            Some("forever") => Some(LoopMode::Forever),
            Some(other) => return Err(anyhow!("--loop must be once|forever (got '{other}')")),
            None => None,
        },
        autoclose_secs: match flag(args, "--seconds") {
            Some(s) => Some(s.parse().context("--seconds must be an integer")?),
            None => None,
        },
    };

    // `--live`: real-time preview of the active target (default OFF).
    if args.iter().any(|a| a == "--live") {
        return run_live().await;
    }

    // Positional FILE is the first arg that isn't a flag; else the latest capture.
    let path = match args.first().filter(|a| !a.starts_with("--")) {
        Some(f) => PathBuf::from(f),
        None => {
            let root = AppConfig::load()
                .map(|c| c.storage.base_dir)
                .map_err(|e| anyhow!("config error: {e}"))?;
            latest_capture(&root)?
                .ok_or_else(|| anyhow!("no captures found under {}", root.display()))?
                .path
        }
    };
    let kind = classify(&path).unwrap_or(CaptureKind::Image);
    let backend = open_with_player(&path, kind, &opts)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "previewing": path.to_string_lossy(),
            "kind": format!("{kind:?}").to_lowercase(),
            "backend": backend,
        }))?
    );
    Ok(())
}

/// `preview --live` — real-time preview of the active target via ffplay.
async fn run_live() -> Result<()> {
    use gentle_eye::preview::live::live_stream_args;
    use gentle_eye::target::geometry::norm_to_pixel;
    use gentle_eye::target::model::TargetSource;
    use gentle_eye::target::store::TargetStore;

    let store = TargetStore::load().map_err(|e| anyhow!("{e}"))?;
    let target = store
        .active()
        .ok_or_else(|| anyhow!("no active target — define one with `gentle-eye target add ...`"))?
        .clone();

    match &target.source {
        TargetSource::Stream { url } => {
            // Best-effort: probe one frame for resolution → crop rect.
            let tmp = std::env::temp_dir().join("gentle-eye/live");
            let crop = gentle_eye::capture::stream::capture_stream_frame(url, &tmp)
                .ok()
                .map(|f| norm_to_pixel(target.region, (f.width, f.height), (0, 0)));
            let args = live_stream_args(url, crop);
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "live": "stream", "url": url, "ffplay_args": args
                }))?
            );
            std::process::Command::new("ffplay")
                .args(&args)
                .status()
                .map_err(|e| anyhow!("ffplay failed (is it installed?): {e}"))?;
        }
        TargetSource::Display { index } => run_display_live(*index, target.region)?,
    }
    Ok(())
}

/// Pump cropped screen frames to ffplay's stdin as rawvideo (blocks until closed).
fn run_display_live(index: usize, region: gentle_eye::target::model::NormRect) -> Result<()> {
    use gentle_eye::preview::live::live_display_args;
    use gentle_eye::target::crop::crop_bgra;
    use gentle_eye::target::geometry::norm_to_pixel;
    use std::io::Write as _;

    let mut cap = gentle_eye::capture::screen::ScreenCapturer::new(index)
        .map_err(|e| anyhow!("display capture unavailable: {e}"))?;
    let (fw, fh) = (cap.width(), cap.height());
    let rect = norm_to_pixel(region, (fw as u32, fh as u32), (0, 0));
    let args = live_display_args(rect.w, rect.h);
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "live": "display", "index": index, "crop": [rect.x, rect.y, rect.w, rect.h]
        }))?
    );
    let mut child = std::process::Command::new("ffplay")
        .args(&args)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| anyhow!("ffplay failed (is it installed?): {e}"))?;
    let mut stdin = child.stdin.take().ok_or_else(|| anyhow!("no ffplay stdin"))?;
    loop {
        let frame = match cap.capture_frame(Duration::from_millis(200)) {
            Ok(f) => f,
            Err(_) => continue,
        };
        let stride = frame.len().checked_div(fh).unwrap_or(fw * 4);
        let (cropped, _, _) = match crop_bgra(&frame, fw, fh, stride, rect) {
            Ok(c) => c,
            Err(e) => return Err(anyhow!("crop failed: {e}")),
        };
        if stdin.write_all(&cropped).is_err() {
            break; // ffplay window closed
        }
    }
    Ok(())
}

async fn run_target(args: &[String]) -> Result<()> {
    use gentle_eye::target::model::{Target, TargetSource};
    use gentle_eye::target::store::TargetStore;

    let sub = args.first().map(String::as_str).unwrap_or("list");
    match sub {
        "list" => {
            let store = TargetStore::load()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "targets": store.list(),
                    "active": store.active().map(|t| t.name.clone()),
                }))?
            );
        }
        "use" => {
            let name = args
                .get(1)
                .ok_or_else(|| anyhow!("usage: target use NAME"))?;
            let mut store = TargetStore::load()?;
            store.set_active(name).map_err(|e| anyhow!("{e}"))?;
            store.save()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({"active": name}))?
            );
        }
        "add" => {
            let name = args
                .get(1)
                .ok_or_else(|| anyhow!("usage: target add NAME (--display IDX | --stream URL) --region x,y,w,h"))?;
            let source = match (flag(args, "--display"), flag(args, "--stream")) {
                (Some(idx), None) => TargetSource::Display {
                    index: idx.parse().context("--display must be an integer index")?,
                },
                (None, Some(url)) => TargetSource::Stream { url: url.to_string() },
                _ => return Err(anyhow!("provide exactly one of --display IDX or --stream URL")),
            };
            let region = parse_region(
                flag(args, "--region").ok_or_else(|| anyhow!("--region x,y,w,h is required"))?,
            )?;
            if !region.is_valid() {
                return Err(anyhow!("region must lie within 0-1 with positive area"));
            }
            let mut store = TargetStore::load()?;
            let mut target = Target::new(name.clone(), source, region);
            target.active = true; // adding makes it active (one at a time)
            store.add(target);
            store.save()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "added": name, "active": name
                }))?
            );
        }
        other => return Err(anyhow!("unknown target subcommand '{other}' (add|use|list)")),
    }
    Ok(())
}

/// One-shot screen grab → PNG (the direct ScreenCapturer→crop→write_bgra_png path,
/// reusing the `target` chain — no video/ffmpeg round-trip). Optional crop via a
/// `--region x,y,w,h` (normalized 0-1) or a named `--target`. Works on Linux (X11)
/// and on macOS with Screen Recording permission; fails over headless SSH (TCC).
async fn run_screenshot(args: &[String]) -> Result<()> {
    use gentle_eye::capture::screen::ScreenCapturer;
    use gentle_eye::capture::stream::write_bgra_png;
    use gentle_eye::target::crop::crop_bgra;
    use gentle_eye::target::geometry::norm_to_pixel;
    use gentle_eye::target::model::{PixelRect, TargetSource};
    use gentle_eye::target::store::TargetStore;

    let out = flag(args, "--out").ok_or_else(|| anyhow!("--out FILE.png is required"))?;

    // If --target names a target, load it up front: a display-bound target ALSO selects which
    // display to capture (unless --display is given explicitly). Without this, `--target` grabbed
    // the target's region off display 0 regardless of which monitor the target lives on.
    let target = match flag(args, "--target") {
        Some(name) => {
            let store = TargetStore::load().map_err(|e| anyhow!("{e}"))?;
            Some(
                store
                    .list()
                    .iter()
                    .find(|t| t.name == name)
                    .cloned()
                    .ok_or_else(|| anyhow!("no target named '{name}'"))?,
            )
        }
        None => None,
    };

    let display: usize = match flag(args, "--display") {
        Some(s) => s.parse().context("--display must be an integer index")?,
        None => match target.as_ref().map(|t| &t.source) {
            Some(TargetSource::Display { index }) => *index,
            _ => 0,
        },
    };

    let mut cap =
        ScreenCapturer::new(display).map_err(|e| anyhow!("screen capture unavailable: {e}"))?;
    let (fw, fh) = (cap.width(), cap.height());
    let buf = cap
        .capture_frame(Duration::from_secs(2))
        .map_err(|e| anyhow!("capture failed: {e}"))?;
    let stride = buf.len().checked_div(fh).unwrap_or(fw * 4);

    // Crop rect: --region (normalized) > --target region > full frame.
    let rect = if let Some(r) = flag(args, "--region") {
        let region = parse_region(r)?;
        if !region.is_valid() {
            return Err(anyhow!("--region must lie within 0-1 with positive area"));
        }
        norm_to_pixel(region, (fw as u32, fh as u32), (0, 0))
    } else if let Some(t) = target.as_ref() {
        norm_to_pixel(t.region, (fw as u32, fh as u32), (0, 0))
    } else {
        PixelRect { x: 0, y: 0, w: fw as u32, h: fh as u32 }
    };

    let (bytes, w, h) =
        crop_bgra(&buf, fw, fh, stride, rect).map_err(|e| anyhow!("crop: {e}"))?;
    write_bgra_png(&bytes, w, h, Path::new(out)).map_err(|e| anyhow!("png write: {e}"))?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "screenshot": out, "width": w, "height": h, "display": display
        }))?
    );
    Ok(())
}

/// Detect terminal/editor PANELS on a display via column-activity divider analysis, then
/// (optionally) read each through the vision provider. Native port of `panel_segment.py`:
/// capture → find low-activity vertical gutters → per-panel crop → analyze.
async fn run_segment(args: &[String]) -> Result<()> {
    use gentle_eye::capture::screen::ScreenCapturer;
    use gentle_eye::capture::stream::write_bgra_png;
    use gentle_eye::target::crop::crop_bgra;
    use gentle_eye::target::model::PixelRect;

    if let Some(p) = flag(args, "--provider") {
        std::env::set_var("GENTLE_EYE_PROVIDER", p);
    }
    let display: usize = match flag(args, "--display") {
        Some(s) => s.parse().context("--display must be an integer index")?,
        None => 0,
    };
    let do_read = args.iter().any(|a| a == "--read");

    let mut cap =
        ScreenCapturer::new(display).map_err(|e| anyhow!("screen capture unavailable: {e}"))?;
    let (fw, fh) = (cap.width(), cap.height());
    // First frame from a fresh X11 capturer is often blank/stale; warm up then grab the real one.
    let _ = cap.capture_frame(Duration::from_secs(2));
    std::thread::sleep(Duration::from_millis(120));
    let buf = cap
        .capture_frame(Duration::from_secs(2))
        .map_err(|e| anyhow!("capture failed: {e}"))?;
    let stride = buf.len().checked_div(fh).unwrap_or(fw * 4);

    let panels = detect_panels_bgra(&buf, fw, fh, stride);
    let server = if do_read { Some(GentleEyeServer::new().await?) } else { None };

    let mut regions = Vec::new();
    for (i, &(x0, x1)) in panels.iter().enumerate() {
        let rect = PixelRect { x: x0 as u32, y: 0, w: (x1 - x0) as u32, h: fh as u32 };
        let (bytes, w, h) =
            crop_bgra(&buf, fw, fh, stride, rect).map_err(|e| anyhow!("crop: {e}"))?;
        let png = std::env::temp_dir()
            .join(format!("ge_segment_{}_{}.png", std::process::id(), i + 1));
        write_bgra_png(&bytes, w, h, &png).map_err(|e| anyhow!("png write: {e}"))?;
        let mut entry = serde_json::json!({
            "index": i + 1, "x": x0, "w": x1 - x0, "h": fh, "png": png.to_string_lossy()
        });
        if let Some(srv) = &server {
            let res = srv
                .vision()
                .analyze_image(
                    png.as_path(),
                    "Transcribe the visible code/text in this single terminal panel briefly, \
                     then one phrase on what it is doing.",
                )
                .await;
            entry["analysis"] = match res {
                Ok(r) => serde_json::to_value(r).unwrap_or(serde_json::Value::Null),
                Err(e) => serde_json::json!({ "error": e.to_string() }),
            };
        }
        regions.push(entry);
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "display": display, "width": fw, "height": fh, "panels": regions.len(), "regions": regions
        }))?
    );
    Ok(())
}

/// Column-activity panel divider detection over a BGRA frame. Gutters between panels are
/// low-activity vertical bands; returns panel x-ranges `[(x0, x1), ...]`.
fn detect_panels_bgra(buf: &[u8], w: usize, h: usize, stride: usize) -> Vec<(usize, usize)> {
    if w == 0 || h == 0 {
        return vec![(0, w)];
    }
    let mut act = vec![0f64; w];
    for x in 0..w {
        let (mut sum, mut sq) = (0f64, 0f64);
        for y in 0..h {
            let o = y * stride + x * 4;
            if o + 2 >= buf.len() {
                break;
            }
            let g = (buf[o] as f64 + buf[o + 1] as f64 + buf[o + 2] as f64) / 3.0;
            sum += g;
            sq += g * g;
        }
        let n = h as f64;
        let mean = sum / n;
        act[x] = (sq / n - mean * mean).max(0.0).sqrt();
    }
    let half = 10usize; // smoothing window ~21
    let mut sm = vec![0f64; w];
    for x in 0..w {
        let a = x.saturating_sub(half);
        let b = (x + half + 1).min(w);
        sm[x] = act[a..b].iter().sum::<f64>() / (b - a) as f64;
    }
    let mut sorted = sm.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let thr = sorted[((sorted.len() as f64) * 0.12) as usize];
    let mut div: Vec<usize> = Vec::new();
    let mut i = 0usize;
    while i < w {
        if sm[i] < thr {
            let start = i;
            while i < w && sm[i] < thr {
                i += 1;
            }
            if i - start >= 10 && start > 100 && i < w.saturating_sub(100) {
                div.push((start + i) / 2);
            }
        } else {
            i += 1;
        }
    }
    let mut bounds = vec![0usize];
    bounds.extend(div);
    bounds.push(w);
    let mut panels = Vec::new();
    for t in 0..bounds.len() - 1 {
        if bounds[t + 1] - bounds[t] > 250 {
            panels.push((bounds[t], bounds[t + 1]));
        }
    }
    if panels.is_empty() {
        panels.push((0, w));
    }
    panels
}

/// List redpen annotation captures (newest first) from `~/.gentle-eye/redpen/`.
///
/// Read-only discovery surface: the `redpen` GUI writes a `<ts>.png` + `<ts>.json`
/// sidecar per session; the agent reads this to find the latest artifact, then
/// closes the loop with `gentle-eye analyze --image <png> --prompt … --provider gemini`.
async fn run_redpen_list(args: &[String]) -> Result<()> {
    let limit: usize = flag(args, "--limit").and_then(|s| s.parse().ok()).unwrap_or(20);
    let dir = Path::new(&std::env::var("HOME").unwrap_or_default()).join(".gentle-eye/redpen");

    let mut sidecars: Vec<(u64, PathBuf)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) == Some("json") {
                let mtime = e
                    .metadata()
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                sidecars.push((mtime, p));
            }
        }
    }
    sidecars.sort_by_key(|(t, _)| std::cmp::Reverse(*t)); // newest first

    let captures: Vec<serde_json::Value> = sidecars
        .into_iter()
        .take(limit)
        .filter_map(|(_, p)| {
            let raw = std::fs::read_to_string(&p).ok()?;
            let mut v: serde_json::Value = serde_json::from_str(&raw).ok()?;
            if let Some(obj) = v.as_object_mut() {
                obj.insert("sidecar".into(), serde_json::json!(p.to_string_lossy()));
            }
            Some(v)
        })
        .collect();

    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "dir": dir.to_string_lossy(),
            "count": captures.len(),
            "captures": captures,
        }))?
    );
    Ok(())
}

/// Close the redpen loop: feed a capture (default: the latest) + the boxes the
/// user drew to a vision provider, so the agent acts on exactly what was marked.
///
/// The PNG already has the boxes burned in (Gemini sees them); we ALSO inject
/// each box's label + normalized/pixel region as text, so the model reasons
/// spatially instead of hunting for the red rectangles.
async fn run_redpen_analyze(args: &[String]) -> Result<()> {
    // Provider: default to gemini for this command (the loop-close target).
    let provider = flag(args, "--provider").unwrap_or("gemini");
    std::env::set_var("GENTLE_EYE_PROVIDER", provider);

    // Resolve the image: --image PATH, else the newest *.png in the inbox.
    let dir = Path::new(&std::env::var("HOME").unwrap_or_default()).join(".gentle-eye/redpen");
    let image: PathBuf = match flag(args, "--image") {
        Some(p) => PathBuf::from(p),
        None => latest_redpen_png(&dir)
            .ok_or_else(|| anyhow!("no redpen captures in {} — run `redpen` first", dir.display()))?,
    };
    if !image.exists() {
        return Err(anyhow!("image not found: {}", image.display()));
    }

    // Load the sidecar (same stem, .json) for the box context, if present.
    let sidecar = image.with_extension("json");
    let box_context = sidecar
        .exists()
        .then(|| std::fs::read_to_string(&sidecar).ok())
        .flatten()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .map(|v| describe_boxes(&v))
        .unwrap_or_default();

    // Compose the prompt: box context (if any) + the user's question (or a default).
    let user_q = flag(args, "--prompt").unwrap_or(
        "Describe what is inside each marked region, and flag any issues, bugs, or improvements you see.",
    );
    let prompt = if box_context.is_empty() {
        user_q.to_string()
    } else {
        format!("{box_context}\n{user_q}")
    };

    let server = GentleEyeServer::new().await?;
    let result = server.vision().analyze_image(&image, &prompt).await?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "image": image.to_string_lossy(),
            "prompt": prompt,
            "analysis": result,
        }))?
    );
    Ok(())
}

/// Newest `*.png` in the redpen inbox (by mtime).
fn latest_redpen_png(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("png"))
        .max_by_key(|e| {
            e.metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0)
        })
        .map(|e| e.path())
}

/// Turn a redpen sidecar JSON into a human-readable annotation description for
/// the prompt. Handles the markup schema (pen/arrow/box, with colors) and falls
/// back to the legacy `targets` (named boxes) schema.
fn describe_boxes(v: &serde_json::Value) -> String {
    let (sw, sh) = v
        .get("size")
        .and_then(|s| s.as_array())
        .and_then(|a| Some((a.first()?.as_f64()?, a.get(1)?.as_f64()?)))
        .unwrap_or((0.0, 0.0));
    // px(): normalized [x,y] → "(px, py)" if size known, else "(x.xxx, y.yyy)".
    let px = |p: &[f64]| -> String {
        if p.len() < 2 {
            return "(?)".into();
        }
        if sw > 0.0 && sh > 0.0 {
            format!("({}, {})", (p[0] * sw).round() as i64, (p[1] * sh).round() as i64)
        } else {
            format!("({:.3}, {:.3})", p[0], p[1])
        }
    };
    let pt = |val: Option<&serde_json::Value>| -> Vec<f64> {
        val.and_then(|a| a.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_f64()).collect())
            .unwrap_or_default()
    };

    // Markup schema (pen / arrow / box).
    if let Some(arr) = v.get("annotations").and_then(|a| a.as_array()) {
        if arr.is_empty() {
            return String::new();
        }
        let mut out = format!(
            "The user drew {} annotation(s) on this screenshot. Read them as visual direction (where to look, what to change, what to move):\n",
            arr.len()
        );
        for a in arr {
            let ty = a.get("type").and_then(|t| t.as_str()).unwrap_or("mark");
            let color = a.get("color").and_then(|c| c.as_str()).unwrap_or("red");
            match ty {
                "arrow" => {
                    let f = pt(a.get("from"));
                    let t = pt(a.get("to"));
                    out.push_str(&format!(
                        "- {color} ARROW from {} to {} — points toward / indicates moving something to the arrow's head.\n",
                        px(&f), px(&t)
                    ));
                }
                "box" => {
                    let r = pt(a.get("rect"));
                    if r.len() == 4 && sw > 0.0 && sh > 0.0 {
                        out.push_str(&format!(
                            "- {color} BOX at pixels ({}, {}) {}×{} — a marked region.\n",
                            (r[0] * sw).round() as i64, (r[1] * sh).round() as i64,
                            (r[2] * sw).round() as i64, (r[3] * sh).round() as i64,
                        ));
                    } else {
                        out.push_str(&format!("- {color} BOX (normalized {r:?}).\n"));
                    }
                }
                "pen" => {
                    let pts = a.get("points").and_then(|p| p.as_array());
                    let n = pts.map(|p| p.len()).unwrap_or(0);
                    // Bounding box of the stroke gives the model a region anchor.
                    if let Some(pts) = pts {
                        let xs: Vec<f64> = pts.iter().filter_map(|p| p.as_array()?.first()?.as_f64()).collect();
                        let ys: Vec<f64> = pts.iter().filter_map(|p| p.as_array()?.get(1)?.as_f64()).collect();
                        if let (Some(&x0), Some(&y0)) = (xs.iter().min_by(|a, b| a.total_cmp(b)), ys.iter().min_by(|a, b| a.total_cmp(b))) {
                            out.push_str(&format!(
                                "- {color} freehand PEN mark ({n} pts) around {}.\n",
                                px(&[x0, y0])
                            ));
                            continue;
                        }
                    }
                    out.push_str(&format!("- {color} freehand PEN mark ({n} pts).\n"));
                }
                other => out.push_str(&format!("- {color} {other} annotation.\n")),
            }
        }
        return out;
    }

    // Legacy schema: named target boxes.
    let targets = match v.get("targets").and_then(|t| t.as_array()) {
        Some(t) if !t.is_empty() => t,
        _ => return String::new(),
    };
    let mut out = format!(
        "This screenshot has {} annotation box(es) drawn in red. Focus on these marked regions:\n",
        targets.len()
    );
    for t in targets {
        let label = t.get("label").and_then(|l| l.as_str()).unwrap_or("box");
        let n = pt(t.get("rect"));
        if n.len() == 4 {
            if sw > 0.0 && sh > 0.0 {
                out.push_str(&format!(
                    "- \"{label}\": normalized [{:.3}, {:.3}, {:.3}, {:.3}] ≈ pixels ({}, {}) {}×{}\n",
                    n[0], n[1], n[2], n[3],
                    (n[0] * sw).round() as i64, (n[1] * sh).round() as i64,
                    (n[2] * sw).round() as i64, (n[3] * sh).round() as i64,
                ));
            } else {
                out.push_str(&format!(
                    "- \"{label}\": normalized [{:.3}, {:.3}, {:.3}, {:.3}]\n",
                    n[0], n[1], n[2], n[3]
                ));
            }
        }
    }
    out
}

/// Parse a `x,y,w,h` normalized-coordinate string into a `NormRect`.
fn parse_region(s: &str) -> Result<gentle_eye::target::model::NormRect> {
    let parts: Vec<f64> = s
        .split(',')
        .map(|p| p.trim().parse::<f64>())
        .collect::<std::result::Result<_, _>>()
        .context("--region must be four numbers: x,y,w,h")?;
    if parts.len() != 4 {
        return Err(anyhow!("--region must be exactly four numbers: x,y,w,h"));
    }
    Ok(gentle_eye::target::model::NormRect::new(
        parts[0], parts[1], parts[2], parts[3],
    ))
}

async fn run_list(args: &[String]) -> Result<()> {
    let limit: usize = flag(args, "--limit")
        .unwrap_or("20")
        .parse()
        .context("--limit must be an integer")?;
    let status = flag(args, "--status").and_then(parse_status);
    let server = GentleEyeServer::new().await?;
    let recordings = server.recording().list_recordings(limit, status).await?;
    println!("{}", serde_json::to_string_pretty(&recordings)?);
    Ok(())
}

async fn run_provider_info(args: &[String]) -> Result<()> {
    if let Some(p) = flag(args, "--provider") {
        std::env::set_var("GENTLE_EYE_PROVIDER", p);
    }
    let server = GentleEyeServer::new().await?;
    let vision = server.vision();
    let health = vision.health_check().await;
    let info = serde_json::json!({
        "provider": vision.name(),
        "model": vision.model(),
        "max_video_size_bytes": vision.max_video_size(),
        "supports_native_video": vision.supports_native_video(),
        "available": health.is_ok(),
        "error_message": health.err().map(|e| e.to_string()),
    });
    println!("{}", serde_json::to_string_pretty(&info)?);
    Ok(())
}

async fn run_read_text(args: &[String]) -> Result<()> {
    use gentle_eye::analysis::ocr;
    if !ocr::ocr_available() {
        return Err(anyhow!(
            "tesseract not found on PATH — install it (e.g. apt install tesseract-ocr)"
        ));
    }
    let text = if let Some(video) = flag(args, "--video") {
        ocr::ocr_video(Path::new(video))?
    } else if let Some(image) = flag(args, "--image") {
        ocr::ocr_image(Path::new(image))?
    } else {
        return Err(anyhow!("provide --image PATH or --video PATH"));
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({ "text": text }))?
    );
    Ok(())
}

async fn run_displays(_args: &[String]) -> Result<()> {
    let displays = DisplayManager::list_available()?;
    println!("{}", serde_json::to_string_pretty(&displays)?);
    Ok(())
}

/// `regions` — the Region engine (E5): fused window/element regions as JSON.
///   `--window`            window-level only (WM / EWMH), fastest
///   `--depth pane|element|text`   also walk the AT-SPI tree (default: element)
/// Every region carries `source` + `trust` + (structural) `role`/`label`.
async fn run_regions(args: &[String]) -> Result<()> {
    use gentle_eye::regions::{detect, locate, Granularity};
    // `--contrast [--display N]` → the salient high-contrast content region (pixel fallback for
    // windowless / AT-SPI-less content). Separate from the structural union.
    if args.iter().any(|a| a == "--contrast") {
        use gentle_eye::regions::providers::contrast::ContrastProvider;
        let display: usize = flag(args, "--display").and_then(|s| s.parse().ok()).unwrap_or(0);
        let region =
            tokio::task::spawn_blocking(move || ContrastProvider::salient_region(display).ok().flatten()).await?;
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({ "contrast_display": display, "region": region }))?
        );
        return Ok(());
    }
    let depth = if args.iter().any(|a| a == "--window") {
        Granularity::Window
    } else {
        match flag(args, "--depth") {
            Some("window") => Granularity::Window,
            Some("pane") => Granularity::Pane,
            Some("text") => Granularity::Text,
            _ => Granularity::Element, // default: WM + AT-SPI structural tree
        }
    };
    let query = flag(args, "--match").map(str::to_string);
    // detect() does blocking work (x11 + a spawned AT-SPI runtime) — keep it off the async thread.
    let regions = tokio::task::spawn_blocking(move || detect(depth)).await?;
    let json = if let Some(q) = query {
        // `--match "<nl>"` → resolve to the single best region (ordinal/position/label, no VLM).
        let matched = locate(&q, &regions).map(|i| &regions[i]);
        serde_json::json!({ "query": q, "matched": matched })
    } else {
        serde_json::json!({ "count": regions.len(), "regions": regions })
    };
    println!("{}", serde_json::to_string_pretty(&json)?);
    Ok(())
}

async fn run_label(args: &[String]) -> Result<()> {
    let index: usize = flag(args, "--display")
        .ok_or_else(|| anyhow!("--display <index> is required"))?
        .parse()
        .context("--display must be an integer index")?;
    let name = flag(args, "--name").ok_or_else(|| anyhow!("--name <label> is required"))?;
    let mut manager = DisplayManager::new(None)?;
    manager.set_display_label(index, name.to_string())?;
    manager.save_config()?;
    println!("{}", serde_json::to_string_pretty(manager.list_displays())?);
    Ok(())
}

/// Resolve a `--display` selector (numeric index or a saved label) to an index.
fn resolve_display(selector: &str) -> Result<usize> {
    if let Ok(index) = selector.parse::<usize>() {
        return Ok(index);
    }
    let displays = DisplayManager::list_available()?;
    DisplayConfig::load()
        .unwrap_or_default()
        .find_by_label(selector, &displays)
        .ok_or_else(|| {
            anyhow!("no display labeled '{selector}' — run `gentle-eye displays` to see options")
        })
}

// ---- MCP server (default) --------------------------------------------------

/// Run the MCP server over stdio with graceful shutdown.
async fn run_serve() -> Result<()> {
    info!(
        "Starting Gentle-Eye MCP server v{}",
        env!("CARGO_PKG_VERSION")
    );
    let config = match AppConfig::load() {
        Ok(cfg) => {
            info!("Configuration loaded successfully");
            cfg
        }
        Err(e) => {
            error!("Failed to load configuration: {}", e);
            return Err(anyhow!("Configuration error: {}", e));
        }
    };

    info!("Running startup validation checks...");
    let validation = validate_startup(&config);
    for warning in validation.warnings() {
        warn!("Startup warning: {}", warning);
    }
    if !validation.all_passed {
        for err in validation.errors() {
            error!("Startup check failed: {}", err);
            match err {
                StartupError::FfmpegNotFound { install_command } => {
                    error!("FFmpeg is required for video encoding.");
                    error!("Install it with: {}", install_command);
                }
                StartupError::StorageDirectoryNotAccessible { path, reason } => {
                    error!("Cannot access storage directory: {:?}", path);
                    error!("Reason: {}", reason);
                }
                StartupError::StorageDirectoryNotWritable { path } => {
                    error!("Storage directory is not writable: {:?}", path);
                }
                StartupError::ScreenCapturePermissionDenied(msg) => error!("{}", msg),
                StartupError::EnvVarMissing { var_name, hint } => {
                    error!("Required environment variable {} is not set.", var_name);
                    error!("Hint: {}", hint);
                }
                StartupError::ConfigError(msg) => error!("Configuration error: {}", msg),
            }
        }
        return Err(anyhow!("Startup validation failed. See errors above."));
    }
    info!(
        "Startup validation passed with {} warning(s)",
        validation.warning_count
    );

    let server = GentleEyeServer::new().await?;
    info!(
        "Configuration: provider={}, fps={}",
        server.config().vision.provider,
        server.config().recording.fps
    );

    let server_handle = tokio::spawn(async move { server.serve_stdio().await });
    tokio::select! {
        _ = signal::ctrl_c() => {
            info!("Shutdown signal received, stopping server...");
        }
        result = server_handle => {
            match result {
                Ok(Ok(())) => info!("Server stopped normally"),
                Ok(Err(e)) => error!("Server error: {}", e),
                Err(e) => error!("Server task failed: {}", e),
            }
        }
    }
    Ok(())
}

/// `gentle-eye dayflow <start|stop|status|timeline|ask>`.
///
/// A thin adapter over the ONE service, exactly like the MCP tools: it parses
/// argv, calls the service, prints JSON. No decision is made here, because a
/// decision made in one surface is a decision the other two do not make.
async fn run_dayflow(args: &[String]) -> Result<()> {
    let server = GentleEyeServer::new().await?;
    let df = server.dayflow();
    let sub = args.first().map(String::as_str).unwrap_or("status");
    let rest: &[String] = if args.len() > 1 { &args[1..] } else { &[] };

    match sub {
        "start" => {
            let mode = match flag(rest, "--mode").as_deref() {
                None | Some("session") => gentle_eye::dayflow::models::DayflowMode::Session,
                Some("daemon") => gentle_eye::dayflow::models::DayflowMode::Daemon,
                Some(other) => return Err(anyhow!("unknown mode '{other}': use session or daemon")),
            };
            let displays = match flag(rest, "--displays") {
                Some(list) => list
                    .split(',')
                    .map(|s| s.trim().parse::<u32>())
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| anyhow!("bad --displays: {e}"))?,
                None => vec![0],
            };
            let id = df.start(mode, displays, Utc::now())?;
            println!("{}", serde_json::json!({ "session_id": id.to_string() }));
        }
        "stop" => {
            let closed = df.stop(Utc::now())?;
            println!("{}", serde_json::json!({ "windows_closed": closed.len() }));
        }
        "status" => {
            let status = df.status(Utc::now())?;
            println!("{}", serde_json::to_string_pretty(&status)?);
            // EXIT 0 EVEN WHEN DEGRADED. A degraded session is running, just
            // not producing; exiting non-zero would make every script treat a
            // recoverable state as a crash, and the state is already in the
            // payload for anything that wants to act on it.
        }
        "timeline" => {
            let (from, to) = parse_cli_range(rest)?;
            let entries = df.timeline(from, to)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "from": from.to_rfc3339(),
                    "to": to.to_rfc3339(),
                    "entries": entries,
                }))?
            );
        }
        "ask" => {
            let question = rest
                .iter()
                .find(|a| !a.starts_with("--"))
                .ok_or_else(|| anyhow!("usage: gentle-eye dayflow ask \"<question>\""))?;
            let (from, to) = parse_cli_range(rest)?;
            let answer = df.ask(question, from, to, |prompt| {
                format!("[no model configured for ask]\n{prompt}")
            })?;
            println!("{}", serde_json::to_string_pretty(&answer)?);
        }
        other => return Err(anyhow!("unknown dayflow subcommand '{other}'")),
    }
    Ok(())
}

/// The same range defaulting the other surfaces use — today so far.
fn parse_cli_range(args: &[String]) -> Result<(DateTime<Utc>, DateTime<Utc>)> {
    let parse = |s: &str| -> Result<DateTime<Utc>> {
        Ok(DateTime::parse_from_rfc3339(s)
            .map_err(|e| anyhow!("bad timestamp '{s}': {e}"))?
            .with_timezone(&Utc))
    };
    let to = match flag(args, "--to") {
        Some(s) => parse(&s)?,
        None => Utc::now(),
    };
    let from = match flag(args, "--from") {
        Some(s) => parse(&s)?,
        None => to.date_naive().and_hms_opt(0, 0, 0).map(|d| d.and_utc()).unwrap_or(to),
    };
    if from > to {
        return Err(anyhow!("range starts after it ends: {from} > {to}"));
    }
    Ok((from, to))
}
