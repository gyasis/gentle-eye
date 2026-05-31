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
  gentle-eye redpen-list [--limit N]                  List redpen annotation captures (newest first) for the agent to ingest
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
        "redpen-list" => run_redpen_list(rest).await,
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
    use gentle_eye::target::model::PixelRect;
    use gentle_eye::target::store::TargetStore;

    let out = flag(args, "--out").ok_or_else(|| anyhow!("--out FILE.png is required"))?;
    let display: usize = match flag(args, "--display") {
        Some(s) => s.parse().context("--display must be an integer index")?,
        None => 0,
    };

    let mut cap =
        ScreenCapturer::new(display).map_err(|e| anyhow!("screen capture unavailable: {e}"))?;
    let (fw, fh) = (cap.width(), cap.height());
    let buf = cap
        .capture_frame(Duration::from_secs(2))
        .map_err(|e| anyhow!("capture failed: {e}"))?;
    let stride = buf.len().checked_div(fh).unwrap_or(fw * 4);

    // Crop rect: --region (normalized) > --target NAME > full frame.
    let rect = if let Some(r) = flag(args, "--region") {
        let region = parse_region(r)?;
        if !region.is_valid() {
            return Err(anyhow!("--region must lie within 0-1 with positive area"));
        }
        norm_to_pixel(region, (fw as u32, fh as u32), (0, 0))
    } else if let Some(name) = flag(args, "--target") {
        let store = TargetStore::load().map_err(|e| anyhow!("{e}"))?;
        let t = store
            .list()
            .iter()
            .find(|t| t.name == name)
            .cloned()
            .ok_or_else(|| anyhow!("no target named '{name}'"))?;
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
