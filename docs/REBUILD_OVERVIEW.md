# gentle-eye — Rebuild Overview (synthesized)

**Date:** 2026-05-08
**Source:** Mining of 3,134 session jsonls + 37 specstory MDs (~944 MB)
**Status:** Substantial design recovery — enough to rebuild from scratch with fidelity.

This is the high-signal synthesis. See companion files for raw extracts:
- [REBUILD_DEPENDENCIES.md](REBUILD_DEPENDENCIES.md) — crate inventory
- [REBUILD_API_INTEGRATIONS.md](REBUILD_API_INTEGRATIONS.md) — API/lib usage frequency
- [REBUILD_KEY_CODE.md](REBUILD_KEY_CODE.md) — 1,111 Rust signatures
- [REBUILD_FILESYSTEM.md](REBUILD_FILESYSTEM.md) — full file tree (10,747 paths)
- [REBUILD_PRD.md](REBUILD_PRD.md) — PRD-style snippets
- [REBUILD_ARCHITECTURE.md](REBUILD_ARCHITECTURE.md) — design content

---

## What gentle-eye is

A **Rust MCP server that gives AI agents access to screen capture and AI-powered video/image analysis**. Implements the Model Context Protocol over `rmcp` (Rust MCP SDK) and exposes 11 tools to AI clients. Vision analysis is pluggable across providers (default: **Gemini 2.0 Flash**).

## The 11 MCP tools (ground truth, recovered verbatim from doc-comments)

| Tool | Purpose |
|---|---|
| `start_recording` | Begin a screen-capture session (configurable FPS, display, max duration) |
| `stop_recording` | End an active recording and save the video |
| `get_recording_status` | Check the state of a recording |
| `analyze_media` | **Unified** — analyze video OR image via vision AI |
| `analyze_video` | Legacy — kept for backwards compatibility (use `analyze_media`) |
| `list_recordings` | Browse recording history with metadata |
| `cancel_recording` | Abort a recording without saving |
| `get_vision_provider_info` | Check which vision AI provider is configured |
| `list_displays` | Enumerate available displays/monitors |
| `set_display_label` | Assign a friendly name to a display (persists) |
| `take_screenshot` | Capture a single frame as PNG (faster than recording) |

Each tool has typed `*Input` and `*Output` structs with `Deserialize`/`Serialize`/`JsonSchema` derives via the `schemars` crate. JSON schemas are auto-generated for MCP tool registration.

## Workspace / crate layout (recovered from `[dependencies]` blocks)

Cargo workspace with **4 crates**:

```
gentle-eye/                                  workspace root
├── Cargo.toml                              (workspace, dev-deps, profile.bench)
├── src/                                    main crate
│   ├── bin/gentle-eye.rs                   binary entry point
│   ├── lib.rs                              re-exports
│   ├── mcp/
│   │   ├── server.rs                       GentleEyeServer (impl ServerHandler)
│   │   ├── tools.rs                        Input/Output types for all 11 tools
│   │   └── handlers.rs                     tool dispatch
│   ├── capture/
│   │   ├── mod.rs
│   │   ├── screen.rs                       ScreenCapturer
│   │   ├── encoder.rs                      EncoderMode, EncoderState, PipeEncoder
│   │   ├── frame_rate.rs
│   │   ├── memory.rs
│   │   ├── service.rs                      RecordingService
│   │   └── display.rs                      multi-monitor support
│   ├── models/
│   │   ├── mod.rs                          (Recording, RecordingStatus, EncoderMode re-exports)
│   │   ├── analysis.rs                     AnalysisRequest, AnalysisResult, TimeRange
│   │   └── config.rs                       RecordingConfig
│   ├── vision/
│   │   └── mod.rs                          VisionProvider trait + Gemini, Ollama impls
│   ├── storage/
│   │   └── mod.rs                          rusqlite-backed history
│   ├── security/
│   │   ├── rate_limiter.rs                 OWASP A04:2021 mitigation
│   │   └── path_validator.rs               CWE-22 / A01:2021 mitigation
│   └── error.rs
├── benches/
│   ├── capture_performance.rs              criterion bench
│   └── mcp_response_time.rs                criterion bench
└── modules/
    └── rust-record/                         sub-workspace
        ├── video-capture/                   reusable Rust video capture lib
        │   └── src/lib.rs                   (exports ScreenCapturer, CaptureEvent, RawFrame, ...)
        ├── region-selector/                 region-selection logic
        └── region-selector-ui/              Slint UI for region selection
```

## Confirmed dependencies (from `[dependencies]` blocks recovered in sessions)

### Production
- `tokio` — async runtime (40k+ mentions across sessions)
- `rmcp` — Rust MCP SDK (the MCP protocol library)
- `serde`, `serde_json` — JSON serialization
- `schemars` — JSON Schema generation for MCP tool schemas
- `chrono` (workspace) — timestamps
- `uuid` (workspace, features=`["v4"]`) — recording IDs
- `rusqlite` (`0.31`, features=`["bundled"]`) — SQLite for recording history
- `reqwest` — HTTP client (vision API calls)
- `clap` / `argh` — CLI arg parsing
- `tracing` / `log` — structured logging
- `image` — PNG screenshot output
- `slint-build = "=1.13.1"` (build-dep for region-selector-ui)

