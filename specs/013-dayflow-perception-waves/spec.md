# Feature Specification: Dayflow — Continuous Screen-Activity Timeline

**Feature Branch**: `013-dayflow-perception-waves`
**Created**: 2026-08-23
**Status**: Draft
**Input**: User description: "Dayflow: continuous screen-activity timeline for gentle-eye, resumed mid-build. Supersedes specs/002-dayflow-mode/tasks.md (Amendment A). Carries forward 13 completed tasks; remaining work is the ffmpeg segment muxer, the two-tier perception ladder (deepseek-ocr base + ornith VLM escalation via the Atelier governor), structural region-joined timeline, real-time scheduler, session+daemon record models with provable liveness, tiered retention, and MCP/CLI/HTTP surfaces."

## Scope & Prior Art *(supersession notice)*

This feature **supersedes** `specs/002-dayflow-mode/tasks.md` (originally specced 2026-05-28,
amended 2026-08-23 as "Amendment A"). It is not a restart: it carries the completed work
forward and re-specs only what remains.

**Authority**: PRD `dayflow_waves_continuance_2026-08-23` (sub-PRD of
`gentle_eye_dayflow_mode_2026-05-28`).

### Already delivered (do not rebuild)

Verified on `main` 2026-08-23: ~581 lines of implementation and 7 passing tests
(chunking 4, summarizer 2, timeline 1). **13 of 39** planned tasks are complete.

| Delivered | Evidence |
|---|---|
| Data model, error type, config surface (`T200`–`T203`) | module tree resolves; `DayflowConfig` round-trips the loader |
| Duration-aware capture-rate heuristic (`T210`) | unit test; `docs/FPS_AND_DAYFLOW.md` |
| Chunk manifest + memory-pressure path (`T221`, `T222`) | 3-chunk boundary test, monotonic non-overlapping ranges |
| Map-Reduce chunk summarizer with rolling context (`T230`–`T233`) | 4 tests incl. end-to-end over a stub provider |
| Timeline table + store round-trip (`T240`, `T241`) | in-memory insert → ordered `query_range` |

### Remaining (the subject of this spec)

Segmented capture files, the two-tier perception ladder, region-joined structural entries,
the real-time scheduler and day-level Q&A, both record models with provable liveness,
tiered retention with a disk guard, the three front-end surfaces, categories and the
standup view, and the release gates.

### Superseded decisions

| # | Original (2026-05-28) | Amended (2026-08-23) |
|---|---|---|
| D2 | Default perception provider = a cloud video model; local = privacy fallback | **Two local tiers**: low-cost text extraction as the base, a richer visual model only for meaning. Cloud is opt-in, never the default. |
| D6 | *(new)* | Low-compute first — never spend a visual-reasoning model on a job text extraction can do. |
| D7 | *(new)* | Structure comes from the region cascade, text from extraction, joined on region identity. Reading order is **computed from geometry**, never requested from a model. |
| D8 | *(new)* | All model calls route through the governed local lane, never a raw model endpoint. |
| D9 | Implied continuous recording, segmented on the fly | **Dayflow SAMPLES; it does not stream video.** Periodic frame snapshots, not a continuously-fed encoder. Video output is **optional and off by default** — gentle-eye already provides video recording; dayflow's artifact is the timeline. |
| D10 | *(new)* | **Two granularities, matching the two record modes.** All-day (daemon) tracking is the COARSE one — one frame every 3 minutes by default. A bounded focused session ("track my dev work for this hour") is the fine one — one frame a minute. Unattended must be the cheap mode. |
| D11 | *(new)* | **Delta-skip.** A sample whose regions are unchanged from the previous is not perceived. Reading is most of a working day, so this drives steady-state cost toward zero. |
| D12 | *(new)* | **Two intents, either/or, BOTH shipped in this feature.** `Activity` (default) answers "what was I doing" — frames are scaffolding, the summary is the artifact. `Content` captures the material on screen verbatim and merged — a lesson, an exam, a reference session — where the collected text IS the deliverable. Neither is a degraded form of the other and neither is deferred. |

