# Tasks: Dayflow — Continuous Screen-Activity Timeline

**Feature**: `013-dayflow-perception-waves` | **Date**: 2026-08-23
**Input**: [spec.md](./spec.md) · [plan.md](./plan.md) · [research.md](./research.md) · [data-model.md](./data-model.md) · [contracts/](./contracts/)
**Supersedes**: `specs/002-dayflow-mode/tasks.md` (left on disk untouched as the audit trail)

## Format contract — read before editing this file

This file is executed by **dev-kid**, whose parser was read directly from
`~/.dev-kid/cli/orchestrator.py` and `~/.dev-kid/cli/task_matching.py`. Violating any of these
breaks the run **silently** — no error, just the wrong work:

| rule | consequence if broken |
|---|---|
| A section header is a wave only via `#{1,6} (Wave\|Phase\|Step\|Stage) <id>` + optional `: title` | any *other* heading using those words becomes a **phantom wave** and reorders everything |
| Wave order is **positional**, not numeric | a block inserted at the end runs **last**, whatever its number |
| Task line is `- [ ] ` then an id matching `^T\d+\b` — **digits only** | `T30A` parses as `<NO-ID>` |
| `[S]` anywhere in the first line marks a sentinel checkpoint | a missing `[S]` drops the checkpoint |
| Prose containing `- [ ]` becomes a **phantom task** | never use a checkbox outside a real task |

Legacy handles from the superseded plan are cross-referenced in each description as
`(was TNNN)` so the audit trail survives renumbering.

## Carried forward — already delivered, do NOT rebuild

Verified on `main` 2026-08-23: ~581 lines, 7 passing tests. Listed as a table, deliberately
**not** as checkboxes, so the parser cannot mistake them for tasks in this run.

| legacy | delivered | evidence |
|---|---|---|
| T200–T203 | module tree, `models.rs`, `errors.rs`, `DayflowConfig` | config round-trips the loader |
| T210 | duration-aware capture-rate heuristic | unit test + `docs/FPS_AND_DAYFLOW.md` |
| T221–T222 | chunk manifest, memory-pressure path | 3-chunk boundary test, monotonic ranges |
| T230–T233 | map-reduce summarizer + rolling context + digest | 4 tests incl. stub-provider end-to-end |
| T240–T241 | `timeline_entries` table, `TimelineStore` round-trip | in-memory insert → ordered `query_range` |

13 of the original 39 tasks. Everything below is the remaining 26, re-scoped.

---

## Phase 1: Setup

- [x] T001 Mirror this file to `.dk/tasks.md` and verify the parse with dev-kid's own regexes — assert wave count and order, zero `<NO-ID>` tasks, zero phantom waves. `.dk/tasks.md` currently holds an unrelated preview-pane plan from 2026-05-30; dev-kid lite reads ONLY `.dk/tasks.md`, so an unmirrored run silently executes the wrong feature.
      `> DONE:` `.dk/tasks.md` matches this file; a parse dump prints the expected wave→task mapping with 0 missing ids.
- [x] T002 [P] Enable the `screensaver` feature on the pinned `x11rb` dependency in `Cargo.toml`, with a comment stating it backs Dayflow idle detection (research R2). No new crates.
      `> DONE:` `cargo check` green; the feature addition carries its justifying comment.
- [x] T003 [P] Extend `DayflowConfig` in `src/config/mod.rs` — `segment_seconds` (replacing the assumption that `chunk_minutes` is uniform), `displays`, `idle` (threshold + hysteresis + enable), `perception` (text tier, reason tier, residency policy), `max_regions_per_segment`, `disk_budget_bytes`. Serde defaults for every field.
      `> DONE:` `DayflowConfig::default()` round-trips through the config loader; existing config files still parse.
- [x] T004 [S] Prove the ffmpeg segment muxer at fractional fps in `tests/dayflow_segmentation.rs` — the schedule-critical UNVERIFIED item from research R1. A short live capture at a ~10-second interval must produce sequential files with the expected durations and one manifest line per boundary.
      `> DONE:` the probe passes and the exact working argument vector is recorded in `research.md`; if `-force_key_frames expr:` does not hold at 0.2–0.5 fps, STOP and record the fallback before building on it.
