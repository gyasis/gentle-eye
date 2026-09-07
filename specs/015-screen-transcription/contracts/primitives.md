# Contract — the three primitives

Each primitive answers ONE question and decides nothing. The caller — an agent,
a playbook, a person — chains them and owns every threshold.

**The rule that makes this work:** a primitive never silently substitutes a
judgement for an answer. If it cannot answer, it says so.

## 1. Frames with sharpness

> "Which frames are there, and how legible is each?"

**Takes**: a recording; a rate; a near-duplicate threshold; an output directory.
**Gives**: one row per kept frame — index, timestamp, path, **sharpness score**.

- **No fixed cap.** A recording longer than any built-in limit must not be
  silently truncated; that cap is the current blocker.
- **The dedup threshold is the caller's.** M1 measured the same clip keeping 285,
  138 or 2 frames across thresholds, because scrolling text genuinely changes
  every frame and slides do not. The tool must not pick for the caller.
- **Sharpness is comparable within a recording, not across recordings.** It is a
  focus measure, not a calibrated scale — a caller compares frames of the same
  source, and the contract says so rather than implying an absolute meaning.
- **ffmpeg absent → a stated failure** naming what is missing. Never an empty
  frame list, which would read as "the recording has no frames".

## 2. Information content

> "Is this text real content, or a reader that has broken down?"

**Takes**: text.
**Gives**: compression ratio, unique-line ratio, unique-token ratio.

- **Never uses length.** M3: length separates the populations by 25×, but a dense
  page of code is legitimately long, so a ceiling truncates real material.
  Compression separates by 85×, unique-lines by 300×.
- **Returns scores, not a verdict.** The reject threshold is the caller's. The
  measured populations are far enough apart that any sane threshold works, and
  that is exactly why the tool need not choose one.
- **Empty text is not a failure** — it scores as empty, and the caller decides
  whether an empty screen is expected.

## 3. Fuzzy merge

> "These two readings overlap. What is the one document?"

**Takes**: two readings; a similarity threshold.
**Gives**: the merged text.

- **Similarity, never equality.** Two readings of the same imperfect line differ.
  Exact matching finds zero overlap and emits the paragraph once per frame that
  showed it — the failure recorded in `DAYFLOW_LIMITATIONS.md` as untestable
  "without real OCR pairs".
- **Containment is not growth.** A reading wholly present in what came before
  must not extend the document.
- **No overlap loses nothing.** Two unrelated readings both survive; the merge
  never drops material to make a join look clean.
- **This IS `coverage` / `merge_scroll` / `TextAggregator`**, made reachable and
  made tolerant. A second implementation beside three unused ones is the defect
  this feature exists to close.

## What every primitive owes the caller

- **Machine-readable output**, so an agent on any harness can chain them with
  only a shell — no integration, no MCP registration required.
- **A stated failure, never a silent one.** An empty result must never be
  ambiguous between "nothing was there" and "I could not look" — the same rule
  as R-DR4d and `samples_read_whole`.
- **Determinism.** Same input, same output. The judgement lives with the caller,
  so the tool has none to vary.

## What they deliberately do NOT do

- Choose a threshold.
- Decide whether a transcript is good enough.
- Orchestrate. That is the playbook's job, and issue #17's.
