# gentle-eye — "target" (region-of-interest / crop) tasks (dev-kid dogfood plan)

Generated **2026-05-30** from PRD `gentle_eye_target_feature_2026-05-29` (§1 vision,
§2 converged Claude×Gemini design, §3 dependency line, §4 TG1–TG7). Covers the
**full scope TG1–TG7** (Phase 1 crop primitive → Phase 2 `imageproc` measurement →
Phase 3 `opencv` tracking, deferred/feature-gated).

Branch: `002-dayflow-mode`. Sentinel test: `./.tooling/bin/cargo check --message-format=short`
(see `dev-kid.yml`). Local tier: Mac ollama LAN; escalation per `ralph-tiers.json`.

**Gemini is the runtime VISION provider** (the VLM "brain" per the PRD's
Vision-First/CV-Second philosophy) — it decides *what* to target; pure-Rust CV
(`geometry`/`imageproc`) is the caliper that measures *where-exactly*. Gemini is NOT
a dev-kid code-gen tier here; code-gen runs all-local first.

## Locked decisions (from the paired-debate, 2026-05-29)

| # | Decision | Choice |
|---|---|---|
| G1 | Coordinate space | Agent passes **normalized 0–1** `NormRect`; a pure utility maps `(NormRect, resolution, display offset) → PixelRect` (the critical "boring" code). |
| G2 | Brain vs caliper | **Vision-First, CV-Second.** VLM (Gemini) decides semantics; classical CV only snaps/measures where the agent points. |
| G3 | Active targets | **One active at a time**, persisted at `~/.config/gentle-eye/targets.json` (mirror the display catalogue `DisplayConfig::{load,save}`). |
| G4 | Phase-1 crop dep | **No new crate.** Screen = pure-Rust BGRA sub-rectangle slice (stride-aware); stream = ffmpeg `-vf crop=w:h:x:y`. (`image` is NOT currently a dep — confirmed in `Cargo.lock` — so P1 avoids it.) |
| G5 | Phase-2 dep | add **`imageproc`** (+ `image` iff imageproc needs it), **pinned**, pure Rust, no system libs (Canny / projection profiles / color-mask). |
| G6 | Phase-3 dep | **`opencv` ONLY here**, behind an **off-by-default `tracking` cargo feature** as an optional dep — default `cargo check` must NOT require `libopencv-dev`. Deferred until real motion-tracking is needed. |

## Conventions

- `[P]` = parallelizable within its wave.
- `[S]` = **sentinel checkpoint**: on completion the crate is compilable AND this
  task is a complete, runnable file with a clear objective. The orchestrator places
  a sentinel here. Skeleton / pure-declaration tasks get **no** `[S]`.
- Every task carries a **`> DONE:`** completion criterion the sentinel/ma-loop checks.
- TG→T mapping: TG1=T310 · TG2=T320–T321 · TG3=T330–T333 · TG4=T340–T341 ·
  TG5=T342 · TG6=T350–T354 · TG7=T360–T361. Gates+docs=T370–T372.

Reuse — do NOT rebuild (existing, green): `capture/display.rs`
(`DisplayConfig::{load,save}` persistence pattern + `DisplayInfo` resolution),
`capture/screen.rs` (`ScreenCapturer` → BGRA `Vec<u8>`, stride note), `capture/
stream.rs` (`capture_stream_frame` + `build_ffmpeg_args` + `probe_dimensions`),
`mcp/{server,tools}.rs` (`GentleEyeServer` `tool_*` + `list_tools`/`dispatch`,
schemars input/output structs), `bin/gentle-eye.rs` (CLI), `analysis/{gemini,
ocr,traits}.rs` (`VisionProvider`), `contracts/errors.rs` (`GentleEyeError`).

---

## Wave 0 — Foundation: module + models + contracts (skeleton, NO sentinel)

Pure declarations so later waves compile incrementally.

