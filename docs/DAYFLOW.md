# Dayflow — the continuous screen-activity timeline

Dayflow watches the screen all day and turns what it sees into a queryable timeline: *"what
was I doing at 2pm?"* answered from the day's own record, with times. This document explains
what it is, how each part works, and — because this feature's whole history is "the obvious
approach measured wrong" — **why** each part works the way it does. The operational runbook
is [`DAYFLOW_OPERATIONS.md`](DAYFLOW_OPERATIONS.md); the honest list of what is *not* yet
wired is [`DAYFLOW_LIMITATIONS.md`](DAYFLOW_LIMITATIONS.md). The design record behind every
figure quoted here is `specs/013-dayflow-perception-waves/research.md` (R1–R40) and the
decision table D1–D12 in the spec.

## What it is, and what it is not

**Dayflow samples; it does not record video** (decision D9). gentle-eye already has
real-time video recording as its own feature. Dayflow exists to *track activity* cheaply
enough to run unattended for a whole working day, and it does that by grabbing a periodic
frame snapshot per display — not by streaming an encoder for eight hours.

The arithmetic that forced this: one frame across this machine's three displays measured
**37.4 MiB of raw BGRA** (research T006). An 8-hour day at the all-day default of one frame
every 3 minutes is **160 samples per display**; at the library's 0.5 fps timelapse rate it
would be **14,400** — roughly **90× more**, before delta-skip removes the unchanged ones.
The sample *count* is what decides affordability, so the design controls the count.

Consequences that follow from D9:

- There is no long-lived ffmpeg child. Video output exists but is **optional and off by
  default** (`dayflow.video.enabled = false`): when enabled, the stills collected for a
  window are assembled into a timelapse *once, at window close*, purely for human review.
  Perception always reads the frames directly and never depends on a video artifact.
- Sampling is expressed as an **interval in seconds**, never as fps. A config reading
  `0.0056 fps` is unreadable, and a typo there costs a factor of sixty. A floor of one
  frame per 10 seconds is enforced — below that Dayflow stops being an activity tracker
  and becomes the video recorder it explicitly is not.
- The permanent artifact is the **timeline**, not the pixels. Raw frames are scaffolding
  and are eventually shrunk and evicted (see Retention); timeline entries are never
  evicted.

## What it captures: a source, not a screen

Dayflow captures a **source**. A source is one of two co-equal kinds:

- an **input taken** — a stream URL, a capture card, an IP camera. Content that may never
  have been rendered on this machine's screen.
- a **display consumed** — a whole screen, a named window, or a saved target region.

Neither is the special case. The loop depends on the `CaptureSource` trait and never
matches on the kind, so a new kind is added by writing an implementor and changing nothing
else (FR-111a, D014-1). That claim is tested against reality rather than asserted: the live
input test reads a word back out of a synthetic video file that was never on this desktop.

| kind | what you point it at | regions |
|---|---|---|
| `--displays 0,1` | whole screens (the default) | from the region cascade |
| `--window <label>` | one window, by title or class | the cascade's, clipped to the window |
| `--target <name>` | a saved normalised region | the cascade's, clipped to the target |
| `--input <url>` | a stream or capture device | **none** — see below |

An input reports **no regions, honestly**. There is no window manager to ask about a video
feed, and synthesising a whole-frame region would be indistinguishable from a real
detection — the whole-frame read would then be invisible. So `regions_for` returns `None`,
no sidecar is written, and the sample is counted into `samples_read_whole`, which `status`
reports (FR-103, D014-3).

### `display_id` means "which source", not "which monitor"

This is a **redefinition** (D014-2) and it matters when reading stored rows. The durable
key has always been `(session, display_id, sequence)`. With sources, `display_id` holds the
source's **ordinal** — its position in the session's source list. For a display session
that is still the monitor index, which is why no schema changed and no migration was
needed. For a window, target or input session, ordinal `0` is *that source*, and is **not**
display 0.

A stored row therefore says which source produced it, not which physical monitor. `status`
carries the `sources` array — kind, name and live availability — so a timeline can always
say what it is a record *of*; `displays` alone cannot, because a window session and a
display session look identical there.

## The two intents: Activity and Content

A run is started with one of two intents (D12). They are **either/or, chosen at start**,
and neither is a degraded form of the other — they produce different artifacts.

