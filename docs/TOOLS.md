# gentle-eye — Agent Tool & CLI Reference

What an agent can do with gentle-eye. Two surfaces over one library:
**MCP tools** (in-agent) and **CLI subcommands** (shell out, JSON on stdout).

> Regenerated 2026-05-30 from the live `tool_catalog()` (`src/mcp/server.rs`) and
> the CLI `HELP` (`src/bin/gentle-eye.rs`); updated 2026-06-01 with `screenshot`
> + redpen; updated 2026-08-30 with **dayflow** (17 MCP tools now); updated
> 2026-09-07 with the feature-015 **transcription primitives** (`frames`,
> `quality`, `merge-text`), `annotate`, `regions`, `segment`, and the provider
> story (`VISION_METHODS.md`).
>
> `tests/docs_agree_with_code.rs` fails if this file and the code disagree — a
> reference that has drifted is worse than none, because it is trusted. It
> checks every MCP tool **and every dispatched CLI verb** against this file (the
> CLI half was added 2026-09-07 — six verbs had shipped in `--help` with no entry
> here while the suite stayed green).

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
| **Structure the screen into boxes, in reading order** (no model) | — | `regions [--depth window\|pane\|element\|text] [--match "…"]` (see [REGION_ENGINE.md](REGION_ENGINE.md)) |
| Split a busy screen into terminal/editor **panels** | — | `segment --display IDX [--read]` |
| **Draw boxes + labels on an image** (agent → human) | — | `annotate --image IN --out OUT --box x,y,w,h [--label …]` |
| **Frames of a recording**, ffmpeg timestamp + sharpness per row | — | `frames --video PATH --out DIR [--fps N] [--dedup …]` (primitive 1) |
| **Score a reading** — real content, or a reader that broke down? | — | `quality [FILE \| -]` (primitive 2) |
| **Merge two overlapping readings** into one document | — | `merge-text --similarity T BLOCK INCOMING` (primitive 3) |
| One-shot screenshot → PNG (optional crop) | — | `screenshot --out FILE.png` |
| **Human marks up a screen to direct the agent** (pen/arrow/box) | — | `redpen` (GUI, user-launched) → `redpen-list` / `redpen-analyze` (see [REDPEN.md](REDPEN.md)) |
| Inspect the configured vision provider | `get_vision_provider_info` | `provider-info` |
| List / label displays | — | `displays` / `label` |

---