D1 (native map-reduce summarizer), D3 (both record models), D4 (real-time summarization)
and D5 (three-tier retention) are **unchanged and still binding**.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Unattended all-day capture I can trust (Priority: P1)

A knowledge worker starts Dayflow in the morning and leaves it alone. It records the screen
continuously for the whole working day, breaking the recording into fixed-length segments as
it goes rather than holding one enormous file. At any moment they can ask whether it is
actually working — and get an answer grounded in what has been **produced**, not merely in
whether a process is alive.

**Why this priority**: Every later story reads the segments this story writes. It is also the
feature's single largest failure risk: a recorder that reports "healthy" while writing nothing
looks correct until the next morning, when a whole day is gone. Nothing downstream is testable
against real data until segments exist.

**Independent Test**: Run a capture with a short segment interval, confirm sequential segment
files appear on disk at the configured boundary with correct wall-clock ranges, then confirm
status reports a segment count and a last-segment timestamp that advance. Separately, force a
capture that produces nothing and confirm status is distinguishably degraded.

**Acceptance Scenarios**:

1. **Given** a configured segment length, **When** a capture runs for longer than three
   segment intervals, **Then** three sequential segment files exist with monotonic,
   non-overlapping wall-clock start/end times.
2. **Given** a running capture, **When** the caller asks for status, **Then** the response
   includes segments-written, last-segment time, and last-summary time.
3. **Given** a capture that is running but writing nothing, **When** the caller asks for
   status, **Then** the response is distinguishable from a healthy one without inspecting
   the filesystem.
4. **Given** an active capture, **When** it is stopped, **Then** the in-progress segment is
   closed and accounted for rather than discarded.
5. **Given** an active capture, **When** the screen locks or the user goes idle, **Then**
   capture pauses, no segments are written for that interval, and status reports paused —
   not degraded.
6. **Given** a paused capture, **When** the user returns, **Then** capture resumes on its own
   and the timeline shows an explicit gap for the paused interval.
7. **Given** two attached displays, **When** a capture runs, **Then** both are recorded and
   their entries land on one timeline, each identifying its source display.
8. **Given** a capture the user turned off at midday, **When** they turn it back on that
   afternoon, **Then** the afternoon's entries join the same day's timeline and the off
   interval reads as a gap.
9. **Given** a session configured at 30-minute segments, **When** it runs, **Then** segments
   close on 30-minute boundaries and entries carry 30-minute ranges.
10. **Given** a day already recorded at 15-minute segments, **When** the interval is changed
    to 30, **Then** the existing entries are untouched and only subsequent segments use the
    new length.

---

### User Story 2 - Ask what I was doing (Priority: P1)

At the end of the afternoon the user asks "what was I doing at 2pm?" or "what did I work on
today?" and gets a grounded answer drawn from the day's own recorded activity, with times.
Entries appear **as the day proceeds** — the user does not have to stop the recording to see
them.

**Why this priority**: This is the feature's reason to exist. It is the first slice that
returns user-visible value, and it closes the loop on the summarizer and store already built.

**Independent Test**: Run a session against a stub perception provider with a fast clock;
confirm timeline entries appear before the session is stopped, then ask a day-level question
and confirm the answer cites entries that exist in the store.

**Acceptance Scenarios**:

1. **Given** an active session, **When** a segment closes, **Then** a timeline entry for that
   segment's time range is written without waiting for the session to end.
2. **Given** a day with recorded entries, **When** the user asks about a specific time,
   **Then** the answer is grounded in entries whose time range covers that time.
3. **Given** a day with no recorded entries for a queried range, **When** the user asks about
   it, **Then** the system says it has no record rather than inventing one.
4. **Given** a queried range, **When** entries are returned, **Then** they are ordered by
   start time and carry app, activity, category and summary.

---

### User Story 3 - Affordable, private perception (Priority: P2)

Reading the screen must be cheap enough to sustain all day and private by default. Screen text
is extracted with a low-cost local text tier; a heavier visual-reasoning model is spent only
when the question is about **meaning** rather than text — and every such escalation is
explicit and logged. Nothing leaves the machine unless the user opts in.