| | **Activity** (default) | **Content** |
|---|---|---|
| question | "what was I doing?" | "what was on screen?" |
| perception | enough to characterize the activity (app, activity, category, summary) | full OCR, aggregated and merged |
| text kept | the summary | **verbatim**, merged across samples |
| stills | discardable once the window is summarized | kept until the material is extracted |
| pairs naturally with | the all-day daemon | a bounded session |

The distinction is not "more detail" — it is a different artifact. Under Activity the
frames are scaffolding and a verbatim transcript of every pane would be paying for
something nobody asked for. Under Content — a lesson, an exam, a reference session — a
one-line summary is worthless and the merged text *is* the deliverable. Running Content
all day would be expensive for no benefit; running Activity over a lesson would throw away
the thing you were trying to keep. That is why it is two intents rather than one mode
with a flag, and why each timeline entry carries the intent it was captured under —
switching intent never re-interprets what was already recorded (FR-039).

Content intent is where the text-aggregation machinery runs (`DayflowIntent::aggregates_text`):
a rolling five-block history of OCR text, and a diff-merge that grows one coherent block
out of a scrolling or edited pane instead of storing N near-duplicates. The similarity
machinery went through **four revisions**, each forced by a fixture closer to reality than
the last (R23→R26): a symmetric ratio decays mechanically as the merged block grows, so
every long document eventually fragments; set-based coverage is inflated by editor chrome,
so two different files behind the same menu bar merged; a contiguous-run rule fixed chrome
and re-broke scrolling. The shipped answer separates two *questions* with two *bars*:
"did this scroll from what I hold?" (contiguous run over the position-stripped content,
bar 0.65) and "is almost none of this new?" (unchanged fraction of the capture, bar 0.90),
plus an absolute rule for a strictly-extending capture (a document being typed into, at any
window size). The bars differ because the questions differ — a scroll may legitimately
replace most of the screen; an edit may not — and reaching for a single tuned number was
the mistake made twice before it was written down.

## Two granularities, and the segment

Two record modes, two sampling rates (D10):

| mode | what it is for | default sampling |
|---|---|---|
| **daemon** | all-day background tracking — the unattended one | one frame every **3 minutes** |
| **session** | a bounded, focused ask — "track my dev work for this hour" | one frame every **minute** |

All-day is deliberately the coarser of the two, and the config validator *refuses* the
inversion: the unattended mode has to be the cheap one, because its interval is what
decides whether the feature is cheap or wasteful over eight hours.

Samples are grouped into fixed-length **segments** (windows). The segment is the unit of
summarization, of timeline entries, and of the liveness clock:

- default **15 minutes** (`segment_seconds = 900`); intended operating range **10–15
  minutes**; permitted range **5 minutes to 1 hour**, with the 5-minute hard floor
  (FR-034). Below the floor, per-segment perception (one pass per region per display)
  cannot keep pace with the cadence and the timeline fills with fragments too short to
  describe an activity.
- a segment must be able to hold **at least two samples** — otherwise it cannot show
  change — so the floor interacts with the sampling interval (the default 3-minute
  all-day interval implies a segment of at least 6 minutes). The validator enforces it.
- **the constraint is scoped to Dayflow only** (FR-034b). It is checked when a Dayflow
  session or daemon *starts*, deliberately not in the library-wide config validator:
  gentle-eye's core use is 1–30 fps real-time recording, and a stale `dayflow.*` value
  must never be able to fail configuration loading for a user recording a ten-second clip.
- the interval is changeable mid-day; the change takes effect at the next boundary and
  never re-times an existing entry (FR-035). A day may therefore contain segments of
  different lengths — pause-truncated stubs, a resized afternoon, one short final window —
  and **nothing downstream may derive a duration by multiplying a count by the configured
  interval**. Every consumer reads the segment's own recorded `start_wall`/`end_wall`
  (R7). The standup digest exists partly to demonstrate why (see below).

Segment identity is `(session, display, sequence)`, never a filename or a per-run index —
an index resets on every pause, resume, interval change and display change, and keying on
it after a pause would silently summarize a different window's samples (R24). The identity
lesson had to be learned twice: the retention planner keyed on `sequence` alone one wave
after the rule was written down, collapsing displays on any multi-monitor machine (R34).

## The perception ladder

Perception is two local tiers behind one router, dispatching on an **explicit
caller-supplied request kind** — never by sniffing the prompt (D6/D8):

| tier | model (config, not architecture) | job | how often |
|---|---|---|---|
| **text** | `deepseek-ocr:latest` via the governed lane | extract on-screen text from a full-resolution region crop | every perceived sample |
| **reason** | `ornith-1.5-9b:latest` via the governed lane | semantic questions — activity, category, meaning | **once per segment**, as an explicit, logged escalation |