## MCP tools (17)

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
| `start_dayflow` | Start continuous activity tracking. Dayflow SAMPLES the screen at an interval rather than recording it, so an all-day session costs a few frames per minute instead of a video stream. |
| `stop_dayflow` | Stop the running dayflow session and close its open windows. |
| `dayflow_status` | Whether dayflow is running, and whether it is actually PRODUCING. A degraded session returns successfully with the degradation in the payload — it is running, just not producing. |
| `get_timeline` | Activity timeline entries overlapping a time range. Defaults to today so far. `standup:true` for the categorised digest. |
| `ask_day` | Answer a question about a time range, grounded STRICTLY on recorded entries. Says so plainly when the range holds no record rather than inventing one. *(The tool's own description in `src/mcp/server.rs` still says "no model is wired yet"; that lags the code — `src/dayflow/answerer.rs` answers through `GE_DAYFLOW_ENDPOINT`, and says so when it is unset.)* |

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

### The "annotate" loop (agent → human)

`annotate` is the **agent's** half of the annotation loop; `redpen` is the
**human's** half. Both exist deliberately and neither replaces the other:
redpen is a GUI the user drives, `annotate` is headless and you drive it — to
answer a markup in kind, to show a human *which* box you mean, or to build a
confirmation image before acting.

```bash
gentle-eye annotate --image shot.png --out marked.png \
  --box 40,120,300,80 --label "this button" \
  --box 600,40,200,200 \
  --color 0,120,255
```

Boxes are **pixel** integers `x,y,w,h` (not normalised — the image is the
frame of reference). Each `--label` attaches to the `--box` immediately before
it; a box with no following `--label` is drawn bare. `--color r,g,b` applies to
every box (default red). Output:

```json
{"annotated":"marked.png","source":"shot.png","width":1280,"height":720,"count":2,
 "font":"/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf","labels_drawn":true,"note":null,
 "boxes":[{"x":40,"y":120,"w":300,"h":80,"label":"this button"},{"x":600,"y":40,"w":200,"h":200,"label":null}]}
```

**Labels need a TrueType font; boxes never do.** The binary tries
`GENTLE_EYE_FONT` first, then the common Linux (DejaVu) and macOS (Arial,
Helvetica) paths. It **reports** the outcome instead of dropping labels
silently: `font` is the path it used (or `null`), `labels_drawn` says whether
labels were rendered, and `note` names the fix (`Set GENTLE_EYE_FONT=…`) when
labels were asked for and no font was found. Read those three keys before
trusting a labelled image.

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
gentle-eye segment --display IDX [--read] [--provider gemini|ollama]   Detect terminal/editor PANELS (column-activity dividers); --read reads each via the vision provider
gentle-eye annotate --image IN.png --out OUT.png --box x,y,w,h [--label TEXT] [--box … --label …] [--color r,g,b]   Draw boxes + labels on an image (headless; agent-driven)
gentle-eye redpen-list [--limit N]        List redpen annotation captures (newest first) — the discovery surface
gentle-eye redpen-analyze [--image PATH] [--prompt TEXT] [--provider gemini|ollama]   Send a capture + its marks to a VLM
gentle-eye provider-info [--provider gemini|ollama]
gentle-eye regions [--depth window|pane|element|text] [--display IDX] [--match "…"] [--contrast]   Structure the screen into boxes, in reading order (geometry, not a model)

gentle-eye frames --video PATH --out DIR [--fps N] [--dedup none|gentle|medium|aggressive]   Frames of a recording: ffmpeg's timestamp + a sharpness score per row (primitive 1; default 1 fps, medium)
gentle-eye quality [FILE | -]             Information content of text as three ratios, no verdict (primitive 2; stdin when no FILE)
gentle-eye merge-text --similarity T BLOCK_FILE INCOMING_FILE   Fuzzy-merge a new reading into a document under YOUR line similarity T in (0,1] (primitive 3)

gentle-eye dayflow …                      The all-day recorder — its own section below
gentle-eye help
```

`regions` accepts `--match "<text>"` (resolve to the single best region, no
model) and `--contrast` (the salient high-contrast region, a pixel fallback for
windowless content) — both are in the code, not yet in `--help`. Default depth
is `element`.

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

## Transcription primitives — `frames`, `quality`, `merge-text`

Three verbs from feature 015 (`specs/015-screen-transcription/`) that turn a
recording into a transcript **without deciding anything**. Each answers one
question; the caller — an agent, a playbook, a person — chains them and owns
every threshold. The contract is
`specs/015-screen-transcription/contracts/primitives.md`; the runnable chain is
[`playbooks/transcribe-a-recording.md`](playbooks/transcribe-a-recording.md).

| Primitive | Question it answers | Emits |
|---|---|---|
| `frames --video PATH --out DIR [--fps N] [--dedup none\|gentle\|medium\|aggressive]` | which frames are there, and how legible is each? | `{video,fps,dedup,out,count,frames:[{index,timestamp_s,path,sharpness}]}` |
| `quality [FILE \| -]` | is this text real content, or a reader that broke down? | exactly three keys: `compression_ratio`, `unique_line_ratio`, `unique_token_ratio` |
| `merge-text --similarity T BLOCK_FILE INCOMING_FILE` | these two readings overlap — what is the one document? | `{similarity,coverage,merged}` |

### `frames` — what an agent must know

- **`timestamp_s` is ffmpeg's own presentation time, NOT `index/fps`.** Under
  any dedup the file numbering counts *survivors*, so `index/fps` is plausible
  and wrong. Measured 2026-09-07 on a 60 s screen recording at
  `--fps 1 --dedup medium`: 49 rows, 22 of them differ from `index/fps`; on a
  six-slide clip at the same settings, 5 of 6 rows differ (row 1 is `t=10.0`,
  `index/fps` says `1.0`). Always read `timestamp_s`.
- **`sharpness` is comparable within one recording only.** It is a focus
  measure, not a calibrated scale — rank frames of the same source, never
  compare across recordings.
- `fps` and `dedup` are echoed back so a caller who took a default can see
  which default it took (`1` and `medium`).
- Nothing describes what was dropped. Re-run with a gentler `--dedup` to see
  more; the rows are the kept frames and only those.
- ffmpeg absent → a stated error naming what is missing, never an empty list.

**Sampling is a strategy — pick the rate and the dedup for the material.**
Measured on one 60 s screen recording unless stated:

| Job | Invocation | Result |
|---|---|---|
| dayflow-style, one image every 5 min | `--fps 0.0033 --dedup none` | **`count: 0` on a 60 s clip.** The period (~303 s) is longer than the recording and ffmpeg's `fps` filter rounds the only slot away; the same rate on a 600 s clip gave 2 frames. A fractional rate works, but its period must fit inside the recording — `count: 0` is a real answer, read it before reading `frames` |
| one image every 10 s | `--fps 0.1 --dedup none` | 6 frames at exactly 10 s spacing |
| one image every 30 s | `--fps 0.0333 --dedup none` | 2 frames, `t = 0.0` and `30.03` |
| transitions in motion | `--fps 2 --dedup gentle` | 120 of 120 kept |
| mixed material (the default) | `--fps 2 --dedup medium` | 91 of 120 |
| wholesale screen changes only | `--fps 2 --dedup aggressive` | **7** of 120 |

**`--dedup aggressive` is the trap.** A change to only *part* of the screen — a
title line, one figure, one cell — reads as a near-duplicate and is **dropped**.
Reproduced 2026-09-07 on a synthetic six-slide clip (10 s per slide, `--fps 2`):
when only a small title patch changed per slide, `aggressive` kept **1 of 120**
— five real slide changes gone — while `gentle` and `medium` kept 6; when the
whole screen changed per slide, all three kept 6. Do not use `aggressive` when
you want one frame per slide.

### `quality` — what an agent must know

- Ratios in `0.0..=1.0`; **lower means more repetitive.** Real content sits
  around `0.5–0.7` on `compression_ratio`; a looped reading collapses towards
  `0.01`. `unique_line_ratio` is the strongest separator measured (300×).
- **Never a length, never a verdict.** A dense page of code is legitimately
  long. The reject threshold is yours — the populations sit far enough apart
  that any sane one works.
- Empty text scores as empty (`0.0`), not as a failure: whether an empty
  screen was expected is the caller's call.
- One text per call: a FILE, or `-` / nothing for stdin.

### `merge-text` — what an agent must know

- **`--similarity` is REQUIRED** — the binary exits 1 without it, on purpose.
  At `1.0` two readings of one imperfect line never match and a paragraph is
  emitted once per frame that showed it; the tolerance that fixes that is a
  property of the *reader's* error rate, which only the caller knows. `0.9`
  tolerates roughly one edit per ten characters.
- Order matters: `BLOCK` is the document so far, `INCOMING` the new reading.
  The merge is asymmetric — the block's reading of a shared line is kept.
- `coverage` is how much of `INCOMING` was already in `BLOCK` (`0.67` = two of
  three lines). Containment never grows the document; no overlap loses nothing.
- Either file may be `-` for stdin, not both.

### The chain, by hand

```bash
gentle-eye frames --video lesson.mkv --out ./frames --fps 2 --dedup medium > frames.json
# pick the sharpest frame per distinct screen — YOUR rule, e.g. top sharpness
gentle-eye read-text --image ./frames/f_00007.png > r7.json
jq -r .text r7.json | gentle-eye quality            # gate it: YOUR threshold
gentle-eye merge-text --similarity 0.85 doc.txt r7.txt | jq -r .merged > doc.next.txt
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
- **Vision providers:** Gemini (cloud, default model `gemini-flash-latest` —
  an alias, on purpose) or Ollama (local — point `OLLAMA_HOST` at the Atelier
  **governor**, `http://$ATELIER_HOST:8799/llm/ollama`, never raw `:11434`).
  Which to choose, the models on the governed lane by *capability*, and the
  thinking-model caveat: [`VISION_METHODS.md`](VISION_METHODS.md). Env keys:
  [`QUICKSTART.md`](QUICKSTART.md).
- An **active target** crops every capture/analysis path (screen record loop,
  stream frame grab) before encode/analyze/OCR.
