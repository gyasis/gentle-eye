# gentle-eye — Rust Dependencies (mined from sessions)

Files scanned: 3078 (skipped 93 files > 2MB)
Dependency blocks found: 9
Unique crate names: 23

## Crates with version specs (from `[dependencies]` blocks)

- **`chrono`**
  - `{ workspace = true }`
- **`criterion`**
  - `{ version = "0.5", features = ["html_reports"] }`
- **`group_imports`**
  - `"StdExternalCrate"`
- **`harness`**
  - `false`
- **`imports_granularity`**
  - `"Crate"`
- **`max_width`**
  - `100`
- **`mockall`**
  - `"0.12"`
- **`newline_style`**
  - `"Unix"`
- **`path`**
  - `"src/bin/gentle-eye.rs"`
- **`reorder_imports`**
  - `true`
- **`reorder_modules`**
  - `true`
- **`rusqlite`**
  - `{ version = "0.31", features = ["bundled"] }`
- **`serial_test`**
  - `"3"`
- **`slint-build`**
  - `"=1.13.1"`
- **`tab_spaces`**
  - `4`
- **`tempfile`**
  - `"3"`
  - `"3.0"`
- **`tokio-test`**
  - `"0.4"`
- **`use_small_heuristics`**
  - `"Default"`
- **`uuid`**
  - `{ workspace = true, features = ["v4"] }`

## External `use ::` crate references (top 80)

- `gentle_eye` (280x)
- `quote` (73x)
- `chrono` (68x)
- `proc_macro2` (68x)
- `hashbrown` (57x)
- `tokio` (54x)
- `itertools` (49x)
- `iri_string` (38x)
- `mio` (28x)
- `serde_json` (24x)
- `traits` (24x)
- `winnow` (24x)
- `rustls_pki_types` (20x)
- `futures` (19x)
- `zerovec` (19x)
- `tower` (18x)
- `num_traits` (16x)
- `serde` (15x)
- `log` (14x)
- `ratatui` (14x)
- `bytes` (14x)
- `heck` (14x)
- `regex` (13x)
- `base64` (13x)
- `tempfile` (12x)
- `crossbeam_deque` (12x)
- `kurbo` (12x)
- `async_lock` (12x)
- `sysinfo` (11x)
- `url` (11x)
- `regex_automata` (11x)
- `bitstream_io` (10x)
- `schemars` (10x)
- `lexical_parse_float` (10x)
- `smallvec` (10x)
- `memchr` (10x)
- `tokio_util` (9x)
- `clap` (9x)
- `crossterm` (9x)
- `image` (8x)
- `syn` (8x)
- `number_prefix` (8x)
- `crossbeam_epoch` (8x)
- `uuid` (8x)
- `errors` (8x)
- `fake` (8x)
- `video_capture` (8x)
- `yoke` (8x)
- `once_cell` (7x)
- `hex` (7x)
- `zerotrie` (7x)
- `data_anonymizer` (7x)
- `percent_encoding` (6x)
- `difference` (5x)
- `tracing_core` (5x)
- `argh` (5x)
- `allocator_api2` (4x)
- `proc_macro` (4x)
- `instead` (4x)
- `encoder` (4x)
- `frame_rate` (4x)
- `memory` (4x)
- `memmap2` (4x)
- `icu` (4x)
- `anyhow` (4x)
- `generic_array` (4x)
- `glob` (4x)
- `deunicode` (4x)
- `zip` (3x)
- `bitflags` (3x)
- `metadata` (3x)
- `screen` (3x)
- `crossbeam_utils` (3x)
- `duckdb` (3x)
- `reqwest` (3x)
- `rmcp` (3x)
- `async_trait` (3x)
- `portable_atomic` (3x)
- `cassowary` (3x)
- `criterion` (3x)

## Sample dependency blocks

### `[dev-dependencies]` from `_seg5_s16451_c07e359c8c0f.md`