- [x] T300 [P] `src/target/mod.rs` — module root: `pub mod {model, geometry, store, crop, measure}` + re-exports. Wire `pub mod target;` into `src/lib.rs`.
      `> DONE:` lib.rs declares `target`; `cargo check` resolves the module tree (stubs allowed).
- [x] T301 [P] `src/target/model.rs` — `NormRect { x, y, w, h: f64 }` (0–1), `PixelRect { x, y, w, h: u32 }`, `TargetSource` enum (`Display(usize)` | `Stream(String)`), `Target { name: String, source: TargetSource, region: NormRect, active: bool }`. Derive `Serialize/Deserialize/Debug/Clone/PartialEq` + `schemars::JsonSchema`. `NormRect::is_valid()` (all in 0–1, w/h>0).
      `> DONE:` types compile; serde round-trip test for `Target`; `is_valid()` rejects out-of-range.
- [x] T302 [P] `src/target/errors.rs` — `TargetError` enum (`Io`, `Config`, `NotFound`, `InvalidRegion`, `NoActive`, `Capture`, `Measure`) + `From<TargetError> for GentleEyeError` + `mcp_error_code()` (mirror the `DisplayError`/`GentleEyeError` pattern in `contracts/errors.rs`).
      `> DONE:` `TargetError` maps into `GentleEyeError`; compiles.

## Wave 1 — TG1: coordinate-mapping utility (the critical "boring" code) `[S]`

- [x] T310 [S] `src/target/geometry.rs` — pure fns: `norm_to_pixel(r: NormRect, res: (u32,u32), offset: (i32,i32)) -> PixelRect` and inverse `pixel_to_norm(r: PixelRect, res, offset) -> NormRect`. Multi-monitor/ultrawide origins; clamp to bounds; round half-up; reject degenerate. No I/O, no deps.
      `> DONE:` unit tests cover (a) a 21:9 ultrawide box, (b) a non-zero multi-monitor offset, (c) norm→pixel→norm round-trips within ≤1px tolerance, (d) clamping past edges; `cargo check` + tests green.

## Wave 2 — TG2: target state + persistence `[S]`

- [x] T320 `src/target/store.rs` — `TargetStore` over `~/.config/gentle-eye/targets.json` (mirror `DisplayConfig::{load,save}` incl. `config_path()` + `HOME` handling): `load`/`save`, `add(Target)`, `list() -> &[Target]`, `remove(name)`, `set_active(name)` (clears any prior active — **one at a time**), `active() -> Option<&Target>`.
      `> DONE:` in a temp `HOME`, add→save→load returns the same set; `set_active` makes exactly one active; `remove` of the active clears active; tests.
- [x] T321 [S] Target lifecycle integration — `define → use → active()` returns the right `Target`; round-trips through disk.
      `> DONE:` integration test (define two, use the second, reload, assert active==second); `cargo check` + tests green.

## Wave 3 — TG3: crop primitive on capture (Phase 1 basis) `[S]`

- [x] T330 `src/target/crop.rs` — pure-Rust **stride-aware** BGRA sub-rectangle crop: `crop_bgra(buf: &[u8], full_w: usize, full_h: usize, stride: usize, rect: PixelRect) -> Result<(Vec<u8>, u32, u32), TargetError>`. No new dep. Bounds-checked.
      `> DONE:` a known small BGRA buffer (e.g. 4×4) cropped to a 2×2 box yields exactly the expected bytes + dims; out-of-bounds rect errors; test.
- [x] T331 [P] Wire **screen** crop — in the screen frame path (`capture/screen.rs` and/or `capture/service.rs`), if an active target with `source = Display` exists, apply `geometry`(T310)+`crop_bgra`(T330) to the BGRA frame BEFORE encode/analyze/OCR. Pass-through when no active target.
      `> DONE:` with an active target the produced frame's dims == the cropped dims; no-target path is byte-identical pass-through; test with a stubbed frame.