**Why this priority**: Throughput, not intelligence, is the binding constraint at all-day
cadence, and an all-day screen recorder is the most privacy-sensitive surface in the toolkit.
The default path must be the cheap local one; the expensive one must be a deliberate exception.

**Independent Test**: Issue a text-extraction request and confirm it is served without loading
the visual-reasoning tier; issue a semantic request and confirm exactly one logged escalation.

**Acceptance Scenarios**:

1. **Given** a request for on-screen text, **When** it is served, **Then** the visual-reasoning
   tier is not invoked and the result returns within the warm-path budget.
2. **Given** a request that requires interpretation ("what was happening here"), **When** it is
   served, **Then** exactly one escalation occurs and is recorded in the log.
3. **Given** a frame containing two side-by-side panes, **When** text is extracted, **Then**
   each pane's text is returned in its own correct reading order and the panes are not
   interleaved.
4. **Given** default configuration, **When** any perception call is made, **Then** no request
   leaves the local network.

**Measured evidence** (same live 1920×1080 frame, 2026-08-23):

| path | warm latency | tokens | outcome |
|---|---|---|---|
| generic OCR utility | instant | n/a | garbled; ~half unusable on dark-theme terminal text |
| text tier, full frame | 2.6s (cold 10.3s) | 444 | near-verbatim but columns scrambled across panes |
| text tier, **cropped pane** | **1.6s** | **231** | correct order and correct glyphs |
| visual-reasoning tier | 39s | 787 | verbatim, ~15× the cost |

Cropping won on latency, cost **and** accuracy simultaneously.

---

### User Story 4 - Entries that remember the layout (Priority: P2)

Cropping to a region gives correct text but throws away where that text was. A timeline entry
should carry its on-screen provenance — which region it came from, that region's position, and
its parent — so a later reader can reconstruct what the screen looked like, not just what words
were on it.

**Why this priority**: The storage schema is the expensive thing to reverse later, so it must
carry geometry from day one even while only text is populated. Deferring this forces a
migration over historical data.

**Independent Test**: Capture a two-pane screen, then confirm the resulting entries' region
and parent references reconstruct the on-screen arrangement, with deterministic ordering.

**Acceptance Scenarios**:

1. **Given** an entry produced from a cropped region, **When** it is read back, **Then** it
   carries its region identity, bounding box and parent region.
2. **Given** a two-pane capture, **When** entries are ordered, **Then** the order is derived
   from geometry and is identical across repeated runs.
3. **Given** entries written before this capability existed, **When** the store is upgraded,
   **Then** those entries survive with empty provenance rather than being rewritten or lost.

---

### User Story 5 - Disk stays bounded (Priority: P3)

Recording all day would fill any disk. Raw video is scaffolding; the timeline is the permanent
artifact. Once a segment has been summarized its raw form is shrunk, and under disk pressure
the oldest scaffolding is discarded oldest-first — but the timeline itself is never touched.

**Why this priority**: Necessary before the feature can run unattended for more than a few
days, but the earlier stories are demonstrable without it.

**Independent Test**: Drive a summarize → shrink → over-budget → evict cycle and confirm total
bytes fall while every timeline entry remains queryable.

**Acceptance Scenarios**:

1. **Given** a segment that has been summarized, **When** the shrink step runs, **Then** the
   raw segment is replaced by a materially smaller artifact that retains its extracted text.
2. **Given** storage over its configured budget, **When** eviction runs, **Then** already-
   summarized raw segments are dropped oldest-first, then shrunk artifacts oldest-first.
3. **Given** any eviction, **When** it completes, **Then** every previously written timeline
   entry is still returned by a range query.
4. **Given** a segment that has **not** been summarized, **When** eviction runs, **Then** it
   is not discarded.

---

### User Story 6 - Reach it from anywhere (Priority: P3)

The same Dayflow engine is drivable from the agent tool surface, from the command line, and
over HTTP — start, stop, status, timeline, ask — with all three producing consistent results
because they share one engine.

