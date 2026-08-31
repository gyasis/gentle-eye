# Implementation Plan: Screen-Text Transcription (primitives + playbook)

**Branch**: `015-screen-transcription` | **Date**: 2026-08-31 | **Spec**: [spec.md](./spec.md)
**Input**: Feature specification from `specs/015-screen-transcription/spec.md`

## Summary

Ship three **deterministic primitives** — frame extraction with a sharpness
score, an information-content score for text, and a fuzzy overlap merge — plus a
**harness-agnostic playbook** that chains them into a transcript. The
orchestration and every threshold belong to the caller, because the measurements
(research.md M1) proved those thresholds are content-dependent.

Two of the three primitives are largely **already written and unreachable**. The
work is mostly making orphans callable and making one of them tolerant of real,
imperfect input.

## Execution model

**dev-kid orchestrates; a Fable-model agent gates every wave.**

`/devkid.orchestrate` produces `execution_plan.json`; waves dispatch under the
task watchdog. At each wave checkpoint an independent Fable agent reviews AND
fixes before the next wave opens. **Strictly serial**: build wave → gate → wait →
fix → verify → next. No wave is built while another is under review; in 014 that
rule was violated once and the wave that jumped ahead landed unreviewed.

**Setup task, first, and it is real work:** `dev-kid.yml` and `.dk/context.json`
both point at feature 014. dev-kid's resolver reads `.dk/context.json` **first**,
so a stale pointer silently runs the previous feature's already-complete tasks
and reports success. In 014 this was T001 and it caught exactly that.

## Technical Context

**Language/Version**: Rust, edition 2021
**Primary Dependencies**: `image` (frame decode), `flate2` or `std`-adjacent for
compression ratio, existing `reqwest`/`VisionProvider`, ffmpeg (external binary)
**Storage**: none new — primitives are stateless; recordings are files
**Testing**: `./.tooling/bin/cargo test`, `#[ignore]`d live tests as the real
certification
**Target Platform**: Linux/X11 first (ffmpeg is cross-platform; nothing here is
X11-bound)
**Project Type**: single Rust project (library + CLI + MCP)
**Performance Goals**: sharpness and entropy scoring cost **no model call** —
that is the point; both must be cheap enough to run over every frame
**Constraints**: `-D warnings`; isolated toolchain `./.tooling/bin/cargo`; ffmpeg
absent must fail with a stated reason; no private host addresses in tracked files
**Scale/Scope**: recordings of arbitrary length — the existing `-frames:v 20` cap
is the blocker being removed

## Constitution Check

| Rule | Status |
|---|---|
| Edition 2021, deps pinned | ✅ no new unpinned deps |
| `pub` items get `///` docs | ✅ enforced per task |
| `#[cfg(test)] mod tests` per module | ✅ each primitive ships tests |
| No `unwrap()` outside tests | ✅ `?` + typed errors |
| No hardcoded secrets/hosts | ✅ and `tests/docs_agree_with_code.rs` enforces |
| Paths validated by `path_validator` | ✅ frame extraction writes under a caller path |
| `cargo check` at every checkpoint | ✅ `[S]` tasks |
| `clippy -D warnings` before merge | ✅ final wave |
| Halt-and-fix dogfood discipline | ✅ this feature exists *because* dogfooding found the gap |

**No violations.** One note: the constitution says "`cargo test` green at Wave
10"; this feature has fewer waves, so the gate is "green at the final wave".

## Design

### The seam-vs-policy split (D014-1 applied)

| In the tool (deterministic) | With the caller (judgement) |
|---|---|
| frame extraction + sharpness score | the sharpness threshold |
| near-duplicate comparison | the dedup threshold |
| information-content score | the reject threshold |
| fuzzy overlap merge | whether the result is good enough |

Each primitive answers a question; none decides what to do about the answer.

### Primitive 1 — frames with sharpness

Extract frames at a caller-chosen rate, score each by variance-of-Laplacian,
emit machine-readable rows. **No fixed cap** (removes `-frames:v 20`).
Near-duplicate suppression happens **before** any cost is paid, with the
threshold supplied by the caller.