The measured basis (same live 1920×1080 frame, R19):

| path | warm latency | tokens | outcome |
|---|---|---|---|
| tesseract (generic OCR) | ~instant | — | garbled; ~half unusable on dark-theme terminal text |
| text tier, full frame | 2.6 s (cold 10.3 s) | 444 | near-verbatim but columns scrambled across panes; `CLI` misread as `CI` |
| text tier, **cropped pane** | **1.6 s** | **231** | correct order, correct glyphs |
| text tier, grounding mode | 10.1 s | 482 | text **plus** per-line bounding boxes (~6× the cost — a per-call choice, never the default) |
| reason tier (VLM) | 39 s | 787 | verbatim, ~15× the cost |

Three conclusions the design is built on:

1. **Crop before extract** (FR-011). Cropping won on latency, cost *and* accuracy
   simultaneously — it is not a tradeoff. The text tier is fed full-resolution region
   crops from the region cascade, never a downscaled full frame; numbers read from a
   downscaled full frame are not trustworthy.
2. **Never spend the reason tier on text.** Text-only work through the VLM is a ~24×
   latency penalty for a worse result. Escalation is a distinct event carrying its reason
   (FR-007/010), and the escalation log is how "the expensive tier ran" stays auditable.
3. **The endpoint and the prompt are load-bearing.** The text tier speaks
   `/api/generate`, never `/api/chat` — the chat template bleeds `>user`/`>system`
   markers into the output. The prompt is pinned to `Free OCR.`: a verbose "transcribe
   verbatim, do not reformat" instruction does not degrade the answer, it destroys it —
   measured at 42.9 s and 7,366 tokens of a degenerate repetition loop, returned with a
   200 and no error (R4). Do not "improve" either.

### Residency: why the default is `OnDemand`

This decision reversed **twice**, and the reversals are the documentation.

- **R5** assumed cold-load dominated (10.3 s cold vs 2.6 s warm) and planned a keep-warm
  pinger — then observed a 32 GB model resident with an expiry ~6 hours out and concluded
  the premise "largely evaporates."
- **R27** took the owed measurement against the *actual* text tier and found R5 had read a
  number off the wrong model: residency windows are **per-model, not per-lane**. For
  `deepseek-ocr:latest`: cold load **3.74 s**, warm call **0.18 s** (20×), resident size
  **7.4 GB**, and the governor's own default window for this model is **~50 s** — shorter
  than even the 3-minute sampling interval. `keep_alive` is honoured per request, so
  residency needs **no background pinger at all**: it is a parameter on calls Dayflow
  already makes, which also means the model unloads by itself when sampling stops.
- **R28** then caught the arithmetic reasoning about a pipeline that did not exist: text
  calls do not fire per sample. They fire **in a burst at segment close**, back-to-back,
  ~0.2 s apart — so a segment pays the cold load **once** (~0.4% overhead at the default
  cadence), and the gap a `Resident` window must survive is the gap between *segments*
  (~900 s), not between samples. The first `Resident` implementation sized its window from
  the 180 s sample interval, so it held 7.4 GB *and* paid every cold load — the worst of
  both, while reporting itself as residency.

Hence the three-valued knob (`dayflow.perception.residency`):

| policy | `keep_alive` sent | for |
|---|---|---|
| `resident` | segment cadence × 2 + 60 s | fine-grained focused sessions where the cold load matters |
| `on_demand` (**default**) | omitted — the governor's own window | all-day tracking, where ~0.4% overhead is not worth holding 7.4 GB of a shared machine |
| `off` | `"0"` — unload immediately | leave the shared box maximally free |

One number that matters more than any of these: **co-tenancy** (R19). The 1.6 s figure is
uncontended. With a peer workload holding a large model on the same 64 GB machine, the
same OCR call degraded to **17.8 s/frame** — more than 10× worse. Dayflow runs all day and
will contend with every other tenant; capacity reasoning must use the contended figure or
reserve residency, because a design sized off 1.6 s collapses the first time the box is
busy.

## The content gate: skip what did not change

Delta-skip (D11) is the single largest saving available, because reading is most of a
working day: a sample whose screen is meaningfully the same as the previous one is
recorded but **not perceived**. Two working systems solved this differently, and Dayflow
ships **both strategies** rather than inheriting one blind spot (R11/R13):

