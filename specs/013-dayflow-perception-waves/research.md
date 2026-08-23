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

**UNVERIFIED**: that `-force_key_frames` with an `expr:` argument behaves as expected against
a `rawvideo` stdin input at fractional fps (0.2–0.5). **Check owed**: a short live capture at
a 10-second interval asserting segment durations and that the manifest gains one line per
boundary. This is the first thing the implementation must prove, before anything is built on
top of it.

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

**UNVERIFIED**: that the screensaver extension is present and reports a usable idle counter on
this host's X server, and how it behaves under a compositor. **Check owed**: a one-shot probe
printing idle-ms and saver state, run locked and unlocked, before the pause logic is written.
Platform note: this is an X11 path. Wayland and macOS need their own probe; the detector must
be behind a small trait so the absence of a backend degrades to "never idle" (record
continuously) rather than to a crash or a permanent pause.

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

**UNVERIFIED**: whether `scrap` on this host can hold concurrent capturers on multiple
displays without contention. **Check owed**: enumerate displays and open two capturers before
building the multi-pipeline supervisor. If concurrent capture is not supported, the fallback is
round-robin capture across displays within one interval, which is a scheduling change, not a
redesign.

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

**UNVERIFIED**: the lane's actual idle-unload window on this host. **Check owed**: read it from
the lane's own status before choosing the ping interval — do not guess it.

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
