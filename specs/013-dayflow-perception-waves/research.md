# Phase 0 Research: Dayflow — Continuous Screen-Activity Timeline

**Feature**: `013-dayflow-perception-waves` | **Date**: 2026-08-23

Every decision below is grounded in the code that exists on this branch, in the measured
evidence recorded in the spec, or is explicitly marked **UNVERIFIED** with the check owed.

---

## R1 — Wall-clock-accurate segmentation from the existing encoder

**Decision**: Keep `capture::encoder::PipeEncoder` (raw BGRA → ffmpeg stdin) as the pixel
path, and switch its ffmpeg argument builder to the **segment muxer**, supervised by a
rotation controller that owns the child process lifetime.

- `-f segment -segment_time <interval_s> -reset_timestamps 1 -segment_list <manifest>` with
  an output pattern of `chunk_%04d.mp4`.
- `-force_key_frames "expr:gte(t,n_forced*<interval_s>)"` so a segment boundary lands on an
  exact time, not on whenever the encoder next felt like emitting a keyframe. Without this,
  segments drift and their wall-clock ranges become approximations.
- The **`-segment_list` file is the liveness artifact**. ffmpeg appends to it as each segment
  closes, so "did a segment actually get written" is answered by reading a file ffmpeg wrote,
  not by asking our own code whether it thinks it is running.

