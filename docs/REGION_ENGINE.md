# gentle-eye Region Engine

> **Status:** DESIGN (building). PRD: `region_engine` (`~/dev/prd/scratch/gentle_eye_region_engine_2026-07-01.md`).
> Research basis: `sparse-delta-perception/docs/research/screen-element-identification-2026-07-01.md`.

The **Region engine** is gentle-eye's answer to one question:

> *Given a screen (or stream), **what and where** are the regions, elements, and text — with
> **provenance** and **confidence**?*

It is **headless and reusable** — a Rust library + a CLI verb + an MCP tool — so any IDE/agent/tool
(Lookout is the first) can ask for regions without reimplementing detection.

---

## 0. Boundary — where code goes (READ FIRST)

- **gentle-eye = the perception ENGINE.** All *detection / identification* lives here: window
  enumeration, accessibility tree, contrast/CV segmentation, OCR, learned detectors, VLM grounding,
  the `Region` model, and the cascade resolver. `opencv`, AT-SPI, and window-manager access are
  gentle-eye dependencies **only**.
- **Consumers (e.g. Lookout / `sparse-delta-perception`)** decide *which* region to watch, *how* to
  present it, and let the user *talk* to it. They **call** this engine; they do **not** reimplement it.
- **The test:** *"Would another IDE/tool want this?"* → gentle-eye. *"Is it the coding-overlay UX?"* → the consumer.

**If you are an agent about to add screen-element detection anywhere: add a `RegionProvider` HERE.**

---

## 1. The `Region` — the common currency

Every provider emits the same shape; every consumer reads the same shape:

```
Region {
  bbox: PixelRect(x,y,w,h)   // screen-absolute pixels, clamped to the display
  source: Wm | AtSpi | Contrast | Hough | Segment | Yolo | Ocr | Vlm
  granularity: Monitor | Window | Pane | Element | Text
  trust: f32                  // source-tier prior × source confidence (0..1)
  role:  Option<String>       // semantic, when known: "button","textbox","editor"…
  label: Option<String>       // name / text content, when known
  parent: Option<RegionId>    // the region this was drilled out of (the cascade edge)
  provenance: Vec<Source>     // full source chain when fused
}
```

**`trust` + `provenance` are load-bearing, not decoration.** They tell a consumer whether it is
holding *ground-truth structure* (AT-SPI / WM) or a *pixel guess* (contrast / VLM) — which is what
drives escalation ("only a contrast box → verify with the VLM") and semantic debugging
("this element is `role=button name='Submit'` but disabled — why?").

---

## 2. The capability cascade (coarse → fine)

Detection is organized as a **trickle-down** over a granularity hierarchy, using the **cheapest /
most structural source that can answer** at each level, escalating to pixels only on a miss:

```
monitor → window → pane/panel → element → sub-element
```

| Granularity | Primary (free / exact) | Fallback | Last resort |
|---|---|---|---|
| **monitor** | `displays` *(existing)* | — | — |
| **window**  | **WmProvider** (X11 EWMH) | ContrastProvider | — |
| **pane**    | AtSpiProvider | Segment + **HoughLinesP** (union) | Vlm |
| **element** | AtSpiProvider (role+name+box) | YoloProvider | Vlm (Set-of-Mark) |
| **text**    | **docTR/OnnxTR** | tesseract *(existing `read-text`)* | — |

Resolver sketch:

```
resolve(request{ target?, mode, depth }):
  region = whole monitor
  for level in [window, pane, element, subelement] up to depth:
      p = cheapest provider whose probe(level, region) == true
      region = best child of `region` from p        # structural/free first
      if none: region = contrast/CV/VLM over region.pixels   # escalate on miss
      if satisfied(request, level): break
  return region (+ provenance/confidence)
```

Fusion across providers = IoU-NMS keeping the highest-trust box and **merging provenance**.

---

## 3. Providers (`RegionProvider` trait)