**Why this priority**: Distribution, not capability. Valuable once the engine is correct, and
actively misleading if shipped before it is.

**Independent Test**: Drive start → status → timeline through each of the three surfaces
against the same underlying state and confirm consistent output.

**Acceptance Scenarios**:

1. **Given** any of the three surfaces, **When** start/stop/status/timeline/ask are invoked,
   **Then** each returns well-formed structured output.
2. **Given** a session started on one surface, **When** status is read from another, **Then**
   it reflects the same session.

---

### User Story 7 - Yesterday in one screen (Priority: P3)

The user opens a standup view and sees the previous day categorized — what they worked on,
for how long, and in what proportions — plus today's carry-over.

**Why this priority**: A presentation layer over data the earlier stories already produce.

**Independent Test**: Seed a day of entries, request the standup shape, and confirm a
categorized, time-ranged digest.

**Acceptance Scenarios**:

1. **Given** a day of entries, **When** the standup view is requested, **Then** each entry
   carries a category drawn from the fixed taxonomy.
2. **Given** the same day, **When** the user asks "what did I do today", **Then** the answer
   is a categorized, time-ranged digest rather than a flat list.

---

### Edge Cases

- **The silent daemon**: the recorder runs for hours and produces nothing (permission revoked,
  display asleep, encoder failure). Status must make this visibly distinct from healthy.
- **Cold model on every segment**: measured cold load is 10.3s against 2.6s warm. At a 15-minute
  cadence with idle-unload in force, *every* segment pays the cold cost unless residency is
  handled deliberately.
- **Perception backend unreachable**: capture must keep producing segments; the affected
  segments are marked unsummarized and retried rather than dropped or silently skipped.
- **Over budget while recording**: eviction must not stall or corrupt the in-progress segment.
- **Pause boundaries**: lock/sleep/idle mid-segment must close the in-progress segment cleanly
  rather than leaving a truncated file, and resume must open a new one — never splice across
  the gap into one entry that claims continuous activity.
- **Idle flapping**: brief inactivity followed immediately by activity must not thrash the
  recorder into a burst of tiny segments; the idle threshold needs hysteresis.
- **Display hot-plug**: a monitor attached or removed mid-session must not stall capture or
  orphan the segments already written for the display that went away.
- **Cost scales with displays**: with every display captured, per-segment perception work
  multiplies by display count and must still finish inside the segment interval (SC-004).
- **Clock discontinuity**: DST shift or manual clock change mid-session must not produce
  overlapping or negative-length entries.
- **Very small or nested regions**: a region too small to carry readable text must be skipped,
  not fed to the extractor as an empty crop.
- **Restart mid-day**: a stop/crash, or a deliberate off/on by the user, must resume onto the
  same day's timeline rather than starting a parallel one.
- **Interval changed mid-day**: a day containing both 15- and 30-minute segments must still
  query, order and total correctly — nothing may assume a uniform segment length.
- **Very long intervals**: a long interval (an hour or more) delays both the first timeline
  entry and the first liveness signal; the degraded-detection window (SC-006) is defined in
  segment intervals, not fixed minutes.
- **Full-frame numeric misreads**: a downscaled full-frame read disagreed with the cropped
  read on a digit. Numbers read from a downscaled full frame are not trustworthy.

## Requirements *(mandatory)*

### Functional Requirements

**Capture and segmentation**

- **FR-001**: System MUST **sample** a frame from every attached display at a configured
  interval, and group those samples into fixed-length windows. It MUST NOT stream a
  continuously-fed encoder for the duration of a recording — dayflow tracks activity, it does
  not record video (D9).
- **FR-001a**: The sampling interval MUST differ by record mode (D10): all-day daemon tracking
  defaults to one frame every 3 minutes; a bounded focused session defaults to one a minute.
  All-day MUST NOT be configurable finer than focused — the unattended mode has to be the cheap
  one.
- **FR-001b**: Sampling MUST NOT be configurable fast enough to constitute video recording; a
  floor of one frame per 10 seconds applies.
- **FR-001c**: A window MUST be able to contain at least two samples, so that it can show
  change. A configuration where it cannot MUST be rejected.
