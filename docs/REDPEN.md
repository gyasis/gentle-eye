# redpen — native visual annotator

`redpen` is gentle-eye's native, low-friction channel for giving the coding agent
*visual* context: capture what's on screen, **mark it up** to communicate intent
("move this here", circle what's broken, point at what matters), and round-trip a
marked-up artifact the agent (or Gemini) can act on.

It is a **markup tool, not a crop-picker** — three tools in a small color palette:

| Tool | Use |
|---|---|
| **Pen** (P) | freehand — circle, underline, sketch, scribble a note |
| **Arrow** (A) | click-drag a directed arrow — "move this *here*", point at something |
| **Box** (B) | outline a region |

It is a **separate binary behind the `ui` feature**, so the default
`gentle-eye` MCP/CLI build never pulls the egui/wgpu GUI stack.

## Build & run

```bash
# Build (only with --features ui; default builds stay egui-free)
cargo build --release --features ui --bin redpen

# Capture the screen on launch, then annotate:
redpen                       # captures display 0
redpen --display 1           # pick a monitor (see `redpen --list`)
redpen --input /path/to.png  # re-annotate an existing image instead of capturing
redpen --list                # print the monitor catalogue and exit
```

| Action | Key / mouse |
|---|---|
| Pick tool | **P** pen · **A** arrow · **B** box (or the toolbar) |
| Pick color | **1** red · **2** blue · **3** green · **4** yellow (or the swatches) |
| Draw | click-drag on the image |
| Undo last mark | **Undo (last)** button |
| Save + quit | **Enter** (or the button) |
| Cancel | **Esc** |

## What a save produces

redpen is pure visual markup — it does **not** create gentle-eye crop targets.
Two artifacts land in `~/.gentle-eye/redpen/`:

- `<ts>.png` — the flattened image with every stroke burned in (what the model sees).
- `<ts>.json` — the sidecar: each annotation in normalized 0–1 coords, with color:

```json
{
  "image": "/home/you/.gentle-eye/redpen/1717157000.png",
  "size": [3440, 1440],
  "annotations": [
    { "type": "arrow", "color": "green", "from": [0.20, 0.50], "to": [0.65, 0.30] },
    { "type": "pen",   "color": "red",   "points": [[0.12,0.04],[0.13,0.05], "…"] },
    { "type": "box",   "color": "blue",  "rect": [0.10, 0.10, 0.25, 0.15] }
  ]
}
```

The sidecar turns a multi-megapixel image into a few lines of spatial text —
the agent gets the marks as geometry (an arrow's direction, a stroke's region),
not just "vision."

## Closing the loop (agent side)

The agent never spawns the GUI. It **discovers** captures (read-only) and feeds
them to Gemini:

```bash
# Newest-first list of captures (the discovery surface):
gentle-eye redpen-list [--limit N]

# Close the loop in one step — picks the latest capture, injects the box
# labels + coordinates into the prompt, and sends it to Gemini:
gentle-eye redpen-analyze [--prompt "your question"]

# Or target a specific capture / provider:
gentle-eye redpen-analyze --image ~/.gentle-eye/redpen/<ts>.png \
  --prompt "What should change in the marked region?" --provider gemini
```

`redpen-analyze` reads the sidecar and prepends each annotation as text, e.g.
`- green ARROW from (688, 720) to (2236, 432) — points toward / indicates moving
something to the arrow's head`, so the model reasons about direction and region
instead of only hunting for the colored marks. Requires `GEMINI_API_KEY` (or
`GOOGLE_API_KEY`) for the default gemini provider. The raw `analyze --image …`
command is still available if you want full manual control.

## Roadmap (deferred — see the PRD)

- Phase 2: redpen stays open; agent pushes new screenshots via a watched folder.
- Phase 3: mic + screen record → ffmpeg-muxed narrated mp4 for Gemini.
- Phase 4: native iPad/iPhone via `uniffi` + Swift/PencilKit reusing the Rust core.

PRD: `gentle_eye_redpen_native_annotator_2026-05-31`.
