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
- **FR-106a**: The reading model MUST be selectable, and each reading model MUST
  have an adapter that owns its prompt and normalises its response to plain text.
  Selecting a different reader MUST be a configuration choice, not a code change.
- **FR-106b**: Normalisation MUST be reported, not hidden — a caller MUST be able
  to see how much of a response was stripped. A silent reduction is
  indistinguishable from a model that said less.
- **FR-106c**: A transcript MUST record which reader produced it. Quality scores
  and merges are only comparable within one reader.
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

---

## Scope Amendment — 2026-09-06: stacking and audio alignment come IN

**Status**: amendment. The Out of Scope section above is left unedited on purpose —
the original decision and its reasoning stay readable, and this records what changed
and on what evidence. Two bullets are reversed:

> - Improving an illegible recording (upscaling, deblurring, super-resolution).
> - Speech, audio, or subtitles — this is screen text.

### The evidence that reversed them

Two real recordings were processed end-to-end on 2026-09-04/05 (a phone filming a
laptop during a working call) with an out-of-tree pipeline. Both exclusions failed
against real material.

**Super-resolution is not cosmetic — it decides whether text resolves at all.**
On recording 1, `))=CONCAT('HCC', G.hcc_gap)`, the surrounding table names, an error
banner and an open autocomplete list were all UNREADABLE in any single frame and
READABLE after a coherent multi-frame stack. Nothing else in the pipeline produced
them.

**Audio is not a companion to screen text — it is the fallback when the screen
fails, and screen text is the fallback when speech is vague.** Recording 2 was
filmed further back; its code never resolved even at 3x. Every finding from it came
from speech. Conversely, speech alone is full of unresolvable deixis — "this table
here", "that one", "go back down" — which only the frames disambiguate. Each channel
covers the other's blind spot, so a screen-text-only tool is undefined on exactly the
material this feature exists for.

**And the pairing carries information neither channel holds.** "Concat. I'm going to
do HCC. I think that'll work" is a plan; an autocomplete list open on `concat` with a
red squiggle on `G.hcc_gap` is a state; together they timestamp the moment a fix was
written. That correlation produced every substantive finding in both recordings.

### What this adds — three primitives, in the existing shape

Each still answers ONE question and decides nothing. The caller owns every threshold.

**4. Stack** — *"Given N frames of the same screen, what is the best single image?"*

**Takes**: frames of one region; a scale; a combine mode; a coherence tolerance.
**Gives**: the combined image, plus per-run scores — frames used, frames dropped for
movement, which frames were kept, maximum drift, a residual spread.

- **Never returns a verdict on its own output.** An earlier implementation returned
  `improved: bool`, and it was measured to be unfalsifiable: it sharpened the result
  and compared it against an unsharpened baseline, so 15 IDENTICAL frames — where
  stacking can recover nothing — scored HIGHER than a real burst and still reported
  success. Scores, not verdicts, per the contract.
- **Sharpness cannot detect the characteristic failure.** A stack whose frames have
  drifted ghosts, and a ghosted result scores WELL on any focus measure. A residual
  spread across the registered frames does move (measured 1.83 clean / 3.87 ghosted /
  6.76 for two frames 24px apart), so that is what the report carries.
- **Frames that show different content must be dropped, not averaged.** A burst
  spanning a scroll puts the same line at two heights. The naive check misses it:
  consecutive-frame difference stays small through a slow scroll while total drift is
  large. This is primitive 1's near-duplicate question asked for the opposite purpose
  — dedup keeps one frame per distinct screen; stacking wants every frame of ONE.
- **Refuses rather than degrades**: mixed frame sizes, non-8-bit-3-channel input, an
  out-of-range scale, or a burst too large for a memory budget are all stated
  failures. A 16-bit input previously saturated to an all-white image returned as
  success.
- **Cannot fix defocus.** Stacking recovers detail from sub-pixel jitter and rejects
  noise; it cannot recover information absent from every frame. One recording had an
  out-of-focus stretch that no processing made readable, and the tool must not imply
  otherwise.

**5. Align** — *"What was on screen when this was said?"*

**Takes**: a timestamped transcript; timestamped frames; a window tolerance.
**Gives**: one row per utterance — its text, the frames inside the window, their
sharpness.

- **The window is the caller's.** Speech leads action ("we're going to change this")
  or trails it, by seconds that vary per speaker.
- **Returns pairs, not conclusions.** Whether an utterance EXPLAINS a screen change is
  judgement, and belongs to the agent.
- **No pairing is not a failure.** Silence over a screen change, and speech over a
  static screen, are both real and must survive.
- **Alignment takes an existing transcript as INPUT — it does not produce one.**

**Where the audio line actually falls.** The engine may carry *simple* audio
perception, in the same sense it already carries a sharpness score: a deterministic
measurement that answers one question and decides nothing. Speech-present vs silence,
utterance boundaries, rough level — these are perception, they are cheap, and
`align` can use them to place utterances even when no transcript exists yet.

What does NOT belong here is **transcription**: turning speech into words, with
punctuation, casing, structure and domain-primed vocabulary. **VoxStruct already does
that**, it is the house tool for it, and a second ASR path in this repo would be
exactly the duplication primitive 3 exists to close. The analogy the repo already
lives by: measuring that a region contains dense text (a score) is perception;
producing the text (a reader) is a subsystem — and the same split applies to audio.

