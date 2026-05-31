# redpen — native visual annotator

`redpen` is gentle-eye's native, low-friction channel for giving the coding agent
*visual* context: capture what's on screen, draw a box around what matters, name
it, and round-trip a marked-up artifact the agent (or Gemini) can act on. The box
you draw is a real gentle-eye **target** — drawing a rectangle replaces typing
`target add … --region 0.25,0,0.5,1`.

It is a **separate binary behind the `ui` feature**, so the default
`gentle-eye` MCP/CLI build never pulls the egui/wgpu GUI stack.

## Build & run

```bash
# Build (only with --features ui; default builds stay egui-free)
cargo build --release --features ui --bin redpen

# Capture the screen on launch, then annotate:
redpen

# Or re-annotate an existing image:
redpen --input /path/to/shot.png
```

| Action | Key / mouse |
|---|---|
| Draw a box | click-drag on the image |
| Re-name a box | edit its `target:` field in the bottom bar |
| Undo last box | **Undo last box** button |
| Save + quit | **Enter** (or the button) |
| Cancel | **Esc** |

## What a save produces

Each drawn box becomes a normalized `NormRect` (0–1) target via
`target::geometry::pixel_to_norm` and is appended to the shared `TargetStore`
(the same store `gentle-eye target list` reads; the last box stays active).

Two artifacts land in `~/.gentle-eye/redpen/`:

- `<ts>.png` — the flattened image with the red boxes drawn in.
- `<ts>.json` — the sidecar the LLM reads:

```json
{
  "image": "/home/you/.gentle-eye/redpen/1717157000.png",
  "size": [3440, 1440],
  "targets": [
    { "label": "broken nav", "rect": [0.12, 0.04, 0.30, 0.08] }
  ]
}
```

The sidecar turns a multi-megapixel image into a few lines of spatial text —
the agent gets exact box coordinates, not just "vision."

## Closing the loop (agent side)

The agent never spawns the GUI. It **discovers** captures (read-only) and feeds
them to Gemini:

```bash
# Newest-first list of captures (the discovery surface):
gentle-eye redpen-list [--limit N]

# Then ingest the marked-up image (Gemini also ingests video natively):
gentle-eye analyze --image ~/.gentle-eye/redpen/<ts>.png \
  --prompt "The red box marks <X>. What should change?" --provider gemini
```

## Roadmap (deferred — see the PRD)

- Phase 2: redpen stays open; agent pushes new screenshots via a watched folder.
- Phase 3: mic + screen record → ffmpeg-muxed narrated mp4 for Gemini.
- Phase 4: native iPad/iPhone via `uniffi` + Swift/PencilKit reusing the Rust core.

PRD: `gentle_eye_redpen_native_annotator_2026-05-31`.