```rust
trait RegionProvider {
    fn granularity(&self) -> Granularity;
    fn cost(&self) -> Cost;                          // Free | Cheap | Heavy — orders the cascade
    fn probe(&self, within: &Region) -> bool;        // can I answer inside this region?
    fn regions(&self, within: &Region, frame: &Frame) -> Vec<Region>;
}
```

| Provider | Granularity | How it identifies | Cost | Notes / dep |
|---|---|---|---|---|
| **WmProvider** | window | X11 EWMH (`_NET_CLIENT_LIST` + geometry + `WM_CLASS`) → exact window rects for **all** apps | Free | `xcb`/`x11rb` |
| **AtSpiProvider** | pane/element | accessibility tree: `Component.get_extents(DESKTOP_COORDS)` + role/name/state | Free | `odilia-app/atspi`; ~60-70% apps (GTK/Qt/Electron/Firefox); VS Code/Cursor need `--force-renderer-accessibility` |
| **SegmentProvider** | pane | existing `segment` (variance gutters) **∪ HoughLinesP** (thin/uniform-pane dividers) | Cheap | `opencv` (`--features cv`); pure-Rust `imageproc` canny fallback |
| **ContrastProvider** | window | saliency: high-contrast rectangle vs busy background (the "browser on the wallpaper" case) | Cheap | `image`/`imageproc` |
| **OcrProvider** | text | `read-text` today; **docTR/OnnxTR** for tighter word boxes | Cheap | onnx / subprocess |
| **YoloProvider** | element | OmniParser `icon_detect` (AGPL → subprocess) OR own YOLO11 (Zenodo RICO+WebUI) | Heavy | AT-SPI-less apps only |
| **VlmProvider** | any | **index/attention ONLY — never raw coordinates** (Set-of-Mark over pre-detected regions, or a coordinate-free model like GUI-Actor) | Heavy | last resort |

> **Why "never raw coordinates":** VLMs (incl. Qwen2.5-VL) emit boxes in their `smart_resize`d
> space and ollama drops the resized dims → off-screen boxes. The whole field's fix is index /
> attention grounding. The VLM here only ever **picks among regions the cheaper providers already found.**

### Adding a provider
1. Implement `RegionProvider` in `src/regions/providers/<name>.rs`.
2. Register it in the resolver's provider list at its `(granularity, cost)` slot.
3. Emit `Region`s with honest `source`, `trust`, and `role`/`label` when you have them.
4. Add a live `#[ignore]`d test proving it against a real screen/app.

---

## 4. Surfaces (how consumers call it)

- **Rust lib (preferred):** `gentle_eye::regions::{Region, RegionProvider, resolve, providers::*}`.
  Depend on the crate directly instead of shelling.
- **CLI:** `gentle-eye regions [--display N] [--window] [--depth pane|element] [--match "text"] [--json]`
  → the fused region set (JSON, same shape family as `segment`). `gentle-eye locate "<nl>"` → single best region.
- **MCP:** `regions` / `locate_region` tools on `GentleEyeServer` — for agents (Claude, etc.).

---

## 5. Build cost & flags
- Default build stays **opencv-free** (`imageproc` canny fallback). `--features cv` pulls `opencv`
  (needs `libopencv-dev` + clang) for HoughLinesP **and** the deferred CSRT motion tracking — one
  dependency, both features. (See `gentle_eye_opencv_region_tracking` PRD.)
- Wayland is out-of-scope for v1 (WM/AT-SPI geometry is compositor-dependent); target X11 first.

## 6. Cross-references
- PRD (engine): `~/dev/prd/scratch/gentle_eye_region_engine_2026-07-01.md`
- PRD (Lookout consumer): `region_capability_cascade_2026-07-01` (sparse-delta-perception)
- PRD (motion tracking, shared `opencv`): `gentle_eye_opencv_region_tracking_2026-05-30`
- Research brief: `sparse-delta-perception/docs/research/screen-element-identification-2026-07-01.md`
- Existing: `docs/TARGET.md` (ROI crop), `GENTLE_EYE_PRD.md`, `docs/ONTOLOGY.md`.
