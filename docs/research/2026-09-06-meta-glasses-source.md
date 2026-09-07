# Meta Ray-Ban glasses as a gentle-eye capture source — research brief

**Date:** 2026-09-06 · **Status:** research complete, nothing built
**Hardware:** Ray-Ban Meta **Gen 1** (2023) · **Method:** 8 parallel research lanes
(DeepLake · githubawesome · paperlake · hypersearch · Meta official docs · repo lane ·
gentle-eye recon · gesture/super-resolution lane) + 2 grounded Gemini passes.

## 0. The goal

Let a coding agent see what Gyasi is looking at. Look at one of three screens, designate
it (by voice or by tracing a square with a finger), and get back an edge-detected,
perspective-corrected crop the agent can read.

**gentle-eye is the brain. The phone app is dumb.** It captures and relays; it never
decides what a screen or a region is.

## 1. Decisions already made

| # | Decision | Why |
|---|---|---|
| D1 | **Meta DAT SDK is the only path** | MentraOS does not support Meta glasses (it runs its own OS across brands it controls). Every other project — VisionClaw, OpenVision, OpenGlass, NixClaw — is a *consumer* of DAT, not an alternative. The one non-SDK approach shipped (`glasses-ai`) screen-scraped an Instagram livestream with hardcoded pixel coords; dead end. |
| D2 | **Everything local** | No cloud legs. Diverges from VisionClaw, which sends frames to Gemini Live. Terminate at the local governor or a local VLM instead. |
| D3 | **Burst, not stream** | 2–3 s typical, 10–15 s outer bound. A shutter, not a camera feed. Kills the battery/thermal problem outright. |
| D4 | **Phone mic, not glasses mic** | See §3 — dissolves the framerate-collapse constraint entirely. |
| D5 | **7 fps** (or 2 with mic) | fps set is fixed at **2 / 7 / 15 / 24 / 30**. There is no 5. |
| D6 | **Tauri v2 + Kotlin plugin** | Fits the existing `appstore publish` → F-Droid → Galaxy pipeline. DAT needs native Kotlin. |
| D7 | **No Neural Band, ever** | Permanently out of scope. Camera-visible hands only. |

## 2. What DAT actually gives you (verified against Meta's docs + repo)

```kotlin
Wearables.initialize(context)
val session = Wearables.createSession(AutoDeviceSelector()).getOrElse { ... }
session.start()
val camera = session.addCamera(StreamConfiguration()).getOrElse { ... }
camera.stream.start()
camera.stream.videoStream.collect { frame -> ... }   // Flow<VideoFrame>
camera.stream.capturePhoto()                          // → PhotoData.data
```

- **`MWDATCamera` is the Gradle module name** (`com.meta.wearable:mwdat-camera`), not a class.
- Resolutions: `HIGH` 720×1280 · `MEDIUM` 504×896 · `LOW` 360×640. Stills capped **1440×1080**.
- Stills only succeed **while the stream is active** — no independent high-res photo path.
- Lifecycle gate: register with Meta AI → pair → permission ("Allow once"/"Allow always")
  → `session.start()` → `camera.stream.start()`.
- Manifest: `APPLICATION_ID` (`0` in dev + Developer Mode), `CLIENT_TOKEN` (prod only),
  plus **`ANALYTICS_OPT_OUT` / `CRASH_REPORTING_OPT_OUT`** — set both (D2).
- Permissions: `BLUETOOTH`, `BLUETOOTH_CONNECT`, `INTERNET`.
- **Meta AI app must be INSTALLED** — docs say only that, never "running". Verified: *"Both
  flows depend on the Meta AI app being installed on the phone"* (`AGENTS.md`, x4 places).