**Rationale**: This is the mechanism the superseded plan named (`T220`, "ffmpeg segment
muxer"), and the segment list directly satisfies FR-006's evidence requirement. It also keeps
one long-lived ffmpeg per display rather than one per segment, avoiding a process spawn every
interval.

**Why a supervisor is still required**: the segment muxer alone cannot express three of our
requirements. Interval change (FR-034/035), pause/resume (FR-030/031) and manual off/on
(FR-033) all require the ffmpeg child to be **stopped and restarted** with new arguments. So
the design is *segment muxer for the steady state, controlled restart for every transition*.
Each restart begins a new `chunk_%04d` run, which is why segment identity must be
`(session, display, sequence)` and not ffmpeg's filename counter alone.

**Alternatives considered**:
- *Rotate `PipeEncoder` ourselves every interval* (stop/start per segment). Simpler and gives
  exact control, but pays a process spawn and a container finalize per segment on every
  display, and re-implements what the segment muxer already does correctly.
- *One long file, split afterwards*. Rejected outright: it defeats real-time summarization
  (D4/FR-014) and means a crash loses the whole day.

**VERIFIED 2026-08-23 (T004)** — was the schedule-critical unknown; it holds. Probed with raw
BGRA on stdin, exactly mirroring `PipeEncoder`, at both ends of the dayflow fps range and at
real capture resolution:

| config | segments | durations | manifest |
|---|---|---|---|
| 0.2 fps, 640×360, 10s | 3 | `10.000000` ×3 | one line per segment, contiguous |
| 0.5 fps, 640×360, 10s | 3 | `10.000000` ×3 | one line per segment, contiguous |
| **0.5 fps, 1920×1080, 10s** | 3 | `10.000000` ×3 | one line per segment, contiguous |

Boundaries are exact, not approximate — `duration` is `10.000000`, not `9.97`. The manifest
carries `name,start,end` per segment, which is the liveness artifact FR-006 reads.

**The working argument vector** (ffmpeg 4.4.2), to be reproduced in `build_ffmpeg_args`:

```
-f rawvideo -pix_fmt bgra -s <W>x<H> -framerate <fps> -i -
-c:v libx264 -preset ultrafast -pix_fmt yuv420p
-force_key_frames "expr:gte(t,n_forced*<seg_seconds>)"
-f segment -segment_time <seg_seconds> -reset_timestamps 1
-segment_list <manifest.csv> -segment_list_type csv
<dir>/chunk_%04d.mp4
```

`-force_key_frames` is what makes the boundary exact; without it `-segment_time` cuts at the
next keyframe and the wall-clock ranges become approximations. Keep both.

**Still to measure at build time, not blocking**: real segment BYTES. The probe's synthetic
low-entropy frames gave ~44 KB per 10s at 1080p; real screen content will be far larger, and
the disk-budget defaults (R9) must be set from a real capture, not from this number.

---

## R2 — Idle, lock and display-sleep detection

**Decision**: Detect idle via the **X11 MIT-SCREEN-SAVER extension** through the already-pinned
`x11rb = "0.13"` (enable its `screensaver` feature), reading idle-milliseconds and the
saver/blank state. Apply hysteresis: pause after a configured idle threshold, resume on the
first activity, and require a short dwell before either transition so brief pauses do not
thrash the recorder (spec edge case "idle flapping").

**Rationale**: No new crate. `x11rb` is already a dependency for the region engine's EWMH
window enumeration, so this reuses an approved, pinned dependency and honours the
constitution's "all deps pinned, no unspecified versions" rule. The screensaver extension
reports both an idle counter and whether the screen is blanked/locked, covering FR-030's three
triggers with one query.

**Alternatives considered**:
- *Shelling out to `xset q`* — parsing human-readable CLI output for a control-flow decision;
  fragile and untyped.
- *A dedicated idle crate* — a new dependency for something a pinned one already exposes.
- *Inferring idle from frame diffing* — appealing (it needs nothing new) but wrong: it cannot
  distinguish "user away" from "user reading", would keep the expensive capture path running
  to make the determination, and would not detect a lock screen at all.

**Enabling a feature on a pinned dependency is a dependency change** and must be recorded in
`Cargo.toml` with a comment saying why, per the house dep discipline.

**PROBED 2026-08-23 (T005) — the design above was HALF WRONG. Corrected.**

Host is GNOME on X11 (`XDG_SESSION_TYPE=x11`, `XDG_CURRENT_DESKTOP=ubuntu:GNOME`).

| signal | result | verdict |
|---|---|---|
| MIT-SCREEN-SAVER extension present | yes (opcode 144) | ✅ |
| `XScreenSaverQueryInfo` → `idle` ms | monotonic; +2000 ms per 2 s sleep (226312 → 232314) | ✅ **use this for idle** |
| `XScreenSaverQueryInfo` → `state` | **3** — outside the documented 0/1/2 range; `since = 0` | ❌ **unusable** |
| `xset q` saver | `timeout: 0` — the classic X saver is disabled under GNOME | ❌ |
| locker daemon (xscreensaver/light-locker/…) | none running | ❌ |
| `org.gnome.ScreenSaver.GetActive` (D-Bus) | `(false,)` — correct, screen is unlocked | ✅ **use this for lock** |
| `org.freedesktop.ScreenSaver.GetActive` | `NotSupported` | ❌ |
| `loginctl` `LockedHint` / `IdleHint` / `Active` | `no` / `no` / `yes` — all readable | ✅ **fallback, no session bus needed** |

**Corrected decision**: idle comes from the **X11 idle counter**; lock does **NOT** come from the
X saver `state` field — it comes from **`org.gnome.ScreenSaver` D-Bus**, with **logind
`LockedHint`** as the fallback. Building lock detection on the X `state` field, as this section
originally implied, would have silently never fired on this desktop.

**Open design question for T013** (do not decide by default): reading D-Bus from Rust means
either a new crate (`zbus`) — which the constitution's dep-minimalism argues against — or
shelling out to `loginctl` / `gdbus`. `loginctl show-session -p LockedHint` needs no session
bus and no new dependency. Decide explicitly at T013 and record it.

**Still UNTESTED**: the *locked* reading. Producing it requires actually locking the screen,
which is disruptive to do unasked. **Check owed** (≈20 s):
`gdbus call --session --dest org.gnome.ScreenSaver --object-path /org/gnome/ScreenSaver --method org.gnome.ScreenSaver.GetActive`
must return `(true,)` while locked. Probe kept at `scratchpad/t005/idle_probe.py`.

Platform note unchanged: this is an X11 + GNOME path. The detector stays behind a trait so a
host with no backend degrades to "never idle" (record continuously), never to a permanent pause.

---

## R3 — Multi-display capture merged into one timeline

**Decision**: One capture+encoder pipeline **per display**, each producing its own segment
series, all writing into **one** timeline keyed by time. Display identity travels with the
entry through region provenance (R8), not through a separate timeline per screen.

`capture::display::{DisplayManager, DisplayInfo, DisplayConfig}` already exists and enumerates
displays, so enumeration is reuse, not new work.

**Rationale**: The user asked for all displays on one timeline (decided 2026-08-23). Keying on
time and carrying display as provenance means a range query naturally returns what was
happening everywhere at 2pm, which is the question the feature exists to answer.

**The cost consequence must be stated plainly**: per-interval perception work multiplies by
display count. With two displays and a 15-minute interval, the budget for all perception on
both displays is 15 minutes — comfortable at the measured 1.6s warm per cropped region, but it
is a multiplier that the region-count cap (R6) has to bound, not a free win.

**Alternatives considered**:
- *A timeline per display* — makes "what was I doing at 2pm" require the caller to merge, and
  fragments the day-level question.
- *A single stitched composite frame across displays* — one encoder, but produces exactly the
  downscaled wide frame that the measured evidence shows scrambles columns and misreads
  digits. Directly contradicts FR-011.

**PARTIALLY VERIFIED 2026-08-23 (T006)** — enumeration done, concurrency still owed.

This host has **three displays, and they are wildly heterogeneous**:

| display | geometry | offset | note |
|---|---|---|---|
| `eDP-1` | 1920×1080 | +0+0 | laptop panel |
| `HDMI-1-0` | 3440×1440 | +1920+0 | ultrawide |
| `DP-1-0` | **1080×2560** | +5360+0 | **portrait (rotated)** |

Virtual desktop: **6440×2560**. This is strong confirmation of the one-encoder-per-display
decision — a stitched composite would be a 6440×2560 frame, i.e. exactly the extreme
downscaling that the measured evidence shows scrambles columns and misreads digits.

**NEW GAP this exposes — bbox coordinate space is undefined.** With displays at offsets +0,
+1920 and +5360, a `Region.bbox` of `(100, 100, 400, 300)` is ambiguous: display-local or
global virtual-desktop coordinates? The portrait monitor makes it worse, since width and height
swap relative to its neighbours. `data-model.md` must state which space `bbox` is in, and the
geometric reading order (T034) must sort **within a display** before merging across displays —
otherwise a top-left region on the portrait screen sorts against a bottom-right region on the
laptop and the ordering is nonsense. **Owed at T009/T033.**

**VERIFIED 2026-08-24 (T006)** — concurrent capture WORKS; no fallback needed.
`tests/dayflow_segmentation.rs::concurrent_capturers_across_all_displays` (`#[ignore]`, live)
holds a `scrap::Capturer` open on **every** display at once and pulls a frame from each:

| display | geometry | frame bytes |
|---|---|---|
| 0 | 1920×1080 | 8,294,400 |
| 1 | 1080×2560 (portrait) | 11,059,200 |
| 2 | 3440×1440 (ultrawide) | 19,814,400 |
| | **total per frame** | **39,168,000 (37.4 MiB)** |

Byte counts are exactly `w × h × 4`, confirming BGRA end to end.

**The number that matters for design**: one frame across all three displays is **37.4 MiB of
raw BGRA**. At the dayflow rate of 0.5 fps that is ~18.7 MiB/s streamed into the encoders,
~1.1 GiB/min of raw pixels — all of it transient (piped straight to ffmpeg, never buffered to
disk), but it sets the floor for the memory-pressure path and it is why compositing displays
into one frame was never viable. Use these figures, not the segment sizes from the synthetic
R1 probe, when setting the disk-budget defaults.

---

## R4 — Two-tier perception routing

**Decision**: A `PerceptionRouter` in front of the existing `VisionProvider` trait
(`contracts::traits::VisionProvider`), holding two configured providers:

| tier | job | binding |
|---|---|---|
| **text** | extract on-screen text from a full-resolution region crop | a compact local OCR model on the governed lane |
| **reason** | semantic and relational questions ("what was happening") | a larger local vision model on the governed lane |

The router dispatches on the **request kind**, which callers state explicitly — it does not
sniff the prompt. Escalation is a distinct, logged event carrying the reason (FR-007/010).

`analysis::ollama::OllamaProvider` already takes a `base_url` and `model`, so both tiers are
instances of the existing provider against different models. **No new provider type is needed
for the model calls themselves** — the new code is the router, the crop feeding, and the log.

**Rationale**: measured — cropped text tier 1.6s / 231 tok against 39s / 787 tok for the
vision tier on the identical frame. Spending the vision tier on text is a ~24× latency penalty
for a worse result. Making the caller name the request kind keeps escalation auditable, which
is what FR-010 asks for.

**On the existing `analysis/ocr.rs` (tesseract)**: retained for its existing callers, but
**demoted** — the measured result was garbled on dark-theme terminal text, roughly half
unusable. It must not become the text tier by default. Nothing in this feature should route to
it, and it is not a fallback for the text tier: a silently-wrong transcript is worse than a
missing one.

**PROBED END-TO-END 2026-08-23 (pre-T026). NOTE: an earlier version of this section was
WRONG — corrected below after the user pointed to the prior working call.**

**The endpoint is the whole story: use `/api/generate`, NEVER `/api/chat`.**

`deepseek-ocr` is not a chat model. Sending it through `/api/chat` wraps the prompt in a chat
template, and the template bleeds into the output. Identical image, identical prompt
(`Extract all text from this image verbatim.`):

| endpoint | output |
|---|---|
| **`/api/generate`** + top-level `prompt` + `images` | `DAYFLOW-PROBE-7742` / `cargo check --message-format=short` / `segment_time 900 fps 0.5 chunk_0003.mp4` — **clean, verbatim, zero artifacts** |
| `/api/chat` + `messages[].images` | the same text interleaved with `>user` and `>system` role markers |

An earlier draft of this section reported "chat-template artifacts leak, so extend the
sanitizer." **That was a misdiagnosis of my own bad call.** There is nothing to sanitize —
there is an endpoint to get right. Prior art already had it right (session `a490673b`), which
is where the corrected shape comes from:

```python
POST http://<governor>:8799/llm/ollama/api/generate
{"model": "deepseek-ocr:latest", "prompt": "<prompt>", "images": ["<b64>"], "stream": false}
```

**Prompt shape IS load-bearing — confirmed on the correct endpoint** (warm, `load=0.2s`):

| prompt | latency | tokens | outcome |
|---|---|---|---|
| `Free OCR.` | **0.5s** | 36 | perfect verbatim text, no artifacts — **the text-tier default** |
| `<image>\n<\|grounding\|>Convert the document to markdown.` | 3.2s | 90 | text **plus bounding boxes** — see below |
| "Output the exact characters… do not summarise, do not reformat…" | 42.9s | **7366** | **degenerate repetition loop**, echoing "Do not add extra spaces between lines." indefinitely |

The verbose-prompt failure is real and endpoint-independent — it just fails *differently* on
each (`/api/chat` returned assistant boilerplate; `/api/generate` loops). Either way it burns
~43s and ~7.4k tokens and returns nothing usable, **without erroring**. So the prompt stays
pinned in config and test-guarded: a test asserts the response is neither boilerplate nor a
repetition loop (e.g. no single sentence repeated more than twice).

**A.4 OPEN QUESTION — FULLY ANSWERED. `deepseek-ocr` DOES emit grounding bounding boxes:**

```
text[[19, 60, 287, 151]]
DAYFLOW-PROBE-7742

text[[21, 255, 522, 366]]
cargo check --message-format=short

text[[21, 450, 623, 558]]
segment_time 900 fps 0.5 chunk_0003.mp4
```

Box-and-text pairs, per block, in 3.2s. **This does NOT collapse T311 into T300, and D7 still
stands** — the region cascade supplies *window and pane* structure from the WM, which no OCR
model can know, and geometric reading order stays computed because a deterministic sort is
testable and a model's box output is not. What the grounding mode DOES add is **sub-region
text geometry**: where each line sits *inside* a pane, which the cascade cannot provide. That
makes it a genuine complement for T035, at a 6× latency cost over `Free OCR.` — so it is used
deliberately when intra-region layout is wanted, never as the default text path.

**Residency (R5) — evidence FOR, not against.** One call in this session took **43.9s with a
cold load** because `qwen3-coder:30b` (32.2 GB) had been resident and evicted the OCR model.
Warm calls in the same session ran 0.5–3.2s. So eviction under memory pressure from *other*
tenants is real and observed, which strengthens the residency case rather than the ~6h
keep-alive reading weakening it. Settle it at T029 with both facts in hand.

**Still to measure**: `Free OCR.` at 0.5s was a small synthetic image; the prior real-frame
measurement was 1.6s on a cropped pane. Use the real-frame numbers for capacity planning.

**Governor routing (D8)** is already enabled by commit `6b256ab` (path-prefixed base URL) with
thinking-model preamble stripping from `d2f5192`. Both tiers therefore address
`<governor>/llm/ollama`, never a raw model port. Nothing further is required in the provider.

---

## R5 — Model residency versus idle-unload

**Decision**: Keep the **text tier resident** for the duration of an active recording, via an
explicit configured policy, and record per-segment latency including any reload (FR-008/013).

**Rationale**: measured cold load 10.3s against 2.6s warm. The governed lane unloads idle
models; at a 15-minute cadence the text tier is idle for essentially the whole interval, so
without residency **every** segment pays the cold cost. That is survivable at 15 minutes and
fatal at short intervals or high display counts, and it silently distorts the per-segment
latency the plan needs to reason about.

**Mechanism**: a keep-warm ping at an interval shorter than the lane's unload window,
active only while a recording is running, and stopping when it stops. The config knob is
three-valued — `resident` (default), `on-demand` (accept the reload, for a machine where
holding the model is unwelcome), and `off` — because the right answer depends on what else
the box is doing.

**Rejected**: pinning the model permanently regardless of recording state. Dayflow is a guest
on a shared box; holding a model resident while nothing is recording is exactly the kind of
unbudgeted occupancy the governed lane exists to prevent.

**MEASURED 2026-08-23 (T006) — and it likely INVERTS this decision.**

`GET /llm/ollama/api/ps` shows `qwen3-coder:30b` resident at 32.2 GB with `expires_at`
**~6 hours in the future**. If that reflects the lane's standing keep-alive rather than a
per-request override, a model stays warm for hours — far longer than any segment interval —
and the cold-load premise behind this whole decision largely evaporates. The keep-warm ping
would then be solving a problem that does not exist, while holding memory on a shared box.

**Do not implement the keep-warm ping until this is settled.** **Check owed at T029**: load the
actual text tier through the governor, read *its* `expires_at` from `/api/ps`, and determine
whether the window is global or per-request. If it is ~6 h, the residency policy default flips
from `resident` to `on-demand` and T029 shrinks to "measure and document", not "build a pinger".

**Model tags corrected (verified against the live lane, 58 models):**

| tier | tag used in this spec | tag that ACTUALLY EXISTS |
|---|---|---|
| text | ~~`deepseek-ocr:3.3b`~~ | **`deepseek-ocr:latest`** |
| reason | `ornith-1.5-9b` | **`ornith-1.5-9b:latest`** (also `ornith-1.5-35b:latest`) |

`deepseek-ocr:3.3b` does **not exist** on the lane; a call using it would fail at T026. Use the
verified tags, and read them from config rather than hardcoding either.

---

## R6 — The rate limiter is a real constraint, and it is not currently wired

**Finding (verified by inspection)**: `security::rate_limiter::RateLimiter::per_minute(10)`
exists and is unit-tested, and the constitution states analyze calls are "enforced by
`src/security/rate_limiter.rs`". But a repo-wide grep for `RateLimiter` outside the module
itself returns **only the `pub use` re-export in `security/mod.rs`** — there are no call sites.
The limiter is defined and exported but **not applied anywhere**.

This matters two ways:

1. **If it is wired as-is, Dayflow breaks against it.** Ten analyze calls per minute is a
   sensible ceiling for a human clicking `analyze_video`. Dayflow at one segment per display
   per interval, times the regions in each, is a different traffic shape entirely.
2. **If it is not wired, the constitution overstates the code**, and a plan that assumes
   enforcement is planning against a document rather than the program.

**Decision**: treat the limiter as **per-key** and give Dayflow's internal perception traffic
its **own key with its own budget**, derived from the configured interval, display count and a
**hard per-segment region cap** — rather than sharing the interactive `analyze_video` bucket.
Bound the work at the source (cap regions per segment) so the budget is a safety net, not the
thing doing the shaping. The interactive tools keep the 10/min ceiling unchanged.

Recorded in the plan's Complexity Tracking as the one constitution tension in this feature.

---

## R7 — Non-uniform segment lengths

**Decision**: Store each segment's **actual** start and end wall-clock, and never derive
duration from configuration. Interval changes take effect at the next boundary by restarting
the encoder child (R1); already-written entries are untouched (FR-035).

**Consequence to enforce in review**: no query, aggregation, standup total or ordering may
assume a uniform segment length. A day may legitimately contain 15-minute segments, 30-minute
segments, several short ones from pause/resume transitions, and one truncated final segment.
`ChunkRef` already carries `start_wall`/`end_wall`, so the model supports this today — the risk
is arithmetic that multiplies a count by a configured interval. Any such computation is a bug.

---

## R8 — Structure from geometry, not from a model

**Decision**: Extend `regions::Region` with a **display identity**, and carry
`region_id` / `bbox` / `parent_region_id` / `display_id` onto the timeline entry as nullable
columns. Reading order is computed by sorting regions geometrically (top-to-bottom by band,
then left-to-right within a band, with the parent tree bounding the comparison) — never
requested from a model.

**Current state (verified)**: `regions::Region` already has `bbox`, `parent: Option<u64>`,
`source`, `granularity`, `trust`, `role`, `label`, `provenance` — the layout tree exists. It has
**no display field**, which is the one gap multi-display introduces. `regions::assign_parents`
already builds the containment tree, so parenting is reuse.

**Rationale**: D7. Geometry is deterministic, free, and already modelled; a model's reading
order is none of those. Determinism is also what makes the ordering test in FR/US4 meaningful —
a geometric sort gives the identical answer on every run, which a model does not.

**Schema-first**: the geometry columns land even while only text is populated, because
migrating historical rows later is the expensive path (US4 rationale). All new columns are
nullable so the existing `timeline_entries` rows from `T240` survive untouched (FR-021).

---

## R9 — Shrink and evict

**Decision**: Shrink = re-encode the summarized raw segment to a low-rate timelapse plus the
retained extracted text, replacing the raw file. Evict = ordered deletion under budget:
summarized-raw oldest-first, then shrunk oldest-first, never the timeline, never an
unsummarized segment.

**Rationale**: D5, unchanged. The timeline is the permanent artifact; video is scaffolding.
The ordering constraint is what makes eviction safe to run automatically.

**Reuse**: the existing memory-pressure monitor (`capture::memory`) is the established shape
for a warm/cold/evict ladder in this codebase and the tier state machine should mirror it
rather than invent a second vocabulary for the same idea.

**The dangerous case is deletion**, so the guard is explicit: eviction reads the summarized
flag and refuses any segment that lacks it (FR-025). A segment that failed summarization
because the perception backend was down must be **retried, not reclaimed** — otherwise a
backend outage silently becomes data loss.

---

## Cross-cutting: what "done" means for a background recorder

The single largest risk in this feature is a false green — a daemon reporting healthy while
producing nothing, for a whole day, discovered the next morning. Two rules follow, and they
shape the design rather than the tests:

1. **Status is derived from artifacts, not from flags.** `chunks_written`, `last_chunk_at`,
   `last_summary_at` come from the segment manifest and the timeline table — things other
   processes wrote — never from a boolean the daemon sets about itself.
2. **A deliberate pause is not degraded.** With idle-pause and manual off/on, "no segments
   recently" has three distinct causes: paused, switched off, and broken. Status must name
   which. Collapsing them is what makes a liveness signal useless in practice.


---

## R10 — Environment trap: `ar` is hijacked on this machine (found at T003)

**Symptom**: `cargo check` passes, `cargo test` fails with
`could not find native static library 'sqlite3', perhaps an -L flag is missing?`, and the build
log is full of PromptChain Python logging that has no business in a C compile.

**Cause**: `~/.local/bin/ar` is **not** GNU `ar`. It is a symlink to
`~/Documents/code/dataengineer/autoresearch/bin/ar` — the "autoresearch" tool — and
`~/.local/bin` precedes `/usr/bin` in `PATH` (positions 21 vs 26).

The failure chain is deliberately confusing:

1. the `cc` crate compiles `sqlite3.c` → `sqlite3.o` **successfully** (the `.o` is on disk);
2. it then invokes `ar` to archive the object into `libsqlite3.a`;
3. the autoresearch tool runs instead, prints PromptChain logs, and creates **no archive**;
4. the build script still emits `cargo:rustc-link-lib=static=sqlite3`;
5. rustc fails much later with a *linker* error naming a *library*, pointing nowhere near `ar`.

**Blast radius is machine-wide, not project-local**: any Rust crate using the `cc` crate's
`.compile()` (i.e. anything vendoring C) breaks the same way. The gentle-eye main checkout is
unaffected only because it was built in May and the symlink is dated 21 June.

**RESOLVED AT THE ROOT 2026-08-23** — not worked around.

An audit of `~/.local/bin` found six entries shadowing system binaries; only `ar` was
toolchain-critical (`ab`, `gh`, `claude`, `mako-render`, `wsdump` are benign or intentional).

Fix applied, in this order so the tool was never unreachable:

1. added `~/.local/bin/ares` and `~/.local/bin/autoresearch`, both → the same entrypoint;
2. verified `ares` runs the tool;
3. removed `~/.local/bin/ar`;
4. confirmed `ar` now resolves to `/usr/bin/ar`, **GNU ar 2.38**;
5. the nine `ar-*` siblings were left untouched — none of them shadow anything.

**Verified the root fix stands on its own**: the local `AR` workaround was *disabled*,
`libsqlite3-sys` was cleaned (29 files, 29.2 MiB removed) and rebuilt from scratch. This time
`out/libsqlite3.a` **was produced** and the suite returned 50/50 green. The workaround was then
deleted rather than left to rot — a stopgap for a fixed bug is future confusion.

`~/.claude/skills/autoresearch/SKILL.md` was updated too (11 references `ar` → `ares`, backed
up first). It is the agent-facing instruction surface, so leaving it would have told a future
agent to feed a research brief to GNU ar.

**Still stale, not touched** (the user's repo, offered rather than edited): docs under
`~/Documents/code/dataengineer/autoresearch/` — `README.md`, `briefs/README.md`,
`docs/ENGINE.md` — still document `ar run …`. Functionally harmless, but they now name a
command that is binutils.

**The transferable lesson**: a PATH shadow does not fail where it is installed. It fails inside
whatever tool happens to invoke the shadowed name, one or two layers down, with an error that
describes the *symptom* (`could not find native static library`) and never the *cause*. When a
build fails with output from an unrelated program in it — here, PromptChain logging inside a C
compile — suspect a PATH shadow before suspecting the build.

---

## R11 — Content-identity gate: REUSED from Lookout, not invented (2026-08-24)

The user asked for "a pixel-by-pixel identity match, so if it's the same image again we don't
need to keep both — the same with OCR", and pointed at Lookout and videoocr as prior art. That
prior art exists, is in production, and is already Rust:
`~/Documents/code/sparse-delta-perception/lookout/src-tauri/src/perception/{engine.rs,capture.rs}`.

### The method, read from the source

**Frame gate** (`capture::gate_gray` + `capture::mean_abs_diff`, driven from `engine::tick`):

1. screenshot the display (optionally cropped to the focus bbox);
2. `ffmpeg -vf scale=GATE_WIDTH:-1 -pix_fmt gray -f rawvideo` → a small grey byte buffer;
3. `mean_abs_diff(prev, cur) > GATE_CHANGE` ⇒ changed, otherwise **do nothing at all**;
4. buffers of differing length return `f64::INFINITY`, so a resolution change can never be
   mistaken for "no change".

**Tuned constants, carried over verbatim:**

| constant | value | meaning |
|---|---|---|
| `GATE_WIDTH` | 240 | gate frames downscale to 240 px wide |
| `GATE_CHANGE` | 6.0 | mean-abs-diff to count as changed (screen grabs) |
| `GATE_CHANGE_ATEM` | 9.0 | higher for noisy MJPEG video — not dayflow's case |
| `CONTENT_STD` | 8.0 | grey std below this ⇒ blank/uniform, no content at all |

**Text gate** (`engine::terminal_pass`): each OCR line is normalised
(`split_whitespace().join(" ").to_lowercase()`), dropped if under 2 chars or already in a
`seen` set; `terms.rs` additionally content-hashes the tail (`hash_str`, `DefaultHasher`) so
identical text never triggers work twice.

### Why this is NOT literal pixel-by-pixel — and why that is better

A full-resolution exact comparison is **both more expensive and more brittle**. A blinking
cursor, one antialiased glyph edge, or a clock ticking a second would all report "changed" and
defeat the entire saving, while costing a full-res compare to discover it. The 240 px grey
downscale plus a 6.0 threshold is cheap *and* robust — it answers "is this meaningfully the
same screen", which is the actual question.

A test asserts the gate stays a downscale (`gate_width <= 320`) and that the threshold is
non-zero, since a zero threshold *is* pixel-exact matching reintroduced by accident.

### Two levels, because they catch different things

- the **frame** gate catches an idle screen — nothing happened, so nothing is stored or perceived;
- the **text** gate catches a screen that moved but says the same thing — scrolled, refocused,
  repainted — where the pixels differ and the content does not.

Neither subsumes the other, and reading is most of a working day, so together they are where
the cost actually goes to zero.

### Still to do

`videoocr` / `videolocr` is referenced by the plan (T038's shrink step cites its
change-extraction as a blueprint) but is **not on disk** under either name — `proj-locate`
returns nothing. Its ideas survive in Lookout (the batched multi-panel VLM call is annotated in
`engine.rs` as "videolocr concept #1"), so treat Lookout as the live source and do not go
looking for videoocr as a dependency.


---

## R12 — videoocr FOUND: the TEXT-level complement to Lookout's pixel gate (2026-08-24)

Corrects R11's closing note, which said videoocr was not on disk. It is. My searches failed
because I looked for a **project** by that name; it is a **module inside `ds-toolkit`**, on an
external drive:

```
/media/gyasis/Blade 15 SSD/Users/gyasi/Google Drive (not syncing)/Collection/ds-toolkit/knowledge/
├── information_ingest_3.py                              (4048 lines — the entry point)
└── ingestors/videolocr/video_process/videoocr.py        (961 lines — the method)
   (also mirrored at knowledge-ingestor/src/legacy_ingestors/videolocr/…)
```

`proj-locate` could never find it: not a repo, not under `$HOME`, and the GitHub
`gyasis/knowledge-ingestor` is a LATER, different incarnation with no video/OCR files at all.
**The lesson generalises**: a name-based project probe cannot find a module, and `$HOME` is not
the whole filesystem. Mounted drives are a real search location.

### What the non-`--gemini` path does — the architecture we are rebuilding

From `information_ingest_3.py:1003-1019`, verbatim:

```
# The Gemini flag drives the engine, and REVERSES the skip flags:
#   --gemini  → use whole-video Gemini   (skip_gemini=False, skip_ocr=True)
#   default   → Ollama cost path: cheap local code/diagram gate → Ollama Cloud
#               vision OCR with per-frame transcript context, no whole-video
#               Gemini (skip_gemini=True, skip_ocr=False)
```

The DEFAULT is the cheap path: **a cheap local gate decides which frames earn expensive
perception.** That is the same two-tier ladder as D6, arrived at independently and already in
production — strong corroboration that the dayflow design is the right shape.

### The method: text-level DIFF-MERGE, not just dedup

`videoocr.py` is not a pixel gate. It works one level up, on OCR'd text, and it does something
Lookout's seen-set does not: it **merges** rather than discarding.

| mechanism | detail |
|---|---|
| `VideoOCR(change_threshold=0.1, check_interval=5)` | sampling + change parameters |
| `_calculate_block_similarity` | `difflib.SequenceMatcher(a, b).ratio()` |
| text-block change gate | append only when similarity **< 0.95** — "higher threshold for text" |
| `_grep_similar_content` | common-line ratio; candidates at **> 0.3** similarity |
| `_diff_merge_chunks` | merges two chunks preserving unique lines, KEEPING code lines even when deleted |
| `CodeTracker` | tracks code blocks across frames by signature + indent, emitting `LineChange`s |
| `chunk_size = 50` | text processed in 50-line chunks |

### Why this matters for dayflow, and how it composes

Lookout and videoocr solve **different halves** of the same problem, and dayflow needs both:

| | Lookout | videoocr |
|---|---|---|
| level | pixels | text |
| question | "is this the same picture?" | "is this the same content, and what is NEW?" |
| method | downscaled grey mean-abs-diff | `SequenceMatcher` ratio + diff-merge |
| on a match | skip entirely | **merge, keeping unique lines** |

The merge behaviour is the part worth stealing for T035. Within a window, a scrolling terminal
or an edited file produces many samples whose text overlaps heavily. A seen-set drops the
duplicates but also drops the *new* lines' relationship to the old; a diff-merge yields the
UNION as one coherent block, which is what a timeline entry should carry.

Threshold note: 0.95 for "same text block" is far stricter than 0.3 for "worth comparing at
all". Both are tuned numbers from a working system — reuse them rather than re-deriving.

### ⚠️ Fragile location

The only copy lives on an **unsynced Google Drive folder on an external SSD**. It is not in
git, not on GitHub (the upstream repo replaced it), and it was ALREADY lost once — 127 hits in
the April-2026 recovery index are file listings of exactly this tree. If it matters, it should
be copied somewhere durable. Flagged, not acted on: it is not this repo's file to relocate.


---

## R13 — The full videolocr cost ladder, read from source (2026-08-24)

`ds-toolkit/knowledge/ingestors/videolocr/`. This is a **four-stage** cost ladder, every stage
local and free before anything expensive runs. Dayflow should adopt its shape wholesale.

### Stage 1 — pixel change gate (`videoocr.py:451`, `_frame_difference`)

```python
prev_gray = cv2.cvtColor(prev_frame, cv2.COLOR_BGR2GRAY)
curr_gray = cv2.cvtColor(current_frame, cv2.COLOR_BGR2GRAY)
frame_diff = cv2.absdiff(prev_gray, curr_gray)
non_zero_count = np.count_nonzero(frame_diff)
total_pixels   = frame_diff.size          # → changed-pixel FRACTION
```

Checked only every `check_interval` seconds (`frame_count % self.check_interval == 0`), and a
frame is kept when the fraction exceeds `change_threshold`.

**This differs from Lookout, and the difference is instructive.** Lookout compares the MEAN
ABSOLUTE MAGNITUDE of the difference; videolocr counts the PROPORTION OF PIXELS that changed
at all. Magnitude is robust to a large subtle shift (a theme change); proportion is robust to a
small intense one (a cursor blink). Neither is strictly better — dayflow should pick one
deliberately and say why, not average them.

| parameter | value |
|---|---|
| `change_threshold` (class default) | 0.1 |
| `change_threshold` (CLI/extractor default) | **0.4** — "lower means more frames" |
| `check_interval` | 5 seconds |

### Stage 2 — the "cheap local code/diagram gate" (`video_code_extractor.py:552`, `_frame_is_informative`)

The stage the `information_ingest_3.py` comment referred to, and the most reusable idea here.
Its own docstring:

> Cheap LOCAL Boolean gate: keep CODE **or** DIAGRAM/SLIDE frames; drop talking-heads/blanks.
> All local + free (tesseract OCR + cv2 line structure). Runs BEFORE the paid cloud vision OCR
> so we only spend on frames that carry real content.
> **FAIL-OPEN: on any error we KEEP the frame — never risk MISSING code/diagrams.**

Signals: `min_code_lines = 2`, `min_words = 12`, and `cv2.Canny(gray, 80, 200)` for line
structure (tables, diagrams, slide furniture).

**FAIL-OPEN is the principle to carry over verbatim.** A content gate that errs toward keeping
costs one wasted perception call; one that errs toward dropping loses the data silently and
forever — and dayflow cannot re-capture yesterday. Every gate in this feature must fail open.

### Stage 3 — OCR aggregation (`multimodal/ocraggregator.py`)

`OCRAggregator(similarity_threshold=0.85, max_history=5)`. Keeps a rolling window of the last 5
`TextBlock`s; for each new frame's lines, `_find_best_overlap` scores against that history with
`SequenceMatcher(...).ratio() >= 0.85` and `append_lines` EXTENDS the matched block rather than
creating a new one.

That is the scrolling-terminal problem solved directly: consecutive samples of a scrolling pane
are one growing block, not N near-duplicate entries.

### Stage 4 — text diff-merge (`videoocr.py`, `CodeTracker`) — see R12

Block-level: append only when similarity **< 0.95**; `_grep_similar_content` at **> 0.3**;
`_diff_merge_chunks` keeps unique and code lines; `CodeTracker` follows blocks by signature and
indentation across frames.

### The tuned-threshold table (reuse; do not re-derive)

| stage | parameter | value |
|---|---|---|
| pixel gate | changed-pixel fraction | 0.1 default / **0.4** in practice |
| pixel gate | check interval | 5 s |
| informative gate | min code lines / min words | 2 / 12 |
| informative gate | Canny thresholds | 80, 200 |
| OCR aggregation | overlap similarity | **0.85** |
| OCR aggregation | history window | 5 blocks |
| text block | "changed" similarity | **< 0.95** |
| text block | comparison candidate | **> 0.3** |

### What dayflow takes

1. **The ladder shape** — cheap local gates first, expensive perception last. Already D6;
   this is independent confirmation from a working system.
2. **FAIL-OPEN on every gate.** Non-negotiable for an all-day recorder that cannot re-capture.
3. **The informative gate**, adapted: videolocr drops talking-heads, dayflow's analogue is
   dropping blank/idle/wallpaper frames. `CONTENT_STD` from Lookout (R11) covers the blank case;
   the Canny line-structure signal is the richer version.
4. **Rolling-window OCR aggregation** at 0.85 over a 5-block history — the right answer for
   scrolling panes within a window, and better than a flat seen-set.
5. **A deliberate choice** between magnitude-based (Lookout) and proportion-based (videolocr)
   pixel gating, recorded rather than defaulted.


---

## R14 — Wave 2 checkpoint review findings (independent reviewer, 2026-08-24)

An independent reviewer (separate model, no authorship stake) checked the Wave 2 commits with
one mandate above all: find tests that pass without proving functionality. Verdict: **FAIL**.
It was right. What it caught, and what changed:

### Confirmed and fixed

1. **T009 was marked done while half-implemented.** The task said thread `display_id` through
   `detect`/`fuse`/`assign_parents`; only the field and builder had been added. `fuse()`
   clustered on `granularity + IoU` alone, so two regions with identical display-local geometry
   on different screens (IoU 1.0) **merged into one and a display vanished** — the exact
   conflation the commit claimed to fix. `assign_parents()` could parent a pane on display 1 to
   a window on display 0. Both now guard on display, **mutation-proven**: removing either guard
   fails the new tests, while `fuse_still_merges_duplicates_on_the_same_display` keeps passing,
   so the guard fixes the bug without disabling fusion.

2. **The old test could not have caught it.** It asserted field inequality on two hand-built
   structs — the only breakage it detected was `on_display` becoming a no-op. Replaced with
   tests that call `fuse` and `assign_parents` and assert on their output.

3. **The `Region.bbox` doc contradicted its only producer.** The doc declared display-local
   while the WM provider translates to ROOT and emits `display_id: 0`. Rather than quietly
   change one, the doc now states the invariant AND the current producer gap, so the
   contradiction is visible instead of hidden until T010.

4. **The gate module doc was contradicted by its own tests.** It claimed Proportion catches "a
   small intense change — a cursor"; the file's own case B proves a ~1% change trips **neither**
   strategy at the shipped thresholds. And only one direction of the asymmetry was demonstrated.
   Doc corrected to describe what is actually proven, the cursor case documented as deliberate
   ("a gate that fires on a cursor blink saves nothing"), and the missing direction added: 30%
   of pixels shifting by 25 gives mean 7.5 > 6.0 with fraction 0.3 < 0.4, so **Magnitude earns
   its place under `Either`** exactly as Proportion does.

5. **Test theatre removed.** Three tests in `models.rs` asserted only on literals the test
   itself supplied (`!c.summarized` where the helper hardcoded `summarized: false`; `0 == 0`;
   tuple inequality of constants). They would pass against any implementation including a broken
   one. Deleted, with a note saying why, and replaced by a test against `plan_chunks` — the code
   that actually assigns `sequence` — which fails if the assignment is broken.

6. **Clippy could not pass.** `-D warnings` comes from `.cargo/config.toml`, and 5 errors made
   the constitution's pre-merge gate unsatisfiable. One was introduced by this branch
   (a manual `!RangeInclusive::contains`) and is fixed. **Four are pre-existing**
   (`regions/providers/wm.rs:41,48`, `regions/mod.rs:302,340`) and are left for **T050**, which
   owns that gate — not fixed as a drive-by.

### Open risks the reviewer raised for T010 — address, do not rediscover

- **`changed_fraction` has no per-pixel tolerance.** It is a faithful port of videolocr's
  `count_nonzero`, so ANY ±1 difference counts as a changed pixel. On a real capture → downscale
  path with resampling jitter, more than 40% of pixels differing by ±1 is entirely plausible,
  which would make Proportion — and therefore the default `Either` — fire on nearly every
  sample and **erode the whole saving**. Untested hypothesis. T010 must measure it on real
  frames and, if confirmed, add a small per-pixel tolerance before counting.
- **`dayflow_samples` primary key includes `taken_at TEXT`.** If T010 writes second-resolution
  timestamps and ever samples twice within a second, the insert violates the key. Either use
  sub-second precision or key on the sample index instead.

### The lesson

The reviewer's question — *"what could I break in the source that this test would fail to
catch?"* — is the one that separates a test from decoration, and it is worth asking of every
test before it is written, not after. Three of the tests it flagged were written in the same
session that adopted a no-test-gaming rule.


---

## R15 — Wave 3 checkpoint review: a real user-facing defect (2026-08-24)

Verdict **FAIL**, and correctly. The headline finding was not a test problem — it was a
control defect in shipped behaviour.

### F1 — a mouse movement silently cancelled a deliberate OFF

`tick_idle`'s `BecameActive` arm called `windows.resume()` unconditionally, and `resume` never
inspected WHY capture was paused. Two live paths, neither covered by a test:

1. Turn capture **off**, walk away past the idle threshold, come back → the tracker emits
   `BecameActive` and **capture resumes on activity**, overriding an explicit instruction to
   stop recording. On an all-day screen recorder that is a privacy defect, not a nit.
2. Idle-pause first, **then** `turn_off` → `pause()` returned early because it was already
   paused, the cause stayed `Idle`, liveness reported **Paused rather than Off**, and the next
   activity resumed it. The off switch did nothing at all.

**Fixed** by making the distinction explicit rather than implicit: `PauseCause::is_automatic()`
(idle / locked / display-sleep lift themselves; `UserOff` does not), `resume_if_automatic()` for
the activity path, and `pause()` upgrading an automatic pause when a deliberate off arrives.
Both orderings now have tests.

### F2 — liveness manufactured FALSE faults

`assess` had no start reference, so a run read **`Degraded` at t=0 every morning** until its
first window closed — up to a full hour at the interval ceiling — and a run resuming from a long
pause read `Degraded` for an interval, contradicting FR-032.

The reviewer also identified why the tests missed it: `liveness_counts_only_windows_that_actually_closed`
called `liveness(at(0))` but asserted only `chunks_written`. Asserting `health` there would have
failed. **A test that stops just short of the interesting assertion is worse than no test — it
looks like coverage.**

**Fixed** with `producing_since` (run start, or the most recent resume). Staleness now measures
from the LATER of that and `last_chunk_at`, so a fresh or just-resumed run is given time to
produce, while a genuinely silent one still degrades. Three new tests, including one asserting
the start clock cannot excuse a recorder that stopped producing.

### F4 — hysteresis was a no-op at every realistic poll rate

`pending_for += since_last` credited time spent under the OPPOSITE condition toward the dwell.
Whenever the poll period is at least the dwell — 30s default, and dayflow polls **minutes**
apart — every transition fired on its first observation and the debounce did nothing. The
existing flap test hard-coded 5s ticks and could not see it; the day simulation ticks at 180s,
where it was provably inert.

**Fixed**: the dwell now starts when the flip is FIRST seen and accumulates only from subsequent
readings. New tests exercise the realistic 180s cadence in both directions.

### F3, F5, F6, S2, S3 — also fixed

- **F3**: `assess` had 8 positional args and tripped `clippy::too_many_arguments`, breaking the
  lint gate this wave. Replaced with `LivenessInput`; clippy is back to the 4 pre-existing
  errors owned by T050.
- **F5**: a test named "…and releases a held pause" exercised no such thing, and the branch it
  named was **unreachable**. Branch deleted, test renamed to what it actually checks.
- **F6**: the day simulation asserted only structural invariants, so it passed even if
  `tick_idle` were a no-op. It now asserts its own scenario — a recorded pause, a window closed
  BECAUSE of it, a post-change window longer than the original interval, and display 1 producing
  nothing after its unplug. **Mutation-proven**: making `tick_idle` a no-op now fails it.
- **S2**: `turn_on` silently resurrected a stopped run. Refused.
- **S3**: `displays_active` reported the selected count while paused or stopped. Now reports
  what is actually capturing.

### F7 — accepted as staging, and the checkbox corrected

The reviewer noted T017's claim that liveness reads "the segment ledger and `timeline_entries`"
overstates what shipped: the evidence currently comes from in-memory engine counters, and
`daemon.rs` is still a stub. `assess` is deliberately parameterised so a ledger can feed it in
Wave 4/5, and the sweep test does prove intent flags cannot yield `Healthy` — but the wording
was ahead of the code, and is corrected in tasks.md.

### The pattern across two reviews

Both checkpoints found the same shape of error: **a test that stops just short of the assertion
that would have failed.** Wave 2's display test asserted field inequality instead of calling
`fuse`. Wave 3's liveness test asserted `chunks_written` instead of `health`. Neither was
lazy — both were written believing they covered the behaviour. The reviewer's question ("what
could I break that this would not catch?") is the only reliable defence, and it has to be asked
of a test *before* it is written.


---

## R16 — Verifying the fixes (2026-08-24). PASS WITH NOTES, and two holes I made

After the Wave 3 review returned FAIL I fixed everything and self-certified. That is exactly
the situation these reviews exist to catch, so the fix commit got its own verification pass —
one whose brief was explicitly to distrust the author's grading.

### The question worth asking of any fix: were tests WEAKENED?

Independently diffed, assertion by assertion, against both pre-fix commits. Verdict: **no
assertion was removed, loosened, or retargeted to something easier**; several were made
strictly harder. The one genuine deletion — a claim about a branch proven unreachable — was
correct to remove.

This matters because weakening a test is worse than the original bug: it removes the alarm
while still looking like coverage, and nothing downstream can tell the difference.

### Two holes the verification found that I did not

**1. The fix was correct and asserted nowhere.** Deleting the LEDGER half of the pause upgrade
(`window.rs`, `pauses.last_mut().cause = cause`) passed all 321 tests. Live state would report
`UserOff` while the durable `pauses` record still said `Idle` — the precise divergence the fix
was written to prevent. Found by mutation, not by reading.

The irony is instructive: one commit earlier I had recorded R15's lesson — *"a test that stops
just short of the assertion that would have failed"* — and then did it again, in the fix for
the very finding that produced the lesson. **Knowing the failure mode does not prevent it.**
Only mutating the code and watching a test fail does.

**2. My F2 fix opened a new hole.** `turn_on` reset `producing_since` unconditionally, so
calling it on an ALREADY-RUNNING run flipped a genuinely `Degraded` state back to `Healthy` —
repeatably, forever. An idempotent "ensure capture is on" caller in the daemon (T019, not yet
written) would have permanently masked a dead sampler. The reviewer proved it by executing a
probe rather than inferring it.

The general shape: **a fix for a false NEGATIVE created a false POSITIVE.** F2 was "liveness
wrongly says broken"; the cure made it "liveness wrongly says fine", which is the far more
dangerous direction for this feature. Widening a health definition needs the same scrutiny as
narrowing one.

**Both fixed and mutation-proven**: removing either guard now fails a test.

### Also closed

- `turn_off` on a stopped run recorded a pause interval that could never close, leaving a
  permanently open gap in a finished run's ledger.
- The day simulation's S3 update lost "an unplug decrements displays_active *while running*" —
  restored as its own test.

### Method note

The verification ran its mutations in a **throwaway git worktree**, leaving the repo untouched
— worth copying. Mutation testing is destructive by nature, and doing it in the working tree
risks leaving a mutation behind if anything interrupts the restore.


---

## R17 — Third verification round: the root cause under the laundering family (2026-08-24)

Verdict **PASS WITH NOTES**. Both prior fixes verified and mutation-killed, no test weakened —
and five more confirmed findings, one of which explains the whole family.

### The root cause: closing a window is not producing

`note_closed` took its evidence timestamp from `ClosedWindow.end_wall`. But a window closed by
a **pause**, an **interval change**, a **display removal** or a **stop** ends at `now` whatever
the sampler was doing. So with a dead sampler and an open window, any of those events refreshed
`last_chunk_at` to the present and the run read **Healthy** again. Proven by probe.

This is the same shape as Hole 2 (`turn_on` laundering health) and the `set_interval` case the
reviewer found — they were not three bugs but **three symptoms of one wrong definition**.

Fixed by recording `last_sample_at` on the window and using it as the evidence: production is
**when a sample was last actually taken**, not when bookkeeping happened to close a window. A
window with no samples now contributes no evidence at all.

**The lesson generalises past this feature**: when a health signal has been laundered in three
different places, stop patching the call sites and ask what the signal is actually measuring.
Every one of those patches was locally correct and none addressed the cause.

### Also fixed

- **`stop()` during a pause left `to: None` forever** in a finished run's ledger — the exact
  defect my own `turn_off` guard's comment condemns. Fixed on one path, missed on the adjacent
  one. A later reader cannot distinguish "paused until end of day" from a truncated record.
- **`last_mut()` → `first_mut()` survived in TWO places** (`resume` and the pause upgrade),
  because no test drove **two** pause cycles — with one pause the first and last element are the
  same. A day with two pauses is ordinary, and the mutant would close the wrong interval or
  rewrite history. Two-cycle tests added.
- **The `producing_since` reset was itself unasserted** — third occurrence of "fix correct,
  asserted nowhere" in three commits.

### Two of my own test premises were wrong, and one was instructive

`closing_a_stale_window_is_not_evidence_of_production` failed at first because the interval
change I used (600s → 1800s) **widened the staleness tolerance** to 2×1800s, making the run
legitimately healthy for that silence. The code was right and the test was measuring the wrong
thing. Worth remembering: **changing the interval changes what "stale" means**, so any test
about staleness must hold the interval fixed or account for it.

The second was mundane — a 50,000-second offset crossed midnight and tripped the cross-day
guard.

### Method

Mutations ran in a **throwaway git worktree**, adopted from the previous review. One mutation
(`M2`) silently failed to apply because its pattern appeared twice in the file, and reported a
passing suite — a false "killed" that would have been read as success. **A mutation that does
not apply looks exactly like a mutation the tests caught.** Assert the pattern is unique, and
treat an unexpectedly passing mutation as suspicious rather than reassuring.


---

## R18 — A half-written file is a DROPPED FRAME (2026-08-25)

The daemon's corrupt-state handling originally "started fresh" silently. The user's correction:
a half-written file is a **possible dropped frame** — it needs a loud warning, it needs to be
**visible**, and where possible the frame for that interval should be **re-acquired** rather
than lost.

That reframing applies far beyond the daemon, and it exposed a category the design did not have.

### Drop is not skip

| | skip | **drop** |
|---|---|---|
| meaning | the gate worked — nothing changed | the frame was WANTED and could not be obtained |
| data | complete | **missing** |
| recoverable later | n/a | **never** — the minute is gone |

Filing a drop as a skip is the same false-green as everything else in this feature: the record
looks healthy while data is absent. `SampleRecord` now carries `drop: Option<DropReason>`
distinct from the gate verdict, and `perceived()` is false for both — but only a drop counts as
a hole.

### Three things a drop now does

1. **Logs at WARN**, naming the display, interval, attempt count and expected-vs-actual bytes.
2. **Is recorded and countable** — `Sampler::drops()`, `dropped_count()`,
   `unrecovered_drops()`, and surfaced through `DayflowLiveness::frames_dropped` so a status
   payload SHOWS holes instead of leaving them to be inferred from a gap.
3. **Triggers re-acquisition**: `observe_with_reacquire` asks the caller for a fresh frame FOR
   THE SAME INTERVAL before giving up. Recovering the frame beats recording the hole, because
   the minute is not repeatable. A recovered interval is still recorded as a drop flagged
   `recovered: true` — success must not erase the anomaly.

### DropPolicy: fail while developing, record in production

Second correction from the user: *an error may be necessary for now so we can dev fixes*. So the
posture is explicit rather than hardcoded:

- **`DropPolicy::Fail` (the default, today)** — a drop returns an error and stops the run. A
  hole quietly recorded in a ledger is easy to scroll past; a build that halts gets fixed.
- **`DropPolicy::Record`** — log, count, carry on. Correct for an unattended all-day recorder,
  where one bad frame must not cost the remaining seven hours.

The policy decides only whether the run CONTINUES. The drop is recorded either way, and a test
asserts that the failing policy still records — otherwise "fail fast" would quietly mean "fail
without evidence".

### The subtle bug this surfaced

A bad frame must not become the gate's comparison baseline. If it did, the next GOOD frame would
be diffed against garbage — or worse, a frame that was never stored would make a real change
look unchanged, turning one dropped frame into an unbounded run of false skips. The gate buffer
is now updated only after the frame is safely handled, with a test that replays the pre-drop
frame and asserts it still reads `Unchanged`.

### Daemon state gets the same treatment

`load_reporting()` returns a `StateAnomaly` alongside the state, because "no state because we
stopped cleanly" and "no state because the file was half-written" are the same value through
`load()` and mean opposite things. Only the second says the last run died mid-write and its
final windows may be incomplete.

### Method: a mutation that does not COMPILE reports nothing at all

R17 warned that a mutation which fails to APPLY looks like success. This session produced the
sharper variant twice: a mutation that applies but does not **compile** prints no `test result`
line whatsoever. Skimming for "FAILED" finds nothing and the eye reads it as fine.

Both times the mutation removed the last use of a variable, and `-D warnings` turned that into a
build error. The fix is to mutate in a way that preserves usage — `max_attempts.max(1).min(1)`
rather than `1u32` — and, more importantly, to **treat a missing result line as a failed
experiment, not a passing one**. Assert that a mutation run produced output before believing it.

---

## R19 — Perception tier: measured numbers, grounding mode, and CO-TENANCY (2026-08-26)

> **Renumbered from R14 on arrival.** It was appended by the duly planning session while this
> tree had already reached R18; the original R14 (line ~743) is the Wave 2 checkpoint review.
> Content is unchanged — only the heading moved to the next free number.

Measured through the Atelier governor (`:8799/llm/ollama`) on a real 1920x1080 screen
frame. **Read before implementing T026 (`perception.rs`).**

### R14.1 — The ladder, measured

| path | latency | tokens | outcome |
|---|---|---|---|
| tesseract (`read-text`) | ~instant | — | garbled; ~half unusable on dark-theme terminal text |
| `deepseek-ocr:3.3b`, FULL frame | 2.6s | 444 | near-verbatim BUT columns scrambled; `CLI` misread as `CI` |
| `deepseek-ocr:3.3b`, CROPPED region | **1.6s** | **231** | correct order, correct text |
| `deepseek-ocr:3.3b`, **grounding mode** | 10.1s | 482 | text **+ per-line bboxes** |
| `ornith-1.5-9b` (VLM tier) | 39s | 787 | verbatim, ~15x the OCR tier |

Cropping wins on latency, cost AND accuracy simultaneously — it is not a tradeoff.

### R14.2 — Grounding mode returns bboxes in ONE call

```
POST /llm/ollama/api/generate
prompt: "<image>\n<|grounding|>OCR this image."
->  Two things I did not do[[56, 36, 407, 61]]
    There's no test suite - verification was by hand against[[55, 99, 908, 126]]
```

Consequences:
- **Line-level reading order comes free from the OCR's own bboxes** — no region cascade
  needed to order lines WITHIN a region. Part of the Phase 6 layout work is already done.
- The cascade is still required for CONTAINMENT (which pane/window a line belongs to).
- **Grounding costs ~6x** (10.1s/482tok vs 1.6s/231tok on the same crop). Make it a
  per-call CHOICE, not a default: plain text when only words are needed.

⚠️ **DeepSeek-OCR is prompt-format-SENSITIVE.** Extra instructions break it — it returns
only `<|im_end|>`. Use the canonical prompt verbatim; do not "improve" it.

### R14.3 — CO-TENANCY dominates, and it is the number that matters for all-day capture

The 1.6s figure is **uncontended**. Measured 2026-08-26: a peer workload hammering
`deepseek-ocr` on the governor **while whisper held `large-v3` on the same 64 GB Mac**
degraded OCR to **17.8s/frame** — more than 10x worse.

Dayflow runs ALL DAY and will contend with every other Atelier tenant. **Size the
perception tier for the CONTENDED case, or reserve residency.** Sizing off 1.6s produces
a design that collapses the first time the Studio is busy. Cold load is a separate,
smaller effect (10.3s cold vs 2.6s warm).

Source: cross-session measurement, and PRD `dayflow_waves_continuance_2026-08-23` §5.


---

## R20 — Wave 4 review: a tautological test hiding real corruption (2026-08-26)

Verdict **FAIL**, five confirmed. The worst was a test I wrote and named for the exact
protection it did not provide.

### C1 — the tautology

```rust
assert!(w.duration().num_seconds() <= 0 || w.end_wall >= w.start_wall)
```

`duration()` **is** `end - start`. The first arm covers `end < start`, the second covers
`end >= start` — together, every possible value. **It cannot fail against any implementation.**
Meanwhile the code genuinely produced a **−3600s window**, and the commit message claimed the
protection existed.

This is a new species of the recurring failure. Earlier ones were tests that stopped *short* of
the interesting assertion. This one asserts something **structurally impossible to violate** —
it reads as rigorous and is worth nothing. A disjunction whose arms are exhaustive is always
this, and it is easy to write when phrasing a property defensively ("either it's zero or it's
positive").

Fixed with a clamp (`end_wall = start_wall` on a backwards step) plus a `clock_anomaly` flag, so
the ledger never holds a negative span AND the anomaly is not silently swallowed. Both halves
mutation-proven, plus a test asserting a normal close is NOT flagged — a flag that is always set
is noise.

### C2 — `frames_dropped` was unreachable plumbing

`note_frames_dropped` was called by nothing, in source or tests, and hardcoding
`frames_dropped: 0` passed all 380 tests. Status would have shown zero holes while frames were
dropped — the precise false-green R18 introduced the drop category to prevent, defeated by a
missing wire. Fixed with `sync_drops_from(&Sampler)`, counting only UNRECOVERED drops so a
successful retry does not inflate the number a reader acts on.

### C3/C4 — the cap

A capped session stamped its final window at ENFORCEMENT time, not the cap boundary: with a
sleep/wake the recorded window ran 13.4 hours past its own limit, claiming the user worked
through it. Same stretched-window failure the pause path already clamps. And `on_sample`
SWALLOWED the window the cap-stop closed, so the FR-005 "closed and accounted" final window
existed only in an in-memory counter and never reached a caller for persistence.

### C5 — the same lint, one wave later

`observe_with_reacquire` took 8 positional arguments and re-broke the clippy gate — the
identical defect fixed in `assess` one wave earlier with `LivenessInput`. Fixed the same way
(`SampleRequest`). **Two occurrences of one lint in consecutive waves means the habit, not the
instance, is the problem**: past about five parameters, bundle them.

### The harness lied to me twice more

1. `grep "^test result" | head -1` reads only the **lib** suite. My C1 test is an INTEGRATION
   test, so two genuinely-killed mutations reported as survivors. A mutation harness must
   inspect EVERY suite's result line, and count failures across all of them.
2. One mutation did not compile and printed no result at all — the R18 trap again.

Combined rule, now three times learned: **a mutation experiment must prove it ran.** Assert the
pattern applied, assert the build produced result lines, and count failures across every suite.
Anything else is indistinguishable from a pass.

### Process correction from the user

Waves had been overlapping — Wave 5 was built while Wave 4 was under review, and the reviewer
noted the commit landing mid-review. The rule is now strictly serial: **build → review → wait →
fix → verify → next wave.** No wave opens while another is being checked. Wave 5 therefore owes
its own review before Wave 6.


---

## R21 — Wave 5 review: a documented invariant with no test, and a query that denied its own record (2026-08-26)

Verdict **FAIL**, four confirmed. Both HIGH findings were bugs the tests were shaped
around rather than aimed at.

### F1 — `push_front` is correct for exactly one window

`failed()` requeued to the FRONT, and the doc, the module header and the commit message all
claimed this preserved time order. It does — **only** for a window taken from the head. One
taken from deeper in the queue jumps ahead of its own predecessors, and the inversion
**compounds per failure**: three windows failing in one outage drained `[2,1,0]`.

That reverses the timeline AND threads the rolling context backwards, so window 2's summary
seeds the prompt for window 1 and then window 0 — FR-016 says each chunk receives the
**preceding** chunk's context, and every prompt in that stretch had the wrong one.

The test could not have caught it: it drove a **single** failing window, for which front and
time-ordered are indistinguishable. The mutation `push_front` ← survived with zero failures.
Fixed by reinserting at the window's position in time, with a regression test that fails
**three** windows concurrently. **A retry invariant needs at least two concurrent failures to
be observable at all** — one is always ambiguous.

### F2 — the range query denied a record it contained

`WHERE start_time >= from AND start_time < to` is start-time *containment*. The headline query
is "what was I doing at 2pm", asked as a narrow range — so an activity that began at 1:50 and
ran through 2pm was **invisible**, and `ask_day` answered `"No activity was recorded"`. FR-018's
guard against invention inverted into denial.

Fixed to overlap (`end_time > from AND start_time < to`), kept half-open so an entry that merely
abuts the range start does not leak in. Both directions mutation-proven — the second matters as
much as the first, since over-wide overlap describes the wrong minute just as confidently.

### F3 — the doc described handling that did not exist, and the test performed it

`enqueue`'s doc said an empty window "is settled immediately by `next_due`". `next_due` never
looked at `sample_count`. The test asserted a literal it had itself supplied and then **called
`settled_empty()` by hand** — verifying its own choreography, not the scheduler's behaviour. The
whole empty-window path could be deleted and it still passed, while a caller trusting the doc
would spend a perception call describing nothing, once per idle window. Fixed by making the doc
true: `next_due` settles empties and never hands one out.

### F4 — "grows and is capped" also describes a linear schedule

The mutation exponential → linear survived. The assertions pinned only monotonicity and the
30-minute ceiling, both of which linear satisfies, while hammering a down provider far harder
than documented. Now pinned at 30/60/120/240 s.

### S1/S2 — the prompt

Times were rendered `%H:%M` in **UTC with no date**: on any non-UTC machine the labels are
offset from the clock the user means by "2pm", and a midnight-spanning range produces two
indistinguishable `02:00` rows. Now local time with the date.

`build_day_prompt` also interpolated `app`/`activity`/`summary` **raw**. That text is a model's
summary of **OCR of the user's screen** — so anything on screen reaches the prompt. A newline
inside a field forged extra timestamped rows or a second `QUESTION:` line. Now the question comes
first, the record is fenced, and every field is flattened.

Worth stating as a standing property: **the perception path makes screen content into prompt
content.** Every later wave that renders an entry into a prompt inherits this, and the assertion
must be structural — "no field can start a new LINE with a marker" — not a substring count, since
harmless prose may legitimately contain the word.

### The harness caught itself twice

R20's rule paid off immediately. Both guards fired on real setup faults: the throwaway worktree
was at `HEAD` and lacked the uncommitted fixes (patterns did not apply), then lacked the
untracked `.tooling/` toolchain (no result lines). Under the old harness both would have printed
"survived" and sent me chasing fixes for code that was already correct.


---

## R22 — Wave 5 re-review: fixing half an invariant, and a test that could not fail (2026-08-26)

Re-review verdict **PASS WITH NOTES** — every F-fix held, but three residuals, and two of
them are lessons about the SHAPE of a fix rather than the fix itself.

### The fix repaired the failure path and left the arrival path broken

R21 made `failed()` reinsert in time order. But `enqueue` still `push_back`ed, so **the sorted
queue that reinsert relies on was never established.** Windows do not arrive sorted: a window
closes on its END while the queue is keyed by its START, and sequence counters are **per
display**, so a short pause-truncated window on one display closes before a longer window that
began earlier on another. Demonstrated on the fixed source: hand-out order `[start 1000,
start 900]` — **out of time order, with no failure involved at all.**

The generalisation: **when a fix establishes an invariant, find every path that can violate
it, not just the one the bug arrived through.** A sorted insert on one path is not an ordering
guarantee. Both paths now go through one `insert_in_time_order` seam, so the queue has exactly
one ordering rule — the same single-seam discipline R-DR4d forced on chunk reads.

The order key also omitted `display_id`. Since sequences are per display, two displays' first
windows both carry sequence 0 and **tie**, making their relative order whatever the scan
happens to do.

### A test whose scenario was symmetric enough to prove nothing

`push_back` **survived** my regression test. The test failed *all three* windows — and when
every window fails once, push_back rotates the queue exactly back into sorted order, so it
cannot distinguish push_back from a correct sorted insert. A **partial** outage breaks the
symmetry (push_back drains `[2,0,1]`), and that is the test that was missing.

This is a distinct failure mode from R20's tautology and worth naming separately: **a scenario
so uniform that the wrong implementation coincidentally produces the right answer.** The tell is
a test that does the same thing to every element. Vary one.

### `is_control()` does not mean "cannot forge a line"

`flatten()` mapped control chars to spaces and I called the injection channel closed. **U+2028
LINE SEPARATOR and U+2029 PARAGRAPH SEPARATOR are categories Zl/Zp, not Cc**, so they passed
straight through into the prompt. Worse, Rust's `str::lines()` does not split on them either —
so my structural assertion stayed green while the bypass was live. The test and the bug shared
the same blind spot, which is why a *test* passing is never on its own evidence that a
*property* holds. Now flattened along with the bidi overrides (Cf), and asserted directly.

### Saying plainly when a fix closed nothing

`#[must_use]` on `PendingWindow` was decorative: `next_due` returns an `Option`, which std
already marks, and the real hazard — a caller that BINDS the window and then early-returns — is
invisible to the attribute. It is now documented as unenforced rather than counted as a fix.
Recording a non-fix as a fix is worse than leaving the item open, because it removes it from the
list without removing the risk.


## R23 — Wave 6 build: three thresholds that did not survive contact (2026-08-26)

The perception ladder (T026/T028/T030/T031/T052/T053). Every defect found here was found by
**mutation testing or by a test failing during construction**, not by review — worth noting,
because it is the first wave where that happened before the reviewer saw it.

### A per-minute budget cannot express a 3-minute cadence

`dayflow_budget_per_minute` computed `ceil(60 / interval)`, which clamps to **1** for every
interval longer than a minute — so the coarse 3-minute default (D10) asked for exactly the same
budget as a 1-minute focused session, and the "derived from the sampling shape" claim was empty.
Caught by the assertion `fine > coarse`. The budget is now measured over a **10-minute window**,
which is long enough for the coarsest supported cadence to be more than one tick.

**General form: a rate window must be longer than the slowest event it meters.**

### Importing a threshold across a change of metric

T052 specifies 0.85, from videolocr. That figure is for difflib's **character-level**
`SequenceMatcher`; mine is **line-level**. Ported unchanged, 0.85 sat ABOVE the legitimate case
— a pane scrolled two lines in a ten-line view shares 80% of its text and scored 0.80 — so every
sample of a scrolling document started a new block, the exact failure T052 exists to prevent.
Recalibrated to **0.65** against the two cases that must be separated (0.80 scroll must merge,
0.50 half-different screen must not), with the calibration itself asserted so a future change to
the metric fails there and says why.

### The metric itself was wrong, and only five samples showed it

Deeper than the threshold: I matched captures against blocks with a **symmetric** ratio
(`2·common / (len(a)+len(b))`). The block GROWS with every merge while a capture stays one
screenful, so the score **decays mechanically** even when each capture overlaps the block's tail
perfectly — 0.80, 0.73, 0.67, **0.62 → splits**. A symmetric metric guarantees every long
document eventually fragments and the threshold only decides when.

**Two samples cannot show this. Five can.** The fixture that found it was the one that modelled
the real usage rather than the minimum case — the same lesson as R22's partial outage, in a
different shape: *a scenario long enough for drift to accumulate.*

Fixed with asymmetric `coverage(block, incoming)` — "is this capture material I already have?" —
which is stable however large the block grows. That property is now its own test.

### Two constants that no behaviour depended on

Mutation `M4` (never merge) **survived**: the `SAME_BLOCK` arm and the `COMPARABLE` (0.3) arm did
the same thing, so everything merged through the weaker one. That is not just redundancy — it
meant two screens sharing **30%** of their lines were folded into one document, 70% unrelated
material. My "unrelated screen" test could not catch it because it shares ZERO lines and passes
under any threshold; the discriminating fixture is one sharing about half.

Mutation `M6` (drop the 0.95 skip) also survived, and correctly: line-level `diff_merge` adds
only lines the block lacks, so a near-identical capture contributes nothing **by construction**.
Deleted rather than wrapped in a test that would only have asserted the optimization was taken.
`tasks.md` records T053's gate as **subsumed**, not implemented — a difference worth writing down
rather than leaving a checkbox to imply the code contains a threshold it does not.

**Both are the same lesson: a surviving mutant sometimes means the code is redundant, not that
the test is weak.** The response is to delete the redundancy, not to manufacture a test for a
distinction that carries no behaviour.

### Test-premise errors, again (two more)

`grown = scrolled(0, 40)` to prove coverage does not decay — but that block genuinely *contains*
more of the capture, so a higher score was correct and the metric was fine. The block has to grow
with material **unrelated to the capture** for the property to be about growth at all. Fifth
occurrence of the same class: **the fixture did not produce the condition the assertion named.**


## R24 — Wave 6 review: a whole module that nothing called, and a merge that ate the document (2026-08-26)

Verdict **FAIL**, eleven findings. Three matter beyond this wave.

### The module was an orphan and I had ticked the box that said otherwise

`PerceptionRouter`, `summarize_segment_via_ladder`, `TextAggregator` and `RateLimiter` had
**zero callers outside `perception.rs`**, while T031 — *"Route the summarizer through the
router"* — was marked `[x]`. Every guarantee the wave proved was proved **in vitro**: the budget
bounded no real traffic, the escalation ledger recorded nothing that happened, and production
perception (had any existed) still took the old unbudgeted path.

**Fourth occurrence of the same pattern**, and the worst, because this time the task's own text
named the wiring as the deliverable. The rule now: **a task whose verb is "route", "wire",
"connect" or "integrate" is not done while `grep` shows the new symbol has no caller outside its
own module.** That check is one command and would have caught all four.

Wiring it surfaced a bug the type system had been holding: `ChunkRef` carries BOTH `index` and
`sequence`, and its own doc says `index` resets on every pause, resume, interval change and
display change. Resolving a window's samples by `index` would, after any pause, silently
summarise **a different window's samples**. The durable identity is `(session, display_id,
sequence)`. Keyed correctly, and mutation-proven.

### The merge ate the document it was supposed to build

`diff_merge` deduplicated by line VALUE, so any line already present anywhere in the block was
dropped from every later capture. For source code that means every closing brace after the first;
for prose, every blank separator; for a table, every repeated row. Under Content intent **the
merged text IS the deliverable**, so this silently corrupted the artifact — and the "absorb never
drops a capture" guarantee held only at BLOCK granularity while losing lines inside them.

Every fixture used globally-unique `line {i}` text, which is exactly why it was invisible.
**A fixture whose values are all distinct cannot exercise duplicate handling** — and duplicates
are the normal case in real text.

### The metric was fitted to fixtures with no UI in them

`coverage` counted shared lines as a SET, so two entirely different files behind the same editor
chrome — menu bar, file tree, status bar, terminal — scored **0.80** and merged. A realistic
full-screen capture is mostly chrome, identical between any two screens of one application, so a
day in one editor folded into a single interleaved blob.

The fix is not a different threshold, it is a different question: a scroll shares a **contiguous**
run, whereas unrelated screens share **scattered** lines. Matching and merging both now use the
longest common run. That also subsumes the earlier calibration: contiguity, not a tuned constant,
is what separates the cases.

**Three metric revisions in one wave** — symmetric ratio (decays as the block grows), set coverage
(chrome inflates it), contiguous run. Each was found by a fixture closer to reality than the last.
The lesson is not about similarity metrics: it is that **a fixture built from synthetic uniform
data validates the implementation against itself.**

### Assertions that could not fail, again

`the_comparison_history_is_bounded` asserted `blocks().len() > HISTORY` — but the length is
already `HISTORY + 1` before the probing call, so it held whether bounding worked or not. The
fixture performed the right experiment and then asserted nothing about its outcome. Now an exact
count.

And the budget's MAGNITUDE was unpinned: every assertion was a ratio, so halving the whole budget
passed. **A ratio-only test permits any uniform scaling of the thing it measures.**

### Known and accepted

`coverage` is exact trimmed-line equality, so OCR that perturbs most lines per sample fragments a
document the same way the symmetric metric did. Untestable without real OCR pairs; recorded here
rather than left as an unstated assumption. Captures of one or two lines are also degenerate —
coverage is 0.0 or 1.0 with nothing between — so `SAME_BLOCK` carries no meaning below about three
lines.


## R25 — Wave 6 re-review: each fixture passed by lacking the other's ingredient (2026-08-26)

Verdict **FAIL** again, and the finding is sharper than the bug.

### The two fixtures validated against each other's blind spot

R24's contiguous-run fix broke the case it was built for. A document scrolling **inside an
application window** — chrome at the top, chrome at the bottom, content between — scored **0.50
on every sample and produced five blocks**, because chrome interrupts the shared run at BOTH
content boundaries. That is the original T052 failure, reintroduced by the fix for the opposite
direction. Meanwhile a contiguous 12-line sidebar still false-merged two different files at 0.75,
so the metric was now wrong in **both** directions.

Neither test could see it:

> **the scroll test had no chrome; the chrome test had no scroll.**

Each passed only because it lacked the other's ingredient. This is R22's uniform-scenario lesson
one level up — not a scenario too uniform *within* a test, but a **test suite whose fixtures are
each missing a different half of reality**, so every individual test is green and the composition
is broken. The check that catches it: *does any single fixture contain all the ingredients that
co-occur in production?* Here one fixture with chrome AND scroll would have caught all three
findings at once, including the indentation loss.

### The metric, third and fourth revision

The answer was not a fourth threshold but a different decomposition. Chrome is positionally
**stable**; content **moves**. So strip the positionally identical head and tail, and compare only
what is between them. Scroll-inside-a-window scores 0.80 and merges; two files behind one window
score 0.0 and separate; a majority-chrome sidebar cannot carry unrelated content across.

That also subsumes the calibration argument: the separation now comes from the decomposition, not
from a tuned constant.

### A merge that fixed one corruption and introduced another

R24 fixed repeated-line loss and silently swapped in **whitespace loss**: the output was rebuilt
from trimmed lines, so every appended line lost its indentation and every blank separator vanished.
For Python that is not cosmetic. The repeated-lines test asserted `contains("two();")` rather than
`contains("    two();")` and could not see it. **Comparison trims; the merge must not** — those are
different operations on the same data and conflating them corrupted the deliverable.

### Validate before you spend

`limiter.check` ran before the empty-reason validation, so a malformed request **consumed a budget
token it could never use** — a caller could starve the day's real work with requests that were
refused anyway. Ordering: reject invalid, then charge.

### A mock uniform enough to hide the bug it was written to catch

The test proving the reasoning call carries every sample's text counted occurrences of a string the
mock returned **identically for every call** — so "all four samples" and "one sample, four times"
produce the same count, and a mutation sending the last sample N times **survived**. Fixed by making
the mock emit per-call distinct text. **A mock that returns a constant cannot witness which input
produced which output.**

### A key naming an isolation nothing implements

`INTERACTIVE_KEY` had no uses, no interactive bucket existed anywhere, and the test "asserting"
isolation built its own local limiter and checked it — an assertion that cannot fail, describing
protection that is absent. Deleted, with a comment saying why, so it is not re-added: isolation will
come from a separate limiter, never from a key string.

### My own edit tooling destroyed a file

A slice built from two markers in the wrong order produced an **empty** pattern, and
`str.replace("")` splices the replacement between every character — 1,070 lines became **4.5
million**. Restored from git. Every scripted edit now goes through a helper that asserts the slice
is non-empty and that the pattern occurs exactly as many times as expected. Same discipline as the
mutation harness, applied to editing: **an edit script must prove it edited what it meant to.**


## R26 — Wave 6, third pass: the metric was answering the wrong question (2026-08-26)

**PASS WITH NOTES**, and the remaining gap was not a regression but a question I had never asked
correctly.

### One metric, two questions

R25's `frame_split` answered *"is the CHANGED region a scroll continuation?"*. That is the right
question for a scroll and the wrong one for everything else. Any small change — typing at the
bottom of a file, an OCR misread of one line, a clock ticking in a status bar — produces a changed
region that is **novel by definition**, scores exactly **0.0**, and forks a near-duplicate copy of
the whole screen. And a mostly-static screen is the **most common state of an all-day capture**, so
that was the common case, not the pathological one. Measured: five samples of a file being typed
into produced five blocks; five samples of a static screen with one OCR-flubbed line, likewise.

The fix is two independent kinds of evidence:

| evidence | question | bar |
|---|---|---|
| contiguous run over the content | did this SCROLL from what I hold? | `SAME_BLOCK` 0.65 |
| unchanged fraction of the capture | is almost none of this new? | `STABLE_SCREEN` 0.90 |
| block's content region empty | does this strictly EXTEND what I hold? | absolute |

**The bars differ because the questions differ**, and that is the part I got wrong twice by
reaching for a single tuned number. A scroll may legitimately replace most of the screen; an edit
may not. One bar for both merges two different files whenever chrome happens to be a large share of
the capture — mutation-proven (E3).

### Window height must not decide document identity

The typing case first failed at `unchanged = 0.889` against a 0.90 bar, and the tempting fix was to
move the bar. That would have been wrong for a reason worth writing down: **two new lines is 5% of a
40-line view and 20% of a 10-line one**, so a fraction-of-screen rule makes the same edit "the same
screen" on one monitor and "a new document" on another. Dayflow is explicitly multi-display with
heterogeneous geometry (D-display selection), so that is not hypothetical.

The window-independent signal was already in the decomposition and I had not read it: when the
BLOCK's content region is empty, the capture strictly extends the block — nothing replaced, only
added. That is a document being written into, at any window size.

### Stating a judgement instead of hiding it

Two of my fixtures failed under the new evidence because they were unrealistic — two content lines
behind six lines of chrome is not an editor. Rewriting a failing fixture is how a test suite gets
quietly fitted to its implementation, so both directions are now asserted explicitly: a realistic
majority-chrome capture with ten differing content lines must SEPARATE, and a capture identical but
for one line in thirteen must MERGE, with the reasoning in the test. The second is a real
judgement — merging costs a stray line inside one block, splitting costs a near-duplicate copy of
the whole screen every interval — and it belongs in the open, not in a threshold.

### The fixture monoculture rotated rather than dissolved

R25 replaced "no chrome" fixtures with "chrome + scroll" ones and I treated that as solved. Every
pair in the family was still either a *perfect* scroll or *entirely different* content — no
fixture had "same screen, small change", which is precisely where both remaining bugs lived. The
blind spot moved rather than closing.

**Fixture families need an axis list, not an example.** For a screen capture the axes are: chrome
present or not · content scrolled, extended, edited, jittered, or replaced · window small or large.
Each new fixture should name which cell it occupies, so a gap is visible as an empty cell instead
of being discovered by the next reviewer.


## R27 — T029 residency: the owed measurement, taken (2026-08-26)

R5 said *"do not implement the keep-warm ping until this is settled"* and named the check owed at
T029: load the ACTUAL text tier through the governor and read ITS `expires_at`. Taken today
against the live lane.

| measurement | value |
|---|---|
| `deepseek-ocr:latest` cold load | **3.74 s** (`load_duration`) |
| warm re-call | **0.18 s** — 20× |
| resident size | 7.4 GB |
| governor's default window for THIS model | **~50 s** |
| explicit `keep_alive: "10m"` in the request | **honoured** — `/api/ps` then reports `expires_in=10.0 min` |

### This inverts R5's inversion

R5 saw `qwen3-coder:30b` resident with `expires_at` ~6 hours out and concluded the cold-load
premise "largely evaporates". That figure does not generalise: it is **per-model / per-request,
not a global lane setting**. The text tier's own default window is ~50 seconds — SHORTER than the
coarse 3-minute sampling interval (D10), so under the default policy **every single sample pays
the 3.74 s cold load**, and a 15-minute segment of 5 samples pays it five times.

**The lesson is narrower than "measure": one model's residency tells you nothing about another's.**
R5 read a number off the model that happened to be loaded and reasoned about a different one.

### And it makes T029 much smaller than planned

Because `keep_alive` is honoured per request, residency needs **no background pinger, no extra
thread, and no wasted keep-warm calls**. It is a parameter on the calls Dayflow is already making,
which also means the model unloads on its own when sampling stops — the failure mode of a pinger
(holding 7.4 GB on a shared box after the session dies) cannot occur.

The three-valued knob therefore becomes:

| policy | `keep_alive` sent | for |
|---|---|---|
| `Resident` | interval × 2 + margin | fine-grained focused sessions where 3.74 s per sample matters |
| `OnDemand` (default) | omitted — governor's own window | coarse all-day tracking, where 3.74 s per 3 min is 2% overhead |
| `Off` | `"0"` — unload immediately | leave the shared box free; pay the load every time |

Default `OnDemand`, because the coarse cadence is the default and 2% is not worth holding 7.4 GB
of someone else's memory.


## R28 — Wave 7 review: I measured the right thing and reasoned about the wrong pipeline (2026-08-26)

Verdict **FAIL**. Three tasks marked done whose defining verbs did not execute, and one genuine
reasoning error that no amount of measurement would have caught.

### The arithmetic described a call pattern my own code does not have

R27 measured correctly — 3.74 s cold, 0.18 s warm, ~50 s window, `keep_alive` honoured — and then
concluded *"under the default every single sample pays the cold load… a 15-minute segment of 5
samples pays it five times."* **False for this pipeline.** T031 batches: every text call for a
segment fires back-to-back inside one `summarize_segment_via_ladder` pass at segment close, ~0.2 s
apart, far inside the 50 s window. A segment pays the cold load **once** — about 0.4%, not 2%, and
not five times.

Worse, `Resident` sized its window from the **sample interval** (180 s) when the gap the model must
actually survive is the gap between **segments** (900 s). So `Resident` expired long before the next
burst: it held 7.4 GB *and* paid every cold load, while reporting itself as residency. T029 and
T031, both marked done, were mutually inconsistent as shipped.

**The lesson is not "measure" — I did measure.** It is that a measurement is interpreted against a
model of the system, and mine was of the pipeline I had planned rather than the one I had built two
commits earlier. **When a number is used to justify a policy, state the call pattern it assumes and
check that against the code, not against the design.**

### The orphan defect, again — in the commit that added the guard against it

`crop_regions` was written, documented, unit-tested… and called by nothing. The text tier was fed
whole frames, committing precisely the failure the function's own doc rails against. Same for
`Residency::keep_alive` (no channel to send it) and `SegmentLatency` (never constructed).

**Fifth occurrence, and the sharpest, because the same commit added `the_ladder_stays_reachable_…`
as an anti-orphan guard.** That test proved the ROUTER was reachable; nothing proved the CROPS
were. A guard written for one seam gives no coverage of the next one, and having written it made
the whole class feel handled.

The wiring also needed a design decision I had skipped: the region cascade runs at CAPTURE time
while summarisation happens at segment close, so regions must be **persisted beside the sample**.
Re-detecting at summarisation would describe a different moment than the pixels do.

### A clamp that manufactured evidence

`x.min(fw - 1)` on a region entirely off-frame dragged the box onto the edge and produced a **1×1
crop of a corner pixel** — content belonging to no region, fed to OCR under that region's name,
burning a budgeted call. And my test **enshrined it**: "clamps to a 1px sliver, not an error". The
`w == 0` skip branch it sat beside was unreachable, so its warning described a case that could
never occur.

Clamping and intersecting are not the same operation. **A clamp moves a box that does not belong
here until it does; an intersection reports that it does not belong.** Fixed to a real intersection,
with the non-intersecting region skipped.

Also found: `Region`'s own docs contradicted themselves — `bbox` was labelled "screen-absolute" on
one line and "display-local" fifteen lines below. Consumers were reading whichever half they saw
first. Corrected in place rather than left for the next reader to pick between, and `crop_regions`
now skips regions belonging to another display, which would otherwise index straight into this
frame's pixels as if they were their own.

### A test that was theatre in both directions

`the_demoted_tesseract_path_is_never_the_text_tier` grepped the source for a word. The reviewer
executed both failure modes: it **fails** on a comment explaining why not to use tesseract, and
**passes** when the real wrapper is reintroduced under a name not containing the word. Deleted.
The property is already held behaviourally — every test injects its own provider, so a local OCR
call could not satisfy the assertions — and a module-boundary rule is a lint, not a test.

### Also

`DynamicImage::crop` takes `&mut self`, so cropping cloned the whole decoded frame per region —
~33 MB at 4K × 8 regions × every sample. `crop_imm` borrows. And `SegmentLatency`'s hardcoded
500 ms cold-load threshold was an absolute valid only for one model on one machine; it now reports
mean-time-per-call, which compares without a magic constant.


## R29 — Wave 7 re-review: the orphan class moves one seam up each time (2026-08-26)

**PASS WITH NOTES.** All eight prior findings held, and the four surviving mutants were gaps
*around* the new code rather than defects *in* it — which turns out to be the pattern worth
recording.

### "Nothing calls it" became "the caller neuters the parameter"

`crop_regions` is now called, and guarded by a test that drives sidecar → crops → text tier. But
that test calls the ladder **directly**, with a literal `max_regions`. So
`RoutedChunkSummarizer` — the type the pipeline actually uses — could pass **0** and crops would
never happen on the production path, with all 453 tests green.

That is the sixth appearance of one class, and it has moved every time:

| # | shape |
|---|---|
| 1–5 | the function exists, is correct, is tested, and **nothing calls it** |
| 6 | the function is called, and **the caller passes a value that disables it** |

A guard proves a specific seam. It says nothing about the seam above it, and writing one makes the
whole class *feel* handled — which is precisely why the next one goes unnoticed. **The test must
run through the type the production path uses, not through the function the unit test finds
convenient.**

Two smaller instances of the same thing: `latencies.push` could be deleted with all tests green
(FR-013 recorded to a Vec nothing read), and `with_max_regions`/`latencies()` had no callers at all.

### The fail-open fixtures were each missing a different half — again

The sidecar tests covered **missing**, **corrupt** and **empty-list**. Not covered: regions
*present* and *every one skipped* — which is exactly the stale-region story the docs tell (a resize,
or boxes belonging to another display). Dropping the sample there violates R13's never-drop rule,
and the mutation survived. R26's axis-list lesson applied to a principle rather than a metric:
**fail-open needs a fixture per way the input can be unusable**, not one representative.

### A budget large enough to livelock

Found by derivation, not by a test: the burst is `samples × regions + 1`, spent back-to-back. A
config the validator **accepts** (30-minute segment, 60 s interval, default 12 regions) needs 361
calls against a budget of 240. The segment is refused mid-burst, requeued by retry-never-drop, and
retries forever — **spending the entire budget on every attempt, starving every other segment, and
never completing.** Silently.

That is worse than an error in the specific way this project keeps rediscovering: it produces no
failure anyone can see. Now refused up front, with a message naming the arithmetic and the three
knobs that fix it.

### A mean that cannot see what it was built to detect

`mean_call()` replaced a hardcoded 500 ms threshold — correctly, since an absolute is only valid
for one model on one machine — but at realistic call counts it is useless: with the default 12
regions a 5-sample segment is 61 calls, so a 3.74 s cold load moves a 180 ms mean to 241 ms, well
inside the measured warm spread. My own unit fixture manufactured separability by implying ~48 ms
warm calls, a rate R27's measurement says does not exist. **The fixture proved the metric by
assuming a system that isn't this one** — R26's failure mode, on a metric this time.

The cold load is deterministically the FIRST call of a burst, so recording `first_call` separately
answers the question with no threshold at all, on any model or machine.

### An owed producer, written down rather than left implied

Nothing writes `.regions.json` yet — the capture loop owes it — and because the path fails open,
its absence is **permanently invisible**: whole-frame reads, every test green, T027's entire benefit
silently absent. Recorded as an explicit deliverable on the task, and `samples_read_whole` now
counts the degradation so it is measurable rather than inferred. Same for
`ResidencyPolicy::keep_alive`, which is computed correctly but has no channel to be sent:
`analyze_image(path, prompt)` cannot carry it, so `Resident` remains a policy the running system
cannot express. Both say so on the task instead of reading as done.


## R30 — Wave 8 (US4): identity is where a thing IS, not where it sits in a vector (2026-08-26)

The structural timeline. Two design points and two fixture lessons.

### An index is not an identity

`Region::parent` is an index into the slice ONE capture produced. Persisting it would have written
provenance that looks precise and points at the wrong pane: the next capture builds a different
vector and index 3 means something else. So identity is derived from `(display_id, bbox)` — where
the region actually is — which is deterministic AND makes the SAME pane in two captures carry the
SAME id. That is what makes "this entry came from the editor pane" answerable across a day rather
than only within one frame, and `parent` is resolved from index to identity at the boundary.

### Reading order is geometry, never a model

A model asked to order regions answers differently on the same input from one run to the next, and
nothing can distinguish a re-ordering from a re-render. Pixels already say where things are.

Sorting by `y` alone is not enough: two columns whose tops differ by three pixels interleave line
by line — the same corruption cropping prevents, arriving one layer later as a timeline that reads
across two documents. Regions are banded first (a new band starts when a region no longer overlaps
the current band vertically), then ordered left-to-right within a band, with displays never
interleaved.

### A fixture that passed by coincidence

The mutation `region_id = vector index` **survived**. My cross-capture identity test put the editor
pane at index 1 in *both* fixtures, so index-as-identity gave the same answer as geometry-as-identity
— the test asserted equality and got it for the wrong reason.

**A test for "X is not derived from Y" must make Y differ.** Obvious once stated; it is the same
family as R22's symmetric scenario, and the tell is the same: the fixture treats both sides alike
where the property under test is about them differing.

### The guard nothing could reach through the API

The mutation "fill missing provenance columns with zeros" also survived, because the writer always
writes all-or-nothing, so no test could produce a half-written row. But such a row can still
appear — a partial migration, a hand-edit, an older writer — and filling the gaps would describe a
region at (0,0) sized 0×0 that was never on screen, indistinguishable from a measured one.

**A defensive branch that the public API cannot reach still needs a test; reach past the API to
write one.** Raw SQL produced the row and pinned the guard.

### Also

A missing comma in a multi-line SQL string turned `summary, region_id` into `summary AS region_id`
— eight columns returned where sixteen were read, caught immediately by an existing round-trip test.
Worth noting only because concatenated SQL strings make this failure invisible on inspection: the
line reads correctly, and the bug lives in the whitespace between two lines.


## R31 — Wave 8 review: the band ended at its tallest member (2026-08-26)

Verdict **FAIL**, on the one thing I had flagged as untested and shipped anyway.

### A tall region absorbed every row beneath it

`band_bottom` grew by `max`, so a band extended to its TALLEST member. Any full-height region — a
sidebar, a file tree, or the window that contains everything — therefore swallowed every
subsequent row into one band, which then sorted column-major: **down the left column, then down
the right.** Measured on a window plus a 2×2 pane grid, and on a sidebar beside two stacked rows.

Two things make this worse than an edge case. It fires on the **ordinary** shape of a capture:
`assign_parents` produces exactly a container plus its panes, and `provenance_in_reading_order`
feeds that whole set in. And it is a **spec deviation I did not notice writing the code** — T034
says "banded top-to-bottom then left-to-right, **bounded by the parent tree**", and my
implementation never read `parent` at all.

The fix is two changes, and the first is one word: a band ends where its **shortest** member ends,
not its tallest. Plus real parent-awareness — siblings ordered among themselves, each region
followed immediately by its own children — without which two windows side by side interleave their
panes into an order describing no screen anyone saw.

**I predicted this failure in the review request and shipped the code anyway.** Writing "I did not
test nesting at all" and marking the task done are not compatible; the honest move was to write the
fixture before the checkbox.

### Every fixture avoided the shape that breaks it

All shipped layouts were full-height columns or clean disjoint rows: no nesting, no mixed heights
in a band, no exact abutment. **Same fixture-family blindness as R26/R29, third occurrence.** The
axes for a layout are now written down — one display or several · side-by-side, stacked, or nested
· overlapping or disjoint · equal or wildly different heights · touching exactly or separated — and
each new fixture should say which cell it occupies.

### `DefaultHasher` is not an identity you can write to disk

Std explicitly does not guarantee its algorithm across releases. That is fine for a `HashMap` and
fatal for an id **persisted to SQLite**: a toolchain upgrade would silently rebind every stored
`region_id`, so pre-upgrade rows stop matching post-upgrade captures — breaking exactly the
cross-day query US4 exists for, with no error anywhere. Replaced with FNV-1a written out in full,
and the value **pinned in a test**, because a stability guarantee nothing checks is a comment.

Identity also hashed geometry alone, so a maximized pane and the window containing it — a routine
layout — collided onto one id, making `parent_region_id` potentially equal to a different region's
`region_id` by construction. `granularity` is now in the key.

### A 50/50 fixture proves nothing half the time

The mutation "read the id back with `unsigned_abs()`" — wrong for every id ≥ 2^63, i.e. half of
them — **survived**. The one fixture whose identity had the sign bit set never asserted `region_id`,
and the fixture that did assert it happened to have the bit clear. **Eighth instance of "the fixture
does not produce the condition the assertion names",** and the sharpest, because the condition here
is a coin flip: an arbitrary box has a 50% chance of proving nothing. The new fixture is chosen for
its id, and asserts the sign bit is set before testing the round-trip.

### Two smaller things

A doc comment ended up separated from its function: my insertion landed between `locate`'s `///`
block and `pub fn locate`, so rustdoc showed `reading_order` opening with "Resolve a
natural-language target…" and `locate` had no docs at all. **Inserting before a `pub fn` is not the
same as inserting before its documentation**, and the compiler is happy either way.

And `the_migration_is_re_runnable` built a **fresh** database each iteration, so no `ALTER` ever hit
"duplicate column name" — it never tested the property in its name. Renamed to what it does; the
real coverage was already one layer down in `database.rs`.


## R32 — Wave 8, third pass: a running band edge was the wrong model, not the wrong threshold (2026-08-26)

**PASS WITH NOTES**, and the note was the interesting part: my "fix" had not removed the failure,
it had **moved** it.

### Both directions fail, which is the diagnosis

- Ending a band at its **tallest** member: a full-height sidebar or window absorbs every row beside
  it → reads down the columns.
- Ending it at its **shortest**: a 10-pixel chip pins the edge at y=10, so a left column overlapping
  the right one for 480 of its 488 pixels is exiled to a later band → reads right before left.

Two opposite failures from the same mechanism means **the mechanism is wrong**, and no third
threshold rescues it. A running edge accumulates state as regions arrive, so one member's height
distorts the answer for everything after it.

The replacement asks a question about the whole set instead: **is there a line no region crosses?**
Horizontal first (rows), else vertical (columns), recursively. That is stateless with respect to
arrival, so no member's size can move a cut that another member spans.

Two details matter:
- The cut is where nothing **crosses**, not where coverage has a hole. Rows that abut exactly still
  separate, because a region ending at `y` and one starting at `y` both fail to cross it.
- Split at the **first** cut only, then recurse. Cutting at every gap in one pass mixes the axes: a
  sidebar beside a 2×2 grid has an x-gap left of the content AND one between its columns, so a
  single pass yields `sidebar | left-column | right-column`. Taking one cut and recursing lets the
  content be split into ROWS first, which is what a person does.

### A surviving mutant meant the semantics were never pinned

`reach = last member` instead of `max` survived the entire suite — twice, before and after the
rewrite. It takes **three** regions to distinguish them: a tall column, a short region inside its
span, and a third below the short one's bottom but still inside the tall one's. With two, both
rules agree.

**A rule that accumulates needs a fixture long enough for the accumulation to diverge.** Same shape
as R25's five-sample scroll and R22's partial outage: the minimum case is the one where every
candidate implementation agrees.

### Recursion on deserialized data is an abort waiting to happen

`Region` is `Deserialize` and `reading_order` is `pub`, so a corrupt or adversarial region set can
carry an arbitrarily deep parent chain. Recursion made that a **stack overflow — an abort, not an
`Err`**: no catch, no log, the daemon simply dies. Measured at 8,000 deep. Rewritten with an
explicit stack; the fixture now runs 50,000 deep and passes.

**Depth that comes from data, not from code, belongs on the heap.** Realistic captures nest four
levels; that is exactly why nobody would have found this without asking what the input CAN be
rather than what it usually is.

### And a comment that claimed a path that cannot happen

The `placed` guard inside the emit step was documented as handling parent cycles. It does not:
every region has at most one parent, so it lands in exactly one group and is reached at most once —
a mutation replacing that branch with `panic!` passes the whole suite. The cycle handling is
elsewhere, in the fallback that sweeps unplaced indices. The guard is kept as defence, with a
comment that says what is actually true.


## R33 — Wave 9 (US5): reclaiming storage may never destroy the only record (2026-08-26)

Retention is the only part of dayflow that deletes, so its whole design is one rule: **eviction is
gated on `summarized`, never on age or on being over budget.** An unsummarised window's raw samples
are the only evidence that period existed; dropping them turns a backend outage into permanent loss
that nothing downstream can tell apart from a genuinely idle hour.

The dangerous shape is the disk emergency — exactly when a "just free something" path gets added.
So the test for it asserts that with every window unsummarised and a one-byte budget, the correct
outcome is to free **nothing** and stay over budget. The fix for that state is to stop capturing,
which is a decision for a layer that can tell the user.

### Two orderings that look equivalent and are not

**Tier before age.** Sweeping both tiers in one oldest-first pass reads naturally and is wrong: the
oldest window is usually already warm, so a single pass drops the timelapse a person actually reads
back while a hundred megabytes of superseded raw frames sit untouched. All reclaimable raw first,
then warm. Caught by my own test during construction.

**Encode before delete.** `shrink` verifies the replacement — including the SC-008 size ceiling —
*before* removing a single raw sample. Delete-then-encode is the ordering that spends the only copy
to save nothing when ffmpeg fails, and a failed encode must cost exactly zero.

### A test that passed because the KERNEL refused

`deleting_outside_the_capture_directory_is_refused` pointed at `/etc/passwd`, which fails with
permission-denied whether or not the validator runs. Removing path validation entirely **survived**
the test: it was proving the OS's refusal, not ours. Now the file outside is one this process owns
and could trivially delete, so only validation can stop it.

**Ninth instance of "the fixture does not produce the condition the assertion names",** and a
distinct sub-species worth naming: *the right outcome arriving through the wrong mechanism*. It is
especially easy in security tests, where the environment often refuses on your behalf — a test that
the sandbox would pass with the check deleted is testing the sandbox.

### The tier is read from disk, not inferred

A window whose raw samples are gone is warm however recent it is; one never shrunk is hot however
old. Deriving the tier from age would report a state the filesystem does not have — and retention
acts on that state by deleting.

### And the planner reports on everything it did NOT do

`plan` returns a decision for every segment, including the untouched ones and the reason. A planner
returning only its actions makes "nothing was reclaimed" and "everything was refused"
indistinguishable — which are precisely the two answers an operator needs to tell apart when the
disk fills up anyway.


## R34 — Wave 9 review: I broke my own identity rule one wave after writing it (2026-08-26)

Verdict **FAIL**, and the headline finding is a rule I had written down in R30 and then violated in
the next module I touched.

### The identity was documented, and I keyed on half of it

`SegmentRecord`'s own field doc says the durable identity is `(session, display, sequence) — never
the filename`, `ChunkRef` says the same, and `daemon.rs` keeps a **per-display** sequence counter.
`plan` keyed its bookkeeping on `sequence` alone.

On any multi-monitor machine display 0 and display 1 both emit sequences 0, 1, 2… so the two
collapse: **one window per colliding sequence is never actioned**, its bytes are credited to the
budget anyway (so the planner believes it freed space it did not), and the ledger reports it as
merely "too recent". Three failures from one missing field, and the third is the exact lie the
return-a-decision-for-everything design exists to prevent.

**Writing the rule down did not stop me applying it.** R30 was about `Region` identity; this is
`SegmentRecord` identity, in a different module, a week later — and the shape is identical.
Documenting an invariant on the type does not enforce it at the use site, and a composite key is
the kind of thing that reads fine when half of it is missing.

### Two whole policies with no test at all

Deleting the age-based **warm expiry** branch entirely survived the suite: every warm-drop assertion
went through the BUDGET path, so the whole "fortnight of timelapses" policy was unverified.

And removing pass 1's contribution to `freed` also survived: every test used a budget of `u64::MAX`
or ~1 byte, so **"age reclaim alone satisfies the budget"** — the interaction between the two passes
— was never exercised. The mutant over-evicts a young window it did not need.

**Both gaps come from fixtures clustered at the extremes.** A budget of `MAX` or `1` never lands in
the region where the logic is interesting, which is the middle. Same family as R32's two-element
fixtures: the values that make a test easy to write are the values that make it prove least.

### An accounting credit that was never real

A shrink credited the full `raw_bytes` while writing a timelapse of up to 10% of that onto the same
disk. The plan could therefore declare the budget met while real usage landed over it — correcting
itself only on the next run, and then by dropping warm artifacts that should never have been needed.
Credited net now.

### Structure that let a caller lie

`shrink` deleted the raw samples and left the caller to clear `raw`/`raw_bytes` by hand — as my own
end-to-end test did, in four lines. A caller that forgot would leave a record whose `tier()` claims
Hot while the files are gone, and the next plan would schedule a shrink whose encode reads deleted
files. It takes `&mut` and maintains the record itself now.

**A function that requires the caller to fix up state afterwards is a function with an undocumented
second half.** The `&` signature made the hazard invisible; `&mut` makes the ownership obvious.

Related, from the same review: a crash between writing the timelapse and clearing the samples
leaves BOTH set. `tier()` reads Hot (the raw frames are the better record), and shrink now deletes
the stale timelapse before writing its replacement — otherwise nothing points at it and retention,
whose whole job is freeing space, can never reclaim it. The SC-008 refusal path also now removes the
oversized file the encode already wrote.

### And a doc that promised a floor the code does not keep

`RetentionConfig.hot` said raw samples are "kept this long before they may be shrunk". Budget
pressure shrinks a summarised window of any age. The only real floor is `summarized`, and the doc
now says so.


## R35 — Wave 9 re-review: the fix introduced the bug it was written to prevent (2026-08-26)

**PASS WITH NOTES**, and the note was a defect my own previous fix had created.

### `take()` before a fallible call

R34 made `shrink` take `&mut` and reclaim the stale timelapse from a crashed earlier attempt —
precisely so nothing would be orphaned. The implementation did `warm_artifact.take()` and then
`reclaim_file(&stale)?`. On a delete failure that returns early with the record's ONLY pointer to
the stale file already erased: **the exact orphan the change was written to prevent**, plus the raw
samples already gone while the record still listed them, so `tier()` claimed Hot for a window whose
files did not exist and every retry handed the encoder deleted paths.

The order was the whole bug. Now: point the record at the new timelapse **first**, reclaim the
stale one best-effort (a failure logs and continues — one transient error must not strand a window
forever), then delete the raw samples, `retain`-ing the ones that could not be removed so the record
matches the disk in both directions, with the surviving bytes **measured** rather than assumed.

**Generalisable: `Option::take()` before a `?` is a lost pointer waiting to happen.** More broadly,
when a function mutates a record and touches the disk, order the mutation so that every early return
leaves a state the system can still act on. Here that state is "raw and warm both present", which
`tier()` already reads as Hot — recoverable, and a retry finishes the job.

### Two fixes that were correct and undefended

The warm tier's oldest-first sweep and the net-of-timelapse credit were both right, and both
survived mutation: every warm fixture had a single warm segment, and no fixture put the budget
inside the 10% gross/net band. Correct code with no test defending it is one refactor from being
wrong code.

And my first attempt at the net-credit test **still** did not discriminate: I evaluated at a time
where both windows were already age-expired, so pass 1 took them unconditionally and the budget
arithmetic under test never ran. **A test for pass-2 logic has to be built at a time pass 1 does not
handle** — otherwise it exercises the wrong pass and passes for the wrong reason. Tenth instance of
the fixture not producing the condition the assertion names.

### A refusal reason the planner cannot produce

`Refusal::TooRecent` turned out to be unreachable: pass 2 exhausts every summarised Hot/Warm segment
before it can end over budget, so an unplanned summarised segment exists only because the budget was
met. Age is never the binding constraint — pass 2 ignores it — so labelling a window "too recent"
would name something that was not protecting it. The variant is deleted rather than left advertising
a state that cannot occur.


## R36 — Wave 10 (US6): parity is a property of having one state, not an agreement (2026-08-26)

Three surfaces — MCP, CLI, HTTP — over one `DayflowService`. The design choice worth recording is
that T044's parity requirement is met **structurally**: there is one state and one set of
transitions, and each surface only translates between its wire format and those calls.

The alternative — three implementations that agree by convention — fails the moment one is changed
alone, and it fails in the worst possible way for a user: *"the CLI says running and the dashboard
says stopped"* is a contradiction they have no way to resolve. Three passing isolated tests are
exactly what that looks like from the inside, which is why the parity tests drive the surfaces
**against each other** rather than each on its own.

Two shared decisions had to move out of the surfaces to make that true: the "today so far" default
for a missing range (a default that differed per surface makes the same question return different
answers depending on how it was asked), and the degraded-is-still-success rule.

### A degraded session is a success on every surface

Running-but-not-producing returns 200 on HTTP, exit 0 on the CLI, and a successful tool result on
MCP — with the degradation in the payload. A 503 makes every monitor treat a recoverable state as
an outage; a non-zero exit makes every script treat it as a crash. The state is already in the body
for anything that wants to act on it.

`is_degraded` delegates to `DayflowHealth::is_fault`, which already draws the distinction that
matters: **a pause and an off switch are quiet on purpose**. Re-deriving "unhealthy" as "not
Healthy" would have made every lunch break look like a broken recorder — and the mutation doing
exactly that was killed by the pause test.

### A behaviour no test can reach is a behaviour nothing defends

The 503 mutation **survived** at first, because a session is only degraded once it has been quiet
longer than its interval, and `route` read the clock itself — so no test could drive it into that
state at all. Adding `route_at(.., now)` made the case reachable, and the mutation died immediately.

**When a rule is time-dependent, the clock has to be a parameter or the rule is undefended by
construction.** This is the same shape as the fixtures clustered at the extremes (R34): the state
the test can easily produce is not the state where the logic lives.

### `+` is a space, and a timestamp is not a form field

`from=2026-08-26T00:00:00+00:00` arrives as `... 00:00` — correct form-decoding, and a genuine trap
for an API whose main parameters are timestamps. It is refused loudly with the mangled value quoted,
rather than parsed leniently into some other instant, because a lenient parse would answer
confidently about the wrong hour. Documented at `percent_decode`, and both forms are tested.

### And a catalog entry with no dispatch arm

The tool-list test now asserts that every advertised tool has a dispatch arm, not just that the
count matches. A tool the server answers but never advertises is undiscoverable; one it advertises
but cannot dispatch returns "unknown tool" to a client that did exactly what the catalog told it to.
Both are silent until someone tries.


## R37 — Wave 10 review: I wrote that the duplication had moved out, while it sat in the tree (2026-08-26)

Verdict **FAIL**, on two things: a crash reachable from one request line, and a parity claim proven
for one surface out of the three it names.

### A str slice on bytes that are not a character boundary

`&s[i + 1..i + 3]` in the percent decoder panics when those two bytes fall inside a multi-byte
character. A request line is any valid UTF-8, so `GET /dayflow/ask?question=%aé` **killed the
serving thread** — no `catch_unwind`, no per-connection thread, so every later request went
unanswered with nothing logged to say why.

And the test named `a_malformed_request_is_refused_without_stopping_the_surface` could not have
caught it: its fixtures were a bad path, a bad method and a bad date, and it drove `route()`
in-process — never the socket loop where "stopping the surface" is even possible. **Eleventh
instance of a fixture not producing the condition the assertion names**, and the name was the most
specific promise of the lot.

Fixed with a byte slice, plus a read timeout (a client that connects and says nothing blocked the
single-threaded loop **forever**) and a panic guard so a future crash degrades one request instead
of the surface.

### The duplication I claimed to have removed was in the tree as I wrote the claim

R36 says the range default "moved out of the surfaces". It did not: `parse_range`,
`parse_cli_range` and `range_from_query` were three verbatim copies. Two of them had no test —
mutating the MCP default and the CLI default to produce an **empty** range, so every question would
be answered about nothing, survived the whole suite.

There is one implementation now and the surfaces call it. The lesson is not "don't duplicate": it is
that **a design claim in a commit message is not evidence, and I wrote one that the diff
contradicted.** The check is mechanical — grep for the second copy before claiming there isn't one.

### Parity executed for one surface, narrated for the other two

Nothing invoked the MCP tool methods or the CLI subcommand. The parity tests drove HTTP against
direct service calls and labelled those "the call the MCP tool and the CLI subcommand both make" —
which tests the service, not the adapters. Replacing `tool_dayflow_status`'s body with a **constant**
survived the entire suite, producing exactly the "the CLI says running and the dashboard says
stopped" contradiction the design argues against.

**An adapter is code. Testing what it adapts to is not testing it.**

### And two things scoped honestly instead of quietly

The CLI's `start`/`stop`/`status` cannot share a session across processes — each invocation builds
its own in-memory service, so `dayflow start` prints an id and the session dies with the process.
`timeline` and `ask` do share state, through SQLite. That is now in `tasks.md` and the help text
rather than implied by a checkbox.

`ask_day` is advertised as answering questions and its answerer is a stub on every surface: only the
refusal path works as described. The tool description says so. A catalog entry that promises more
than the code does is a lie told to every client that reads it — and the stub strings even differed
per surface, which is a parity violation in the wave about parity.