- **FR-001d**: Video output MUST be optional and default to OFF. When enabled it assembles
  sampled frames into a timelapse at window close, purely for human review; perception MUST
  read frames directly and never depend on a video artifact.
- **FR-001e**: System MUST skip perception for a sample whose regions are unchanged from the
  previous sample (D11), and this MUST be the default.

**Intent — both modes are required capabilities (D12)**

- **FR-036**: System MUST support two run intents, selected when a run starts and mutually
  exclusive: **Activity** (default) and **Content**. Both MUST be fully implemented; neither may
  ship as a stub or a flag on the other.
- **FR-037**: Under **Activity**, perception MUST extract only what is needed to characterize
  the activity (application, activity, category, summary). It MUST NOT run text aggregation or
  diff-merge, and stills MAY be discarded once their window is summarized.
- **FR-038**: Under **Content**, the system MUST preserve extracted text **verbatim**, MUST
  aggregate it across samples so a scrolling or edited pane reconstructs as one coherent block
  rather than many near-duplicates, and MUST retain stills until the material has been
  extracted.
- **FR-039**: Switching intent MUST NOT alter or re-interpret entries already recorded under the
  other intent; each entry carries the intent it was captured under.
- **FR-002**: System MUST record, for every segment, its index, storage location, and
  wall-clock start and end.
- **FR-003**: System MUST support both an explicit bounded session (with a configurable
  maximum duration) and an unbounded continuous daemon, and both MUST feed one shared
  segment → summarize → timeline pipeline.
- **FR-004**: System MUST persist daemon lifecycle state so that status survives the caller
  going away, and MUST stop cleanly on request.
- **FR-005**: System MUST close and account for the in-progress segment when a capture stops.
- **FR-029**: System MUST merge all displays into a **single** timeline, and every entry MUST
  identify which display it came from via its region provenance.
- **FR-030**: System MUST detect that the screen is locked, the display is asleep, or the user
  is idle, and MUST **pause** capture for the duration — writing no segments and sending no
  perception requests while paused.
- **FR-031**: System MUST resume capture automatically on the next user activity, without the
  user restarting the session or the daemon.
- **FR-032**: A paused interval MUST appear in the timeline as an explicit **gap** with a
  recorded pause and resume time, and MUST NOT be reported as degraded (FR-006) or as an
  activity entry.
- **FR-033**: Users MUST be able to turn capture off and back on at will, from any surface,
  at any time. Turning it back on the same day MUST rejoin that day's existing timeline
  rather than starting a parallel one, and the off interval MUST be recorded as a gap in the
  same way as an idle pause (FR-032).
- **FR-034**: The segment interval MUST be user-configurable and MUST NOT be hardcoded. The
  intended operating range is **10 to 15 minutes**, with a **hard floor of 5 minutes** and a
  sanity ceiling of 1 hour; the default is 15 minutes. The configured value applies to
  segmentation, to the summarization cadence, and to the liveness window in FR-006 alike.
- **FR-034a**: The floor exists because below it the per-segment perception cost (one pass per
  region per display) cannot keep pace with the cadence, and the timeline fills with fragments
  too short to describe an activity. Second-scale intervals MUST be rejected for a real
  recording.
- **FR-034b**: This interval constraint is **scoped to Dayflow** and MUST NOT gate the wider
  product. gentle-eye's core use is real-time and short-clip screen recording at 1–30 fps; a
  stale or nonsensical Dayflow interval MUST NOT be able to fail configuration loading, or
  block a recording, for a user who is not using Dayflow at all.
- **FR-035**: Changing the segment interval MUST take effect from the next segment boundary
  without discarding, rewriting or re-timing any timeline entry already recorded under the
  previous value. A day MAY therefore contain segments of differing lengths.

**Liveness and observability**

- **FR-006**: Status MUST report segments-written, last-segment time and last-summary time,
  such that "running" and "running but producing nothing" are distinguishable by a caller
  without inspecting the filesystem.
- **FR-007**: System MUST log every perception-tier escalation, including which tier served
  the request and why it escalated.
