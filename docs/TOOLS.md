# gentle-eye — Agent Tool & CLI Reference

What an agent can do with gentle-eye. Two surfaces over one library:
**MCP tools** (in-agent) and **CLI subcommands** (shell out, JSON on stdout).

> Regenerated 2026-05-30 from the live `tool_catalog()` (`src/mcp/server.rs`) and
> the CLI `HELP` (`src/bin/gentle-eye.rs`); updated 2026-06-01 with `screenshot`
> + redpen; updated 2026-08-30 with **dayflow** (17 MCP tools now).
>
> `tests/docs_agree_with_code.rs` fails if this file and the code disagree — a
> reference that has drifted is worse than none, because it is trusted.

**For humans:** `docs/GENTLE_EYE_GUIDE.md` is the task-shaped guide. This file is
the lookup table.

---

## Capability map — "what can be done"

| Want to… | MCP tool | CLI |
|---|---|---|
| **Record a whole day, by itself** | `start_dayflow` / `stop_dayflow` / `dayflow_status` | `dayflow serve`, `dayflow start\|stop\|status` |
| **Ask what happened earlier** | `ask_day` | `dayflow ask "…"` |
| **Read the day's timeline / standup** | `get_timeline` (`standup:true`) | `dayflow timeline`, `dayflow standup` |
| Record the screen | `start_recording` / `stop_recording` / `get_recording_status` / `cancel_recording` | `record` |
| List recordings | `list_recordings` | `list` |
| Analyze a video/image with a VLM | `analyze_video` | `analyze` |
| OCR on-screen text | `read_screen_text` | `read-text` |
| Grab a frame from a live stream (ATEM/RTSP/HTTP/SRT) | `capture_stream_frame` | `capture-stream` |
| Focus capture on a sub-region (crop) | `define_target` / `focus_target` | `target add` / `target use` / `target list` |
| Preview a capture (image/video) or live feed | — | `preview [FILE]` / `preview --gallery` / `preview --live` (see [PREVIEW.md](PREVIEW.md)) |
| Snap a rough region to real edges / find a red marker | `measure_target` | — |
| One-shot screenshot → PNG (optional crop) | — | `screenshot --out FILE.png` |
| **Human marks up a screen to direct the agent** (pen/arrow/box) | — | `redpen` (GUI, user-launched) → `redpen-list` / `redpen-analyze` (see [REDPEN.md](REDPEN.md)) |
| Inspect the configured vision provider | `get_vision_provider_info` | `provider-info` |
| List / label displays | — | `displays` / `label` |

---

## MCP tools (12)

The server (`gentle-eye serve`) exposes these over stdio; an agent sees them via
`tools/list`.

| Tool | Description |
|---|---|
| `start_recording` | Start a new screen recording session. |
| `stop_recording` | Stop an active recording and finalize the video file. |
| `get_recording_status` | Get the current status of a recording. |
| `analyze_video` | Analyze a recorded video with the configured vision AI provider. |
| `list_recordings` | List recent recordings with metadata. |
| `cancel_recording` | Cancel a recording without saving the video. |
| `get_vision_provider_info` | Get information about the configured vision AI provider. |
| `read_screen_text` | Extract on-screen text (OCR) from an image or video. |
| `capture_stream_frame` | Grab a single frame from a live stream URL (RTSP/HTTP/SRT, e.g. an ATEM output) as a PNG. |
| `define_target` | Define a region-of-interest ("target") to crop a display or stream to, using **normalized 0–1 coordinates**. Returns a confirmation image so you can self-correct the region. |
| `focus_target` | Switch the active target by name (one active at a time). All subsequent capture/analysis crops to it. |
| `measure_target` | **Zoom-then-Snap**: snap a rough normalized region to real edges, detect a tiled-pane grid, optionally find a red marker. Returns a `snapped_rect` + a Redline overlay to supervise the CV. |

### The "target" agent loop (region-of-interest)

1. `define_target { name, source, region }` with `region` in **normalized 0–1**
   (`{x,y,w,h}` as fractions). Inspect the returned **confirmation image**.
2. If off, re-call (or `measure_target` to snap to edges) and confirm.
3. `focus_target { name }` to activate — capture/analysis then crops to it.

`source` is `{"kind":"display","index":0}` or `{"kind":"stream","url":"rtsp://…"}`.
See [`TARGET.md`](TARGET.md) for the full design.