- **Camera connectivity is DIRECT BLE from your own app's process** — verified from a real BT
  stack log (`BtGatt.GattService: clientConnect()`, issue #150). NOT proxied through Meta AI.
  Only *registration* uses Binder IPC (`ACDCRegistrationService`), internal to the AAR — the
  real sample manifest declares no `<queries>`, no `bindService`, no Meta AI reference.
- **A third component exists: the "DAT Wearables App" (DWA), running ON THE GLASSES**,
  separately versioned from both SDK and phone app. Errors: `DAT_APP_ON_THE_GLASSES_UPDATE_REQUIRED`,
  `DWA_SKIPPED_GATE_DISABLED`, `"Heartbeat timeout (5000ms) — DWA unavailable"`.
  Update via `Wearables.openDATGlassesAppUpdate(activity)`.
- Real error enum values (verified): `NO_ELIGIBLE_DEVICE`, `DAT_APP_ON_THE_GLASSES_UPDATE_REQUIRED`,
  `SESSION_ENDED_BY_DEVICE`. ⚠️ `SERVICE_NOT_FOUND` / `SERVICE_RESTORED` / `SERVICE_DISCONNECTED` /
  `DAT_APP_UPDATE_REQUIRED` **do not exist** — a grounded-search answer confabulated them.
- Apache-2.0, v0.9.0. **Binaries come from GitHub Packages** — `gradle sync` needs a PAT
  with `read:packages` or it won't build.
- **Mock Device Kit** exists — build and test without the glasses.
- Background capture on Android via foreground service (per the `samples/CameraAccess` README).
- **Still Developer Preview — cannot publish to end users.** Irrelevant here: sideloaded
  personal build. Preview sharing is capped at 100 testers.

### Device support (settled)
Gen 1 (2023) ✅ — *first* model supported, best tested. Gen 2 ✅, Oakley ✅, Display ✅.
**Ray-Ban Stories (2021) ❌** — excluded entirely, no Meta AI bridge.
The "Gen 1 only" claim circulating is a stale snapshot of the Dec 2025 preview.

⚠️ **Version numbers disagree across sources** — Meta's version-dependencies page gives
DAT 0.9.0 → Meta AI app **V282** → firmware **V126**; a grounded search said v272/v125.
Trust the doc, verify against the actual glasses.

## 3. The transport constraint, and how D4 dissolves it

**Glasses → phone is Bluetooth** for the DAT stream. ~200–500 ms latency. The resolution
cap is a *radio bandwidth* limit, not an artificial SDK restriction.

But the glasses **do** have WiFi — used for media offload:

| path | resolution | transport | trigger |
|---|---|---|---|
| DAT live stream | 720×1280 / 1440×1080 stills | Bluetooth | programmatic |
| native capture + offload | **3024×4032 (12MP) / 3K** | **WiFi Direct** | **physical button only** |

**Framerate trap #1 — the mic:** the glasses mic opens a Bluetooth SCO channel that competes
with video — framerate collapses from ~15–17 fps to **1–3 fps**. HFP and A2DP are also mutually
exclusive, so the speaker degrades too. There is no `mwdat-audio` module; mic was always
going to be raw platform Bluetooth APIs.

**Framerate trap #2 — the Meta AI app is a BLE COMPETITOR, not a proxy.** It holds its own
separate BLE link to the same glasses and contends with yours. Issue #162: a developer
force-stopped Meta AI mid-session and measured **steady ~30 fps** — until Android auto-restarted
it 11 s later for its own `ConnectivityCompanionDeviceService`, ending the session with
`SESSION_ENDED_BY_DEVICE`. **Your fps ceiling may be contention, not bandwidth.** Measure this
before accepting 7 fps as the limit. Related instability: issues #150, #162.

**D4 removes trap #1.** Phone mic → no SCO channel → video keeps full framerate, audio is
44.1/48 kHz instead of 8 kHz, and less code gets written. Mux on the phone before anything
leaves. Only caveat: glasses video lags ~200–500 ms behind locally-captured audio, so align
by timestamp. Loose sync is fine for this use case.

## 4. Architecture

```
Tier A  glasses ──BT 7fps──┐
        (quick look)       ├──> phone app ──LAN──> gentle-eye ──> local VLM
Tier B  glasses ──WiFi 12MP┘     (dumb:              (brain)
        (read properly)           capture + mux
                                  + phone mic)
```

**The phone runs a local MJPEG/HTTP server.** One producer, two consumers:
- Tauri webview previews it at `127.0.0.1:PORT` — the viewfinder, for aiming
- gentle-eye pulls `http://<phone>:PORT/stream` with ffmpeg

That second arrow is **the ATEM precedent exactly**: `docs/ATEM_STREAMING.md` added a whole
new source with **zero Rust changes** — just a relay producing a URL. Do not pipe frames
through Tauri IPC into the webview; the Kotlin side serves, both consumers pull.

### gentle-eye already has the processing (verified in-tree)
- `src/regions/mod.rs:30` — OpenCV `HoughLinesP` panel/pane dividers
- `src/target/measure.rs:181` — Canny edges + snapped rect
- `src/target/crop.rs`, `src/dayflow/perception.rs:514` `crop_regions`
- `src/bin/gentle-eye.rs:595` `detect_panels_bgra`
- `src/capture/stream.rs:39` `capture_stream_frame_cropped` — injects an ffmpeg crop filter
- `CaptureSource` trait `src/dayflow/source/mod.rs:375-407` — contract says a new kind means
  "writing an implementor and nothing else"
- `TargetSource::Stream{url}` `src/target/model.rs:61` — named crops on a stream, already exists

**✅ VERIFIED (2026-09-06):** Tier B is reachable. Per Meta's own help docs, *"Auto-import is
on by default for Android (13+)"* and media *"is transferred to your phone's photo app"* — the
**shared** photo library, not a Meta-private sandbox. A third-party app with ordinary media-read
permission can see it. Import fires when the glasses are in the charging case or hinges closed,
with Bluetooth + WiFi on. Sources: help.meta.com/ai-glasses/1510723473009423 and /272319252352130.
Remaining unknown: latency from capture to file-visible, and whether hinges-closed is
acceptable ergonomically mid-session.

## 5. Gesture: trace a square with a finger

**Validated as shipped prior art, not a novel idea.** EgoGestAR / GestARLite defines an
explicit **"Rectangle" gesture assigned to Region-of-Interest selection**, on exactly this
hardware profile (monocular RGB, no depth sensor). DrawInAir, same group: **88% accuracy,
1.73 s end-to-end, 14 MB**.

Tracing beats the alternatives *because it needs no depth*: the traced path's coordinates
*are* the region, in image space. Deictic pointing needs a ray from eye through fingertip to
a target at unknown depth — documented as suffering "monocular depth ambiguity," which is
why stereo-event-camera work exists to fix it. The "two L-hands framing" idea did not surface
once across ~20 queries; it has no literature behind it.

**Pipeline:**
1. Track **landmark 8** (index tip) per frame — MediaPipe HandLandmarker (21 landmarks;
   4 = thumb tip, 8 = index tip)
2. Gate start/stop on the **4↔8 pinch distance** — crisp binary, already computed, no timing
   ambiguity. Better than dwell (adds latency) or geometric loop-closure (fragile on a sloppy square)
3. Stabilize traced points into one reference frame — `calcOpticalFlowPyrLK` + `findHomography`
   with RANSAC on background features. Head drift during a ~1-2 s trace is real and standard to fix
4. Fit a quad — `convexHull` + `minAreaRect` / `approxPolyDP`
5. Rank frames by **variance-of-Laplacian**, keep top-K
6. `getPerspectiveTransform` + `warpPerspective` → de-skewed rectangle
7. Local VLM confirm/tighten pass
8. Optional: OCR each corrected crop, vote across frames

**Local geometry first, VLM second — do not skip step 1-4 and just ask the VLM.**
Evidence: on ScreenSpot-Pro (real cluttered screenshots) **Qwen2-VL-7B scores <2%**; the best
specialised open grounding models still sit under 20%. General VLMs are bad at grounding on
real screens. Get a geometric prior locally, then let the VLM refine it.

**VLM choice:** **Molmo / MolmoPoint** is the local-capable pointer — trained to emit
`<point x="…" y="…">` as plain text, 70.7% PointBench, open weights. Gemini's bbox mode
(`box_2d` = `[ymin,xmin,ymax,xmax]` on a 1000×1000 virtual grid) is cloud-only, which
conflicts with D2. Florence-2 (0.23B/0.77B) is the cheapest local option for per-frame use.

## 6. Multi-frame super-resolution: **NO** — build sharpest-frame instead

Tempting because a 2–3 s burst of a static screen gives 20–45 frames and head tremor looks
like free sub-pixel dither. **The evidence says don't.**

1. **Wronski et al. (SIGGRAPH 2019) requires RAW Bayer frames** — it merges *before*
   demosaicing, to exploit the sensor's actual sampling grid. DAT gives compressed,
   bandwidth-adaptive JPEG. Wrong input format, not a tuning problem.
2. **JPEG destroys precisely the signal SR needs.** DCT block quantization discards the
   high-frequency coefficients carrying sub-pixel aliasing detail, and adds blocking/ringing
   that SR models aren't trained to model.
3. **No sub-pixel motion ground truth.** Google's pipeline knows the shake (same device, IMU
   available). Over Bluetooth you'd have to estimate it blind from compressed frames — exactly
   where these pipelines break down.
