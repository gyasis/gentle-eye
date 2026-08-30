# Tasks: Dayflow Capture Loop with Pluggable Sources

**Feature**: `014-dayflow-capture-loop` | **Spec**: [spec.md](spec.md) | **Plan**: [plan.md](plan.md)

**Execution**: orchestrated and dispatched by **dev-kid** (`/devkid.orchestrate` →
`execution_plan.json`, then `/devkid.execute` under the task watchdog). A
**Fable-model agent GATES every wave checkpoint** — it reviews and it fixes what
it finds before the next wave opens. **Strictly serial**: build → gate → wait →
fix → verify → next.

**Conventions** (carried from 013): `> DONE:` states the OBSERVABLE evidence, not
the activity. `[S]` = the DONE line includes `cargo check` + clippy `-D warnings`
= 0. `[P]` = parallelisable with its neighbours. `[US1]`–`[US4]` map to the
spec's user stories. Where a checkbox will not cover its whole text, the DONE
line says so.

**Toolchain**: `./.tooling/bin/cargo`, never plain cargo.

---

## Phase 1: Setup

- [x] T001 Repoint dev-kid at this feature: `dev-kid.yml` (`branch: 014-dayflow-capture-loop`) and `.dk/context.json` → `specs/014-dayflow-capture-loop/tasks.md`.
      `> DONE:` `python3 ~/.dev-kid/cli/resolver.py resolve` prints the 014 task file, not the 013 one. dev-kid's resolver reads `.dk/context.json` FIRST — a stale pointer silently runs the previous feature's tasks to completion and reports success.
- [x] T002 [P] Create the module skeleton: `src/dayflow/source/mod.rs` re-exported from `src/dayflow/mod.rs`, and empty `tests/dayflow_source.rs` / `tests/dayflow_loop.rs`.
      `> DONE:` `cargo check` green with the new modules declared and nothing else changed.

## Phase 2: Foundational — the source abstraction (blocks every user story)

- [x] T003 [US2] Define `CaptureSource` in `src/dayflow/source/mod.rs` per `contracts/capture-source.md`: `next_frame`, `regions_for`, `availability`, `identity`, `ordinal`.
      `> DONE:` the trait compiles; its doc states the D014-1 rationale (a new kind must be addable without editing the loop) and the D014-2 note that `ordinal` occupies the existing `display_id` position.
- [x] T004 [P] [US2] `Availability` (`Available`/`Occluded`/`Ended`) and `SourceIdentity` in `src/dayflow/source/mod.rs`.
      `> DONE:` **AMENDED (D014-9)** — the original wording said "distinct in a gap record", which is not implementable: `Gap` is per-SESSION and has no source field, so a gap for one of three displays would claim the whole session stopped. A test instead proves the three states drive distinct OUTCOMES (`gap_cause` + `retryable`), that a per-source failure is a `DropReason::SourceUnavailable` and not a gap, and that `Available` warrants NO gap; `SourceIdentity` is stable across a simulated move (position is NOT identity) and uses a SPECIFIED hash, with its value pinned — `DefaultHasher` is not stable across toolchains and this id is written to disk (013/R31).
- [x] T005 [S] [US2] `DisplaySource` in `src/dayflow/source/display.rs`, wrapping the existing `capture::display` path and the region cascade.
      `> DONE:` an existing display session behaves byte-identically through the trait — same sample filenames, same `display_id` values, same regions; `cargo check` + clippy `-D warnings` = 0.

## Phase 3: User Story 1 — Dayflow runs by itself (P1)

- [x] T006 [US1] The driver in `src/dayflow/loop.rs`: tick on cadence, call the source, hand frame + regions to the `Sampler`, close windows via `DayflowRun`.
      `> DONE:` a fake source and an injected clock drive several segments; the test asserts SEQUENCING and TIMING only. Policy (windowing, gating, budget) is NOT re-asserted here — it has its own tests, and re-asserting it would pass while the loop bypassed the component entirely (013/R29).
