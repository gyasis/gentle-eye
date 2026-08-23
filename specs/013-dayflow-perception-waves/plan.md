# Implementation Plan: Dayflow — Continuous Screen-Activity Timeline

**Branch**: `013-dayflow-perception-waves` | **Date**: 2026-08-23 | **Spec**: [spec.md](./spec.md)
**Input**: Feature specification from `/specs/013-dayflow-perception-waves/spec.md`
**Supersedes**: `specs/002-dayflow-mode/tasks.md` (2026-05-28, Amendment A 2026-08-23)
**Authority**: PRD `dayflow_waves_continuance_2026-08-23`

## Summary

Finish Dayflow: record every attached display all day in fixed-length segments, perceive each
segment through a cheap local text tier (escalating to a vision tier only for meaning), and
write a queryable, layout-aware timeline in real time — with liveness that is provable from
artifacts, storage that stays inside a budget, and the same engine reachable from the agent
tool surface, the CLI and HTTP.

Thirteen of the original thirty-nine tasks are already done and green on `main` (~581 lines,
7 passing tests): the data model, config surface, capture-rate heuristic, chunk manifest,
map-reduce summarizer with rolling context, and the timeline table and store. This plan
re-specs only what remains and does not rebuild any of it.

**The technical spine** is four changes, in dependency order: (1) make the encoder emit real
segment files on a wall-clock boundary — nothing downstream is testable against real data
until this exists; (2) put a tier router in front of the existing `VisionProvider` so text
work never reaches a vision model; (3) carry region geometry onto the timeline row so layout
survives the crop; (4) supervise the whole thing with a controller whose status is read from
what was written, not from what it believes about itself.

## Technical Context

**Language/Version**: Rust, **edition 2021** (constitution-locked; not 2024)
**Primary Dependencies**: `tokio` 1 (full), `scrap` 0.5 (capture), `x11rb` 0.13 (EWMH regions;
**+ `screensaver` feature** for idle detection), `rusqlite` 0.31 (bundled), `reqwest` 0.12
(json), `rmcp` 0.1 (server/macros/transport-io), `image`/`imageproc` 0.25, `serde`/`serde_json`,
`config` 0.14 + `toml` 0.8 + `directories` 5, `thiserror` 1 / `anyhow` 1, `schemars` 0.8.
**External binary**: `ffmpeg` (already required by `capture::encoder::PipeEncoder`).
**No new crates.** The only dependency change is enabling an existing pinned crate's feature.
**Storage**: SQLite via `rusqlite` (bundled) — `timeline_entries`, extended additively.
Segment video and shrunk artifacts on disk under the managed recordings directory.
**Testing**: `cargo test`; deterministic unit and integration tests against a **stub
`VisionProvider`** and an in-memory SQLite, plus `#[ignore]` live tests for the real paths.
**Target Platform**: Linux/X11 primary (this host). Idle detection is behind a trait so other
platforms degrade to "never idle" rather than failing.
**Project Type**: Single Rust crate — library + binaries (CLI, MCP server, HTTP).
**Performance Goals**: per-segment perception must complete inside the configured interval
across all displays (SC-004); warm text extraction on a cropped region under 5s (SC-003,
measured 1.6s); day question answered under 10s (SC-002).
**Constraints**: local-only by default — zero off-box perception requests (SC-009); all model
calls through the governed lane, never a raw model port (D8); storage within configured budget
with the timeline never evicted (FR-024).
**Scale/Scope**: one user, one machine, N attached displays; an 8-hour day at a 15-minute
interval is ~32 segments per display; the timeline is the only unbounded-growth artifact and
it is small.

## Constitution Check

*GATE: evaluated before Phase 0, re-evaluated after Phase 1 design.*