| | `magnitude` (Lookout) | `proportion` (videolocr) |
|---|---|---|
| measures | mean absolute difference across all pixels | fraction of pixels that changed at all |
| robust to | a small **intense** change | a large **subtle** shift (a theme change) |
| tuned threshold | 6.0 on a 0–255 scale | 0.4 of pixels |

Each is blind to a case the other catches — half the pixels shifting subtly trips only
proportion; 30% of pixels shifting hard trips only magnitude — so the default strategy is
**`either`** (fires when either signal trips), on the reasoning that a false "changed"
costs one wasted perception pass while a false "unchanged" loses the moment permanently.
`both` exists for machines where perception cost dominates.

Mechanics, carried over from Lookout with its production constants: the frame is
downscaled to **240 px** wide greyscale before comparison (a full-resolution pixel-exact
compare is both more expensive and more brittle — a blinking cursor or one antialiased
edge would defeat the whole saving), and a greyscale standard deviation below **8.0**
means the frame is blank/uniform and is skipped as `blank`. One adaptation videolocr did
not need: a **per-pixel tolerance of 2** before the proportion count, because Dayflow
compares downscaled *captures* where resampling jitter alone can move a large share of
pixels by ±1 — with zero tolerance the gate would fire on nearly every sample and erode
the entire saving (a risk flagged in review R14 and confirmed by test).

Deliberate and documented so nobody "fixes" it: at these thresholds **neither strategy
fires on a genuinely tiny change** such as a cursor blink (~1% of pixels). A gate that
trips on a cursor saves nothing; "changed" here means *meaningfully* changed.

## Fail open, always

Every gate and every optional input in the perception path **fails toward keeping the
sample**. This is a principle inherited verbatim from videolocr's informative gate
(*"FAIL-OPEN: on any error we KEEP the frame — never risk MISSING code"*, R13), and the
reason is specific to this feature: **Dayflow cannot re-capture yesterday.** A gate that
errs toward keeping costs one wasted perception call; one that errs toward dropping turns
any bug into silent, permanent data loss.

Concretely: an empty or failed gate buffer returns `Indeterminate`, which is perceived; a
missing or corrupt `.regions.json` sidecar means the frame is read whole rather than
skipped (and `samples_read_whole` counts the degradation so it is measurable rather than
invisible); a region set in which every region is skippable still does not drop the
sample. The known cost of fail-open is that the *absence* of an upstream producer is
invisible — see the limitations ledger.

## Drops versus skips

A distinction the design initially did not have, added after a user correction (R18): a
half-written file is a *possible dropped frame*, and filing it as a skip is the same false
green this feature exists to prevent.

| | **skip** | **drop** |
|---|---|---|
| meaning | the gate worked — nothing changed | the frame was WANTED and could not be obtained |
| data | complete | **missing** |
| recoverable later | n/a | **never** — the minute is gone |

A drop is logged at WARN with the display, interval, attempt count and expected-vs-actual
bytes; it is counted (`Sampler::drops()`, and `frames_dropped` on the liveness payload, so
a status reader **sees holes** instead of inferring them from a gap); and it triggers
**re-acquisition** — the sampler asks for a fresh frame *for the same interval* before
giving up, and a recovered interval is still recorded as a drop flagged `recovered: true`,
because success must not erase the anomaly. A bad frame is also never allowed to become
the gate's comparison baseline — that would make the next real change look unchanged and
turn one drop into an unbounded run of false skips.

What happens *after* a drop is policy (`dayflow.delta.on_drop`): `fail` (the current
default — stop the run so the cause gets investigated rather than scrolled past; right
while the feature is under development) or `record` (log, count, carry on; right for an
unattended production recorder where one bad frame must not cost the remaining seven
hours). The drop is recorded either way — the policy only decides whether the run
continues.

## The structural timeline: geometry, never a model

Timeline entries can carry region provenance — which region the text came from, its
bounding box, its parent, its display, and its rank in reading order (FR-019). Two rules
govern it (D7):

**Reading order is computed from geometry and never requested from a model** (FR-020). A
model asked to order regions answers differently on the same input from run to run, and
nothing can distinguish a re-ordering from a re-render; pixels already say where things
are, deterministically and for free. The algorithm itself was rewritten under review
(R31/R32), and the rewrite is worth understanding because both intermediate versions
looked plausible: a band whose bottom edge grows to its **tallest** member lets a
full-height sidebar swallow every row beside it (the timeline then reads down the columns);
ending it at the **shortest** member lets a 10-pixel chip exile an overlapping column to a
later band (right column read before left). Two opposite failures from one mechanism means
the *mechanism* is wrong — a running edge accumulates state, so one member's height
distorts the answer for everything after it. The shipped rule is stateless with respect to
arrival order: **find a line no region crosses** — horizontal first (rows), else vertical
(columns) — split at the *first* cut only, and recurse. Displays are never interleaved,
and each region is followed by its own children so two side-by-side windows cannot
interleave their panes into an order describing no screen anyone saw.