- **FR-008**: System MUST record per-segment processing latency, including any model reload,
  so that cadence overruns are measurable rather than inferred.

**Perception**

- **FR-009**: System MUST use a low-cost local text tier as the default for extracting
  on-screen text, and MUST NOT route text-only work through the visual-reasoning tier.
- **FR-010**: System MUST escalate to the visual-reasoning tier only for semantic or
  relational questions, and the escalation MUST be explicit rather than implicit.
- **FR-011**: System MUST feed the text tier full-resolution crops of regions, never a
  downscaled full frame.
- **FR-012**: System MUST route all model calls through the governed local lane, and MUST NOT
  send any perception request off the local network unless the user has explicitly opted in.
- **FR-013**: System MUST make model residency versus reload an explicit configured choice,
  not an emergent behaviour.

**Timeline**

- **FR-014**: System MUST summarize each closed segment while the session is still running and
  write its timeline entry immediately, not in a batch after the session ends.
- **FR-015**: Each timeline entry MUST carry a time range, application, activity, free-text
  summary, and a category from the fixed taxonomy.
- **FR-016**: Each chunk summary MUST receive the preceding chunk's rolling context so that
  narrative threads forward across segment boundaries.
- **FR-017**: System MUST support range queries returning entries ordered by start time, and
  MUST be safe against injection through query parameters.
- **FR-018**: Users MUST be able to ask a natural-language question about a day and receive an
  answer grounded strictly in stored entries, which states that it has no record when the
  queried range is empty.

**Structure**

- **FR-019**: Timeline entries MUST be able to carry region provenance — region identity,
  bounding box, and parent region — with all such fields optional.
- **FR-020**: Reading order MUST be computed from region geometry and MUST NOT be requested
  from any model.
- **FR-021**: Adding region provenance MUST be an additive, re-runnable change that preserves
  existing entries.

**Retention**

- **FR-022**: System MUST classify stored material into hot (raw), warm (shrunk) and cold
  (timeline-only) tiers based on age and whether it has been summarized.
- **FR-023**: After a segment is summarized the system MUST replace its raw form with a
  materially smaller artifact that retains the extracted text.
- **FR-024**: When storage exceeds its configured budget the system MUST evict summarized raw
  segments oldest-first, then shrunk artifacts oldest-first, and MUST NEVER evict timeline
  entries.
- **FR-025**: System MUST NOT evict any segment that has not yet been summarized.

**Surfaces**

- **FR-026**: Users MUST be able to start, stop, query status, read the timeline and ask a day
  question from the agent tool surface, the command line, and over HTTP.
- **FR-027**: All three surfaces MUST drive the same engine and report consistent state.
- **FR-028**: System MUST provide a standup view presenting a day categorized with time
  ranges and proportions.

### Key Entities

- **Segment**: one fixed-length slice of a recording — index, location, wall-clock start and
  end, and whether it has been summarized.
- **Chunk Summary**: the structured result of perceiving one segment, plus the rolling context
  it passes to the next segment.
- **Rolling Context**: the carry-forward narrative handed from one segment's summary to the
  next, so activity spanning a boundary reads as continuous.
- **Timeline Entry**: the permanent artifact — a time range with application, activity,
  category, summary, and optional region provenance. Survives every retention tier.
- **Region**: a bounded area of a display with a bounding box, a source display, and an
  optional parent — supplying both the crop fed to the text tier and the layout an entry
  remembers.
- **Session**: one bounded or continuous recording run with lifecycle state, liveness evidence,
  and a record of its paused intervals.
- **Retention Tier**: the hot/warm/cold classification governing what may be shrunk or evicted.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An unattended recording runs a full working day (8 hours) across every attached
  display and produces a timeline that is complete for all active periods, with paused
  intervals shown as explicit gaps.
- **SC-002**: A user asking about any past moment in the recorded day receives a grounded
  answer in under 10 seconds.
- **SC-003**: Text extraction on a cropped region returns in under 5 seconds on the warm path,
  and text-only work never invokes the visual-reasoning tier.