- [x] T005 [P] Prove the idle signal — a probe reading X11 screensaver idle-ms and blank state, run both locked and unlocked, confirming the two are distinguishable (research R2).
      `> DONE:` idle counter verified monotonic (+2000ms/2s). LOCK DETECTION DESCOPED 2026-08-23 (user: "we don't really lock the screen anyway") — idle threshold is the primary and sufficient trigger. The X saver `state` field is unusable here regardless (reports 3); if lock is ever wanted, use org.gnome.ScreenSaver D-Bus or logind LockedHint, never the X field. No-backend fallback stays "never idle", never a permanent pause.
- [x] T006 [P] Prove concurrent multi-display capture and read the governed lane's real idle-unload window (research R3, R5).
      `> DONE:` two concurrent capturers open without contention, or the round-robin fallback is recorded; the measured unload window is written into `research.md` rather than guessed.

## Phase 2: Foundational

Blocking prerequisites. Nothing in any user story is testable against real data until this
completes — every downstream slice consumes segment files.

- [x] T007 Additive, idempotent migration in `src/storage/database.rs` — `dayflow_segments` and `dayflow_pauses` tables plus the range and eviction indexes per `data-model.md`. Must not rewrite the shipped `T240` migration.
      `> DONE:` migration is re-runnable; an in-memory test inserts and selects from both new tables and confirms pre-existing `timeline_entries` rows are untouched.
- [x] T008 [P] Extend `ChunkRef` in `src/dayflow/models.rs` with `display_id`, `sequence` (monotonic within the session, across encoder restarts) and `summarized`. Identity becomes `(session_id, display_id, sequence)` — never ffmpeg's per-run `index`.
      `> DONE:` types compile; a test asserts `sequence` keeps increasing across a simulated encoder restart while `index` resets.
- [x] T009 [P] Add `display_id` to `regions::Region` in `src/regions/mod.rs` and thread it through `detect`/`fuse`/`assign_parents`.
      `> DONE:` two regions with identical bboxes on different displays are distinguishable; existing region tests stay green.