4. **Head-tremor-as-dither is unverified** — no research found comparing head-mounted tremor
   amplitude to handheld. Load-bearing unknown.

**Do instead, in effort order:**
- **Variance-of-Laplacian frame selection** — `cv2.Laplacian(gray, CV_64F).var()`, O(1)/frame,
  zero alignment. Out of 20–45 frames of a static subject, one will have near-zero motion blur.
- **Perspective correction** — the real win. Skew causes baseline drift and character
  deformation (o↔0); rectifying is the standard OCR preprocessing step.
- **Multi-frame OCR voting** — the *one* multi-frame technique worth keeping. Errors across
  frames are largely uncorrelated; ROVER-style consensus removes **20–50%** of single-pass OCR
  errors. Works at existing resolution, no sub-pixel alignment needed.
- **Tiling** — hand the VLM 2–4 native-resolution crops rather than one downsampled composite.

Revisit SR only if Meta ever exposes raw/uncompressed frames.

## 7. Prior art worth stealing from

| project | take | avoid |
|---|---|---|
| **VisionClaw** (2.5k★, active, DAT-based) | 1 fps vision throttle decoupled from real-time audio; local LAN-bound agent execution | its single WS with no documented reconnect/backoff; 3 services needed for voice |
| **OpenVision** (MIT, iOS) | multi-backend swap (on-device MLX ↔ cloud) behind one interface | — |
| **MentraOS** (Apache-2.0) | managed-vs-unmanaged streaming as an API choice; capability negotiation | **does not support Meta glasses** |
| **glasses-ai** | — | screen-scraping a livestream with hardcoded pixel coords |
| **watch-skill** | index frames + timestamps, agent queries them | — |