**Region identity is derived from where the region IS**, not from where it sits in a
vector (R30). A slice index is only meaningful within one capture; the durable id is a
hash of `(display_id, bbox, granularity)`, so the same pane in two captures carries the
same id — which is what makes "this entry came from the editor pane" answerable across a
day. The hash is FNV-1a written out in full, its value pinned by a test, because
`DefaultHasher` explicitly does not guarantee its algorithm across Rust releases and an id
persisted to SQLite must not silently rebind on a toolchain upgrade. `granularity` is in
the key because a maximized pane and the window containing it — a routine layout — share
geometry and would otherwise collide onto one id.

Provenance columns are nullable and the migration is additive: entries written before the
capability existed survive with `None`, which is the honest value — the pixels are gone,
so it can never be filled in (FR-021).

## Retention: hot → warm → cold, and the one rule

Retention is the only part of Dayflow that deletes, so its design is one rule stated
first (R33): **eviction is gated on `summarized` — never on age, never on budget
pressure.** An unsummarized window's raw samples are the only evidence that period
existed; dropping them turns a perception-backend outage into permanent loss that nothing
downstream can distinguish from a genuinely idle hour. A segment that failed to summarize
is retried, not reclaimed.

The tiers:

| tier | holds | becomes the next tier when |
|---|---|---|
| **hot** | raw sample stills | summarized + past the grace window → shrunk |
| **warm** | a timelapse (≤ 10% of raw, SC-008) plus the extracted text | past the warm age, or under budget pressure → dropped |
| **cold** | timeline entries only | never — the timeline is permanent |

The tier is **read from the disk state, not inferred from age**: a window whose raw
samples are gone is warm however recent it is; one never shrunk is hot however old.
Deriving it from age would report a state the filesystem does not have — and retention
acts on that state by deleting.

Orderings that look equivalent and are not (both caught during construction or review):

- **Tier before age.** Under budget, *all* reclaimable raw is dropped oldest-first before
  any warm artifact. A single oldest-first sweep across both tiers reads naturally and is
  wrong: the oldest window is usually already warm, so that sweep deletes the timelapse a
  person actually reads back while megabytes of superseded raw frames sit untouched.
- **Encode before delete.** Shrink verifies the replacement — including the 10% ceiling —
  *before* removing a single raw sample, and orders its mutations so every early return
  leaves a state a retry can finish (R35: an `Option::take()` before a fallible call is a
  lost pointer waiting to happen). A failed encode costs exactly zero.

And the disk emergency is answered deliberately: with every window unsummarized and a
one-byte budget, the correct outcome is to free **nothing** and stay over budget — the fix
for that state is to stop capturing, which is a decision for a layer that can tell the
user. The planner returns a decision *for every segment including the untouched ones*,
with the refusal reason, because a planner that reports only its actions makes "nothing
was reclaimed" and "everything was refused" indistinguishable — precisely the two answers
an operator needs to tell apart when the disk fills anyway.

## The standup digest

The standup view (US7) presents a range categorized, with proportions. One property
carries it (R39): **proportions come from real durations, never from a count times the
configured interval.** Counting is one line and reads as correct, and it is confidently
wrong, because windows genuinely differ in length — a pause truncates one, an interval
change resizes the next, the last window ends when the session does. A day with one long
meeting and four short interruptions reports, by count, that the interruptions were 80% of
it; by time, the meeting was 88%. The ordering *inverts*, and a reader has no way to tell.

Two totals are reported, separately and on purpose (R40):

- **`recorded_seconds`** — the **union** of entry intervals: wall-clock actually covered,
  counting overlap once.
- **`attributed_seconds`** — the **sum** of per-category time, which can legitimately
  *exceed* `recorded_seconds` on a multi-monitor machine, because windows are per display
  and two screens can be doing different things in the same minute.