So: simple audio understanding MAY live in the engine; anything specific about
transcripts goes to VoxStruct, and its output arrives here as input.

**6. Locate** — *"Where is the screen in this frame, and are its corners actually
visible?"*

**Takes**: a frame; an area floor; a detection tolerance.
**Gives**: the quadrilateral; **which of its corners sit on the frame boundary**; and
how it was found — a clean convex 4-gon, or a bounding-box fallback.

- **Never rectifies on its own.** It reports; the caller decides whether to warp. A
  prior implementation both detected AND applied, and silently accepted a quad with a
  DUPLICATE corner — returning a flat grey slab as success. Splitting the question
  from the action is what makes that failure impossible to repeat.
- **Clipping is reported, not hidden.** A corner on the frame boundary means the
  screen extends past the frame and only a PARTIAL correction is possible. The caller
  must be able to see that rather than receive a confidently wrong warp.
- **A fallback detection is flagged as such.** A minimum-area bounding box is not the
  screen; it is what you get when no convex 4-gon was findable, and it must not be
  returned as though it were a real detection.
- **Not finding a screen is a stated answer**, never an empty one.
- **Brightness does not find a dark screen.** Otsu on a dark-themed editor filmed in a
  dark room locks onto the white laptop body and reflections. Text is dense
  high-frequency energy while a desk and bezel are smooth, so the measurement is
  texture, not luminance.

**Corner visibility is a RECORDING decision, and the spec should say so.** Measured on
the two reference recordings: **4 of 4 corners clipped on both**, and both fell back to
a bounding box. Filling the frame with the screen maximises pixel density — which is
what actually made small code readable — and destroys the corners rectification needs.
Backing up to include the bezel inverts that trade. Neither is wrong; the tool cannot
choose, and a caller who knows which one they made can skip `locate` entirely. This is
the same judgement/perception split as everywhere else: *skewed phone footage framed
wide* is worth locating; *a screen recording* never needs it; *phone footage that fills
the frame* cannot benefit no matter how good the detector is.

### Where every piece lives — the whole workflow, placed

`AGENTS.md` states the test: *"Would another IDE/tool want this?"* -> gentle-eye.
*"Is it about the coding-overlay experience / talking to it?"* -> the consumer.
The workflow that produced the evidence above, run through that test:

| what we built | perception or orchestration? | home |
|---|---|---|
| frame extraction + sharpness | perception | **015** (done) |
| information content | perception | **015** (stubbed) |
| fuzzy merge | perception | **015** (stubbed) |
| **stacking / super-resolution** | perception | **015** <- this amendment |
| **align: what was on screen when this was said** | perception | **015** <- this amendment |
| **locate: where is the screen, are its corners visible** | perception | **015** <- this amendment |
| pull off the phone over MTP | orchestration | the consumer |
| transcript-driven span selection | orchestration | the consumer |
| clip cutting, contact sheets | orchestration | the consumer |
| provenance-marked document assembly | orchestration | the consumer |
| which threshold, is this readable, does this utterance explain that change | **judgement** | the playbook / agent |
| ASR itself | neither — already solved | **VoxStruct** |

Nothing on that list is discarded; it splits three ways along a line this repo
already drew. The three-way split IS the architecture: perception is deterministic
and belongs in the engine, orchestration is a sequence and belongs to the caller,
judgement is content-dependent and belongs to whoever can see the content.

### Orchestration still does NOT move here

The scope decision at the top of this spec holds. Pulling a recording off a phone,
selecting which spans are worth processing, cutting clips, and assembling a document
are ORCHESTRATION and stay with the caller — that is also what gentle-eye's own
`AGENTS.md` boundary requires ("would another IDE want this?" → engine; the
coding-assistant loop → consumer). The out-of-tree pipeline that produced the evidence
above becomes a CONSUMER of these primitives rather than a parallel implementation.

Transcript-driven span selection deserves a specific mention because it is the step
that made the work tractable — regexing domain vocabulary over the transcript and
keeping only those spans cut a 26-minute recording to 9.7 minutes and 5.48 GB to
108 MB, before any expensive processing. It is orchestration and it stays with the
caller, but a playbook that omits it will be slow for no reason.

### Findings the playbook must carry (measured, not assumed)

- **Frontier VIDEO understanding FABRICATES on this material.** On oblique,
  low-contrast footage it invented column names and patient IDs present nowhere on
  screen, with no signal of uncertainty. Multi-image STILLS to the same model were
  reliable and correctly answered "not legible" when they were. Video models for
  orientation; frames for content; never quote a model's transcription of code nobody
  has looked at.
- **Ask a VLM for refusal explicitly** — "write [?] for unreadable text; an honest [?]
  is more useful than a plausible guess" — or it fills gaps with fiction.
- **A local 7B VLM (~12s/frame) is good for DESCRIPTION** (which panel is focused, is
  there an error, is text selected) and unreliable for small code text. It is
  scriptable, so it suits the bulk pass while a human or stronger model reads the
  moments the transcript flags.
- **Prime ASR with the domain vocabulary, and treat a near-miss as the term.**
  `HF_CHR_LUNG` came back as "chronic HFCR lung"; searching the literal acronym
  returned zero hits on a topic discussed for 26 minutes.
- **Corroborate against source.** On-screen values are a hypothesis until something
  independent agrees. The decisive reading in recording 1 was only a diagnosis once a
  library's own source confirmed the same structure.
