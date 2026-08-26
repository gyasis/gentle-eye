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
