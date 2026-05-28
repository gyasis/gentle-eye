# gentle-eye — Product Requirements Document (Reconstructed)

**Doc version:** 1.0 — Reconstructed from disaster recovery
**Date:** 2026-05-08
**Project status:** WIPED — rebuilding from session-recovered design intent
**Original development window:** 2025-12-21 → 2025-12-22 (and possibly later)
**SpecKit feature:** `001-mcp-screen-tools`

> Note: This PRD is reconstructed by mining 633 session files (3,134 total) and 18 SpecStory MDs from the EC2 OpenSearch recovery index. Direct file content for `specs/001-mcp-screen-tools/spec.md` was NOT captured; this doc synthesizes the design intent from the conversational record. Mark gaps explicitly with `[GAP: ...]`.

---

## 1. Vision

**gentle-eye** is a Rust **Model Context Protocol (MCP) server** that gives AI agents (Claude Desktop, Claude Code CLI, Cursor, others) the ability to:
1. Capture the user's screen (full or region) at a configurable frame rate.
2. Save recordings to disk with metadata for later retrieval.
3. Send video or single screenshots to a vision-AI provider for analysis.
4. Return structured analysis results to the calling AI agent.

Effectively — **a "gentle eye" that lets AI agents see what the user is seeing**, on demand, without intrusive always-on telemetry. Conceived as a Rust + privacy-conscious alternative to Dayflow.

## 2. Personas & use cases

| Actor | Use case |
|---|---|
| Developer using Claude Code | "Record me trying this bug for 30s, then explain what went wrong" |
| Documentation writer | "Take a screenshot of the active window and describe what's on screen" |
| QA engineer | "Record a test session, then analyze if the user flow worked correctly" |
| AI agent (autonomous) | Diagnose visual UI states without screen-reader heuristics |

## 3. Architecture (recovered ASCII diagram)

```
+-------------------+     +-------------------+     +-------------------+
|   Claude Desktop  |     |      Cursor       |     |  Claude Code CLI  |
+--------+----------+     +---------+---------+     +---------+---------+
         |                          |                         |
         +------------+-------------+-------------+-----------+
                      |
                      v  MCP Protocol (JSON-RPC over stdio)
         +------------+-------------+
         |      gentle-eye server   |
         |  +---------------------+ |
         |  |     MCP Handler     | |  <-- Tool dispatch & response
         |  +----------+----------+ |
         |             |            |
         |  +----------v----------+ |
         |  |      Contracts      | |  <-- Trait interfaces
         |  +--+-------+-------+--+ |
         |     |       |       |    |
         |  +--v--+ +--v--+ +--v--+ |
         |  |Capt.| |Stor.| |Anal.| |  <-- Implementation modules
         |  +--+--+ +--+--+ +--+--+ |
         +-----|-------|-------|----+
               |       |       |
               v       v       v
         +---------+ +----+ +--------+
         |  scrap  | |SQL | | Gemini |
         | +FFmpeg | |ite | | Ollama |
         +---------+ +----+ +--------+
```

**Three modules behind trait interfaces:**
- **Capture** — `scrap` for cross-platform screen capture, FFmpeg for encoding to MP4
- **Storage** — `rusqlite` (SQLite, bundled feature) for recording history & analysis results
- **Analysis** — pluggable `VisionProvider` trait; built-in Gemini + Ollama implementations

**Three MCP clients confirmed as integration targets:** Claude Desktop, Cursor, Claude Code CLI.

## 4. The 11 MCP tools

Recovered verbatim from the doc-comment of `mcp/server.rs`:

| Tool | Description |
|---|---|
| `start_recording` | Begin a new screen-capture session (configurable FPS, display, max duration) |
| `stop_recording` | End an active recording and save the video |
| `get_recording_status` | Check the state of a recording |
| `cancel_recording` | Abort a recording without saving |
| `analyze_media` | **Unified** — analyze video OR image via vision AI |
| `analyze_video` | Legacy alias of analyze_media (kept for backwards compatibility) |
| `list_recordings` | Browse recording history with metadata |
| `get_vision_provider_info` | Check which vision AI provider is configured |
| `list_displays` | Enumerate available displays/monitors |
| `set_display_label` | Assign a friendly label to a display (persists in SQLite) |
| `take_screenshot` | Capture a single frame as PNG (faster path than recording) |

