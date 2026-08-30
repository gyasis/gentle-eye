# Implementation Plan: Dayflow Capture Loop with Pluggable Sources

**Branch**: `014-dayflow-capture-loop` | **Date**: 2026-08-29 | **Spec**: [spec.md](spec.md)
**Input**: Feature specification from `specs/014-dayflow-capture-loop/spec.md`

## Summary

Feature 013 built every stage of the Dayflow pipeline and nothing that runs them.
This feature builds the driver, and gives it a **capture source** that is either
an input taken or a display consumed. All eight limitations in
`docs/DAYFLOW_LIMITATIONS.md` close together, because all eight are the same
absence.

The loop owns sequencing and nothing else: windows, gating, summarisation,
retention and budget are existing components with their own tests, and a driver
that re-decides any of them creates the second source of truth that 013's
reviews caught twice (R37, R40).

## Technical Context

**Language**: Rust, edition 2021 (locked by the constitution)
**Toolchain**: isolated at `./.tooling/bin/cargo` — NOT plain cargo
**Lint gate**: `-D warnings` via `.cargo/config.toml`; currently at zero and must stay there
**Capture**: `scrap = "0.5"` (locked) · **MCP**: `rmcp = "0.1"` with `["server","macros"]` (locked)
**New dependencies**: none anticipated. An input source uses the existing stream path; the answerer uses the governed lane already in use.
**Testing**: unit + integration under `./.tooling/bin/cargo test`, plus the `#[ignore]`d live run in `tests/dayflow_live.rs`
**Scale**: one session, one to a few sources, ~160 samples/day at the coarse cadence
**Constraints carried (locked, from 013)**: samples not recording · two granularities with a 5-minute hard floor · Activity/Content either-or · every gate fails open · eviction gated on `summarized` · reading order is geometry · perception budget refused up front

**Unknowns resolved in [research.md](research.md)**: the source trait shape (D014-1), the `display_id` identity question (D014-2), where region detection attaches (D014-3), the `keep_alive` signature (D014-4, deliberately left as a design task with options stated), cross-process attach (D014-6), availability states (D014-8).

## Constitution Check

| Rule | Status |
|---|---|
| Edition 2021, not 2024 | ✅ unchanged |
| All deps pinned in `Cargo.toml` | ✅ no new deps anticipated |
| Capture via `scrap = "0.5"` | ✅ `DisplaySource` wraps the existing path |
| MCP via `rmcp = "0.1"` `["server","macros"]` | ✅ unchanged |

No violations. If the `keep_alive` design (D014-4) ends up requiring a new
dependency, that is a gate failure and returns here before proceeding.

## Project Structure

```
src/dayflow/
  source/            NEW — the CaptureSource trait and its implementors
    mod.rs           trait, Availability, SourceIdentity
    display.rs       DisplaySource (today's behaviour, one kind among several)
    window.rs        WindowSource
    target.rs        TargetSource-backed
    input.rs         InputSource (stream / capture device)
  loop.rs            NEW — the driver: cadence, segment close, summarise, sweep
  (existing modules unchanged except where the data model names a change)
tests/
  dayflow_source.rs  NEW — the trait's contract, per implementor
  dayflow_loop.rs    NEW — sequencing and timing, NOT policy
  dayflow_live.rs    EXTENDED — an input source (SC-103a)
```

## Phasing

Waves are shaped so each is independently testable and the riskiest decisions
land first. dev-kid derives the actual wave grouping from `tasks.md`; this is the
intended order.

| Wave | Delivers | Independently testable by |
|---|---|---|
| Setup | dev-kid repointed at 014; the source module skeleton | `/devkid.orchestrate` runs against the 014 task file |
| 1 | `CaptureSource` + `DisplaySource` | today's display behaviour, unchanged, now through the trait |
| 2 | The loop driving one source end to end | a session left alone produces entries as it runs |
| 3 | Region sidecar producer + `samples_read_whole` in status | crops actually reach the text tier; a missing cascade is VISIBLE |
| 4 | `WindowSource`, `TargetSource`, availability states | a window session records only that window; minimise ≠ ended |
| 5 | `InputSource` | content recorded that was never on this machine's screen (SC-103a) |
| 6 | Scheduled retention sweep + provenance attachment | disk falls while unsummarised segments survive; entries carry regions |
| 7 | `keep_alive` channel + residency expressible | `Resident` measurably avoids the cold load per segment |
| 8 | Cross-process sessions | `status` from a new terminal sees the daemon's session |
| 9 | Real `ask_day` answerer | a range with records answers; an empty range still consults no model |
| Polish | live run extended, docs, clippy zero | the live test passes with an input source |

## Execution model — binding, not incidental

The task crate is orchestrated and dispatched by **dev-kid**:
`/devkid.orchestrate` produces `execution_plan.json`, `/devkid.execute` dispatches
waves under the task watchdog. Tasks are marked `[x]` in `tasks.md` as they
complete — the wave executor halts otherwise.

A **Fable-model agent is the GATE at every wave checkpoint**. It reviews the
wave, and it FIXES what it finds, before the next wave opens.

**Strictly serial**: build wave → Fable gate → wait → fix → verify → next. No
wave starts while another is under review — in 013 a wave built during another's
review landed unreviewed and the reviewer saw a moving tree.

Setup obligation before any wave: `dev-kid.yml` names branch `013-…` and
`.dk/tasks.md` points at the 013 task file. Both are repointed to 014.

### What the gate checks

Beyond correctness, the failure modes 013 actually produced:
a task marked done whose defining verb does not execute (`grep` for a caller
outside the defining module); a fixture that does not produce the condition its
assertion names (thirteen instances); a duplicate that was relocated rather than
removed; a rule that is time-dependent with the clock inside the function, so no
test can reach it; and a mutation experiment that did not prove it ran.

## Complexity Tracking

| Item | Why it is not simpler |
|---|---|
| A trait rather than an enum | FR-111a: a new source kind must not edit the loop. An enum the loop matches on fails that by construction. |
| `display_id` redefined, not renamed | The field is in five durable keys and every sample filename. R34 records what a half-identity key cost; a rename is a separate mechanical change, not this feature's. |
| `keep_alive` left as a design task | `VisionProvider` is used well outside dayflow. The options are stated in D014-4; picking one blind here would be a guess dressed as a plan. |

## Progress

- [x] Phase 0 — research complete, no unresolved unknowns
- [x] Phase 1 — data model, contract, quickstart
- [ ] Phase 2 — `/speckit.tasks`
- [ ] dev-kid orchestrate + execute, Fable gating each wave
