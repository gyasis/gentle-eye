# gentle-eye — "target" (agent-driven region-of-interest / crop)

A **target** is an OBS-style crop on a capture source (a display or a stream).
Instead of capturing the whole ultrawide, gentle-eye focuses on a sub-region —
e.g. one of four tiled code/terminal columns — and **all capture + analysis
operate on just that crop**. A full-res crop of one pane gives near-perfect
OCR/vision versus a garbled full-frame downscale.

Source PRD: `gentle_eye_target_feature_2026-05-29`.

## Philosophy: Vision-First, CV-Second

The agent (a VLM, e.g. **Gemini**) is the brain — it decides *what* to focus on
from intent ("the editor", "the second-to-last pane") and passes a rough region.
Pure-Rust CV is the caliper — it measures *where exactly*. Classical CV only sees
edges/colors, so it's a tool the agent points, never the brain.

## The agent loop (normalized coords + confirmation)

1. The agent passes a **rough region in normalized 0–1 coordinates**:
   `region.x`/`y` = top-left, `region.w`/`h` = size, as fractions of the source.
   Example — the 2nd of 4 equal columns: `{x: 0.25, y: 0, w: 0.25, h: 1.0}`.
2. `define_target` crops to it and returns a **confirmation image** so the agent
   can SEE the crop and re-call with an adjusted box if it's off.
3. (Phase 2) `measure_target` runs **Zoom-then-Snap**: it snaps the rough box to
   the nearest strong edges, detects a tiled-pane grid via a projection profile,
   optionally finds a hand-drawn red marker, and returns a `snapped_rect` plus a
   **Redline Overlay** (green = edges found, red = the snapped box) for the VLM
   to confirm/re-target.
4. One target is **active at a time**, persisted at
   `~/.config/gentle-eye/targets.json` (sibling of the display catalogue).
5. With an active target, every captured frame is cropped to it BEFORE
   encode/analyze/OCR (screen = pure-Rust BGRA slice; stream = ffmpeg `crop=`).

## Surfaces

**MCP tools**
- `define_target { name, source, region, set_active? }` → confirmation image.
- `focus_target { name }` → switch the active target.
- `measure_target { source, region, find_red_marker? }` → `snapped_rect` +
  Redline overlay.

**CLI**
- `gentle-eye target add NAME (--display IDX | --stream URL) --region x,y,w,h`
- `gentle-eye target use NAME`
- `gentle-eye target list`

`source` is `{"kind":"display","index":0}` or `{"kind":"stream","url":"rtsp://…"}`.

## Dependency line in the sand

| Phase | Capability | Dependency |
|---|---|---|
| 1 | Crop primitive | **No new crate.** Screen = pure-Rust stride-aware BGRA slice; stream = ffmpeg `-vf crop=w:h:x:y`. (`image` is not needed for P1.) |
| 2 | Measurement (snap / gutters / red-marker / overlay) | **`image` + `imageproc`** — pinned, pure Rust, **no system libraries**. |
| 3 | Real-time tracking | **`opencv`**, behind the **off-by-default `tracking` cargo feature**. |

### Phase 3 is deferred — and why

`opencv`'s cost is the **system/build dependency**: it needs `libopencv-dev`
installed plus C++ linkage and is cross-platform-brittle (this project has hit
dep friction before). It is **not** about runtime compute, and classical CV
can't do semantics anyway (the VLM does). Static screens need no tracking, so
opencv buys nothing until there's real motion to follow.

Therefore:
- The **default build needs no system libraries** — `cargo build` / `cargo check`
  do not pull or compile opencv.
- The opencv-backed tracker is opt-in: **`cargo build --features tracking`**,
  which requires `libopencv-dev` to be installed on the host.
- Until then, `target::track` ships only the `RegionTracker` trait + a `NoopTracker`
  that holds the region unchanged.