**"PortWorld" does not exist.** Checked across ~8 search angles in two independent lanes;
the name resolves to an unrelated touchscreen vendor. Worth re-checking the source.

## 8. Literature (paperlake)

- **Ego2Web** (DeepMind, arXiv:2603.22529) — egocentric video → agent benchmark, *online*
  evaluation. Structured captions of the stream raised task success to 23.6% SR.
- **MEMORA** (arXiv:2607.14252) — 10 s clips + running textual summary; the memory
  architecture a glasses-fed assistant needs.
- **EviSelect** (arXiv:2608.05780) — dynamic keyframe selection beating uniform sampling.
- **Semantic-Aware Adaptive Visual Memory** (arXiv:2605.07897) — distinguishes *true*
  streaming (causal) from *pseudo*-streaming; most prior work is the latter.
- **What Should a Streaming Video Model Remember?** (arXiv:2606.16353)

## 9. Open items

1. ~~Can the companion app read the 12MP offloaded file?~~ **RESOLVED — yes**, auto-import is
   default-on for Android 13+ and lands in the shared photo library. Remaining: measure the
   capture→visible latency, and check the hinges-closed/charging-case trigger isn't a blocker.
1b. **[MEASURE FIRST]** What fps do you actually get, and how much of the ceiling is Meta AI
   BLE contention vs. raw Bluetooth bandwidth? Issue #162 suggests 30 fps is attainable when
   Meta AI is not competing. This changes D5.
2. Verify DAT version requirements against the actual glasses (V282/V126 vs v272/v125).
3. Does MediaPipe HandLandmarker hold up at **7 fps**? No official latency figure found; at 2 fps
   tracing is likely impossible.
4. Benchmark LFM2-VL-3B locally — the `<100 tokens/image` figure is from an article, unverified.
5. Decide Molmo vs Florence-2 for the confirm pass.
6. `paperlake` corpus path in the rules file is wrong — real store is
   `/media/gyasis/Drive 2/paperlake/corpus_unified` with `--store-name corpus`. Every query
   silently returned 0 hits until pointed there. Fix before it bites another session.

## 10. Cross-references
- `~/.claude/rules/domains/edge-vlm.md` — R-EVL1..3, token economy for visual feeds (authored from this research)
- `docs/ATEM_STREAMING.md` — the template: a source enters as a URL, zero Rust changes
- `docs/DAYFLOW.md` — `CaptureSource` contract
- `~/.claude/rules/domains/appstore.md` — the Tauri → F-Droid → Galaxy pipeline
