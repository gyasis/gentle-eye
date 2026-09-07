# Research — feature 015, screen-text transcription

Every decision below is grounded in a measurement taken **2026-08-31** against a
live ATEM Mini feed, not in expectation. The clip: 15.019 s, 1920×1080 h264,
captured by stream copy (~172 kbps, 1.22 MB), 325 frames, showing a Windows/WSL
terminal being scrolled — dense text with real motion.

The raw evidence is reproduced verbatim in "Measured evidence" below; the
decisions that follow cite it.

---

# Measured evidence — 2026-08-31, live ATEM feed

Source: 15.019 s, 1920x1080 h264, captured from an ATEM Mini over RTMP
(stream copy, ~172 kbps, 1.22 MB). 325 frames. Content: a Windows/WSL terminal
being scrolled — dense text, real motion.

## M1 — Pixel dedup is content-dependent, not a fixed win
| approach | frames to OCR |
|---|---|
| naive 1 fps | 15 |
| mpdecimate default | 285 of 325 |
| mpdecimate hi=64*48:lo=64*24:frac=0.5 | 138 |
| mpdecimate hi=64*200:lo=64*100:frac=0.7 | 2 |
Scrolling content genuinely changes every frame; pixel dedup cannot help and
should not. Static slides collapse enormously. The threshold is a knob, not a
constant.

## M2 — OCR looping is systematic, not incidental
21 frames OCR'd via deepseek-ocr:latest on the governed lane.
- median output: 1,002 chars
- 5 of 21 frames (24%) returned 22,607-24,875 chars
A 1080p screen cannot hold 24k characters. The model looped.

## M3 — Entropy separates loops from dense content; LENGTH DOES NOT
Same model, one good frame (f_0005) and one looped frame (f_0019):

| metric | GOOD | LOOPED | separation |
|---|---|---|---|
| length | 1,000 | 24,875 | 25x |
| unique lines / total | 0.955 | 0.003 | **300x** |
| unique tokens / total | 0.781 | 0.004 | 195x |
| zlib compression ratio | 0.610 | 0.0072 | **85x** |

A dense page of code is legitimately long, so a character ceiling would truncate
real content. No legitimate text has 0.3% unique lines. Compression ratio is an
entropy proxy and separates the two populations by 85x — two regimes, not a
tuned threshold.

## M4 — Blur PREDICTS OCR failure, and is measurable before the model call
Variance-of-Laplacian focus measure over the first 24 frames:

| frames | sharpness | OCR |
|---|---|---|
| 1-16 | 1,443-1,458 | clean (~1,000 chars, 0.955 unique lines) |
| 17-24 | 396-507 | **looped** (24k chars, 0.003 unique lines) |

A 3.6x sharpness drop, and every OCR failure lands on the blurred frames, with
no exceptions. The blur is motion blur from scrolling.

CONSEQUENCE: a higher frame rate is not for capturing more CONTENT — it is for
capturing a SHARP INSTANCE of the same content. Sharpness costs no model call,
so it can gate the expensive stage.

## M5 — Exact line equality cannot merge noisy OCR
`DAYFLOW_LIMITATIONS.md` already records: "Content-merge coverage is exact
trimmed-line equality, so OCR that perturbs most lines per sample will fragment
a document... Recorded as a known accepted limit (R24) — untestable without real
OCR pairs."
The pairs now exist. Two OCR passes of the same blurry line differ, so exact
matching finds zero overlap and emits the paragraph twice, per frame.

---

# Decisions

## D015-1 — A sibling to dayflow, not a mode of it

**Decision.** A separate capability with its own surface. Dayflow's
`MIN_INTERVAL_SECONDS = 10`, `MIN_SEGMENT_SECONDS = 300` and budget admission
are left untouched.

**Why.** Those floors exist so an all-day session cannot starve itself; the
admission check that would refuse 3,600 calls for one segment is the guard
working correctly, not an obstacle. Loosening them to fit a 50-minute
transcription would remove the protection from the eight-hour case that needs it.
The two workloads have opposite shapes: dayflow is sparse-capture over a long
horizon, this is dense-capture over a short one.

**Shared, not duplicated:** the vision seam, `crop_regions`,
`regions::reading_order`, and the text-merge machinery.

## D015-2 — Record first, process offline

**Decision.** Recording and reading are separate phases. The recording is the
durable artifact.

**Why.** Recording measured at ~172 kbps / ~0% CPU (M-source), so capturing at a
high frame rate costs almost nothing. Decoupling removes the real-time budget
entirely: nothing is dropped for want of keeping up, and the recording can be
**re-processed** with a different model, different regions or different
thresholds. Live sampling permanently discards whatever it did not sample.

This also dissolves a question that looked hard — "can the gate keep up at 2 fps"
— by making it not apply.

## D015-3 — Sharpness gates the expensive stage (from M4)

**Decision.** Frames are scored by a focus measure **before** any model call.
Where several frames show the same content, the sharpest is read. Frames below
the sharpness floor are not read at all.

**Why.** M4 is unambiguous: sharp frames (1,443–1,458) all read cleanly; blurred
frames (396–507) **all** failed, without exception. The blur is motion blur from
scrolling. A focus measure costs no model call, so the cheapest stage decides
what the most expensive stage spends its time on.

**The consequence that is not obvious:** a higher frame rate is not for capturing
more *content* — it is for capturing a **sharp instance** of the same content.
This inverts the usual reason to raise a frame rate, and it is only affordable
because of D015-2.

## D015-4 — Unreadability is detected by ENTROPY, never by length (from M3)

**Decision.** A reading is rejected when its information content collapses —
compression ratio and unique-line ratio — not when it exceeds a character count.

