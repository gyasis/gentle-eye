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
| `dayflow` — sampling a source all day, summarising it, answering about it | which day to ask about, how to show the answer |
| `redpen` ingestion (reading a human's markup) | the drawing UI itself (`redpen` is a separate binary) |

**The test for any new capability:** *"Would another IDE/tool want this?"* → **here**.
*"Is it about the coding-overlay experience / talking to it?"* → the **consumer**.

**Consequences:** `opencv`, AT-SPI, and window-manager access are **gentle-eye dependencies only** —
a consumer never links them; it calls this engine (lib / CLI / MCP). If you are about to add screen
detection in a consumer repo, **stop** — add a `RegionProvider` here instead.

## The one thing to understand first

**Everything that UNDERSTANDS an image goes through `VisionProvider`** (`src/contracts/traits.rs`),
with two implementations: ollama (local, via the Atelier governor) and Gemini (cloud). Nothing has a
private path to a model. Two cheaper tiers sit beside it deliberately: tesseract for plain OCR, and
**geometry** for reading order — never a model, because the order boxes should be read in is a
spatial fact, not an opinion.

If you are about to call a model directly from a feature, **stop**: route through `VisionProvider`,
or you have created the second path that drifts.

## Where to start

- **The whole system, for people:** `docs/GENTLE_EYE_GUIDE.md` — what each tool is for, the
  workflows that chain them, and what it is built on.
- **The whole system, for agents:** `docs/TOOLS.md` — every MCP tool and CLI command, the flags, the
  JSON shapes. `tests/docs_agree_with_code.rs` FAILS if either doc drifts from the code, so trust
  them; if one is wrong, that test will say so.
- **Runnable playbooks (harness-agnostic):** `docs/playbooks/` — task-shaped recipes in plain
  markdown, using only the CLI. No MCP registration, no harness-specific format: any agent that can
  run a shell can follow them.
- **Region engine:** `docs/REGION_ENGINE.md` — the `Region` model, providers, the cascade.
- **Dayflow:** `docs/DAYFLOW.md` (what it is), `docs/DAYFLOW_OPERATIONS.md` (how to run it),
  `docs/DAYFLOW_LIMITATIONS.md` (**read this before trusting it** — a live ledger, items removed when
  closed).
- **Existing surfaces:** `gentle-eye --help` is authoritative and guarded by the drift test — read it
  rather than a list copied into a doc. `gentle-eye serve` = the preview server; the MCP server is
  the `serve` subcommand of the MCP binary.
- **Design docs:** `GENTLE_EYE_PRD.md`, `docs/TARGET.md`, `docs/ONTOLOGY.md`, `docs/REGION_ENGINE.md`.
- **PRDs (in `~/dev/prd/`, `repo:gentle-eye`):** `gentle_eye_region_engine_2026-07-01` (the engine),
  `gentle_eye_opencv_region_tracking_2026-05-30` (motion tracking — shares the `opencv` feature).

## Build notes
- Default build needs **no system libs**. CV/tracking is behind a cargo feature that pulls `opencv`
  (`libopencv-dev` + clang). Keep it that way — the default must build clean anywhere.
- Target **X11** first (Wayland WM/AT-SPI geometry is compositor-dependent). This is a real limit,
  not a preference: `x11rb` is an unconditional dependency with no `cfg` gate, so window features
  COMPILE on macOS and fail at connect. See `docs/DAYFLOW_LIMITATIONS.md`.
- **Live tests are the certification.** A green `cargo test` proves none of the hardware behaviour.
  The `#[ignore]`d tests are listed in the limitations ledger with how to run them; run them before
  claiming a capture or perception path works.
