# Dayflow limitations — the honest ledger

Everything a green test suite does **not** certify about this feature, gathered from the
SCOPE notes in `specs/013-dayflow-perception-waves/tasks.md` and the research log
(R1–R40). Each item says what is missing, why it is that way, and what closes it.

This file exists because the project's recurring defect class was the orphan: a function
that is written, documented, unit-tested — and called by nothing, with every test green
(six escalating occurrences, R24/R28/R29). A checkbox that implies more than the code does
is a lie told to the next reader, so the gaps are written down here instead.

## The big one: no live capture loop yet

The engine (`DayflowRun`), the daemon's durable state, the sampler, the window controller,
the perception ladder and the retention rules are all built and mutation-tested — **but
nothing yet wires them to real hardware in a running binary** (T018/T019 SCOPE notes:
`daemon.rs` is an island; the types are not called from a running process). Several items
below are downstream consequences of this one seam. It closes when the capture-loop task
lands; T051 (the `#[ignore]`d live validation answering "what was I doing at 2pm?"
against real displays and real perception) is the acceptance test for it, and it has not
been run green. **A green `cargo test` alone does not certify this feature.**

## The itemised gaps

### 1. Nothing writes the `.regions.json` sidecar

The crop-before-extract path (T027) consumes a `<sample>.regions.json` sidecar written at
capture time — because the region cascade runs when the frame is taken, while
summarization happens at segment close, and re-detecting later would describe a different
moment than the pixels do. **No producer exists yet**; the capture loop owes it.

Why it is invisible: the path fails **open** (a missing sidecar means the frame is read
whole rather than dropped — correct, per the never-lose-a-sample rule), so its absence
produces no error, ever. Every test passes while T027's entire benefit — correct per-pane
reading order, no column scramble, no misread digits — is silently absent.
`SegmentLatency::samples_read_whole` counts the degradation so it is at least measurable.
Closes with the capture loop, which must name this file as a deliverable.

### 2. `Resident` is a policy the running system cannot express

`ResidencyPolicy::keep_alive` computes the right value against the right cadence (the
segment gap, not the sample interval — itself a fixed bug, R28), and the governor honours
`keep_alive` per request (R27). But `VisionProvider::analyze_image(path, prompt)` **has no
parameter to carry it**, so nothing sends it: the effective residency is always
`OnDemand`, whatever the config says. Closes with a provider-level signature change.
Mitigating fact: at the default cadence the burst pattern makes `OnDemand`'s cold-load
overhead ~0.4%, so the default behaviour is the intended one — only the `resident` knob is
aspirational.

### 3. `ask_day`'s answerer is a stub on every surface

The grounding rules are real and enforced — an empty range returns
`"No activity was recorded for that period."` without consulting anything, and the
`grounding` array always accompanies the answer. But no model is wired behind the prompt:
a non-empty range returns `[no model configured for ask_day]` plus the built prompt. Only
the refusal path works as advertised, and the tool description says so (T041 SCOPE, R37 —
the stub strings originally even differed per surface, a parity violation in the wave
about parity; they are one string now). Closes when a provider is attached to the
answerer seam, which every surface already passes through.

### 4. CLI `start` / `stop` / `status` cannot span processes

Each CLI invocation builds a fresh in-memory service, so `gentle-eye dayflow start`
prints a session id and the session dies with the process; a later `status` from a new
process reports nothing running. `timeline`, `standup` and `ask` DO share state across
processes, through SQLite (T042 SCOPE). This is a truthful consequence of item 0 — a
cross-process session needs the daemon to be the process that owns it. The help text and
this ledger say so rather than letting the checkbox imply otherwise.

### 5. No daemon loop calls retention on a schedule

`plan` / `shrink` / `reclaim_file` are built, ordered correctly (tier before age, encode
before delete) and mutation-proven, and the end-to-end test drives
summarize → shrink → evict through them. But **no scheduled sweep exists** (T040 SCOPE:
"marked done for the retention RULES, not for a running sweep"). Until the capture loop
lands, disk-budget enforcement happens only when something calls the planner. Watch the
disk on any long manual run.