**Why.** M3 measured the two populations: unique-lines 0.955 vs 0.003 (300×),
compression ratio 0.610 vs 0.0072 (85×). Length separates them by only 25×, and
worse, **length is a legitimate property of the content**: a dense page of code
is genuinely long, so a character ceiling would truncate real material. No
legitimate text has 0.3% unique lines.

These are two regimes, not a threshold to tune.

## D015-5 — A rejected reading is VISIBLE (from M2)

**Decision.** Rejected readings are counted and reported per transcript. They are
never silently dropped, and never merged.

**Why.** M2 measured 24% of frames failing on real material — this is a normal
operating condition, not an anomaly. A transcript that quietly omits a quarter of
the screens is worse than one that says so: the reader cannot tell what is
missing. This is `samples_read_whole` from feature 014, applied to a new stage:
a degradation that fails open must be counted, or it is invisible.

## D015-6 — Merging must be fuzzy, not exact (from M5)

**Decision.** Overlap detection uses normalised similarity per line, not
equality.

**Why.** `DAYFLOW_LIMITATIONS.md` already records this: *"Content-merge coverage
is exact trimmed-line equality, so OCR that perturbs most lines per sample will
fragment a document… Recorded as a known accepted limit (R24) — **untestable
without real OCR pairs**."*

The pairs now exist (M2, M4). Two readings of the same imperfect line differ, so
exact matching finds zero overlap and emits the paragraph again for every frame
that showed it. This is why `merge_scroll` was never wired: it could not work on
real reading output, and nobody had real reading output to prove it.

## D015-7 — Pixel deduplication is a knob, not a constant (from M1)

**Decision.** The near-duplicate threshold is configurable, with a default tuned
for mixed content.

**Why.** M1 measured the same clip keeping 285, 138 or 2 frames depending on
threshold. Scrolling content genuinely changes every frame and **should not** be
deduplicated — that change is the material. Slide-based content collapses
enormously. How much of a recording is new is a property of the content, so it
cannot be a constant.

## D015-8 — Close three orphans rather than write a fourth

**Decision.** This feature's implementation consumes the existing
`coverage` / `merge_scroll` / `TextAggregator` machinery and the `Content`
intent, rather than reimplementing them.

**Why.** All three are built, unit-tested, and have **no production caller**.
Feature 014 found nine such orphans across eleven waves; the dominant defect in
this codebase is code that is green and inert. Writing a fourth implementation of
text merging beside three unused ones would be the same mistake with a new name.

**Also to fix, in `analysis::ocr::ocr_video`:** the hard `-frames:v 20` cap (the
actual blocker for long material), deduplication happening *after* the cost has
been paid rather than before, and a reading path that bypasses the shared vision
seam.

## D015-9 — Readers are swappable, and each owns its own output normalisation

**Decision.** A **reader adapter** layer sits between the primitives and the
`VisionProvider` seam. One adapter per reading model. Each owns three things:

1. the **prompt** that model responds best to,
2. the **normalisation** of that model's raw response down to plain text,
3. a declaration of its **quirks**, so a caller can tell readers apart.

Selecting a different reader is a configuration choice, not a code change.

**Why this is not optional.** No two reading models return the same shape. Some
emit plain text; some wrap it in markdown fences; some return JSON; some emit a
chain-of-thought preamble first. This repo already measured the last case —
`strip_reasoning`'s own doc records, from 2026-08-23 against `ornith-1.5-9b`:

> *"roughly 60% of `analysis_text` was reasoning noise ('Let me carefully
> transcribe… Actually, let me re-read…') ahead of the real transcription."*

That fix was written ad-hoc, for one quirk, in one provider. Feature 015 makes
the problem structural rather than incidental, because **two of its three
primitives silently depend on output shape**:

| Primitive | What un-normalised output does to it |
|---|---|
| Information content (D015-4) | A thinking preamble is high-entropy PROSE. It **masks a degenerate reading** — the guard scores the deliberation, not the transcription, and a broken reading passes as healthy. |
| Fuzzy merge (D015-6) | Matching is per LINE. Markdown fences, JSON wrapping or a `<think>` block break line alignment, so genuine overlaps stop being recognised and material repeats. |

So a reader that is not normalised does not merely produce untidy output — it
**defeats both guards this feature is built on**.

**Why an adapter, and not a method on `VisionProvider`.** T020 faced the mirror
of this question for `keep_alive` and answered it the other way, for a reason
that inverts cleanly:

- `keep_alive` had to modify the **request body**, which a wrapper cannot reach
  without re-implementing the provider's transport — so it went ON the shared
  trait as a defaulted method.
- Normalisation operates on the **response**, which a wrapper reaches trivially.

A wrapper is therefore correct here, and it keeps a reading-specific concern out
of a trait with five implementors, four of which have no use for it. Same
principle, opposite conclusion, because the direction of the data differs.

**What a reader must guarantee:**

- **Never return empty when the model said something.** A response that is
  entirely preamble is passed through rather than normalised to nothing — the
  existing `strip_reasoning` already takes this care, and it is the same rule as
  "a failed read must never look like an empty result".
- **Normalisation is reported, not hidden.** A caller can see that 60% of a
  response was stripped. A silent 60% reduction is indistinguishable from a model
  that simply said less.
- **Idempotent.** Normalising twice changes nothing, so a caller cannot corrupt
  text by handling it carefully.

**Consequence for comparing runs.** Scores and merges are only comparable across
frames read by the **same** reader. A transcript records which reader produced
it; mixing readers within one document is a caller decision, and the record makes
it visible rather than assumed.