| Constitution rule | Status | How this plan satisfies it |
|---|---|---|
| Edition 2021 | **PASS** | No edition change. |
| All deps pinned, no unspecified versions | **PASS (with a recorded change)** | No new crates. `x11rb` gains its `screensaver` feature for idle detection (R2); the `Cargo.toml` edit carries a comment saying why, per house dep discipline. |
| Capture via `scrap` 0.5 | **PASS** | Multi-display reuses `capture::display::DisplayManager` over `scrap`. |
| MCP via `rmcp` 0.1 | **PASS** | New Dayflow tools register on the existing server. |
| `thiserror` typed / `anyhow` app errors | **PASS** | New failures extend `dayflow::DayflowError`, which already maps into `GentleEyeError`. |
| Vision providers via `reqwest` 0.12 | **PASS** | Both tiers are `analysis::ollama::OllamaProvider` instances; no new transport. |
| `pub` items get a `///` doc comment | **PASS** | Applies to all new public items. |
| Each module has `#[cfg(test)] mod tests` | **PASS** | Every new module ships constructor + validation tests. |
| No hardcoded secrets; env vars only | **PASS** | Local governed lane needs no key; the opt-in cloud path continues to read `GEMINI_API_KEY` from env. |
| No `unwrap()` outside tests | **PASS** | `?` + typed errors throughout. Explicitly enforced in the long-lived daemon loop, where a panic is a silent all-day outage. |
| Paths validated by `security::path_validator` | **PASS** | Segment, shrunk-artifact and eviction paths all validate before write or delete. Eviction deletes files — this is the highest-consequence path in the feature. |
| Recording IDs are validated UUID v4 | **PASS** | `DayflowSession`/`TimelineEntry` already carry `Uuid`. |
| `analyze_rate_limit_per_minute = 10`, enforced | **⚠ TENSION — see Complexity Tracking** | The limiter exists and is unit-tested but has **no call sites** outside its own module; and 10/min is an interactive-tool ceiling that Dayflow's internal traffic would breach. Resolved by a separate per-key budget plus a hard per-segment region cap. |
| `cargo check` at every wave checkpoint | **PASS** | Every `[S]` checkpoint. |
| `cargo clippy -- -D warnings` before merge | **PASS** | Final gate. |
| `cargo test` green at the final gate | **PASS** | Final gate. |
| Halt-and-fix: a tool bug is the valued finding | **PASS** | Carried forward; the `.dk/tasks.md` mismatch and the unwired rate limiter are already instances. |

**Post-Phase-1 re-evaluation**: no new violations introduced by the design. The data model adds
only nullable columns to an existing table; the contracts add tools to an existing server; no
new crate, no new process, no new network surface. The single tension remains the rate limiter,
recorded below.

## Project Structure

### Documentation (this feature)

```text
specs/013-dayflow-perception-waves/
├── spec.md              # Feature specification (complete)
├── plan.md              # This file
├── research.md          # Phase 0 — decisions, alternatives, and what is still UNVERIFIED
├── data-model.md        # Phase 1 — entities, schema delta, state transitions
├── quickstart.md        # Phase 1 — how to run and verify it
├── contracts/           # Phase 1 — MCP / CLI / HTTP contracts
│   ├── mcp-tools.md
│   ├── cli.md
│   └── http.md
├── checklists/
│   └── requirements.md  # Spec quality checklist (passing)
└── tasks.md             # Phase 2 — /speckit.tasks output
```

### Source Code (repository root)

```text
src/
├── dayflow/
│   ├── mod.rs           # (exists) module root
│   ├── models.rs        # (exists) extend: SegmentRef display/seq, pause intervals, liveness
│   ├── errors.rs        # (exists) extend
│   ├── chunking.rs      # (exists, 111 lines, 4 tests) segment manifest
│   ├── summarizer.rs    # (exists, 292 lines, 2 tests) map-reduce + rolling context
│   ├── timeline.rs      # (exists, 208 lines, 1 test) extend: region provenance, gaps
│   ├── engine.rs        # (stub, 7 lines) session + daemon lifecycle over one pipeline
│   ├── daemon.rs        # (stub, 6 lines) continuous supervisor, persisted state, liveness
│   ├── retention.rs     # (stub, 7 lines) hot/warm/cold state machine, shrink, evict
│   ├── scheduler.rs     # NEW — per-segment real-time summarize→write task
│   ├── perception.rs    # NEW — tier router, explicit escalation, logging
│   └── idle.rs          # NEW — idle/lock detector behind a trait, X11 backend
├── capture/
│   ├── encoder.rs       # (exists) extend: ffmpeg segment muxer + manifest
│   ├── display.rs       # (exists) DisplayManager/DisplayInfo — reuse for multi-display
│   ├── service.rs       # (exists) extend: per-display segmented capture supervision
│   ├── frame_rate.rs    # (exists, done) duration-aware heuristic
│   └── memory.rs        # (exists) pressure ladder — mirror its vocabulary in retention
├── regions/mod.rs       # (exists) extend Region with display identity; reuse assign_parents
├── analysis/ollama.rs   # (exists) both tiers instantiate this against different models
├── storage/database.rs  # (exists) additive migration for region provenance
├── mcp/{server,tools}.rs, bin/gentle-eye.rs, api.rs   # (exist) add the five surfaces
└── security/{path_validator,rate_limiter}.rs          # (exist) apply at write/delete/perceive

tests/
├── dayflow_segmentation.rs   # segment boundaries, non-uniform lengths, pause gaps
├── dayflow_perception.rs     # tier routing, escalation logging, no-VLM-for-text
├── dayflow_timeline.rs       # real-time write, range query, region provenance, ask
├── dayflow_retention.rs      # shrink → over-budget → evict, timeline preserved
└── dayflow_live.rs           # #[ignore] — real capture, real models, real ffmpeg
```