- [x] T010 [S] Frame SAMPLER in `src/dayflow/sampler.rs` (supersedes T220's segment muxer per D9) — grab one frame per display at `sampling.interval_for(mode)`, persist it as a compressed still, and record it in the segment ledger. NO long-lived encoder: dayflow samples, it does not stream video. Delta-skip (D11) marks a sample unchanged rather than perceiving it.
      `> DONE:` a run at a short interval yields the expected number of stills per window with correct wall-clock stamps; an unchanged screen marks samples skipped and performs no perception; no ffmpeg process outlives a single assembly call; `cargo check` green.
- [x] T011 [S] Sample-window controller in `src/dayflow/window.rs` (RELOCATED from `src/capture/service.rs` 2026-08-24, user-confirmed: that file is gentle-eye's GENERAL recording service, shared with real-time video recording. Dayflow's window rules — a 5-minute floor, an intent, pause-on-idle, mid-day interval changes — are properties of THIS feature, and growing them inside the shared service would make every recording carry dayflow's constraints. The controller uses capture rather than living inside it.) — closes a window on the segment boundary, records its ACTUAL `start_wall`/`end_wall` and sample count in the ledger, and handles interval change, pause, resume and display change. Optional timelapse assembly (`video.enabled`, default off) invokes ffmpeg ONCE per window over the collected stills, using the argument vector proved in T004.
      `> DONE:` a simulated run yields non-overlapping windows with real timestamps and unbroken `sequence`; with video disabled NO video artifact is produced and the timeline is unaffected; with it enabled one timelapse per window appears; `cargo check` + tests green.

## Phase 3: User Story 1 — Unattended all-day capture I can trust (P1)

**Goal**: record every display all day, pause on idle/lock, resume on activity, survive manual
off/on, and report liveness a caller can trust.

**Independent test**: run with a short interval; confirm sequential segments per display with
correct ranges; confirm status distinguishes healthy / paused / off / degraded.

- [x] T012 [US1] Per-display capture pipelines, coordinated by `src/dayflow/engine.rs` (RELOCATED from `src/capture/service.rs`, same reasoning as T011 — the shared recording service must not carry dayflow's rules), one encoder per display via the existing `DisplayManager`, all writing into one session (FR-029).
      `> DONE:` a two-display run produces two segment series under one session id; `displays_active` reports 2.
- [x] T013 [P] [US1] New `src/dayflow/idle.rs` — an `IdleDetector` trait plus the X11 screensaver backend from T005, with hysteresis on both transitions. A platform with no backend reports "never idle".
      `> DONE:` unit tests cover idle→active→idle with hysteresis and the no-backend fallback; no `unwrap()` outside tests.
- [x] T014 [US1] Wire pause/resume into the window controller — SCOPE CORRECTED: T011 already BUILT pause/resume in `dayflow/window.rs` (mutation-tested), so this task is the WIRING of the idle detector to it via `engine.rs::tick_idle`, not construction of the mechanism — pause closes the in-progress segment cleanly and records a `PauseInterval` with its cause; resume opens a new segment (FR-030/031/032).
      `> DONE:` a simulated lock mid-segment yields a closed segment plus a recorded pause, and no truncated file; resume starts a new segment rather than splicing across the gap.
- [x] T015 [US1] Manual off/on in `src/dayflow/engine.rs` — turning capture on again the same day rejoins that day's session by `day` rather than opening a second timeline (FR-033).
      `> DONE:` off→on within a day returns the original session id and the off interval is recorded as a gap; a new day starts a new session.
- [x] T016 [US1] Apply a segment-interval change at the next boundary — SCOPE CORRECTED: `window.rs::set_interval` was built and mutation-tested in T011; this is its wiring through `engine.rs::set_interval` plus liveness reporting the interval actually in force without re-timing any existing entry (FR-034/035); persist the interval in force on the session.
      `> DONE:` a run switched 15→30 mid-session leaves earlier entries untouched and later segments 30 minutes long; a test asserts nothing derives duration from config.
- [x] T017 [US1] `DayflowLiveness` in `src/dayflow/models.rs` + `daemon.rs`, every field read from the segment ledger and `timeline_entries` — never from an in-memory flag (FR-006/008).
      `> DONE:` a daemon that captures zero segments reports `degraded`; a healthy one advances `chunks_written`; a paused one reports `paused` with its cause and is NOT degraded.
- [ ] T018 [S] [US1] Flesh `src/dayflow/engine.rs` — `start_session` / `stop_session` / `status` over one shared pipeline, honouring the session max-duration cap and closing the in-progress segment on stop (FR-003/005).
      `> DONE:` start→stop drives the pipeline, the cap is enforced in a test, and the final segment is accounted for; `cargo check` green.
- [ ] T019 [S] [US1] Flesh `src/dayflow/daemon.rs` — continuous supervision, persisted lifecycle state, clean stop, and a restart that resumes onto the same day.
      `> DONE:` daemon starts, auto-segments across a short interval, survives a restart onto the same day, stops cleanly; `cargo check` + tests green.
- [ ] T020 [P] [US1] Integration coverage in `tests/dayflow_segmentation.rs` — boundaries, non-uniform lengths, pause gaps, clock discontinuity, display hot-plug.
      `> DONE:` all listed edge cases from `spec.md` are asserted; no test assumes a uniform segment length.

## Phase 4: User Story 2 — Ask what I was doing (P1)

**Goal**: entries appear as the day proceeds, and a day-level question is answered from them.

**Independent test**: with a stub provider and a fast clock, entries exist before `stop`; a
question returns an answer citing stored entries.

- [ ] T021 [US2] New `src/dayflow/scheduler.rs` (was T242) — a tokio task that summarizes each closed segment and writes its `TimelineEntry` immediately, channel-based and concurrency-safe with the capture loop (FR-014).
      `> DONE:` a running session with a stub provider produces entries BEFORE stop; a fast-clock test asserts it.
- [ ] T022 [US2] Retry, don't drop — a segment whose summarization fails stays `summarized = false` and is retried; capture keeps producing meanwhile.
      `> DONE:` with the provider unreachable, capture continues, the segment is marked unsummarized, and a later retry succeeds; nothing is silently skipped.
- [ ] T023 [US2] Return pause intervals as `gaps` alongside entries from `query_range` in `src/dayflow/timeline.rs`, so a gap is a recorded fact rather than missing data.
      `> DONE:` a range spanning a pause returns the entries plus the gap with its cause; parameters stay bound, never interpolated.
- [ ] T024 [S] [US2] `ask_day(question)` in `src/dayflow/timeline.rs` (was T243) — grounded strictly on `query_range` entries, and stating it has no record when the range is empty (FR-018).
      `> DONE:` seeded entries yield a grounded answer; an empty range yields an explicit no-record answer, never an invented one; `cargo check` + tests green.
- [ ] T025 [P] [US2] Integration coverage in `tests/dayflow_timeline.rs` — live-write ordering, range queries, gaps, empty-range grounding.
      `> DONE:` all US2 acceptance scenarios from `spec.md` are asserted.

## Phase 5: User Story 3 — Affordable, private perception (P2)

**Goal**: text work never reaches a vision model; escalation is explicit and logged; nothing
leaves the box by default.

**Independent test**: a text request is served without loading the vision tier; a semantic
request escalates exactly once, with a logged reason.

- [ ] T026 [P] [US3] New `src/dayflow/perception.rs` (was T300) — a `PerceptionRouter` over two configured `VisionProvider` instances (text tier, reason tier), dispatching on an explicit caller-supplied request kind, never by sniffing the prompt.
      `> DONE:` a stub-provider test proves a text request never touches the reason tier; `cargo check` green.
- [ ] T027 [US3] Crop before extract (was T301) — feed the text tier full-resolution region crops from the region cascade, never a downscaled full frame (FR-011). Reuse the existing target/measure path; no new detection code.
      `> DONE:` a two-pane frame yields per-pane text in correct order and a test asserts the full-frame column-scramble does NOT occur.
- [ ] T028 [US3] Log every escalation with its reason and serving tier (FR-007/010).
      `> DONE:` a semantic request emits exactly one escalation record naming the reason; a normal recording interval emits none.
- [ ] T029 [US3] Residency policy (was T331) — keep the text tier warm while a recording is active, using the unload window measured in T006; record per-segment latency including any reload (FR-008/013). Three-valued knob: resident / on-demand / off.
      `> DONE:` a multi-segment run records per-segment latency; with residency on, no segment pays a cold load; the knob is documented.
- [ ] T030 [US3] Give Dayflow's internal perception traffic its own rate-limit key and budget derived from interval, display count and `max_regions_per_segment`, leaving the interactive `analyze_video` 10/min ceiling untouched. See the plan's Complexity Tracking — the limiter currently has no call sites anywhere in the repo.
      `> DONE:` a test proves Dayflow's traffic cannot exhaust the interactive bucket and vice versa; the region cap bounds work at the source.
- [ ] T031 [S] [US3] Route the summarizer through the router (was T302) so per-segment perception uses the text tier by default and escalates only for category and meaning.
      `> DONE:` a text query resolves without a vision model, a semantic query escalates once, both covered by tests; `cargo check` + clippy `-D warnings` = 0.
- [ ] T032 [P] [US3] Integration coverage in `tests/dayflow_perception.rs`, including an assertion that the demoted tesseract path is never used as the text tier.
      `> DONE:` all US3 acceptance scenarios from `spec.md` are asserted.
- [ ] T052 [US3] Content intent — rolling OCR aggregation in `src/dayflow/perception.rs`, ported from videolocr's `OCRAggregator` (research R13): a 5-block history, `SequenceMatcher`-style overlap at 0.85, APPENDING to the matched block instead of creating a new one. Runs only when `intent.aggregates_text()`.
      `> DONE:` a scrolling pane sampled repeatedly yields ONE growing block, not N near-duplicates; under Activity intent the aggregator is never invoked; `cargo check` green.
- [ ] T053 [S] [US3] Content intent — text diff-merge (research R12): append a block only below 0.95 similarity, treat above 0.3 as comparable, and merge preserving unique lines. Every gate in the ladder FAILS OPEN (research R13) — on any error the sample is KEPT, never dropped, because dayflow cannot re-capture yesterday.
      `> DONE:` a test proves the union of two overlapping captures is preserved with no duplication; a fault injected into each gate results in the sample being KEPT; both intents covered; `cargo check` + tests green.

## Phase 6: User Story 4 — Entries that remember the layout (P2)

**Goal**: an entry carries where on screen its text came from; ordering is geometric and
deterministic.

**Independent test**: a two-pane capture yields entries whose region and parent references
reconstruct the arrangement, identically on every run.

- [ ] T033 [US4] Additive migration in `src/storage/database.rs` (was T310) — `region_id`, `bbox_*`, `parent_region_id`, `display_id`, `reading_order` on `timeline_entries`, all nullable, idempotent, not rewriting T240.
      `> DONE:` re-runnable; existing rows survive with empty provenance; an in-memory test round-trips the new columns.
- [ ] T034 [US4] Compute reading order from geometry in `src/regions/mod.rs` — banded top-to-bottom then left-to-right, bounded by the parent tree (FR-020). Never ask a model.
      `> DONE:` a deterministic test gives the identical order across repeated runs on the same input.
- [ ] T035 [S] [US4] Join extracted text to the region tree on region identity and persist provenance onto each entry (was T311).
      `> DONE:` a two-pane capture yields entries whose parent/bbox/display reconstruct the on-screen layout; `cargo check` + tests green.
- [ ] T036 [P] [US4] Integration coverage in `tests/dayflow_timeline.rs` for provenance and ordering, including entries written before the migration.
      `> DONE:` all US4 acceptance scenarios from `spec.md` are asserted.

## Phase 7: User Story 5 — Disk stays bounded (P3)

**Goal**: raw shrinks after summarization; eviction runs oldest-first under budget; the
timeline is never touched.

**Independent test**: drive summarize → shrink → over-budget → evict; bytes fall, entry count
does not.

- [ ] T037 [P] [US5] Tier state machine in `src/dayflow/retention.rs` (was T260) — `RetentionConfig` + hot/warm/cold computed from age and the `summarized` flag, mirroring the vocabulary of `capture::memory`.
      `> DONE:` unit tests cover every transition including the never-summarized case.
- [ ] T038 [US5] Shrink step (was T261) — after summarization, replace the raw segment with a timelapse plus retained extracted text.
      `> DONE:` the warm artifact is at most 10% of raw (SC-008) and its text is retained; a test asserts both.
- [ ] T039 [US5] Evict step (was T262) — over budget, drop summarized raw oldest-first, then warm oldest-first; never a timeline entry, never an unsummarized segment. Validate every path through `security::path_validator` before deleting.
      `> DONE:` a simulated over-budget run evicts in the correct order, refuses unsummarized segments, and leaves `timeline_entries` untouched.
- [ ] T040 [S] [US5] End-to-end retention (was T263) — summarize → shrink → evict with the timeline preserved throughout.
      `> DONE:` total bytes fall while `query_range` still returns every entry; `cargo check` + tests green.

## Phase 8: User Story 6 — Reach it from anywhere (P3)

**Goal**: one engine behind three surfaces.

**Independent test**: start → status → timeline through each surface against the same state.

- [ ] T041 [US6] MCP tools in `src/mcp/tools.rs` + `src/mcp/server.rs` (was T270) — `start_dayflow`, `stop_dayflow`, `dayflow_status`, `get_timeline`, `ask_day` per `contracts/mcp-tools.md`, with schemars schemas.
      `> DONE:` `tools/list` shows all five; a `call_tool` round-trip for each returns valid JSON against a stub provider.
- [ ] T042 [P] [US6] CLI subcommands in `src/bin/gentle-eye.rs` (was T271) per `contracts/cli.md`. `status` reporting `degraded` still exits 0.
      `> DONE:` each subcommand prints valid JSON; a degraded status exits 0 with the degradation in the payload.
- [ ] T043 [P] [US6] HTTP endpoints in `src/api.rs` (was T272) per `contracts/http.md`, no new dependency; a degraded recorder still returns 200.
      `> DONE:` each endpoint returns correct JSON on the hand-rolled server; a live curl test passes.
- [ ] T044 [S] [US6] Parity — all three surfaces drive the same engine and report the same state (was T273).
      `> DONE:` a session started on one surface is visible from the other two; `cargo check` + clippy `-D warnings` = 0.

## Phase 9: User Story 7 — Yesterday in one screen (P3)

**Goal**: a categorized day digest.

**Independent test**: seed a day, request the standup shape, get a categorized time-ranged digest.

- [ ] T045 [US7] Apply the activity-category taxonomy in the summarizer prompt in `src/dayflow/summarizer.rs` (was T280) so every entry carries a category.
      `> DONE:` a test asserts every produced category is a member of `ActivityCategory`.
- [ ] T046 [S] [US7] Standup view (was T281) — `get_timeline --standup` and `ask_day("what did I do today")` return a categorized, time-ranged digest with proportions computed from ACTUAL segment durations, never from a count times the configured interval.
      `> DONE:` the digest totals match summed real durations on a day with mixed segment lengths; `cargo check` + tests green.

## Phase 10: Polish

- [ ] T047 [P] Validate every segment, shrink and eviction path through `security::path_validator` in `src/dayflow/retention.rs` and `src/capture/service.rs`.
      `> DONE:` a path-escape attempt is refused by a test on both the write and the delete path.
- [ ] T048 [P] Document the feature in `docs/FPS_AND_DAYFLOW.md` — the two tiers, the residency knob, the idle policy, the interval knob, and how to read a status payload.
      `> DONE:` the doc covers all five and the quickstart's verification steps resolve against it.
- [ ] T049 [S] `cargo test` — all unit and integration tests green (was T290).
      `> DONE:` `./.tooling/bin/cargo test` exits 0.
- [ ] T050 [S] `cargo clippy --all-targets -- -D warnings` — zero warnings (was T291). NOTE: 4 PRE-EXISTING errors are outstanding on this branch — `regions/providers/wm.rs:41,48` (`and_then(|x| Ok(y))`) and `regions/mod.rs:302,340` (`map_or`). `-D warnings` comes from `.cargo/config.toml`, so the gate is unsatisfiable until they are fixed; they were deliberately NOT fixed as a drive-by by another task.
      `> DONE:` clippy exits 0.
- [ ] T051 Live validation in `tests/dayflow_live.rs`, `#[ignore]` (was T292) — a real multi-display session through real segments, real perception tiers and a real timeline, answering "what was I doing at 2pm?".
      `> DONE:` `cargo test --test dayflow_live -- --ignored` produces a real timeline; the quickstart's six manual checks all pass. A green `cargo test` alone does NOT certify this feature.

---

## Dependencies

```
Setup (1) ──▶ Foundational (2) ──▶ US1 (3) ──▶ US2 (4) ──▶ US4 (6) ──▶ US7 (9) ──▶ Polish (10)
                                        │           │         ▲          ▲
                                        │           └── US5 (7)          │
                                        └── US3 (5) ────────────┴── US6 (8)
```

- **Phase 2 gates everything.** No user story is testable against real data until segment files
  exist. This is why the plan orders by dependency rather than by story priority.
- **US3 is stub-testable in parallel** with US1/US2 — its router and tier logic need no real
  segments. Only T027 (crop before extract) needs real frames.
- **US4 depends on US3** for crops and on US2 for entries to attach provenance to.
- **US5 depends on US2** — nothing can be shrunk before it has been summarized.
- **US6 and US7 depend on US2** for something to serve.

## Parallel opportunities

| where | tasks |
|---|---|
| Setup probes | T002, T003, T005, T006 (T004 is the gating one — run it first) |
| Foundational models | T008, T009 |
| Within US1 | T013 alongside T012 |
| Across stories | all of US3 (T026, T028–T032) alongside US1/US2, using stubs |
| Test authoring | T020, T025, T032, T036 |

## Implementation strategy

**MVP = Phases 1–4** (Setup, Foundational, US1, US2): an all-day recorder over every display
that pauses when you step away, proves it is alive from its own artifacts, and answers "what
was I doing at 2pm?". That is the feature's reason to exist; everything after it is cost,
fidelity and reach.

**Then, in order of value**: US3 makes it affordable and private enough to leave running, US5
makes it survivable for more than a few days, US4 makes entries layout-aware, US6 spreads it
across surfaces, US7 presents it.

**Highest-risk task is T004.** It is the earliest, it is UNVERIFIED, and everything consumes
its output. If the segment muxer will not hold exact boundaries at 0.2–0.5 fps, stop and record
the fallback before any of Phase 2 is written on top of it.

## Summary

| | count |
|---|---|
| Tasks in this run | 51 |
| Carried forward (not re-run) | 13 |
| Sentinel checkpoints `[S]` | 13 |
| Waves the parser will see | 10 |
| User stories covered | 7 (US1–US7) |