- [x] T332 [P] Wire **stream** crop — in `capture/stream.rs::build_ffmpeg_args`, when an active target with `source = Stream` exists, inject `-vf crop=w:h:x:y` (pixel rect from `geometry` over the stream resolution via `probe_dimensions`). Filter omitted when no active target.
      `> DONE:` `build_ffmpeg_args` with an active target adds the correct `crop=W:H:X:Y` filter in the right position; without a target the args are unchanged; unit test on the arg vector.
- [x] T333 [S] Crop integration — screen sub-image + stream `crop=` arg path both exercised end-to-end.
      `> DONE:` integration test (screen crop dims + stream arg presence); `cargo check` + tests green.

## Wave 4 — TG4 + TG5: MCP/CLI surface + confirmation image `[S]`

- [x] T340 `mcp/tools.rs` + `mcp/server.rs` — add `define_target` (input: `name`, `source`, normalized `region`) and `focus_target` (input: `name` → set active) following the existing `tool_*` + schemars input/output struct pattern; register in `list_tools`; route in `dispatch`. Tool **descriptions teach** the normalized-0–1-coords contract + the confirmation-image self-correction loop.
      `> DONE:` `tools/list` shows both tools with schemas; a `call_tool` round-trip defines then focuses a target and returns valid JSON; `cargo check` green.
- [x] T341 [P] `bin/gentle-eye.rs` — CLI subcommands `target add <name> --display N|--stream URL --region x,y,w,h`, `target use <name>`, `target list` (JSON out, reuse lib `TargetStore`).
      `> DONE:` each subcommand prints valid JSON; `target list` reflects the store; `target use` sets the active target.
- [x] T342 [S] TG5 **confirmation image** — `define_target`/`focus_target` return the resulting crop so the agent sees + self-corrects (no CV yet). Screen: grab one frame → `crop_bgra` → encode PNG (reuse the existing screen→PNG encode path; add `image` ONLY if no encoder exists). Stream: `capture_stream_frame` + `crop=`. Returns a path (and/or base64) to the cropped PNG.
      `> DONE:` `define_target` returns a crop image whose pixel dims match the requested normalized box (within rounding); `cargo check` + tests green.

## Wave 5 — TG6: Phase 2 measurement mode (`imageproc`, Zoom-then-Snap) `[S]`

- [x] T350 Add **`imageproc`** (pinned, e.g. `imageproc = "0.25"`; add matching `image` if required) to `Cargo.toml` per the constitution's pin discipline. `src/target/measure.rs` skeleton: `MeasurementResult { snapped_rect: NormRect, aspect_ratio: f64, confidence: f64, detected_grid: Option<(u32,u32)>, edge_alignment: f64 }` + `measure(buf, pixel_rect, full_dims) -> Result<MeasurementResult, TargetError>` stub.
      `> DONE:` `imageproc` pinned; `MeasurementResult` derives serde+schemars; `cargo check` green (stub).
- [x] T351 **Zoom-then-Snap** — crop a ~10%-padded high-res buffer around the rough rect, run Canny edge detection, snap the rough rect to the nearest strong horizontal/vertical edges; fill `snapped_rect` + `edge_alignment` + `confidence`.
      `> DONE:` a synthetic image with a known bordered rectangle → `snapped_rect` aligns to its borders within ≤2px; test.
- [x] T352 [P] **Projection-profile gutter detection** for tiled panes — sum column intensities → find low-variance gutters between panes → pane bounds; fill `detected_grid`.
      `> DONE:` a synthetic 4-column image → detects 3 gutters / 4 panes with correct boundaries; test.
- [x] T353 [P] **Red-marker** color-mask → bbox ("find the red box") + **Redline-Overlay** diagnostic crop (green = edges found, red = unsure) so the VLM supervises the CV.
      `> DONE:` a synthetic image with a red rectangle → bbox matches the marker; an overlay image is produced; test.