- **SC-004**: Per-segment processing completes inside the segment interval, so the timeline
  never falls further behind real time as the day proceeds.
- **SC-005**: Text extracted from a two-pane screen preserves each pane's reading order, with
  no interleaving between panes.
- **SC-006**: A recorder that is running but producing nothing is identified as degraded within
  two segment intervals — at whatever interval is configured — without a human inspecting the
  filesystem, and is never confused with a deliberate off or an idle pause.
- **SC-007**: Storage stays within its configured budget over a multi-day run while every
  timeline entry written during that run remains queryable.
- **SC-008**: Storage consumed per recorded day, after shrinking, is at most 10% of the raw
  captured volume.
- **SC-009**: With default settings, zero perception requests leave the local network, and zero
  segments or perception requests are produced while the screen is locked or the user is idle.
- **SC-010**: Start, stop, status, timeline and ask behave identically across all three
  surfaces on the same underlying session.

## Assumptions

- **Carry-forward is authoritative**: the 13 completed tasks listed under Scope & Prior Art are
  treated as done and correct. Their handles remain permanent; new work uses a fresh range.
- **Existing subsystems are reused, not rebuilt**: capture service, encoder, frame-rate
  heuristic, memory monitor, region cascade, perception provider abstraction, storage manager,
  and the three existing front-end surfaces already exist and are green.
- **Perception binding**: the text tier is currently bound to a compact local OCR model and the
  visual-reasoning tier to a larger local vision model, both reached through the governed local
  lane. These are configuration, not architecture — the tiering contract is what is specified.
- **Cloud remains available but opt-in**: a cloud video-native path may still be selected
  explicitly; it is never the default.
- **Structured-output capability is untested**: whether the text tier can emit layout directly
  (markdown or grounding boxes) has not been measured. The spec therefore requires geometry to
  be computed from regions; if the model proves capable, that becomes an optimization, not a
  redesign.
- **Residency default**: the text tier is assumed to be kept resident during an active
  recording, on the measured basis that a 15-minute cadence otherwise pays the cold-load cost
  on every segment.
- **Segment length**: default 15 minutes; intended operating range 10–15 minutes; permitted
  5 minutes to 1 hour; changeable mid-day (FR-034/FR-035). Nothing downstream may assume a
  uniform segment length. The floor is enforced by a Dayflow-scoped validator invoked when a
  session or daemon starts — deliberately NOT by the library-wide config validator (FR-034b).
- **Sampling rate**: all-day one frame every 3 minutes; focused session one a minute; floor of
  one per 10 s; ceiling one per hour. Expressed as an INTERVAL IN SECONDS rather than as fps,
  because a config reading `0.0056 fps` is unreadable and a typo there costs a factor of sixty.
  The library's existing duration-aware fps heuristic (0.2–0.5 fps for long recordings) serves
  general recording and is deliberately NOT reused here — dayflow overrides it with its own
  much coarser rate.
- **Cost consequence**: one frame across three displays measured 37.4 MiB of raw BGRA (T006), so
  the sample COUNT decides affordability. An 8-hour day at the default interval is 160 samples
  per display, versus 14,400 at 0.5 fps — roughly 90× less, before delta-skip.
- **Machine-local configuration**: the perception endpoint configuration lives outside the
  repository, so a fresh machine requires it to be recreated before live validation.
- **Display scope**: every attached display is captured and merged into one timeline
  (decided 2026-08-23). Per-segment cost therefore scales with display count, which SC-004
  bounds.
- **Idle policy**: capture pauses on lock/sleep/idle and resumes on activity (decided
  2026-08-23). A paused interval is a legitimate gap, not a fault — the liveness check in
  FR-006 must not confuse the two.
- **Idle threshold**: assumed a few minutes of no input, with hysteresis, configurable.
- **Single user, single machine**: no multi-tenancy, no sync between machines, no
  authentication layer beyond the loopback binding the existing surfaces already use.

## Dependencies

- The governed local model lane must be reachable for any non-stub perception path.
- The region cascade supplies the crops and the layout tree; no new detection work is in scope.
- A working video segmentation toolchain must be present on the host for real segment output.