```toml
tokio-test = "0.4"
tempfile = "3.0"
serde_json.workspace = true
uuid = { workspace = true, features = ["v4"] }
chrono = { workspace = true }
tracing-subscriber.workspace = true
_**Agent (claude-haiku-4-5-20251001) (sidechain)**_
Let me get the remaining Cargo.toml files:
_**Agent (claude-haiku-4-5-20251001) (sidechain)**_
Tool use: **Bash** Read region-selector crate Cargo.toml
`cat /home/gyasis/Documents/code/gentle-eye/modules/rust-record/region-selector/Cargo.toml`
_**Agent (claude-haiku-4-5-20251001) (sidechain)**_
Tool use: **Bash** Read region-selector-ui crate Cargo.toml
`cat /home/gyasis/Documents/code/gentle-eye/modules/rust-record/region-selector-ui/Cargo.toml`
name = "region-selector"
version.workspace = true
edition.workspace = true
authors.workspace = true
license.workspace = true
```

### `[dev-dependencies]` from `_seg3_s257812_dec63cf018aa.md`

```toml
mockall = "0.12"
tempfile = "3"
serial_test = "3"
criterion = { version = "0.5", features = ["html_reports"] }
name = "gentle-eye"
path = "src/bin/gentle-eye.rs"
name = "mcp_response_time"
harness = false
name = "capture_performance"
harness = false
_**Agent (claude-opus-4-5-20251101) (sidechain)**_
I see that the video_capture module is not directly referenced in the main Cargo.toml. Let me check the capture module in the main crate to understand how it accesses display functionality.
_**Agent (claude-opus-4-5-20251101) (sidechain)**_
Tool use: **Glob**
/home/gyasis/Documents/code/gentle-eye/src/capture/screen.rs
/home/gyasis/Documents/code/gentle-eye/src/capture/encoder.rs
/home/gyasis/Documents/code/gentle-eye/src/capture/frame_rate.rs
/home/gyasis/Documents/code/gentle-eye/src/capture/mod.rs
/home/gyasis/Documents/code/gentle-eye/src/capture/service.rs
/home/gyasis/Documents/code/gentle-eye/src/capture/memory.rs
/home/gyasis/Documents/code/gentle-eye/src/capture/display.rs
_**Agent
```

### `[dev-dependencies]` from `_seg4_s231387_b788e109c566.md`

```toml
mockall = "0.12"
tempfile = "3"
name = "gentle-eye"
path = "src/bin/gentle-eye.rs"
```

### `[build-dependencies]` from `_seg5_s16458_f094086a9c9a.md`

```toml
slint-build = "=1.13.1"
_**Agent (claude-haiku-4-5-20251001) (sidechain)**_
Perfect! Now let me check the main lib.rs to understand what's exported and see if there are any more details on features:
_**Agent (claude-haiku-4-5-20251001) (sidechain)**_
Tool use: **Read** `./modules/rust-record/video-capture/src/lib.rs`
_**Agent (claude-haiku-4-5-20251001) (sidechain)**_
Tool use: **Bash** List public exports from video-capture crate
`grep -r "^pub " /home/gyasis/Documents/code/gentle-eye/modules/rust-record/video-capture/src/ | head -40`
pub mod capture;
pub mod config;
pub mod display_manager;
pub mod encoder;
pub mod error;
pub mod frame_rate;
pub mod memory;
pub mod metadata;
pub mod prerequisites;
pub mod service;
pub mod storage;
// Re-export main types for convenience
pub use capture::{CaptureEvent, CaptureResult, RawFrame, ScreenCapturer};
pub use config::Config;
pub use display_manager::{DisplayInfo, DisplayManager};
pub use encoder::{EncoderMode, EncoderState, Frame, PipeEncoder
```

### `[dev-dependencies]` from `_seg7_s230495_dde26cde54cf.md`

```toml
mockall = "0.12"
tempfile = "3"
serial_test = "3"
name = "gentle-eye"
path = "src/bin/gentle-eye.rs"
//! MCP Server implementation for Gentle-Eye
//! This module provides the main `GentleEyeServer` struct that implements
//! the MCP `ServerHandler` trait. The server exposes 7 tools for screen
//! recording and AI video analysis.
//! # Tools
//! - `start_recording` - Start a new screen recording session
//! - `stop_recording` - Stop an active recording and save the video
//! - `get_recording_status` - Get the current status of a recording
//! - `analyze_video` - Analyze a video using vision AI
//! - `list_recordings` - List recent recordings with metadata
//! - `cancel_recording` - Cancel a recording without saving
//! - `get_vision_provider_info` - Get information about the vision AI provider
use chrono::Utc;
use rmcp::{
    ServerHandler,
    model::ServerInfo,
```