### The "redpen" visual-direction loop (human → agent)

When the user wants to *show* you something — point where a thing should move,
circle what's broken, sketch a layout — they run the **`redpen`** GUI (a native
markup tool: freehand pen / arrow / box in a color palette). They draw, press
Enter, and an artifact lands in `~/.gentle-eye/redpen/`. **You never launch the
GUI; you discover what they drew:**

1. `gentle-eye redpen-list [--limit N]` — newest-first list of captures. Each
   entry has the PNG path + its annotations (type, color, normalized coords).
2. `gentle-eye redpen-analyze [--prompt "…"]` — picks the latest capture (or
   `--image PATH`), injects the marks as text (e.g. *"green ARROW from (x,y) to
   (x,y) — points toward…"*), and sends the marked-up PNG to the VLM (default
   gemini). The image has the strokes burned in **and** you get the geometry.

This is the human-side mirror of the `target` loop: targets are *you* cropping a
region; redpen is the *user* drawing direction onto a screen. See
[`REDPEN.md`](REDPEN.md).

---

## CLI subcommands

```
gentle-eye [serve]                        Run as an MCP server over stdio (default)
gentle-eye analyze --image PATH --prompt TEXT [--provider gemini|ollama]
gentle-eye analyze --video PATH --prompt TEXT [--start S --end E] [--provider …]
gentle-eye record  [--duration SECS] [--fps N] [--out FILE.mp4] [--display IDX|LABEL]
gentle-eye capture-stream --url URL [--out DIR] [--region x,y,w,h]   (--region crops, normalized 0-1)
gentle-eye list    [--status all|recording|completed|cancelled|failed] [--limit N]
gentle-eye read-text --image PATH | --video PATH      OCR → JSON
gentle-eye displays                       List available displays (the catalogue)
gentle-eye label   --display IDX --name "left"        Label a display (persists)
gentle-eye target add NAME (--display IDX | --stream URL) --region x,y,w,h
gentle-eye target use NAME
gentle-eye target list
gentle-eye preview [FILE] [--loop once|forever] [--seconds N]   Preview a capture (default: most recent)
gentle-eye preview --gallery [--port N]   Browser media gallery (Range video) until idle
gentle-eye preview --live                 Live preview of the active target (default off)
gentle-eye screenshot --out FILE.png [--display IDX] [--region x,y,w,h | --target NAME]   One-shot grab → PNG
gentle-eye redpen-list [--limit N]        List redpen annotation captures (newest first) — the discovery surface
gentle-eye redpen-analyze [--image PATH] [--prompt TEXT] [--provider gemini|ollama]   Send a capture + its marks to a VLM
gentle-eye provider-info [--provider gemini|ollama]
gentle-eye help
```

(The `redpen` GUI itself is a separate binary built with `--features ui`; the
agent never launches it — the user does. See the loop below.)

All CLI subcommands print JSON to stdout (logs go to stderr).

### Examples

```bash
# OCR a screenshot
gentle-eye read-text --image /tmp/shot.png

# Grab + crop one frame of a stream's center-right region, then describe it
gentle-eye capture-stream --url rtmp://localhost:7001/live/atem \
  --out /tmp/frames --region 0.5,0,0.5,1
gentle-eye analyze --image /tmp/frames/stream_*.png \
  --prompt "What's in this region?" --provider gemini

# Define + activate a persistent crop target (one active at a time)
gentle-eye target add editor --display 0 --region 0.25,0,0.25,1
gentle-eye target use editor
gentle-eye target list
```

---

## Dayflow — the all-day recorder

Dayflow records a **source** and answers questions about it later. It is the one
subsystem that runs *by itself*: a daemon owns the session, so start it once and
query it from anywhere.

```bash
# 1. Start the daemon. It owns the session and serves the other surfaces.
gentle-eye dayflow serve [--port 7431] [SOURCE]

# 2. Every other invocation ATTACHES to it (no second engine, no MCP install).
gentle-eye dayflow status
gentle-eye dayflow timeline --from 2026-08-30T09:00:00Z --to 2026-08-30T17:00:00Z
gentle-eye dayflow standup
gentle-eye dayflow ask "what was I doing at 2pm?"
gentle-eye dayflow stop
```

### SOURCE — exactly one kind