M4 is why this exists: sharp frames (1,443–1,458) all read cleanly; blurred
frames (396–507) **all** failed. Sharpness costs nothing and predicts the
expensive stage's outcome, so the cheapest measurement decides where the
expensive one is spent.

### Primitive 2 — information content

Given text, return compression ratio and unique-line ratio. **Never length** —
M3 measured length separating the populations by only 25× while a dense page of
code is legitimately long, so a character ceiling truncates real material.
Compression separates by 85×, unique-lines by 300×.

### Primitive 3 — fuzzy merge

Join two readings on their overlap using normalised similarity per line rather
than equality. **This is `coverage` / `merge_scroll` / `TextAggregator` made
reachable and made tolerant** — not a fourth implementation. M5: the existing
exact-equality version cannot merge real readings, which is recorded in
`DAYFLOW_LIMITATIONS.md` as untestable "without real OCR pairs". The pairs exist
now.

### Reading path

`analysis::ocr::ocr_video` is repaired rather than replaced: cap removed, dedup
moved ahead of the cost, and reading routed through the **`VisionProvider` seam**
instead of calling tesseract directly. No component gains a private path to a
model.

### Honesty requirement

A rejected reading is **counted and reported**, never silently dropped and never
merged. M2 measured 24% of frames failing on real material — a normal operating
condition, so a transcript that hides it is lying about its own completeness.
This is `samples_read_whole` from 014 applied to a new stage.

## Project Structure

### Documentation (this feature)

```
specs/015-screen-transcription/
├── spec.md              # done
├── research.md          # done — M1..M5 + D015-1..D015-8
├── plan.md              # this file
├── data-model.md
├── quickstart.md
├── contracts/
│   └── primitives.md    # the CLI/JSON contract each primitive keeps
└── checklists/
    └── requirements.md  # done
```

### Source Code

```
src/
├── analysis/
│   └── ocr.rs                 # repair: cap, dedup order, VisionProvider
├── transcribe/                # NEW — the primitives
│   ├── mod.rs
│   ├── frames.rs              # extraction + sharpness
│   └── quality.rs             # information content
├── dayflow/
│   └── perception.rs          # merge_scroll/coverage made reachable + fuzzy
├── bin/gentle-eye.rs          # CLI subcommands
└── mcp/{server,tools}.rs      # parity

docs/
├── playbooks/
│   └── transcribe-a-recording.md   # NEW — the chaining procedure
└── TOOLS.md                        # parity, enforced by a test

tests/
├── transcribe_primitives.rs
└── transcribe_live.rs              # #[ignore]d — the real certification
```

**Structure Decision**: single project. A new `src/transcribe/` module holds what
is genuinely new; the merge lives where it already lives, in `perception.rs`,
because moving it would create the second copy this feature exists to avoid.

## Wave order (for orchestration)

| Wave | Content |
|---|---|
| Setup | repoint dev-kid at 015; confirm the resolver returns the 015 task file |
| W1 | frames + sharpness, cap removed |
| W2 | information content |
| W3 | fuzzy merge — the orphans made reachable |
| W4 | reading through the VisionProvider seam; rejected readings counted |
| W5 | CLI + MCP parity |
| W6 | the playbook |
| W7 | live test, docs, full green |

## Testing emphasis (carried from 014's review record)

- A task whose verb is **wire / expose / reach** is not done while `grep` shows
  the new symbol has no caller outside its own module. Seven orphans in 014 were
  found this way; three of this feature's inputs are orphans right now.
- **A fixture must produce the condition its assertion names.** Twenty-two
  instances in 014 — the dominant defect. For the fuzzy merge specifically, use
  the **real degraded readings** from M2/M4 as fixtures; a synthetic clean pair
  cannot fail the way real input does, which is precisely why the existing
  exact-equality merge looked correct.
- **A mutation must prove it ran.** Under `-D warnings` an orphaned binding fails
  to compile and prints *no result line*, which reads as a pass.
- Live tests are the certification; a green `cargo test` certifies none of it.

## Complexity Tracking

No constitution violations requiring justification.

One deliberate deferral: the end-to-end `transcribe` command is **issue #17**,
not this feature. M1 showed the pipeline's parameters are content-dependent; a
judgement that must vary by content should not be compiled in before real use has
shown which defaults are right.
