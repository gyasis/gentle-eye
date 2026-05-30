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
  gentle-eye capture-stream --url URL [--out DIR]     Grab one frame from a stream (ATEM/RTSP/HTTP)
  gentle-eye list    [--status all|recording|completed|cancelled|failed] [--limit N]
  gentle-eye read-text --image PATH | --video PATH    Extract on-screen text (OCR) as JSON
  gentle-eye displays                                 List available displays (the catalogue)
  gentle-eye label   --display IDX --name \"left\"      Label a display (persists across runs)
  gentle-eye target add NAME (--display IDX | --stream URL) --region x,y,w,h   Define a crop (normalized 0-1)
  gentle-eye target use NAME                          Make NAME the active target
  gentle-eye target list                              List targets + the active one
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
    let frame = gentle_eye::capture::stream::capture_stream_frame(url, Path::new(out))?;
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