- [x] T007 [US1] Take the clock as a parameter throughout the loop; no `Utc::now()` inside a decision path.
      `> DONE:` every loop test drives time explicitly; a state only reachable after hours of wall-clock is reachable in a test. A rule with the clock inside the function is undefended by construction (013/R36).
- [x] T008 [US1] Drive summarisation: closed windows enter `SummaryScheduler`, due windows are summarised through `RoutedChunkSummarizer` while the session continues.
      `> DONE:` entries appear DURING a simulated run, not only after stop; a failed summary is requeued and never marked summarised.
- [x] T009 [S] [US1] Wire the loop into `DayflowService::start`/`stop` so a started session actually runs.
      `> DONE:` `grep` shows `loop` called from the service, not only from its own tests — a "wire" task is not done while the new symbol has no caller outside its module (013/R29). `cargo check` + clippy = 0.

## Phase 4: User Story 1 — regions reach the ladder (P1)

- [x] T010 [US1] Write the region sidecar beside every sample: `perception::regions_path(sample)`, produced by the loop from `CaptureSource::regions_for`.
      `> DONE:` an integration test drives the loop and then asserts the ladder took CROPS — text-tier call count is samples × regions, not samples. The consumer has existed since 013 with no producer.
- [x] T011 [S] [US1] Surface `SegmentLatency::samples_read_whole` in `DayflowStatus`, and count a `None` from `regions_for` into it.
      `> DONE:` a session whose source yields no regions reports a NON-ZERO count in `status`, on all three surfaces. The path fails open by design, so its degradation is otherwise invisible — every test green and crop-before-extract entirely absent (013/R29). `cargo check` + clippy = 0.

## Phase 5: User Story 2 — watch one specific thing (P1)

- [x] T012 [P] [US2] `WindowSource` in `src/dayflow/source/window.rs`: frames cropped to a named window, regions scoped to it.
      `> DONE:` a session records only that window's content; a change elsewhere on the desktop produces NO sample. Asserted on stored text, not on intent.
- [x] T013 [P] [US2] `TargetSource`-backed source in `src/dayflow/source/target.rs`, using the existing named-target store.
      `> DONE:` a defined target drives a session; the stored entries carry that target's region provenance.
- [x] T014 [US2] Availability handling in the loop: a failed frame records a gap with its CAUSE and continues; `Ended` stops retrying, `Occluded`/`Available` retry next tick.
      `> DONE:` **AMENDED (D014-9, W6 gate)** — the original wording said "three DIFFERENT gap causes"; per D014-9 a per-source failure is a DROP, not a gap, so three fixtures — minimised, dropped, quit — prove three different OUTCOMES (availability, retry, what is recorded). The wave shipped only that per-source arm; the gate found `Availability::gap_cause()` called by nothing in production and wired D014-9's SECOND row: when NO source produces (all failed, or all retired — the whole case for a single-window/target session), the loop records a session pause with the cause (`SourceOccluded` lifts on recovery, `SourceEnded` never does), which is what lets health tell quiet-on-purpose from dead (FR-113; models.rs SourceEnded→Degraded was unreachable before this wire).
- [x] T015 [S] [US2] `DayflowStatus` names the session's source and its availability.
      `> DONE:` `status` on every surface says WHAT the record is a record of; `cargo check` + clippy = 0.

## Phase 6: User Story 2 — an input taken, not a display consumed (P1)

- [x] T016 [US2] `InputSource` in `src/dayflow/source/input.rs`: a stream or capture-device URL, using the existing stream path.
      `> DONE:` a session records frames from an input; `regions_for` returns `None` HONESTLY — it must not synthesise a whole-frame region, which would be indistinguishable from a real detection and hide the whole-frame read (contract, D014-3).