The difference is real information (the concurrency), and reconciling it away would make a
two-monitor day look either twice as long or half as busy depending on which number was
kept. Summing instead of unioning was a live bug: it double-counted every dual-display
minute, and on a mostly-idle two-monitor day the inflation was enough to defeat the sparse
check — the one safeguard against over-trusting the digest was the first thing the bug
destroyed. Percentages are shares of *attributed* time (shares of `recorded` could sum
past 100), entry counts are reported **alongside** durations (never instead — "eleven
short interruptions" and "one long meeting" are both useful facts), entries are clipped to
the queried range, and when less than half the range is covered the rendering says so
*first*, before any percentage.

The category taxonomy (`coding`, `docs`, `comms`, `browsing`, `meeting`, `idle`, `other`)
is declared once, in a macro that generates the enum, the `ALL` list, and the wire names
together — because both weaker forms drifted in practice: a literal in the prompt, and
then a hand-written `ALL` const, each let a new variant silently come back as `other` with
nothing anywhere reporting a problem (R39/R40).

## Three surfaces, one engine

Start, stop, status, timeline, standup and ask are reachable from the MCP tool surface,
the CLI, and HTTP (US6). Parity between them is **structural, not conventional** (R36):
there is exactly one `DayflowService` holding one state and one set of transitions, and
each surface is a thin adapter that translates its wire format into those calls. Three
implementations kept in step by agreement drift the moment one is changed alone, and the
drift presents as *"the CLI says running and the dashboard says stopped"* — a
contradiction the user cannot resolve. Shared decisions live in the service for the same
reason: the "today so far" default for a missing range is one function
(`resolve_range`) called by all three surfaces, after the three-copies version left two of
them provably undefended (R37). The parity tests drive the *adapters* — the actual MCP
tool methods, the actual CLI argument parsing, the actual HTTP routes — not just the
service underneath, because an adapter is code and testing what it adapts to is not
testing it.

## How to read a status payload

`dayflow status` (any surface) returns:

```json
{
  "running": true,
  "session_id": "0d9e…",
  "started_at": "2026-08-26T09:00:00Z",
  "displays": [0, 1],
  "liveness": {
    "chunks_written": 14,
    "last_chunk_at": "2026-08-26T12:30:00Z",
    "last_summary_at": "2026-08-26T12:30:04Z",
    "segment_seconds": 900,
    "displays_active": 2,
    "frames_dropped": 0,
    "health": "healthy"
  }
}
```

**Every liveness number is derived from produced artifacts, never from a flag the daemon
sets about itself.** A daemon asked "are you healthy?" will always say yes; the ledger
cannot. This rule was hardened three times: closing a window is *not* producing (a pause,
an interval change or a stop closes a window at `now` whatever the sampler was doing, and
using that as evidence let a dead sampler read Healthy after every bookkeeping event —
R17); restarting an already-running run must not reset the evidence clock (R16); and the
evidence timestamp is *when a sample was last actually taken*.

`health` is one of five states, and the distinctions are the entire point:

| state | meaning | is it a fault? |
|---|---|---|
| `healthy` | running and producing | no |
| `paused` | deliberately quiet — idle, or the display asleep | **no** (FR-032) |
| `off` | the user turned capture off | **no** |
| `degraded` | running, not paused, and producing **nothing** | **yes** — the only fault state |
| `stopped` | the session ended | no |

Paused, off and degraded look identical from the outside (no new windows). Collapsing
them is what makes a liveness signal useless in practice: an operator who cannot tell a
lunch break from a fault learns to ignore both, and then a whole day is lost before anyone
notices. A pause is quiet *on purpose* — intent explains silence that was asked for, but
it can never make a silent recorder look healthy (a deliberate off with a dead sampler
still surfaces once capture is resumed). Degradation is declared after **two segment
intervals** of silence (SC-006) — two, so one slow window does not raise a false alarm —
measured from the later of the last real sample and `producing_since` (run start or the
most recent resume), so a freshly started or just-resumed run is given time to produce
rather than reading Degraded every morning.

And the rule every surface applies identically: **a degraded session is a successful
call.** HTTP returns 200, the CLI exits 0, MCP returns a normal tool result — with the
degradation in the payload. A 503 makes every monitor treat a recoverable state as an
outage; a non-zero exit makes every script treat it as a crash. "The tool failed" and
"the recorder is unhealthy" are different facts, and conflating them makes the liveness
signal unreadable from anything automated. Similarly, a **stopped** service reports
`liveness: null` rather than a fault — not running is an absence, not a failure.

Skips versus drops in the payload: a quiet screen produces `chunks_written` advancing with
samples marked skipped (the gate working); `frames_dropped > 0` means wanted frames could
not be obtained — those are holes, and they are surfaced precisely because a dropped
minute can never be recaptured and must be investigable rather than inferred from a gap.
