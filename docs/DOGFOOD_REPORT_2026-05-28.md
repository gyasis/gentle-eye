# Dogfood Report — dev-kid / Integration Sentinel / ma-loop (micro-agent)

**Subject under test:** dev-kid (orchestrate→execute), the Integration Sentinel,
and the micro-agent/ma-loop fixer.
**Vehicle:** the real `gentle-eye` Rust rebuild (deliberately *outside*
micro-agent's TS/JS comfort zone).
**Rule of the exercise:** *halt-and-fix any tool bug we hit — the tool bug IS the
valued finding.*
**Compiled:** 2026-05-28. Sources: PRD `gentle_eye_devkid_dogfood_2026-05-26`,
`dev-kid/docs/architecture/SENTINEL_ORCHESTRATOR_REWORK_2026-05-28.md`,
`.claude/sentinel/SENTINEL-SENTINEL-T002/`.

---

## TL;DR / Verdict

The dogfood **succeeded at its actual goal** — it surfaced a stack of real tool
bugs/gaps. **It did not** let the tools autonomously drive the build: the pipeline
**halted on the first sentinel checkpoint (`SENTINEL-T002`)**, and the gentle-eye
rebuild was ultimately **completed by hand** using the methodology the tools were
supposed to automate.

- **Bugs found:** ~8 distinct issues across the three tools (below).
- **Fixed:** most — 4+ dev-kid bugs, the micro-agent TS/JS-only limitation, the
  hanging-TUI tier path, and the 3 structural orchestrator gaps (committed to the
  `dev-kid` + `micro-agent` forks).
- **Biggest open gap:** the reworked tools were **never re-run end-to-end on
  gentle-eye** — fixes are unit/functional-tested in isolation, not validated
  driving a real `[S]`-marked wave. (The build was finished manually.)
- **Architectural finding:** dev-kid is missing a **"task-instructor"** stage —
  the grounding/decision work between `orchestrate` and `ma-loop` that a human
  had to do by hand here.

---

## 1. Integration Sentinel

| # | Finding | Status |
|---|---------|--------|
| S1 | **Predicate boundary** — the sentinel ran `cargo check` against `src/models/mod.rs`, which was a **38 KB `[Tool:]` recovery-junk container, not compilable Rust** (the clean 10.7 KB synthesized version was never promoted). A sentinel placed on a file that *cannot* pass guarantees a halt. | ✅ Fixed (b): `[S]` markers |
| S2 | **Tiering / escalation** — `manifest.json` shows tier1 ran `model: openai/gpt-4o-mini` with **`ollama_url: null`**: it skipped the local Mac-ollama tier entirely, went straight to paid cloud, ran **1801 s (30 min)**, `iterations: 0`, then `"mixed-budget timed out"` → "all tiers exhausted" → **WAVE HALTED**. | ⚠ Config wiring (R-OC8 class) |
| S3 | **Zero-work failure** — the halt produced **0 lines changed**, `cascade_tasks_annotated: []` (empty `diff.patch`). It burned 30 min and couldn't even tell the operator *what* to fix. | ⚠ Partially (S1/S2 fixes) |
| S4 | **`sentinel-health` false-green** — preflight read the ollama URL from a different path than the runner, so it reported local tiers ✅ while the runner saw `null` — masking the mismatch. | ⚠ Guard added |

**Codified fix — sentinel checkpoint principles:** P1 validate only **real +
compilable** targets; P2 intelligence lives in the **task author**, not the
sentinel; P3 **attribute errors to origin**; P4 whole-suite vs per-unit is a
deliberate choice.

---

## 2. micro-agent / ma-loop (the fixer)

| # | Finding | Status |
|---|---------|--------|
| M1 | **Legacy `micro-agent` TUI hangs headless** — `run_tier1`/`run_tier2` invoke the builder.io `micro-agent` binary, which opens an interactive onboarding TUI ("Want to set up a new project?") and **blocks ~300 s/tier in a non-TTY**. | ✅ Fixed (a): deleted |
| M2 | **Headless `ma-loop` existed but was never wired** — the headless runner landed in `c23c9e4`, but `runner.py` dispatch still routed non-`tiers_file` projects to the hanging legacy path. | ✅ Fixed (a): rewired (−300 lines) |
| M3 | **TS/JS-only** — micro-agent's roles (artisan/librarian/critic) assumed JS/TS; gentle-eye is Rust. | ✅ Fixed: new `language-support.ts` (pushed `micro-agent` fork `feat/language-aware-agents`) |
| M4 | **Single-file blindness** — `ma-loop` repairs one file; pointed at file A for an error whose real cause is dependency B, it would **mangle a correct A**. | ✅ Fixed (c): cross-file attribution |

---

## 3. dev-kid (orchestrate / execute)

| # | Finding | Status |
|---|---------|--------|
| D1 | **Per-task sentinel injection** — a sentinel after *every* task (incl. skeleton/stub waves) placed checks on non-compilable stubs → the S1 halt. | ✅ Fixed (b): opt-in `[S]`-marker injection |
| D2 | **tasks.md routing divergence** — dev-kid resolves a canonical tasks.md via a priority chain; the `[S]`-marked file placed at **repo-root is not in the chain** (lite resolves to `.dk/tasks.md`; orchestrate *overwrites* root). The `[S]` edits were **silently ignored**; orchestrate read a stale 94-line copy. | ⚠ Workaround: run `dev-kid spec-resolve` first; **proposed guard:** warn on root/resolved divergence |
| D3 | **2 crashes patched mid-run** — a `sys`-module-shadowing crash (fires when 100% tasks marked `[x]`) and over-aggressive dependency inference. | ✅ Fixed (pushed `dev-kid` fork) |
| D4 | **preflight 100%-gate + ollama-url nesting** bugs. | ✅ Fixed |
| D5 | **execute is dispatch+checkpoint, not autonomous** — `execute` registers a wave for the *in-session agent* to implement, waits for `[x]`, then the checkpoint runs sentinel+ma-loop. Useful to know: ma-loop only authors when the agent's output fails the sentinel. | ℹ Clarified (not a bug) |

---

## 4. Architectural finding — the missing "task-instructor"

The biggest meta-finding: the **recover → audit → paired-debate → strategy →
grounded-tasks** work done *by hand* this session is exactly what dev-kid should
automate. dev-kid is missing a stage between `orchestrate` and `execute`/`ma-loop`:

> **task-instructor** — per task: (1) assemble a *Context-Packet* (PRD section +
> direct-dependency trait + recovered partial + a Librarian cheat-sheet of real
> APIs to kill hallucination); (2) for non-trivial files, run a decision step
> (Claude×Gemini paired-debate or model reasoning) to pick the build approach;
> (3) hand `ma-loop` a **grounded objective** — instead of just the bare task
> line + cargo errors.

The `(c)` cross-file attribution work is the seed of this (the agent, not the
single-file ma-loop, decides where a cross-file fix belongs).

---

## 5. Fixes shipped

- **`dev-kid`** fork branch `fix/language-agnostic-headless-tiering` — commits
  `86afc9a` + `e9f42aa`: removed legacy TUI tiers (a), `[S]` marker injection (b),
  cross-file attribution + agent-mediated recovery (c); CHANGELOG v2.4.0; design
  doc `docs/architecture/SENTINEL_ORCHESTRATOR_REWORK_2026-05-28.md`.
- **`micro-agent`** fork branch `feat/language-aware-agents` — language-aware
  roles + error-feedback + observability.
- Verification: (a) `py_compile` clean; (b) functional test (skeleton → 0
  sentinels, marked → 1 each); (c) `_attribute_cargo_errors` unit-tested.

## 6. Open / not-yet-validated (the honest gap)

1. **End-to-end never re-run.** The reworked pipeline
   (orchestrate → execute → `[S]` checkpoint → sentinel → ma-loop → (c)
   attribution) has **not** been run clean on gentle-eye. Fixes are validated in
   isolation only. A real `[S]`-driven wave + a real broken-dependency recovery
   remain unproven against live tooling.
2. **Tiering must be wired** for a real run — `tier1.ollama_url` must point at the
   Mac ollama (`<LAN_OLLAMA_HOST>`), or it repeats S2.
3. **D2 guard** (warn on tasks.md divergence) is proposed, not implemented.
4. **(c) attribution is cargo-only** (rustc JSON spans); other languages fall back
   to plain escalation.

## 7. gentle-eye outcome (built without the tooling)

The product was finished **manually** and is fully working: recover → synthesize
→ whole-crate `cargo` gate → live validation. Three interfaces off one library
(Rust crate / MCP server / CLI), both vision providers + OCR proven live, clean
under `clippy -D warnings` with 146 tests + 19 doctests. See
`docs/REBUILD_SESSION_2026-05-28.md` and the PRD.

**The irony, and the point:** the manual build *is* the spec for the
task-instructor dev-kid still needs. The valued findings were the bugs — they're
captured and mostly fixed; the remaining work is proving the fixes drive a build.

## Cross-references
- PRD: `~/dev/prd/scratch/gentle_eye_devkid_dogfood_2026-05-26.md` (full chronology)
- Rework design + debate: `dev-kid/docs/architecture/SENTINEL_ORCHESTRATOR_REWORK_2026-05-28.md`
- Halt artifact: `.claude/sentinel/SENTINEL-SENTINEL-T002/` (summary + manifest)