- [x] T017 [S] [US2] Source selection on all three surfaces: `--displays` / `--window` / `--target` / `--input`, and the equivalent MCP and HTTP parameters.
      `> DONE:` **AMENDED (W7 gate)** — "starts" here means starts a SESSION of that kind: each surface parses through the one `SourceSpec`, `start_session` names the source in `status` (availability `None` until something actually reads from it), and each adapter is tested delivering its own fields (CLI argv, MCP input, HTTP query). It does NOT start capture — no surface has ever started capture (`start_capture` has no production caller); driving the loop needs a summariser and a sample directory, which the daemon owns, so that wire is T022's, not a regression here. `start_with_source` (the join T022 will call) exists, is documented as unwired, resolves "every display" ONCE so the run's ordinals and the thread's built sources cannot come from two disagreeing enumerations (`build_sources` refuses an unresolved display spec), and is proven end to end by an `#[ignore]`d ffmpeg test. `cargo check` + clippy = 0.

## Phase 7: User Story 1 — the loop's remaining duties (P1)

- [x] T018 [US1] Run `retention::plan` on a schedule during a session and execute its decisions.
      `> DONE:` **AMENDED (W8 gate)** — a simulated long run shows disk falling while NO unsummarised segment is reclaimed — the rule is unchanged and re-proven at the loop level. The wave shipped `sweep_retention` with no production caller and `config::RetentionConfig` with no reader — "on a schedule during a session" existed only where a test invoked it (the orphan class, 013/R29). The gate wired it: the capture thread sweeps after any settle that lands entries and on a 60 s timer (the two moments eligibility can change), through the one `RetentionConfig::from_policy` join, proven by a live-thread test that watches a summarised window's samples vanish while the session runs. The gate also made a Shrink reclaim the region SIDECAR beside each sample — the executed Shrink deletes raw without yet producing the warm timelapse (`shrink` is unwired, so the segment goes Hot→Cold and the plan's `freed_by_shrink` under-credits what was actually freed — conservative, but the warm tier does not exist in production yet).
- [x] T019 [US1] Attach provenance at `scheduler::entry_from`: entries carry the regions their text came from, replacing `provenance: None`.
      `> DONE:` **AMENDED (W8 gate)** — the original wording said "a two-pane capture reconstructs its arrangement from stored rows", which the schema cannot satisfy: `TimelineEntry::provenance` is a single `Option<EntryProvenance>` and the timeline columns are single-valued, so one window yields one entry carrying ONE region — the second pane's box is stored nowhere (the full per-capture list exists only in `provenance_in_reading_order`, unpersisted, and in the sidecars, which retention reclaims). What a live loop's entries DO carry is non-null provenance naming the rank-0 region of the FIRST sample's sidecar — the arrangement the window OPENED with (a mid-window relayout is not represented) — with the cascade edge (`parent_region_id`) and `reading_order: 0`, ids matching the sidecar; a whole-frame window stores NULL rather than an invented region. Multi-region provenance is a persisted-schema change (a 1:N provenance table) and belongs with the daemon's persistence work, not silently here — the same DONE-vs-schema class as T004/T014. The gate also removed the wave's field-by-field re-implementation of the region→provenance mapping: `provenance_from_regions` now delegates to T035's `provenance_in_reading_order` (the R37/R40 duplicate-drift class).
- [x] T020 [US1] DESIGN + implement the `keep_alive` channel (D014-4, deliberately open). Options: an optional request-options struct on `VisionProvider` with a default (no call-site churn, one more type) or a dayflow-local provider wrapper (no shared-trait change, a second path to keep in step). Pick one, state why in the code.
      `> DONE:` `ResidencyPolicy::Resident` is expressible end to end — a test proves the value reaches the provider; a provider that IGNORES `keep_alive` still behaves correctly. `VisionProvider` is used well outside dayflow, so the choice is justified in a comment, not assumed. **W8 gate note:** `with_residency` has no production caller yet — nothing outside tests constructs a `RoutedChunkSummarizer` at all, so the channel is exactly as reachable as summarisation itself; T022, which builds the production summariser, MUST call `.with_residency(cfg.perception.residency, segment_cadence)` or `Resident` reverts to a policy the running system cannot express.