### 6. Provenance is not attached end-to-end

The structural pieces are all real — provenance columns, geometric reading order,
persisted region identity, the SQLite round-trip, all mutation-proven — but
`scheduler::entry_from` still writes `provenance: None`, because it sees a summary and a
window and has no regions (T035 SCOPE). Attaching provenance needs the perception path to
return *which region each extracted text came from*, which is the US3+US4 seam the capture
loop closes. Until then every new entry looks like a pre-migration entry.

### 7. Recorded pauses are not surfaced as `gaps` in timeline queries

Pauses are recorded durably — the window controller's pause ledger and the
`dayflow_pauses` table, each interval with its cause and close time — and tests assert a
gap is a recorded fact rather than an absence of rows. But the timeline query surfaces
(`get_timeline` on MCP, `GET /dayflow/timeline`, the CLI) return `entries` only: the
`gaps` array that `contracts/mcp-tools.md` promises, and that T023's checkbox describes,
is **not present in any surface's response**. A caller who wants to distinguish "paused"
from "missing" for a past range currently cannot do it through the query API. Closes by
joining `dayflow_pauses` into the range query result.

### 8. The HTTP surface is single-threaded, with a 5-second read timeout

One connection is served at a time. The read timeout stops a silent client from freezing
the surface *forever* (which it did), but it is a **mitigation, not a fix** — measured,
one silent client still delays the next request by 5.06 s, N of them serialise into ~5N
seconds, and an honest client sending a large request line slowly is cut off (R38).
Thread-per-connection is the real answer; loopback-only binding is what makes the current
behaviour survivable. The caveat is also recorded next to the code it constrains.

### 9. Four known clippy errors block the lint gate (T050)

`-D warnings` comes from `.cargo/config.toml`, so `cargo clippy --all-targets` cannot pass
until four **pre-existing** errors are fixed: `regions/providers/wm.rs:41,48`
(`and_then(|x| Ok(y))`) and `regions/mod.rs:302,340` (`map_or`). They predate this branch
and were deliberately not fixed as a drive-by — T050 owns that gate, and drive-by fixes in
unrelated tasks are how reviews lose track of what changed and why.

## Smaller honest notes

- **T053's 0.95 append gate is subsumed, not implemented.** Line-level `diff_merge` adds
  only lines the block lacks, so a near-identical capture contributes nothing *by
  construction*; the explicit threshold branch was implemented, shown by mutation to
  change no behaviour, and deleted rather than defended by a test that would only assert
  an optimization was taken (R23).
- **`DropPolicy` defaults to `fail`** — the development posture. Before any genuinely
  unattended all-day run, flip `dayflow.delta.on_drop` to `record`, or the first
  unobtainable frame stops the run.
- **Lock detection is descoped.** Idle-threshold pausing is the primary and sufficient
  trigger (user decision, T005); the X saver `state` field is unusable under GNOME
  regardless. A host with no idle backend records continuously — it never falls into a
  permanent pause.
- **Content-merge coverage is exact trimmed-line equality**, so OCR that perturbs most
  lines per sample will fragment a document; and captures of one or two lines are
  degenerate (coverage is 0.0 or 1.0 with nothing between). Recorded as a known accepted
  limit (R24) — untestable without real OCR pairs.
- **`security::path_validator` coverage (T047) is still open** as a polish task: retention
  validates deletion paths, but the sweep of every segment/shrink/eviction path on both
  the write and delete side has not been completed.
- **The perception endpoint is machine-local configuration.** The committed default is a
  neutral loopback placeholder; a fresh machine must supply the real governed-lane host
  before any live perception works (spec assumption). Nothing in-repo will ever contain
  the real host.
- **Legacy `chunk_minutes` fallback is effectively unreachable.** `segment_duration()`
  consults `chunk_minutes` only when `segment_seconds == 0`, but `segment_seconds`
  serde-defaults to 900 — so a legacy config file that sets *only* `chunk_minutes` gets
  900 s, not its author's value. Flagged here rather than silently relied on; treat
  `segment_seconds` as the only real knob.
