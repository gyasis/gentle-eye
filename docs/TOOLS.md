# gentle-eye — Agent Tool & CLI Reference

What an agent can do with gentle-eye. Two surfaces over one library:
**MCP tools** (in-agent) and **CLI subcommands** (shell out, JSON on stdout).

> Regenerated 2026-05-30 from the live `tool_catalog()` (`src/mcp/server.rs`) and
> the CLI `HELP` (`src/bin/gentle-eye.rs`); updated 2026-06-01 with the
> `screenshot` + `redpen-list` / `redpen-analyze` CLI commands and the redpen
> visual-direction loop. The 12 MCP tools are unchanged (redpen is CLI-only).

---

## Capability map — "what can be done"

| Want to… | MCP tool | CLI |
|---|---|---|
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

## Notes

- **fps** is duration-aware — see [`FPS_AND_DAYFLOW.md`](FPS_AND_DAYFLOW.md).
- **Vision providers:** Gemini (native video, default) or Ollama (privacy
  fallback). Configure via `GENTLE_EYE_PROVIDER` + keys — see [`QUICKSTART.md`](QUICKSTART.md).
- An **active target** crops every capture/analysis path (screen record loop,
  stream frame grab) before encode/analyze/OCR.