- [x] T021 [S] [US1] Residency measurably works: a `Resident` multi-segment run pays the cold load once, an `OnDemand` run pays it per segment.
      `> DONE:` `SegmentLatency::first_call` shows the difference across segments; `cargo check` + clippy = 0.

## Phase 8: User Story 4 — the record survives a restart (P2)

- [ ] T022 [US4] The daemon owns the session: `DaemonState`/`DaemonStateStore`/`decide_resume` get their first real owner, persisting the `SessionSpec` including its sources.
      `> DONE:` a restarted process resumes the SAME session with the same sources; the interruption is a gap with a cause, not an absence.
- [ ] T023 [S] [US4] Surfaces attach to the running daemon instead of constructing their own engine.
      `> DONE:` `status` from a NEW process reports the daemon's session; `stop` from a new process stops it. There must be exactly ONE state store — a second is how the two diverge (013/R29). `cargo check` + clippy = 0.

## Phase 9: User Story 3 — a real answer (P2)

- [ ] T024 [US3] Replace `ask_day`'s stub answerer with a call through the governed lane at the reasoning tier.
      `> DONE:` a range WITH records returns prose naming what those entries contain, on all three surfaces.
- [ ] T025 [S] [US3] The refusal path is unchanged and still consults NO model on an empty range.
      `> DONE:` the existing test still passes unmodified; an answer carries its grounding, so confident prose with empty grounding stays detectable. `cargo check` + clippy = 0.

## Phase 10: Polish

- [ ] T026 [P] Extend `tests/dayflow_live.rs` with an INPUT source.
      `> DONE:` the live run records content that was never rendered on this machine's screen (SC-103a), proving the abstraction is real rather than a filter over screen capture. `#[ignore]`d; fails loudly and specifically when the environment is absent.
- [ ] T027 [P] Update `docs/DAYFLOW.md`, `docs/DAYFLOW_OPERATIONS.md` and `docs/DAYFLOW_LIMITATIONS.md`: the source model, the new commands, and which limitations this feature closed.
      `> DONE:` every limitation this feature closes is REMOVED from the ledger rather than left standing, and `display_id`'s redefinition (D014-2) is documented — in a stored row it means "which source", not "which monitor".
- [ ] T028 [S] Full green: `cargo test` and `cargo clippy --all-targets -- -D warnings` = 0.
      `> DONE:` both clean on a fresh checkout of the branch.

---

## Dependencies

```
Setup (T001-T002)
  └─ Foundational: the source abstraction (T003-T005)   ← blocks everything
       ├─ US1 loop (T006-T009) ─ US1 regions (T010-T011) ─ US1 duties (T018-T021)
       ├─ US2 display-consumed sources (T012-T015)
       │    └─ US2 input-taken source (T016-T017)
       ├─ US4 restart (T022-T023)   [needs the loop]
       └─ US3 answerer (T024-T025)  [independent of the loop]
```

**MVP** = Setup + Foundational + Phase 3 (T001–T009): a session that runs itself
against a display source. Every later phase is an independent increment.

## Parallel opportunities

- T012/T013 — different files, different source kinds.
- T026/T027 — the live test and the docs touch nothing in common.
- T024/T025 are independent of the entire loop and can proceed alongside it.

## What the Fable gate checks at every wave

Beyond correctness, the failure modes 013 actually produced: a task marked done
whose defining verb does not execute (`grep` for a caller outside its module); a
fixture that does not produce the condition its assertion names (thirteen
instances); a duplicate relocated rather than removed; a time-dependent rule with
the clock inside the function; and a mutation experiment that did not prove it
ran — applied, COMPILED (`-D warnings` makes an orphaned binding a build failure
with no result line), and failures counted across ALL suites.