**Schema convention:** every tool has typed `*Input` and `*Output` structs, all derive `Serialize` + `Deserialize` + `JsonSchema` (via `schemars` crate). JSON Schema is auto-generated for MCP tool registration.

## 5. Functional requirements

> Note: original `specs/001-mcp-screen-tools/spec.md` was not captured. The list below is reconstructed from session discussions and recovered tool doc-comments. **Numbering is suggested**, not original.

### Recording
- **FR-001**: System MUST allow starting a new recording with configurable FPS (default 1, range 1–60) via `start_recording`.
- **FR-002**: System MUST allow specifying a target display (by index or label) at recording start.
- **FR-003**: System MUST enforce a configurable maximum duration (default 1800 s / 30 min) per recording.
- **FR-004**: System MUST stop a recording on demand via `stop_recording` and save the resulting MP4.
- **FR-005**: System MUST allow cancellation of an active recording without saving via `cancel_recording`.
- **FR-006**: System MUST report current recording state (idle, recording, encoding, saved, failed) via `get_recording_status`.
- **FR-007**: System MUST persist recording metadata (id, timestamps, fps, dimensions, file_path, file_size, status, display_name) to SQLite.
- **FR-008**: System MUST list recent recordings with metadata via `list_recordings`.

### Capture
- **FR-009**: System MUST capture from a specified display in a multi-monitor setup.
- **FR-010**: System MUST support both file-based and pipe-based encoder modes (`EncoderMode::FileBased`, `EncoderMode::PipeBased`).
- **FR-011**: System MUST capture single screenshots as PNG via `take_screenshot` (faster path than full recording).

### Display management
- **FR-012**: System MUST enumerate available displays with their properties (index, resolution, label) via `list_displays`.
- **FR-013**: System MUST support assigning custom labels to displays via `set_display_label`, with labels persisted to SQLite.

### Analysis
- **FR-014**: System MUST support pluggable vision providers via the `VisionProvider` trait.
- **FR-015**: System MUST ship with at least Gemini (default model `gemini-2.0-flash`) and Ollama implementations.
- **FR-016**: System MUST accept a video OR image input plus a natural-language prompt via `analyze_media`.
- **FR-017**: System MUST optionally accept a `TimeRange` (start_seconds, end_seconds) to scope analysis to a video segment.
- **FR-018**: System MUST return structured `AnalysisResult` (analysis_text, model_used, processing_time_ms, success/failure).
- **FR-019**: System MUST persist analysis requests and results to SQLite for replay/audit.
- **FR-020**: System MUST report current vision provider via `get_vision_provider_info`.