**Structure Decision**: Single Rust crate, unchanged. Dayflow is a module inside the existing
`gentle-eye` library, reusing capture, region, analysis, storage and security subsystems and
adding three new modules (`scheduler`, `perception`, `idle`) plus flesh on three existing
stubs (`engine`, `daemon`, `retention`). No new project, no new binary, no new service.

## Implementation Sequence

Ordered by hard dependency, not by user-story priority. The first item gates everything else.

| # | Slice | Delivers | Gated on |
|---|---|---|---|
| 1 | **Segmented capture** | real `chunk_NNNN.mp4` files on wall-clock boundaries + a manifest ffmpeg writes | — |
| 2 | **Multi-display + idle/pause** | per-display pipelines; pause on lock/idle, resume on activity; manual off/on; interval changeable mid-day | 1 |
| 3 | **Perception ladder** | tier router, crop-before-extract, explicit logged escalation, residency policy | — (parallel with 1–2, stub-testable) |
| 4 | **Real-time timeline** | scheduler writes an entry as each segment closes; `ask_day` grounded on stored entries | 1, 3 |
| 5 | **Structural provenance** | additive schema + geometric reading order joined on region | 3, 4 |
| 6 | **Liveness** | artifact-derived status distinguishing healthy / paused / off / degraded | 1, 4 |
| 7 | **Retention** | shrink after summarize; ordered evict under budget; timeline never touched | 4 |
| 8 | **Surfaces** | MCP + CLI + HTTP over one engine | 4, 6 |
| 9 | **Categories + standup** | taxonomy applied; day digest view | 4 |
| 10 | **Gates** | tests, clippy `-D warnings`, live validation | all |

**Slice 1 carries the project's schedule risk.** Its `-force_key_frames` behaviour at
fractional fps is UNVERIFIED (research R1) and everything downstream consumes its output. It
must be proven with a live short-interval capture before slices 2, 4 and 7 are built on it.

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|---|---|---|
| Dayflow perception bypasses the constitution's single `analyze_rate_limit_per_minute = 10` ceiling, using a separate per-key budget plus a hard per-segment region cap | 10/min is sized for a human invoking `analyze_video` interactively. Dayflow issues one text extraction per region per segment per display; at two displays and a modest region count it breaches 10/min while behaving exactly as specified. Refusing its own traffic would make the feature fail closed against itself. | *Raise the global limit* — rejected: it would silently loosen the ceiling protecting the interactive tools, which is what the rule exists for. *Serialize Dayflow under the existing bucket* — rejected: it would push per-segment work past the segment interval and break SC-004. *Leave the limiter unwired* — rejected: the tension is real either way, and discovering it at runtime is worse than deciding it now. **Note**: the limiter currently has no call sites anywhere in the repo (verified by grep), so the constitution presently overstates enforcement. Wiring it correctly for both traffic shapes is part of this feature, not a precondition of it. |
| A new `idle.rs` module and a feature flag on a pinned dependency | FR-030/031 require pausing on lock/sleep/idle. Detection needs a real platform signal. | *Frame-diff inference* — cannot distinguish "away" from "reading", cannot see a lock screen, and keeps the expensive capture path running to decide. *A new idle crate* — `x11rb` is already pinned and already exposes this; adding a crate for it violates the dep-minimalism the constitution encodes. |

## Phase Status

- [x] Phase 0 — research complete (`research.md`); four items explicitly marked UNVERIFIED with the check owed for each
- [x] Phase 1 — design complete (`data-model.md`, `contracts/`, `quickstart.md`); agent context updated
- [x] Constitution check — passed pre-Phase-0 and re-checked post-Phase-1; one recorded tension
- [ ] Phase 2 — `tasks.md` (produced by `/speckit.tasks`, not by this command)
