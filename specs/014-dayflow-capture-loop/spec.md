# Feature Specification: Dayflow Capture Loop with Pluggable Sources

**Feature Branch**: `014-dayflow-capture-loop`
**Created**: 2026-08-29
**Status**: Draft
**Input**: The daemon that actually drives the Dayflow pipeline, a capture SOURCE
abstraction so Dayflow can watch one specific thing rather than only whole
screens, and a real answerer for `ask_day`.

## Context — what this continues

Feature 013 (`specs/013-dayflow-perception-waves`) is complete: 53/53 tasks, 576
tests, clippy zero, one live end-to-end run passed. It built every stage of the
pipeline — window lifecycle and segmentation, the content gate, idle detection,
the drop/skip taxonomy, the summarisation scheduler, the two-tier perception
ladder, the structural timeline, tiered retention, the standup digest, and three
surfaces over one engine.

What it did not build is the thing that **runs** them. Every one of the eight
limitations in `docs/DAYFLOW_LIMITATIONS.md` traces to that single absence, and
they close together rather than one at a time.

This feature does not supersede 013. It completes it.

## User Scenarios & Testing *(mandatory)*

### User Story 1 — Dayflow runs by itself, all day (Priority: P1)

I start Dayflow in the morning and stop it in the evening. In between, nobody
drives it: it samples on its cadence, closes segments, summarises them, writes
the timeline, keeps the model warm when that is worth doing, and reclaims disk
as it goes. When I ask what I did at 2pm, the answer is there because the
recorder was running, not because I remembered to poke it.

**Why this priority**: without this there is no product. Every other capability
already exists and is tested; none of them execute on their own.

**Independent Test**: start a session, leave it for longer than several
segments, stop it — and find timeline entries covering that period, written
while it ran rather than at the end.

### User Story 2 — Dayflow TAKES AN INPUT or CONSUMES A DISPLAY (Priority: P1)

I point Dayflow at a source and it tracks only that. A source is one of two
co-equal things:

- **an input Dayflow takes** — a stream, a feed, a capture device: an RTSP or
  SRT stream, a capture card carrying another machine's HDMI, a camera. The
  content need not be on my screen at all, or even on this machine.
- **a display Dayflow consumes** — one or more screens, or something on one: a
  named window, a defined target/region.

My QA work watches one application through the same flows repeatedly, or watches
a device under test through a capture card. My agent work watches an AI coding
agent's terminal. In every case the rest of my screen is noise: it costs
perception budget, it pollutes the summary, and it puts unrelated content into a
record I may share.

**Why this priority**: co-equal with US1. It is the difference between a
whole-desktop activity log and a tool that can answer "what happened in THIS
thing" — which is where the proven value is, and a broader use than main-screen
capture.

**Independent Test**: run one session against an input (a stream) and one
against a display source (a single window). Confirm each produces entries
describing only that source, that a change elsewhere produces no sample, and
that the input session records content that was never on this machine's screen.

### User Story 3 — Ask a real question and get a real answer (Priority: P2)

I ask "what was I doing at 2pm" and get an answer grounded in what was recorded
— not the prompt that would have produced one.

**Why this priority**: the grounding rules, the refusal path, and the surfaces
are all built and tested; only the answerer is a placeholder. Small, and the
most visible gap to anyone reading the feature list.

**Independent Test**: seed a range with entries, ask a question through each
surface, and get prose that names what those entries contain; ask about an empty
range and still get the refusal, with no model consulted.

### User Story 4 — The record survives a restart (Priority: P2)

My machine sleeps, or the daemon is restarted, or I run the CLI in a new
terminal. The session is still there, `status` reports it, and the timeline has
no invented gap where the interruption was.

**Why this priority**: 013 built durable daemon state and a resume decision, and
the CLI's session verbs currently cannot span processes. This makes the
already-built state authoritative.

**Independent Test**: start a session, restart the process, run `status` from a
fresh invocation, and see the same session — with any interruption recorded as a
gap with its cause, not as absence.

### Edge Cases

- The source disappears mid-session — the window is closed, the stream drops,
  the display is unplugged. Capture must record a gap with its cause and
  continue, not stop the session or silently write nothing.
- The source is occluded or minimised. This is not the same as the source being
  gone, and must not be recorded as one.
- Two sources are configured and one fails. The healthy one keeps recording.
- The machine sleeps mid-segment. The segment must close honestly rather than
  stretching across the sleep.
- A perception backend is unreachable for an hour. Segments queue and retry;
  nothing is marked summarised that was not summarised; nothing is reclaimed.
- The disk fills while unsummarised segments exist. Capture must stop and say
  so, rather than reclaiming the only record of a period.

## Requirements *(mandatory)*

### Functional Requirements

**The loop**

- **FR-101**: A running session MUST sample on its configured cadence without
  external prompting, and continue across segment boundaries.
- **FR-102**: Each sample MUST be recorded with the regions detected at the
  moment of capture, so the perception ladder can crop before extracting rather
  than reading a whole frame.
- **FR-103**: The absence of region data MUST be visible in the session's own
  reporting, not merely survivable. A degraded reading that no one can detect is
  the failure this requirement exists to prevent.
- **FR-104**: Closed segments MUST be summarised while the session continues,
  and entries MUST appear as the day proceeds.
- **FR-105**: Retention MUST run on a schedule during a session, subject
  unchanged to the rule that nothing unsummarised is ever reclaimed.
