# Tasks: Screen-Text Transcription (primitives + playbook)

**Feature**: `015-screen-transcription` | **Spec**: [spec.md](./spec.md) | **Plan**: [plan.md](./plan.md)

## Conventions (carried from 014)

- Every task carries a `> DONE:` line stating the **observable evidence** that
  closes it — not the activity.
- `[S]` — a checkpoint task whose DONE includes `cargo check` + clippy
  `-D warnings` = 0.
- `[P]` — parallelisable with its neighbours (different files, no ordering
  dependency).
- `[US1]`..`[US4]` map to the spec's user stories.
- A task whose verb is **wire / expose / reach** is NOT done while `grep` shows
  the new symbol has no caller outside its own module.

---

## Phase 1: Setup

- [x] T001 Repoint dev-kid at this feature: `dev-kid.yml` (`branch: 015-screen-transcription`) and `.dk/context.json` → `specs/015-screen-transcription/tasks.md`.
      `> DONE:` `python3 ~/.dev-kid/cli/resolver.py resolve` prints the 015 task file, not the 014 one. The resolver reads `.dk/context.json` FIRST — a stale pointer silently runs the previous feature's completed tasks and reports success. This was T001 in 014 and it caught exactly that.

## Phase 2: Frames and sharpness (US1)

- [ ] T002 [P] Module skeleton: `src/transcribe/{mod.rs,frames.rs,quality.rs,reader.rs}`, re-exported from `src/lib.rs`; empty `tests/transcribe_primitives.rs`.
      `> DONE:` `cargo check --all-targets` green with the modules declared and nothing else changed.
- [ ] T003 [US1] `sharpness(image) -> f64` in `src/transcribe/frames.rs` — variance of Laplacian over greyscale.
      `> DONE:` scoring the M4 fixtures reproduces the measured separation: the sharp frames score ~3x the blurred ones. Uses REAL frames from the research clip, not synthetic gradients — a synthetic blur cannot fail the way motion blur does.
- [ ] T004 [US1] `extract_frames(video, rate, dedup, out_dir) -> Vec<FrameRow>` — ffmpeg extraction with per-frame sharpness, NO fixed cap.
      `> DONE:` a recording longer than 20 frames yields more than 20 rows — the `-frames:v 20` cap in `analysis::ocr::ocr_video` is the actual blocker and a test pins that it is gone. ffmpeg absent produces a stated error naming it, never an empty row list.
- [ ] T005 [US1] The dedup threshold is the CALLER's: `dedup` is a parameter, and different values yield different frame counts.
      `> DONE:` the same recording at two thresholds returns two different counts (M1 measured 285 vs 138 vs 2 on one clip). A test asserts the parameter CHANGES the result — a threshold that is accepted and ignored is the orphan pattern in miniature.
- [ ] T006 [S] [US1] `gentle-eye frames` subcommand: machine-readable rows on stdout.
      `> DONE:` `grep` shows `extract_frames` called from the CLI, not only from tests; the output parses as JSON; `--help` lists it (enforced by `tests/docs_agree_with_code.rs`). `cargo check` + clippy = 0.

## Phase 3: Information content (US2)

- [ ] T007 [P] [US2] `TextQuality` + `quality(text)` in `src/transcribe/quality.rs`: compression ratio, unique-line ratio, unique-token ratio. NO length field.
      `> DONE:` the struct has no `length` field and the doc says why — including it invites the character-ceiling mistake M3 rules out, since a dense page of code is legitimately long while a broken reading is merely repetitive.
- [ ] T008 [US2] The scores separate the measured populations.
      `> DONE:` the M3 fixtures (a real good reading and a real degenerate one, from research.md) score ~0.610 vs ~0.0072 compression and ~0.955 vs ~0.003 unique-lines. A test asserts the SEPARATION, and asserts that LENGTH alone does not separate them — proving the metric earns its place.
- [ ] T009 [S] [US2] `gentle-eye text-quality` subcommand; returns scores, never a verdict.
      `> DONE:` `grep` shows a caller outside the module; the output carries no pass/fail field — the reject threshold is the caller's (D015-4). `cargo check` + clippy = 0.

## Phase 4: The fuzzy merge — closing three orphans (US3)

- [ ] T010 [US3] Make `coverage`/`merge_scroll` tolerate imperfect readings: similarity per line instead of equality.
      `> DONE:` two REAL readings of the same scrolled content (M2/M4 pairs) merge with the shared portion appearing ONCE. The existing exact-equality version fails this same test — run it before the change to prove the fixture produces the condition. A synthetic clean pair cannot fail this way, which is exactly why the old version looked correct.
- [ ] T011 [US3] Preserve the three existing guarantees under fuzziness: containment does not grow the document; no-overlap loses nothing; the merge never drops material to make a join look clean.
      `> DONE:` a test for each, and a mutation for each that fails exactly that test.
- [ ] T012 [S] [US3] `gentle-eye merge-text` subcommand, and `TextAggregator`/`aggregator_for` reachable.
      `> DONE:` `grep` shows `merge_scroll`, `coverage` and `TextAggregator` each have a production caller outside `perception.rs` — they have had ZERO since they were written, which is D015-8's whole point. `cargo check` + clippy = 0.

