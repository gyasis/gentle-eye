# Feature Specification: Screen-Text Transcription

**Feature Branch**: `015-screen-transcription`
**Created**: 2026-08-31
**Status**: Draft
**Input**: Give an agent the primitives it needs to turn a recorded video of a
screen into a full markdown document — and let the agent own the orchestration.

**Scope decision (2026-08-31)**: this feature ships **deterministic primitives**
plus a **harness-agnostic playbook**, NOT a single end-to-end command. A
self-contained `transcribe` pipeline is filed as **issue #17** for later.

## What this delivers, and what it deliberately does not

The measurements (research.md, M1–M5) established a pipeline. They also
established that **its parameters are content-dependent**: M1 measured the same
clip keeping 285, 138 or 2 frames depending on one threshold, because how much of
a recording is genuinely new is a property of the material.

A judgement that must vary by content does not belong compiled into a binary.

| | Owner | Why |
|---|---|---|
| Frame extraction, sharpness score, entropy score, fuzzy merge | **the tool** | deterministic — same input, same answer; testable |
| Which thresholds, whether to crop, whether the output is good enough to accept | **the agent** | judgement, and content-dependent |

This is the same split feature 014 settled on for capture sources: the seam lives
in the tool, the policy lives with the caller. Specifying the policy into the
tool is what would make M1's finding unactionable.

## Why this exists

Long-form material displayed on a screen — a lesson, a training session, an
exam, a reference walkthrough — is currently unrecoverable as text. Dayflow can
tell you *what you were doing*; it cannot give you back *what was written*. For
this class of material **the material is the artifact**.

The content may also arrive as a video feed rather than a local screen: a
capture card, a stream, a second machine on an HDMI input.

## Why this is NOT a dayflow mode

Dayflow's floors are correct for an all-day recorder and wrong here:

| Dayflow constraint | Value | This feature needs |
|---|---|---|
| `MIN_INTERVAL_SECONDS` | 10 (0.1 fps) | 1–2 fps or better |
| `MIN_SEGMENT_SECONDS` | 300 | no segment concept |
| Budget admission | refuses `samples × regions + 1 > budget` | 300 × 12 = 3,600 calls, correctly refused |

Those guards stop an all-day session from starving itself. **They must not be
loosened.** This is a sibling that shares dayflow's components without inheriting
its cadence.

## The governing insight: record first, process offline

Recording is nearly free — measured at ~172 kbps and ~0% CPU for a stream copy.
Processing then has **no real-time budget**, which means:

- nothing is ever dropped for want of keeping up;
- the video is a durable artifact that can be **re-processed** with a different
  model, different regions, or different thresholds;
- frame rate becomes free, which matters for a reason that is not obvious
  (see FR-104).

Sampling live throws away whatever it did not sample. Recording keeps everything
and decides later.

## User Scenarios & Testing *(mandatory)*

### User Story 1 — An agent can find the readable frames (Priority: P1)

An agent has a recording. Before spending anything on reading, it needs to know
which frames are worth reading: which are distinct, and which are sharp enough to
be legible.

**Why this priority**: This is the finding that makes the whole pipeline
affordable (M4). Without it an agent pays to read frames that cannot be read.

**Independent Test**: Score a recording's frames and confirm the scores separate
the legible frames from the motion-blurred ones.

**Acceptance Scenarios**:
1. **Given** a recording, **When** an agent asks for its frames, **Then** it
   receives them with a sharpness score for each, and may set the rate and the
   near-duplicate threshold itself.
2. **Given** a set of frames showing the same content, **When** the agent asks
   which is clearest, **Then** the sharpest is identifiable without any model call.
3. **Given** a threshold the agent chooses, **When** it changes that threshold,
   **Then** the number of frames returned changes accordingly — the decision is
   the agent's, not the tool's.

### User Story 2 — An agent can tell a failed reading from a dense one (Priority: P1)

An agent has text recovered from a frame. It needs to know whether that text is
real content or a reader that has degenerated into repetition — and it cannot use
length, because dense material is legitimately long.

**Why this priority**: M2 measured 24% of frames failing on real material. This is
a normal operating condition; without this check a quarter of the transcript is
silently garbage.

**Independent Test**: Score a known-good reading and a known-degenerate one and
confirm the scores separate them.

**Acceptance Scenarios**:
1. **Given** any text, **When** an agent asks for its information content,
   **Then** it receives measures that do not depend on length.
2. **Given** a degenerate reading, **When** scored, **Then** it is
   distinguishable from dense legitimate text by a wide margin.

### User Story 3 — An agent can merge overlapping readings (Priority: P1)

An agent has readings from consecutive frames of a scrolled screen. It needs one
continuous document, not the same paragraph repeated once per frame.

**Why this priority**: Without it, scrolled material is unusable — and the
existing merge cannot do it, because it requires exact equality (M5).

**Independent Test**: Merge two readings of the same scrolled content that differ
in the way real readings differ, and confirm the shared portion appears once.

**Acceptance Scenarios**:
1. **Given** two readings whose overlapping lines are similar but not identical,
   **When** merged, **Then** the shared portion appears once.
2. **Given** a reading wholly contained in what came before, **When** merged,
   **Then** the document does not grow.