### Development
- `mockall = "0.12"` — mocking
- `tempfile = "3"`
- `serial_test = "3"` — sequential test execution
- `criterion = { version = "0.5", features = ["html_reports"] }` — benchmarks
- `tokio-test = "0.4"`

### Inferred from `use ::` frequency
ratatui (TUI?), crossterm, kurbo (Slint geometry), ndarray (frame buffers), tower, hyper, futures, anyhow, hex, base64, glob, bytes, regex, hashbrown, smallvec, once_cell, sysinfo, async_trait

## Core data models (recovered struct definitions)

### `Recording`
```rust
struct Recording {
    id: Uuid,                          // Uuid::new_v4()
    status: RecordingStatus,           // Recording, Stopped, Cancelled, Failed
    start_time: DateTime<Utc>,
    end_time: Option<DateTime<Utc>>,
    duration_ms: Option<u64>,
    file_path: Option<PathBuf>,
    fps: u32,                          // default 1
    width: u32,                        // 1920
    height: u32,                       // 1080
    file_size_bytes: Option<u64>,
    error_message: Option<String>,
    display_name: Option<String>,      // "Primary Display"
    encoder_mode: EncoderMode,         // EncoderMode::FileBased
}
```

### `AnalysisRequest`
```rust
struct AnalysisRequest {
    recording_id: Option<Uuid>,
    video_path: PathBuf,
    prompt: String,
    provider: String,                  // "gemini"
    timestamp: DateTime<Utc>,
    timeframe: Option<TimeRange>,
}
```

### `TimeRange`
```rust
struct TimeRange { start_seconds: f64, end_seconds: f64 }
// Methods: new (panics on invalid), try_new (Option), duration_seconds(),
//          duration_ms(), contains(time_seconds: f64) -> bool
```

### `AnalysisResult`
```rust
// Constructors: success(request_id, analysis_text, model_used, processing_time_ms)
//               and presumably failure(...)
```

### `RecordingConfig`
```rust
struct RecordingConfig {
    fps: u32,                          // default 1
    max_duration_seconds: u64,         // default 1800 (30 min)
    // ... output_directory, etc.
}
// Methods: new() (defaults), with_fps(fps: u32), output_directory(path)
```

## Vision provider architecture

Pluggable via `VisionProvider` trait. Confirmed providers:
- **Gemini** (default; model: `gemini-2.0-flash`)
- **Ollama** (local fallback)
- API key env vars: `GEMINI_API_KEY`, `GOOGLE_API_KEY`, `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`

## Security (OWASP-mapped)

The MCP server includes explicit security middleware:
- **Rate limiter** for `analyze_video`/`analyze_media` — 10 req/min default, configurable. **OWASP A04:2021** (Insecure Design), **task T103**.
- **Path validator** for `video_path` inputs — prevents directory traversal. **CWE-22**, **OWASP A01:2021**, **task T101**.
- **Prompt validation** to prevent prompt injection. **task T061**.

## SpecKit feature: `001-mcp-screen-tools`

This was the project's first (and primary) feature spec. The spec lives at `specs/001-mcp-screen-tools/` and includes spec.md, plan.md, tasks.md, research.md, data-model.md, quickstart.md, analysis-report.md, plus contracts/ and checklists/. **The spec content itself was not captured**, but task IDs T061 / T101 / T103 are referenced inline in code doc-comments above.

## Rebuild path

1. **`cargo new --lib gentle-eye`** + workspace setup; carve out `modules/rust-record/{video-capture,region-selector,region-selector-ui}` as workspace members.
2. **Pin deps from REBUILD_DEPENDENCIES.md** — start with the explicitly recovered versions, fill the rest from `use ::` frequency.
3. **Define core models first** (`models/`): `Recording`, `RecordingStatus`, `EncoderMode`, `AnalysisRequest`, `AnalysisResult`, `TimeRange`, `RecordingConfig` — all directly recoverable from this doc.
4. **Define MCP tool I/O types** (`mcp/tools.rs`) for the 11 tools listed above. Each `*Input`/`*Output` derives `Serialize`/`Deserialize`/`JsonSchema`.
5. **Implement `GentleEyeServer`** (`mcp/server.rs`) — struct holding `recording_service`, `vision_provider`, `storage_manager`, `display_manager`, `rate_limiter`, `path_validator`. Impl `ServerHandler` from `rmcp`.
6. **Build out `capture/`** — screen.rs, encoder.rs, frame_rate.rs, memory.rs, service.rs, display.rs (multi-monitor).
7. **Vision provider** — trait + Gemini impl (default model `gemini-2.0-flash`) + Ollama impl.
8. **SQLite storage** via `rusqlite` for recording history.
9. **Security middleware** — rate limiter (10 req/min default) and path validator (CWE-22 mitigation).
10. **Bench harness** — `benches/capture_performance.rs` + `benches/mcp_response_time.rs` with `criterion`.

The two days of session content (Dec 21–22, 2025) plus subsequent re-explorations gave us this much detail; deeper read of `RECOVERY/sessions/_seg11_s245320_665eb1419fc4.md` and similar large session files will fill in additional struct fields and method signatures.