## Phase 5: Readers (D015-9)

- [ ] T013 Reader adapter in `src/transcribe/reader.rs`: owns the prompt, normalises the response, declares its quirks. Wraps `VisionProvider` — does not extend it.
      `> DONE:` a reader that emits a `<think>` preamble and a reader that emits markdown fences both normalise to the same plain text; `VisionProvider` gains no new method. The wrapper choice is justified in a comment against T020's opposite conclusion — that had to reach the REQUEST body, this reaches the RESPONSE.
- [ ] T014 Normalisation is REPORTED, not hidden.
      `> DONE:` the result carries how much was stripped; a response that is ENTIRELY preamble passes through rather than normalising to empty. A silent 60% reduction is indistinguishable from a model that said less — and 60% is the measured figure for `ornith-1.5-9b`.
- [ ] T015 Normalisation is idempotent.
      `> DONE:` normalising twice equals normalising once, for every adapter — a caller cannot corrupt text by handling it carefully.
- [ ] T016 [S] Reader is selectable by configuration, and a transcript records which reader produced it.
      `> DONE:` changing the configured reader changes which adapter runs, with no code change; the recorded reader is present in the output. Scores and merges are only comparable within one reader (FR-106c). `cargo check` + clippy = 0.

## Phase 6: The reading path repaired (US1)

- [ ] T017 Route `analysis::ocr::ocr_video`'s reading through the `VisionProvider` seam via a reader adapter, instead of calling tesseract directly.
      `> DONE:` `grep` shows no direct model/tesseract invocation on that path; every call goes through the one seam (FR-106).
- [ ] T018 Move deduplication BEFORE the cost is paid.
      `> DONE:` a recording of N frames with M distinct screens performs M readings, not N. Asserted by a counting reader — the current code dedups AFTER paying for every frame.
- [ ] T019 [S] A rejected reading is COUNTED and VISIBLE.
      `> DONE:` a run over material containing unreadable frames reports how many were rejected; the rejected text does NOT appear in the merged document. M2 measured 24% rejection on real material — a transcript that hides that cannot say how complete it is. `cargo check` + clippy = 0.

## Phase 7: Parity and the playbook (US4)

- [ ] T020 [P] MCP tools for the primitives, at parity with the CLI.
      `> DONE:` `tests/docs_agree_with_code.rs` passes — it FAILS if a registered MCP tool is absent from `docs/TOOLS.md`, and if a dispatched CLI command is absent from `--help`.
- [ ] T021 [P] [US4] The playbook: `docs/playbooks/transcribe-a-recording.md`.
      `> DONE:` it chains the primitives with only a shell; it states WHICH parameter to change for WHICH material AND how to tell the output is wrong (a repeating paragraph = similarity too tight; missing material = dedup too aggressive). Following it end to end on a real recording produces a transcript.
- [ ] T022 [US4] `docs/GENTLE_EYE_GUIDE.md` and `docs/TOOLS.md` gain the primitives and the transcription workflow.
      `> DONE:` the guide names the workflow; the agent reference lists every new command and MCP tool; the drift test passes.

## Phase 8: Certification

- [ ] T023 Live test `tests/transcribe_live.rs` against a REAL recording.
      `> DONE:` `#[ignore]`d, fails loudly and specifically when the environment is absent, and recovers text that exists only in the recording. A green `cargo test` certifies none of this — the live test is the certification (the 014 pattern).
- [ ] T024 [S] Full green: `cargo test` and `cargo clippy --all-targets -- -D warnings` = 0 on a FRESH checkout of the branch.
      `> DONE:` both clean in a clean worktree with its own target dir — a warm tree cannot prove it.

---

## Dependencies

Written as `X requires Y` — the unambiguous form. An arrow (`A -> B`) is read by
the orchestrator as "A depends on B", which is the reverse of how it reads as
prose; that ambiguity produced a wrong wave order on the first orchestration.

```
T002 requires T001
T003 requires T002
T004 requires T003
T005 requires T004
T006 requires T005

T007 requires T002
T008 requires T007
T009 requires T008

T010 requires T002
T011 requires T010
T012 requires T011

T013 requires T002
T014 requires T013
T015 requires T013
T016 requires T014, T015

T017 requires T013
T018 requires T017
T019 requires T018

T020 requires T006, T009, T012, T016, T019
T021 requires T020
T022 requires T021

T023 requires T020
T024 requires T022, T023
```

**Why the frames chain is strictly serial:** T003 (`sharpness`), T004
(`extract_frames`) and T005 (the dedup parameter) all write
`src/transcribe/frames.rs`, and each needs the one before it — `extract_frames`
scores each frame with `sharpness`, and the dedup threshold is a parameter of
`extract_frames`. They cannot be parallel and they cannot be reordered.

## MVP

**T001–T012** — the three primitives, callable, with the orphans closed. At that
point an agent can run the whole pipeline by hand, which is the feature's actual
promise; readers, parity and the playbook refine it.