3. **Given** two readings with nothing in common, **When** merged, **Then**
   neither is lost.

### User Story 4 — Anyone can follow the pipeline without writing code (Priority: P2)

A user or an agent on any harness follows a written procedure that chains the
primitives into a transcript, choosing thresholds as the material demands.

**Why this priority**: The primitives are only useful if the way to combine them
is written down. This is the deliverable that makes the feature usable.

**Independent Test**: Follow the playbook end to end on a real recording and
obtain a transcript.

**Acceptance Scenarios**:
1. **Given** the playbook, **When** followed with only shell access, **Then** a
   transcript is produced without writing any code.
2. **Given** material of a different character, **When** the playbook is
   followed, **Then** it says which parameter to change and how to tell.

### Edge Cases

- A recording with no text at all — must produce an empty document that says so,
  not an error and not invented content.
- A screen held motionless for minutes — must not pay to read the same screen
  repeatedly.
- Continuous scrolling, where nearly every frame differs — must not discard
  genuine new material in the name of deduplication.
- A frame the reader cannot parse — must be counted and visible, never merged.
- ffmpeg absent — must fail with a stated reason.
- A recording longer than any fixed frame budget — must not silently truncate.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-101**: The tool MUST expose the frames of a recording with a **sharpness
  score** for each, at a caller-chosen rate, with no fixed cap on how much of the
  recording is examined.
- **FR-102**: The tool MUST expose a **near-duplicate threshold** the caller sets.
  It MUST NOT impose a single value, because how much of a recording is genuinely
  new is a property of the content (M1).
- **FR-103**: The tool MUST expose an **information-content score** for a piece of
  text, sufficient to distinguish a degenerate reading from dense legitimate
  content, and MUST NOT rely on length to do so (M3).
- **FR-104**: The tool MUST expose a **merge** that joins two readings on their
  overlap using similarity rather than equality, so that imperfect readings of the
  same line still merge (M5).
- **FR-105**: `analysis::ocr::ocr_video`'s fixed frame cap MUST be removed, so
  material longer than that cap is not silently truncated (FR-109 in the original
  framing).
- **FR-106**: Every model call MUST go through the project's single vision seam.
  No component may hold a private path to a model.
- **FR-107**: Each primitive MUST be usable independently, from the command line,
  returning machine-readable output — so an agent on ANY harness can chain them
  without an integration.
- **FR-108**: The existing `coverage`, `merge_scroll` and `TextAggregator`
  machinery MUST be the implementation the merge primitive uses. A second
  implementation MUST NOT be written beside them.
- **FR-109**: A written **playbook** MUST chain the primitives into a transcript,
  state which parameter to change for which kind of material, and say how to tell
  the output is wrong.
- **FR-110**: When a required external program is unavailable, the tool MUST fail
  with a message naming what is missing and what to install.
- **FR-111**: The primitives MUST be reachable from the command line and from the
  agent tool surface, consistently (the parity rule established in 014).

### Key Entities

- **Recording** — the durable source artifact. Everything else is derived and
  can be regenerated from it.
- **Distinct screen** — one state of the screen, however many frames captured it.
  The unit of reading, and of cost.
- **Reading** — the text recovered from one distinct screen, with a quality
  judgement attached (usable / rejected).
- **Transcript** — the merged document, plus the counts that say what it is and
  is not made of.

## Success Criteria *(mandatory)*

- **SC-101**: A user recovers the text of a recorded session as a readable
  document, without transcribing anything by hand.
- **SC-102**: Text that appeared once in a scrolled recording appears **once** in
  the output, not once per frame that showed it.
- **SC-103**: Text that appeared **only** in the recording — never on the
  transcribing machine's own screen — is recovered, proving the source is not
  limited to local screen capture.
- **SC-104**: A 50-minute recording is transcribable on local hardware with no
  per-call cost, and the number of readings is proportional to the number of
  distinct screens rather than to the number of frames.
- **SC-105**: Every reading rejected as unreadable is visible in the report; a
  reader can always tell how complete the transcript is.
- **SC-106**: A recording of a motionless screen costs one reading, not one per
  frame.
- **SC-107**: The same recording transcribed twice produces the same document.

## Assumptions

- The material is **text on a screen**, not natural-scene photography.
- The recording is legible to a human; this feature recovers text, it does not
  enhance an illegible source.
- Reading happens on local hardware with no per-call cost, so the design
  optimises for *number of readings* rather than for money per reading.
- The reading order within a frame is a geometric fact and is treated as such.
- Recordings are kept. Disk is cheaper than a lost session, and re-processing
  requires the source.

## Dependencies

- An external video tool for recording and frame extraction.
- A text-reading model reachable through the project's existing vision seam.
- The existing region cascade, for the optional region case only.

## Out of Scope

- **A single end-to-end `transcribe` command.** Filed as issue #17
  (gyasis/gentle-eye) with the measured basis attached. This feature deliberately gives the
  agent the pieces and lets it own the orchestration.

- Improving an illegible recording (upscaling, deblurring, super-resolution).
- Speech, audio, or subtitles — this is screen text.
- Live, real-time transcription. The design deliberately records first.
- Replacing dayflow. Dayflow answers *what was I doing*; this answers *what did
  it say*.