### Configuration
- **FR-021**: System MUST read API keys from environment (`GEMINI_API_KEY`, `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `GOOGLE_API_KEY` as appropriate).
- **FR-022**: System MUST support a `gentle-eye.toml` configuration file (path/output dir, default FPS, default provider, rate-limit thresholds).
- **FR-023**: System MUST validate environment + prerequisites at startup (required env vars present, FFmpeg available, output dir writable).

### MCP integration
- **FR-024**: System MUST register itself as a stdio MCP server compatible with Claude Desktop, Cursor, and Claude Code CLI.
- **FR-025**: System MUST emit JSON Schema for every tool via `tools/list` per MCP spec.
- **FR-026**: System MUST dispatch `tools/call` requests to the correct handler with input validation.
- **FR-027**: System MUST map internal errors to MCP error codes with clear messages.

## 6. Non-functional requirements (security & quality)

| ID | Description | Mapping |
|---|---|---|
| **NFR-001** | Path traversal mitigation: all `video_path` inputs MUST be validated to stay within the configured recording directory | **CWE-22**, **OWASP A01:2021**, task **T101** |
| **NFR-002** | Rate limiting on `analyze_video`/`analyze_media` to prevent vision-API abuse: default 10 req/min, configurable | **OWASP A04:2021** (Insecure Design), task **T103** |
| **NFR-003** | Prompt validation to prevent prompt-injection attacks via the MCP `prompt` field | task **T061** |
| **NFR-004** | API keys MUST never be logged or returned in tool responses | Standard hygiene |
| **NFR-005** | The MCP server MUST handle Ctrl+C gracefully (cancel in-flight ops, save partial recordings, no zombie FFmpeg processes) | Operational |
| **NFR-006** | Capture latency target: <50 ms from `start_recording` to first frame written | Performance |
| **NFR-007** | MCP `tools/call` p95 response time: <100 ms for non-AI tools, <30 s for `analyze_*` (network-bounded) | Performance |
| **NFR-008** | Bench harnesses: `benches/capture_performance.rs` and `benches/mcp_response_time.rs` MUST run via `cargo bench` and produce HTML reports (criterion 0.5) | Quality |

## 7. Data model

### `Recording`
```rust
struct Recording {
    id: Uuid,                                // Uuid::new_v4()
    status: RecordingStatus,                 // Recording, Stopped, Cancelled, Failed
    start_time: DateTime<Utc>,
    end_time: Option<DateTime<Utc>>,
    duration_ms: Option<u64>,
    file_path: Option<PathBuf>,
    fps: u32,                                // default 1
    width: u32,                              // default 1920
    height: u32,                             // default 1080
    file_size_bytes: Option<u64>,
    error_message: Option<String>,
    display_name: Option<String>,            // e.g. "Primary Display"
    encoder_mode: EncoderMode,               // FileBased | PipeBased
}
```

### `AnalysisRequest`
```rust
struct AnalysisRequest {
    recording_id: Option<Uuid>,
    video_path: PathBuf,
    prompt: String,
    provider: String,                        // "gemini", "ollama"
    timestamp: DateTime<Utc>,
    timeframe: Option<TimeRange>,
}
```

### `AnalysisResult`
```rust
// Constructors:
//   AnalysisResult::success(request_id, analysis_text, model_used, processing_time_ms)
//   AnalysisResult::failure(request_id, error_message, model_used, processing_time_ms)
```

### `TimeRange`
```rust
struct TimeRange { start_seconds: f64, end_seconds: f64 }
// new(s, e)             — panics on invalid
// try_new(s, e)         — Option<Self>
// duration_seconds()    — f64
// duration_ms()         — u64
// contains(time: f64)   — bool
```

### `RecordingConfig`
```rust
struct RecordingConfig {
    fps: u32,                                // default 1
    max_duration_seconds: u64,               // default 1800
    output_directory: PathBuf,
    // ... other recording defaults
}
// new(), with_fps(u32), output_directory(PathBuf)
```

## 8. Implementation tasks (recovered from sessions)

### Setup phase (T001–T010)
- **T001**: `Cargo.toml` workspace + main crate with rmcp, tokio, scrap, rusqlite, reqwest deps
- **T002**: `src/` directory structure (lib.rs, bin/gentle-eye.rs, capture/, storage/, analysis/, mcp/, config/)
- **T003**: `tests/` directory structure (contract/, integration/, unit/)
- **T004**: Dev tools config (rustfmt.toml, clippy.toml, .cargo/config.toml)
- **T005**: `.gitignore` with full Rust patterns
- **T006**: `README.md` with project overview and MCP tools table
- **T007**: `.env.example` with configuration variables

### Tool implementation phase
- **T040**: Integrate conversation/recording history with router *(suggested)*
- **T041**: Implement `start_recording` MCP tool in `src/mcp/tools.rs`
- **T042**: Implement `stop_recording` MCP tool
- **T043**: Implement `get_recording_status` MCP tool
- **T044**: Add storage integration (rusqlite recording table)
- **T045**: Add tracing logging (env-based filter)

### Analysis phase
- **T059**: Implement `analyze_video`/`analyze_media` MCP tool in `src/mcp/tools.rs`
- **T060**: Save analysis to database
- **T061**: Add prompt validation *(NFR-003)*

### MCP integration & docs
- **T069**: Create `gentle-eye.toml` configuration template
- **T070**: Document MCP server startup for Claude Desktop in README.md
- **T071**: Document MCP server startup for Cursor in README.md
- **T072**: Document MCP server startup for Claude Code CLI in README.md
- **T073**: Add environment variable validation at startup
- **T074**: Add prerequisite checks at startup
- **T078**: Add MCP error code mapping
- **T079**: Add clear error messages
- **T080**: Add credential security (no logs, no echo)

### Security hardening (T101–T103)
- **T101**: Path validator for `video_path` inputs *(NFR-001, CWE-22)*
- **T103**: Rate limiter for analyze_* tools *(NFR-002, OWASP A04)*

### `[GAP: T011-T039, T046-T058, T062-T068, T075-T077, T081-T100, T102 — task descriptions not captured. Numbering suggests these existed.]`

## 9. Workspace / file system

```
gentle-eye/                                    workspace root
├── Cargo.toml                                workspace manifest
├── Cargo.lock
├── rustfmt.toml
├── clippy.toml
├── .cargo/config.toml
├── .gitignore
├── .env.example
├── README.md
├── INSTALL.md
├── QUICKSTART.md
├── CHANGELOG.md
├── GENTLE_EYE_VISION.md
├── docs/
│   ├── API.md                                MCP tool API reference
│   └── INSTALLATION_SYSTEM.md
├── memory-bank/
│   ├── CLAUDE.md                             project-specific Claude instructions
│   └── projectbrief.md
├── prd/
│   └── (this PRD, or earlier versions)
├── specs/
│   └── 001-mcp-screen-tools/
│       ├── spec.md
│       ├── plan.md
│       ├── tasks.md
│       ├── research.md
│       ├── data-model.md
│       ├── quickstart.md
│       ├── analysis-report.md
│       ├── checklists/
│       └── contracts/
├── .specify/
│   ├── memory/constitution.md
│   └── templates/{spec,plan,tasks,checklist,agent-file}-template.md
├── .claude/
│   ├── commands/speckit.{specify,plan,tasks,implement,analyze,checklist,clarify,constitution,taskstoissues}.md
│   └── settings.local.json
├── src/
│   ├── lib.rs                                public re-exports
│   ├── bin/gentle-eye.rs                     server entry point
│   ├── error.rs
│   ├── mcp/
│   │   ├── mod.rs
│   │   ├── server.rs                         GentleEyeServer (impl ServerHandler)
│   │   ├── tools.rs                          *Input/*Output types for the 11 tools
│   │   └── handlers.rs                       per-tool handlers
│   ├── capture/
│   │   ├── mod.rs
│   │   ├── screen.rs                         ScreenCapturer
│   │   ├── encoder.rs                        EncoderMode/State, PipeEncoder, FFmpeg integration
│   │   ├── frame_rate.rs
│   │   ├── memory.rs                         frame buffer mgmt
│   │   ├── service.rs                        RecordingService
│   │   └── display.rs                        multi-monitor support
│   ├── models/
│   │   ├── mod.rs                            Recording, RecordingStatus, EncoderMode
│   │   ├── analysis.rs                       AnalysisRequest, AnalysisResult, TimeRange
│   │   └── config.rs                         RecordingConfig
│   ├── analysis/                             [CORRECTED — not "vision/"]
│   │   ├── mod.rs                            re-exports
│   │   ├── traits.rs                         VisionProvider / AnalysisProvider trait
│   │   ├── config.rs                         provider config types
│   │   ├── gemini.rs                         Gemini impl (default model: gemini-2.0-flash)
│   │   └── ollama.rs                         Ollama impl
│   ├── contracts/                            [NEW — discovered in panic dump]
│   │   ├── mod.rs                            module root
│   │   └── traits.rs                         core trait contracts
│   ├── storage/
│   │   ├── mod.rs
│   │   ├── database.rs                       SQLite connection / pool
│   │   ├── manager.rs                        StorageManager (high-level API)
│   │   ├── metadata.rs                       recording metadata persistence
│   │   └── schema.rs                         SQLite migrations
│   ├── security/
│   │   ├── rate_limiter.rs                   token bucket, 10 req/min default (T103)
│   │   ├── path_validator.rs                 CWE-22 mitigation (T101)
│   │   └── uuid_validator.rs                 [NEW] UUID format validation
│   ├── config.rs                             top-level gentle-eye.toml loader (5 KB recovered)
│   ├── config/                               (sub-module that complements src/config.rs)
│   │   ├── mod.rs                            (15.8 KB recovered)
│   │   └── loader.rs
│   └── startup.rs                            (3.4 KB recovered — startup checks, secret redaction)
├── benches/
│   ├── capture_performance.rs
│   └── mcp_response_time.rs
├── tests/
│   ├── contract/                             MCP contract tests
│   ├── integration/                          end-to-end tests
│   └── unit/
└── modules/
    └── rust-record/                          sub-workspace
        ├── video-capture/                    reusable Rust capture lib
        ├── region-selector/                  region-selection logic
        └── region-selector-ui/               Slint-based region selector UI
```

## 10. Dependencies (recovered)

### Production
| Crate | Version | Purpose |
|---|---|---|
| `rmcp` | latest | Rust MCP SDK — server impl |
| `tokio` | workspace | async runtime |
| `serde` / `serde_json` | latest | JSON serialization |
| `schemars` | latest | JSON Schema generation for MCP tools |
| `chrono` | workspace | timestamps |
| `uuid` | workspace, features=`["v4"]` | recording IDs |
| `rusqlite` | `0.31`, features=`["bundled"]` | SQLite history |
| `reqwest` | latest | HTTP client (Gemini, Ollama) |
| `clap` or `argh` | latest | CLI args |
| `tracing` / `tracing-subscriber` | workspace | structured logging |
| `image` | latest | PNG output for screenshots |
| `scrap` | latest | cross-platform screen capture |
| `slint-build` | `=1.13.1` | (build-dep for region-selector-ui) |

### Development
| Crate | Version | Purpose |
|---|---|---|
| `mockall` | `0.12` | trait mocking |
| `tempfile` | `3` | tempdirs for tests |
| `serial_test` | `3` | tests that share global state |
| `criterion` | `0.5`, features=`["html_reports"]` | benchmarks |
| `tokio-test` | `0.4` | async test helpers |

## 11. Configuration

`gentle-eye.toml`:
```toml
[recording]
default_fps = 1
max_duration_seconds = 1800
output_directory = "~/Documents/gentle-eye/recordings"

[vision]
default_provider = "gemini"
gemini_model = "gemini-2.0-flash"
ollama_url = "http://localhost:11434"

[security]
analyze_rate_limit_per_minute = 10
allowed_video_paths = ["~/Documents/gentle-eye/recordings"]

[mcp]
server_name = "gentle-eye"
log_level = "info"
```

Env vars: `GEMINI_API_KEY`, `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `GOOGLE_API_KEY` (provider-specific).

## 12. Acceptance criteria

The product is "done" (MVP) when:
1. ✅ All 11 MCP tools listed in §4 are implemented and pass contract tests
2. ✅ Server starts and registers with Claude Desktop, Cursor, and Claude Code CLI per docs in T070-T072
3. ✅ Recordings are captured, saved as MP4, and listed correctly
4. ✅ At least one vision provider (Gemini) returns analysis results end-to-end
5. ✅ T101 path validator rejects `../` and absolute paths outside output dir
6. ✅ T103 rate limiter blocks 11th analyze_* call within 60 s and returns clear error
7. ✅ `cargo bench` runs both bench harnesses and produces HTML reports
8. ✅ Quickstart.md walks a new user from install to first analysis in <10 min

## 12a. Recovered source files (in `/home/gyasis/Documents/code/gentle-eye/recovered_source/`)

8 files of clean Rust source recovered (~95 KB total):

| Path | Size | Notes |
|---|---|---|
| `src/bin/gentle-eye.rs` | 4.6 KB | Full main entry point — tokio::main, signal handling, init_logging, GentleEyeServer::new + serve_stdio |
| `src/startup.rs` | 3.4 KB | redact_secret, parse_ffmpeg_version, StartupCheckResult, full test suite |
| `src/config.rs` | 5.1 KB | Top-level config loader |
| `src/config/mod.rs` | 15.8 KB | Config module root |
| `src/capture/display.rs` | 19.7 KB | Multi-monitor display management |
| `src/capture/screen.rs.partial` | 2.0 KB | ScreenCapturer impl fragment |
| `src/mcp/tools.rs` | 13.0 KB | Input/Output type definitions for the 11 MCP tools |
| `src/mcp/server.rs.partial` | 1.0 KB | CaptureStreamFrameOutput fragment (full file is ≥1577 lines) |
| `src/models/config.rs` | 20.8 KB | Recording/RecordingConfig data model |
| `_panic_dump.md` | 25.2 KB | Binary panic-location dump revealing module structure |

**`src/mcp/server.rs` is at least 1577 lines** (panic locations found at lines 158, 167, 171, 180, 190, 194, 199, 207, 214, 219, 227, 278, 283, 377, 489, 501, 530, 536, 562, 594, 599, 625, 670, 678, 684, 703, 773, 796, 799, 812, 833, 836, 1005, 1063, 1085, 1090, 1116, 1139, 1190, 1246, 1266, 1314, 1316, 1319, 1340, 1494, 1577). Most of this file was NOT recovered; only the 1 KB `CaptureStreamFrameOutput` fragment.

## 13. Recovery / `[GAP]` items requiring fresh design

The following were referenced but never captured in detail:
- `[GAP-1]` Exact `*Input`/`*Output` schemas for each of the 11 tools (some fields are recoverable via `REBUILD_KEY_CODE.md`, but full JSON schemas need re-derivation)
- `[GAP-2]` SQLite schema (table definitions for recordings, analyses, displays). Inferred from `Recording` struct but DDL needs to be written.
- `[GAP-3]` Detailed error taxonomy and MCP error code mappings (T078)
- `[GAP-4]` Exact rate-limit algorithm choice (token bucket vs sliding window) — text says "10 req/min default" but algorithm not specified
- `[GAP-5]` `region-selector-ui` Slint UI design — only file paths and Slint dependency are recovered, not actual `.slint` definitions
- `[GAP-6]` `analysis-report.md` content (was generated by `/speckit.analyze` against the spec)
- `[GAP-7]` `.specify/memory/constitution.md` content (project-level constitution doc)
- `[GAP-8]` Tasks T011–T039, T046–T058, T062–T068, T075–T077, T081–T100, T102 (descriptions not captured)
- `[GAP-9]` README.md actual content (only that it has a "project overview and MCP tools table" is known)
- `[GAP-10]` GENTLE_EYE_VISION.md actual content (only that it exists is known)
- `[GAP-11]` Most of `src/mcp/server.rs` (≥1577 lines, only 1 KB recovered) — this is the largest critical source file
- `[GAP-12]` `src/main.rs`, `src/lib.rs`, `src/error.rs` (root crate files)
- `[GAP-13]` `src/mcp/{handlers,errors,mod}.rs`
- `[GAP-14]` `src/capture/{encoder,frame_rate,memory,service,mod}.rs` + full `src/capture/screen.rs`
- `[GAP-15]` `src/models/{mod,analysis,edit_session}.rs`
- `[GAP-16]` All of `src/analysis/{config,gemini,ollama,mod,traits}.rs` (not "vision/" — corrected from panic dump)
- `[GAP-17]` `src/storage/{database,manager,metadata,schema,mod}.rs`
- `[GAP-18]` `src/security/{rate_limiter,path_validator,uuid_validator}.rs`
- `[GAP-19]` `src/contracts/{mod,traits}.rs`
- `[GAP-20]` `benches/{capture_performance,mcp_response_time}.rs`
- `[GAP-21]` All `modules/rust-record/{video-capture,region-selector,region-selector-ui}/src/*.rs`

## 14. Recommended rebuild sequence

1. **Bootstrap** the workspace (T001–T007) per §9 layout
2. **Define data model first** (`src/models/`) — recoverable verbatim from §7
3. **Define MCP tool I/O types** (`src/mcp/tools.rs`) — 11 tool name+description pairs are §4; flesh out schemas per [GAP-1]
4. **Stub `GentleEyeServer`** (`src/mcp/server.rs`) implementing rmcp's `ServerHandler` trait
5. **Implement Capture module** with `scrap` (T041–T043)
6. **Implement Storage module** with `rusqlite` (T044, address [GAP-2])
7. **Implement Vision module** — Gemini first (T059, T060), Ollama second
8. **Wire security middleware** (T061, T101, T103)
9. **Document MCP integration for the three clients** (T070–T072)
10. **Bench + acceptance test pass** per §12

---

**End of reconstructed PRD.** Companion files in this directory provide the raw recovery data this PRD synthesizes.