| flag | captures | regions |
|---|---|---|
| `--displays 0,1` | whole screens (default: all) | from the region cascade |
| `--window <label>` | one window, by title or class | cascade, clipped to the window |
| `--target <name>` | a saved normalised region (`gentle-eye target add`) | cascade, clipped to the target |
| `--input <url>` | a stream / capture device / video file | **none** — reported honestly |

Two kinds at once is **refused**, not resolved. An input reports no regions
because there is no window manager to ask about a video feed; the sample is
counted into `samples_read_whole`, which `status` shows.

### What an agent should know

- `display_id` in a stored row means **which source**, not which monitor. For a
  window/target/input session ordinal `0` is that source and is not display 0.
- `status.sources[]` gives kind, name and live availability — `displays` alone
  cannot tell a window session from a display session.
- `ask` needs `GE_DAYFLOW_ENDPOINT` (the governed lane). Unconfigured, it says so
  and does **not** echo the prompt.
- An **empty range never reaches a model** — with no evidence it would invent a
  day. Every answer carries `grounding`, so confident prose with no evidence
  stays detectable.
- `timeline` returns `entries` **and** `gaps`. A gap is a recorded pause with a
  cause; absence of entries is not the same fact.

## How the pieces compose

None of these tools is meant to be used alone. The chains that matter:

| Chain | What it does |
|---|---|
| `target add` → `dayflow serve --target` | record only one region of one screen, all day |
| `regions` → `dayflow` sidecars | the cascade's boxes become the crops perception reads, instead of whole frames |
| `redpen` (user draws) → `redpen-analyze` | the human points at something; the agent reads the markup |
| `dayflow ask` → the governed lane | a question answered from the day's own grounded records |
| `screenshot --target` → `analyze` | grab one region, ask a VLM about it |

`redpen` is the **inbound** channel (human → agent: markup on a screenshot);
`target`/`regions` are the **outbound** one (agent → screen: pick a region to
watch). They are mirrors, not alternatives.

## What gentle-eye is built ON

| Library | Used for | Constraint it imposes |
|---|---|---|
| `scrap` 0.5 | display capture | handles are **thread-affine** — a capture source cannot be `Send` |
| `x11rb` | window geometry, idle detection | **X11 only**; `--window` misreports on macOS/Wayland (a known gap) |
| `atspi` | accessibility tree regions | needs the a11y bus; absent under some sandboxes |
| `rmcp` 0.1 | the MCP server | — |
| `reqwest` | vision providers, daemon client | split connect/read timeouts matter (cold loads reach ~95 s) |
| `rusqlite` | the timeline store | one store, shared by every surface |
| `image` | PNG encode/decode | — |

External processes: **ffmpeg** (input sources, video), **ffplay** (preview),
**ffprobe**, **tesseract** (OCR). Absent, each degrades to a stated failure.

Services beside it: the **Atelier governor** (`:8799/llm/ollama`) is the
perception and reasoning lane; **ollama** and **Gemini** are the providers behind
it. A cold model load is slow but normal — budget for it rather than treating it
as a fault.

## How other tools consume gentle-eye

gentle-eye is a library, a CLI, and an MCP server, in that order of generality.

- **As an MCP server** — 17 tools for a coding agent. Register it once per host.
- **As a CLI, from any harness** — every subcommand prints JSON on stdout,
  diagnostics on stderr, and exits 0 on a *degraded but recoverable* state so a
  script does not treat it as a crash. This is the zero-install path: no MCP
  registration, works from any agent that can run a shell.
- **As an HTTP surface** — `dayflow serve` exposes `/dayflow/{status,start,stop,
  timeline,standup,ask}`. A client on another machine can drive a capture daemon
  it is not running on: the daemon must be native to the box being captured, a
  client need not be.
- **As a Rust library** — `gentle_eye::{capture,target,regions,dayflow,analysis}`.

## Notes

- **fps** is duration-aware — see [`FPS_AND_DAYFLOW.md`](FPS_AND_DAYFLOW.md).
- **Vision providers:** Gemini (native video, default) or Ollama (privacy
  fallback). Configure via `GENTLE_EYE_PROVIDER` + keys — see [`QUICKSTART.md`](QUICKSTART.md).
- An **active target** crops every capture/analysis path (screen record loop,
  stream frame grab) before encode/analyze/OCR.
