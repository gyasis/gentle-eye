# Dayflow limitations — the honest ledger

Everything a green test suite does **not** certify about this feature, gathered from the
SCOPE notes in `specs/013-dayflow-perception-waves/tasks.md` and the research log
(R1–R40). Each item says what is missing, why it is that way, and what closes it.

This file exists because the project's recurring defect class was the orphan: a function
that is written, documented, unit-tested — and called by nothing, with every test green
(six escalating occurrences, R24/R28/R29). A checkbox that implies more than the code does
is a lie told to the next reader, so the gaps are written down here instead.

## Closed by feature 014 (the capture loop)

The item that used to head this file — *"no live capture loop yet"* — is **closed**.
`CaptureLoop` drives the pipeline, `gentle-eye dayflow serve` runs it as a daemon that
owns the session across restarts, and the live validation has been run green against
three real displays, real models and a real input.

Six itemised gaps went with it and have been **removed from this ledger rather than left
standing**: the missing `.regions.json` producer, `Resident` being inexpressible,
`ask_day`'s stub answerer, cross-process `start`/`stop`/`status`, retention never being
called on a schedule, and provenance never reaching an entry. What replaced each is in
`specs/014-dayflow-capture-loop/research.md` (D014-1 … D014-15).

**A green `cargo test` still does not certify the whole feature.** The parts that need
real hardware are `#[ignore]`d and must be run deliberately — see *What only a live run
certifies*, below.

## The itemised gaps

### 1. The durable window ledger is unwired

`dayflow_segments` and `dayflow_samples` exist in the sqlite schema — and that schema's
own comment says liveness and eviction "must be answerable from rows another process
wrote" — but **nothing reads or writes them**. The capture loop keeps `closed`,
`samples` and `summarized` in memory, so what a restarted daemon knows about the previous
process comes from adoption (reconstructing windows from the samples on disk) rather than
from a ledger. Adoption is the mitigation; the ledger is the fix. No task in feature 014
owns it.

### 2. Cropping works only on display 0

The region cascade's WM provider tags every region `display_id: 0` in root coordinates,
so on a multi-monitor desk only display 0 gets a usable sidecar. Displays 1 and 2 are
**visibly** degraded rather than silently: they write no sidecar, and every one of their
samples is counted into `samples_read_whole`, which `status` reports on all three
surfaces. Closes when the cascade attributes real per-display origins.

### 3. The HTTP surface is single-threaded, with a 5-second read timeout

One connection is served at a time. The read timeout stops a silent client from freezing
the surface *forever* (which it did), but it is a **mitigation, not a fix** — measured,
one silent client still delays the next request by 5.06 s, N of them serialise into ~5N
seconds, and an honest client sending a large request line slowly is cut off (R38).
Thread-per-connection is the real answer; loopback-only binding is what makes the current
behaviour survivable. The caveat is also recorded next to the code it constrains.

### 4. Cross-session orphaned samples are neither adopted nor deleted

A daemon that resumes the same day adopts the previous process's orphaned samples,
summarises them under the resumed session and lets retention reclaim them normally. A
**fresh** session (a new day, or after an explicit stop) deliberately does neither: adopting
them would misattribute another session's screen, and deleting them would destroy
unsummarised evidence. They stay on disk. A policy is needed — most likely filing them
under the prior session id read from the pre-overwrite state.

### 5. `WindowSource` is X11-only, and says the wrong thing off X11

`WmLocator` is the only production `WindowLocator`, and `x11rb` is an unconditional
dependency with no `cfg` gate — so it compiles on macOS and fails at connect. A failed
connect maps to `Minimised`, which is right for a transient X error and **wrong for a
platform that has no X11**: a `--window` session there retries every tick forever,
capturing nothing, while `status` reports the window as minimised. D014-14 records the
fix — a fourth `Unsupported` state that fails loudly once. The trait seam is already
correct; a CoreGraphics or Wayland locator drops in without touching the loop.

### 6. Shrink reclaims raw without producing the warm timelapse

`retention::shrink` (the timelapse encoder) is built and tested but not wired into the
loop's sweep: an executed `Shrink` decision deletes the segment's raw samples and their
sidecars and produces **no warm artifact** — the segment goes Hot→Cold directly, and the
plan's `freed_by_shrink` under-credits what was actually freed. Conservative in the safe
direction (nothing is kept that should have been deleted, and nothing unsummarised is
ever touched), but `warm_days` and `DropWarm` currently govern a tier that production
never populates. Closes by running the encoder in `sweep_retention` before the reclaim.

## What only a live run certifies

These are `#[ignore]`d because they need real hardware, real models, or ffmpeg. A green
`cargo test` says nothing about them:

| Test | What it proves | How to run |
|---|---|---|
| `dayflow_live::a_real_session_flows_from_pixels_to_a_grounded_answer` | capture → gate → ladder → timeline → a grounded answer, on real displays | `GE_DAYFLOW_ENDPOINT=… cargo test --test dayflow_live -- --ignored` |
| `dayflow_live::an_input_source_records_content_never_shown_on_this_screen` | **SC-103a** — the source abstraction is real, not a filter over screen capture | same, plus `ffmpeg` |
| `dayflow_surfaces::a_range_with_records_returns_real_prose_about_them` | `ask_day` returns prose naming what the entries contain | same |
| `regions::providers::wm::window_states_reports_visibility` | a minimised window is detected by state, not by zero area | `DISPLAY=:1 cargo test --lib window_states_reports_visibility -- --ignored` (unfiltered, `--lib -- --ignored` also runs pre-014 env-bound tests — atspi under AppArmor, an OCR fixture — that fail on boxes without their fixtures) |

Measured, 2026-08-30: the input run read `ZEPHYRANTHES` — a word existing only inside a
synthetic video file — back out of the perception ladder.

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
- **The lint gate is clean.** The four clippy errors this file used to list are fixed;
  `cargo clippy --all-targets` passes under `-D warnings`, which is T028's gate.
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
