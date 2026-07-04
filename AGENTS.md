# AGENTS.md — gentle-eye

**gentle-eye is the reusable perception ENGINE** for screen understanding: a Rust **library** +
**CLI** + **MCP server**. It is consumed by other tools (Lookout / `sparse-delta-perception` is the
first, but it must stay generic enough for any IDE/agent).

## The boundary (do not cross it)

| Belongs in **gentle-eye** (this repo) | Belongs in the **consumer** (e.g. Lookout) |
|---|---|
| capture (screenshot / stream / `displays`) | the overlay UI + window/lifecycle |
| region/element **detection & identification** | *which* region to watch, *how* to present it |
| the `Region` model + `RegionProvider`s + cascade resolver | focus/lock UX, "offer regions" chips, NL phrasing |
| WM enumeration, AT-SPI, contrast, CV (`opencv`), OCR, YOLO, VLM grounding | chat / agentic layer, model tiering, persistence |
| `target` (ROI crop), `analyze` (VLM), `read-text` (OCR) | the coding-assistant loop / sparse-delta loop |

**The test for any new capability:** *"Would another IDE/tool want this?"* → **here**.
*"Is it about the coding-overlay experience / talking to it?"* → the **consumer**.

**Consequences:** `opencv`, AT-SPI, and window-manager access are **gentle-eye dependencies only** —
a consumer never links them; it calls this engine (lib / CLI / MCP). If you are about to add screen
detection in a consumer repo, **stop** — add a `RegionProvider` here instead.

## Where to start

- **Region engine (in progress):** `docs/REGION_ENGINE.md` — the `Region` model, providers, the
  capability cascade, and lib/CLI/MCP surfaces. This is the home for all detection work.
- **Existing surfaces:** `gentle-eye --help` (verbs: `displays screenshot segment target read-text
  analyze record capture-stream redpen serve`). `serve` = the MCP server.
- **Design docs:** `GENTLE_EYE_PRD.md`, `docs/TARGET.md`, `docs/ONTOLOGY.md`, `docs/REGION_ENGINE.md`.
- **PRDs (in `~/dev/prd/`, `repo:gentle-eye`):** `gentle_eye_region_engine_2026-07-01` (the engine),
  `gentle_eye_opencv_region_tracking_2026-05-30` (motion tracking — shares the `opencv` feature).

## Build notes
- Default build needs **no system libs**. CV/tracking is behind a cargo feature that pulls `opencv`
  (`libopencv-dev` + clang). Keep it that way — the default must build clean anywhere.
- Target **X11** first (Wayland WM/AT-SPI geometry is compositor-dependent).