- [x] T354 [S] Measurement integration + MCP wiring — `define_target` gains a measure mode ("snap" / "find red marker") returning `MeasurementResult` + the Redline overlay for the agent to confirm/re-target.
      `> DONE:` measure mode returns a `MeasurementResult` + overlay over a synthetic frame; `cargo check` + `cargo clippy -D warnings` + tests green.

## Wave 6 — TG7: Phase 3 tracking (`opencv`) — DEFERRED, feature-gated skeleton `[S]`

PRD §3: opencv is deferred (system/build-dep friction). Land only an **opt-in
skeleton** so the default build never requires `libopencv-dev`.

- [x] T360 `Cargo.toml` `[features] tracking = ["dep:opencv"]` with `opencv` as an **optional** dep; `src/target/track.rs` — `RegionTracker` trait (`init(frame, rect)`, `update(frame) -> Option<PixelRect>`) + a default no-op stub impl compiled without the feature. opencv-backed impl gated behind `#[cfg(feature = "tracking")]`.
      `> DONE:` default `cargo check` (no features) compiles WITHOUT pulling opencv; `RegionTracker` trait + stub present; the `libopencv-dev`/deferral note is written in `docs/TARGET.md`.
- [x] T361 [S] Gate — confirm tracking is opt-in only.
      `> DONE:` `./.tooling/bin/cargo check` (default features) green and opencv absent from the default build graph; the `tracking` feature is documented as opt-in.

## Wave 7 — Gates + docs `[S]`

- [x] T370 [S] `cargo test` — all unit + integration tests green.
      `> DONE:` `./.tooling/bin/cargo test` exit 0.
- [x] T371 [S] `cargo clippy --all-targets -- -D warnings` — zero warnings.
      `> DONE:` clippy exit 0.
- [x] T372 `docs/TARGET.md` — the Vision-First/CV-Second design, the normalized-coords + confirmation-image agent loop, the 3-phase dependency line (image-free P1 → `imageproc` P2 → deferred feature-gated `opencv` P3), and MCP/CLI usage examples.
      `> DONE:` `docs/TARGET.md` exists and documents the agent-facing target workflow + the dep decisions.

---

## Run mode: dev-kid LITE + in-session checkpoints

Runs with **dev-kid lite**: it only needs this `.dk/tasks.md` to orchestrate waves.
Lite **dispatches** Wave N to the in-session Developer agent to implement, then waits
for `[x]`. The **`[S]` checkpoints are run by the in-session agent** — i.e. run
`./.tooling/bin/cargo check` at each `[S]` (and `cargo test` + `cargo clippy -D
warnings` at gate waves). ma-loop / tier escalation is the fallback only on a stuck
file.

- At each `[S]`: green → mark `[x]` and advance. Red → fix in place (attribution-aware:
  if the primary error span is a *dependency* file, fix that, don't mangle the target),
  then re-check.
- **Halt-and-fix:** any dev-kid / sentinel / ma-loop bug encountered IS the valued
  dogfood finding. Stop, capture, fix the TOOL, resume.
- **Constitution:** Edition 2021, all deps pinned, `scrap 0.5`, `rmcp 0.1`. New deps
  (`imageproc` W5, optional `opencv` W6) MUST be pinned; opencv MUST stay feature-gated.

- [ ] SENTINEL-T004: Sentinel validation for T004: verify implementations pass tests
- [ ] SENTINEL-T006: Sentinel validation for T005, T006: verify implementations pass tests
- [ ] SENTINEL-T010: Sentinel validation for T007, T008, T009, T010: verify implementations pass tests
- [ ] SENTINEL-T013: Sentinel validation for T013: verify implementations pass tests
- [ ] SENTINEL-T018: Sentinel validation for T014, T015, T016, T017, T018: verify implementations pass tests
- [ ] SENTINEL-T020: Sentinel validation for T019, T020: verify implementations pass tests
- [ ] SENTINEL-T021: Sentinel validation for T021: verify implementations pass tests
- [ ] SENTINEL-T022: Sentinel validation for T022: verify implementations pass tests
