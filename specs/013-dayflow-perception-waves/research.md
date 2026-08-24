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