- **FR-106**: Every timeline entry MUST carry the provenance of the regions its
  text came from.
- **FR-107**: The session MUST hold the perception tier warm when the configured
  policy asks for it, and MUST NOT when it does not.
- **FR-108**: A session MUST survive process restart, and MUST be visible and
  controllable from a surface invoked in a different process.

**The source**

- **FR-110**: A session MUST accept a capture SOURCE, which is either an INPUT
  Dayflow takes or a DISPLAY it consumes. These are co-equal: neither is the
  default that the other is an exception to.
  - **Inputs taken**: a stream (RTSP, RTMP, SRT, HTTP), a capture device or
    card, a camera. The content may never appear on this machine's screen.
  - **Displays consumed**: one or more whole displays, a named window, or a
    defined target/region on a display.
- **FR-111**: Displays MUST remain ONE source kind, not the privileged case —
  the existing multi-display behaviour is preserved exactly, as one option among
  several rather than as the shape everything else is fitted into.
- **FR-111a**: A source kind MUST be addable without changing the loop. The loop
  asks a source for a frame; how that frame is obtained is the source's business.
- **FR-112**: A source MUST be identified durably enough to survive the thing
  moving or being re-created. A window that is dragged or reopened is the same
  source; identity by screen position alone is not sufficient.
- **FR-113**: When a source becomes unavailable, the session MUST record a gap
  with its cause and continue. Unavailable, occluded, and ended are distinct and
  MUST NOT be conflated.
- **FR-114**: Captured content MUST be confined to the source. Nothing outside
  it may reach a sample, a summary, or the timeline.
- **FR-115**: A session's status MUST name its source, so a reader can tell what
  the record is a record OF.

**The answer**

- **FR-120**: `ask_day` MUST return a model-generated answer grounded strictly on
  the retrieved entries.
- **FR-121**: The existing refusal MUST be unchanged: an empty range answers
  "no record" and consults no model.
- **FR-122**: An answer MUST carry its grounding, so confident prose with no
  supporting entries is detectable.
- **FR-123**: All three surfaces MUST return the same answer to the same
  question over the same range.

### Carried-forward constraints (locked, from 013)

These are not re-litigated. They constrain every requirement above.

- Dayflow **samples**; it does not record. Video stays optional and default off.
- Two granularities: coarse all-day (default one frame per three minutes) and
  focused session (one frame per minute). Segments operate in a 10–15 minute
  range with a **five minute hard floor**, scoped to Dayflow alone.
- Activity and Content intents, either/or, Activity by default.
- Every gate **fails open**: on any error the sample is KEPT, because Dayflow
  cannot re-capture yesterday.
- Eviction is gated on `summarized`, never on age and never on budget pressure.
- Reading order is geometry, never a model.
- The perception budget is derived from cadence, source count and region cap, and
  a segment whose burst cannot fit MUST be refused up front — a segment that
  fails mid-burst is requeued and would otherwise retry forever, spending the
  whole budget on every attempt.

### Key Entities

- **Capture source** — what a session watches, and the feature's central
  abstraction. Either an INPUT taken (stream, capture device, camera) or a
  DISPLAY consumed (display set, window, target/region). Has a durable identity
  and an availability state. The loop knows only that a source yields frames.
- **Session** — a run of the loop over one source, durable across restart.
- **Sample** — one captured frame plus the regions detected with it.
- **Gap** — a period with no recording, carrying its cause.

## Success Criteria *(mandatory)*

- **SC-101**: An eight-hour unattended session produces timeline entries
  covering the whole period, with every gap accounted for by a recorded cause.
- **SC-102**: Entries appear while the session runs — a question asked at 2:30
  about 2:00 is answerable without stopping.
- **SC-103**: A session watching a single window produces no content from
  outside it, verified by inspection of the stored text.
- **SC-103a**: A session taking an INPUT records content that was never rendered
  on this machine's display, proving the source abstraction is real rather than
  a filter over screen capture.
- **SC-104**: A source that disappears and returns produces a gap with its cause
  and then resumes, without ending the session.
- **SC-105**: A session survives a process restart with its identity intact and
  is controllable from a new invocation.
- **SC-106**: Disk use over a full day stays within the configured budget while
  no unsummarised segment is ever reclaimed.
- **SC-107**: A question over a range with records returns prose naming what
  those records contain; over an empty range it returns the refusal.
- **SC-108**: Region-cropped extraction is demonstrably in use — the count of
  samples read as whole frames for lack of regions is reported and is zero in a
  healthy session.

## Assumptions

- The existing region cascade can supply regions at capture time for a display
  or window source. If a source kind cannot produce regions, the loop reads the
  whole frame and reports it (FR-103), rather than failing.
- One session watches one source. Watching several things at once is served by
  the display-set kind or by future work, not by multiple concurrent sessions.
- The answerer uses the same locally-governed model path the perception ladder
  uses; no new external dependency.
- Input sources are best-effort: their availability semantics belong to the
  input, and a dropped stream or an unplugged capture device is an
  unavailability (a gap with a cause), not an error that ends the session.
- An input source may have no region cascade available — nothing on it is a
  window in this machine's window manager. Such a session reads whole frames and
  reports that it is doing so (FR-103), rather than pretending to crop.

## Out of Scope

- Real-time video recording — gentle-eye already does that, and Dayflow
  deliberately does not.
- Cross-machine sessions or any network-visible surface beyond the existing
  loopback one.
- Editing or correcting the timeline after the fact.
