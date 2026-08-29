# Research — 014 Dayflow Capture Loop

Phase 0. Each entry: the decision, why, and what was rejected. Constraints from
feature 013 (`specs/013-dayflow-perception-waves/research.md`, R1–R40) are locked
inputs here, not open questions.

---

## D014-1 — The capture source is a TRAIT yielding frames, and displays are one implementor

**Decision.** Introduce `CaptureSource`: given an instant, it yields a frame
(BGRA + dimensions, the `RawFrame` the sampler already takes) and, when it can,
the regions detected for that frame. The loop depends only on this trait.

**Why.** FR-110/FR-111a require an input taken and a display consumed to be
co-equal, and a new kind to be addable without touching the loop. A loop written
against `scrap`/display enumeration makes screen capture the real thing and
everything else a filter over it — which is the shape the spec explicitly
rejects, and it would put a stream behind a fake display.

The trait is small on purpose: `next_frame`, `regions_for`, `availability`,
`identity`. Anything richer leaks a specific kind's vocabulary into the loop.

**Rejected.** Extending `TargetSource` with more variants and matching on it in
the loop — that is an enum the loop must know exhaustively, so every new kind
edits the loop, failing FR-111a.

**Reuses.** `TargetSource::{Display, Stream}` already models exactly this split
as co-equal variants; it becomes the *configuration* of a source, with the trait
as the runtime side.

---

## D014-2 — `display_id` becomes a SOURCE ORDINAL; the schema does not change

**Decision.** Keep the existing `display_id: u32` field and its position in every
durable key, and redefine it as the **source ordinal within the session**. For a
display source it remains the display index (so today's data is unchanged); for
an input source it is its slot in the session's source list.

**Why.** `(session, display_id, sequence)` is load-bearing in more places than it
looks: `SegmentRecord::key`, `ChunkRef`'s durable identity, `Region::display_id`
(and therefore `Region::identity` and the reading-order display grouping),
`sample_prefix` in the sampler's filenames, the `dayflow_segments` primary key,
and the provenance columns. R34 records what happened when one of those keyed on
half the identity: on a multi-display machine one window per colliding sequence
was silently never actioned, its bytes credited to a budget it never freed, and
the ledger reported it as merely "too recent".

Renaming the concept without changing the key preserves every one of those
invariants and needs no migration. A rename of the FIELD is deferred to a
separate mechanical change, so this feature does not mix a semantic change with
a schema churn.

**Rejected.** A new `source_id` alongside `display_id` — two identity fields is
how they drift apart. A string source id — it would force a migration of five
tables and every filename, for no capability this feature needs.

**Consequence to state in the docs.** `display_id` in a stored row means "which
source", not "which monitor", for any session whose source is not a display.

---

## D014-3 — Region detection is a property of the SOURCE, not of the loop

**Decision.** `CaptureSource::regions_for` returns `Option<Vec<Region>>`. A
display or window source asks the existing cascade. An input source that has no
window manager to ask returns `None`.

**Why.** FR-102 wants regions written beside every sample so the ladder can crop;
FR-103 requires the ABSENCE of them to be visible rather than merely survivable.
Putting detection in the loop would force the loop to know which kinds can be
asked — the same exhaustive-match problem as D014-1.

**The visibility requirement is the sharp part.** The consumer already fails open
(no sidecar → read the whole frame), and 013's R29 records that this makes the
degradation *permanently invisible*: whole-frame reads, every test green, and the
entire benefit of crop-before-extract silently absent. `SegmentLatency` already
carries `samples_read_whole` for exactly this; the loop MUST surface it in
session status, not merely compute it.

---

## D014-4 — `keep_alive` needs a provider-level channel; it does not exist today

**Decision.** Extend the vision provider seam with an optional per-request
`keep_alive`, threaded from `ResidencyPolicy::keep_alive(segment_cadence)`.

**Why.** R27 measured that the governor honours `keep_alive` per request, which
is what makes residency need no background pinger — the model unloads by itself
when sampling stops, so a pinger's failure mode (holding 7.4 GB after a dead
session) cannot occur. But `analyze_image(path, prompt)` has no parameter to
carry it, so `Resident` is currently a policy the running system cannot express.

**This is a real signature question and the plan does not pretend otherwise.**
`VisionProvider` is implemented by Gemini and Ollama and used well outside
dayflow. Options, to be settled in the design task rather than here: an optional
request-options struct with a default (no call-site churn, one more type), or a
dayflow-local provider wrapper (no shared-trait change, but a second path to keep
in step). Whichever is chosen, a provider that ignores `keep_alive` must remain
correct — it simply gets the governor's default window.

---

## D014-5 — The loop owns cadence, segments, summarisation, retention and residency; it owns no policy

**Decision.** The loop is a driver. It calls existing components on a schedule
and holds no rules of its own: `DayflowRun` decides windows, the gate decides
keeps, `SummaryScheduler` decides what is summarised next, `retention::plan`
decides what is reclaimed, `PerceptionRouter` decides tiers and budget.

**Why.** Every one of those rules was built, reviewed and mutation-proven across
eleven waves. A driver that re-decides any of them creates a second source of
truth — the failure R37 and R40 record twice, where a duplicate was *relocated*
rather than removed and the copies drifted apart untested.

**Test consequence.** The loop's tests assert *sequencing and timing*, not
policy. Policy already has its own tests; asserting it again in loop tests would
pass while the loop bypasses the component entirely — the orphan class from R29.

---

## D014-6 — Cross-process sessions: the daemon owns the session, surfaces attach

**Decision.** The session lives in the daemon process, with `DaemonStateStore`
as its durable record. CLI and HTTP invocations attach to the running daemon
rather than constructing their own engine.

**Why.** R38 records that each CLI invocation builds its own in-memory service,
so `dayflow start` prints an id and the session dies with the process, and
`status` from a new terminal always reports stopped. `DaemonState`,
`DaemonStateStore` (atomic temp+rename) and `decide_resume` are already built and
tested; they have never had an owner.

**Open for the design task.** The attach mechanism — the existing loopback HTTP
surface is the obvious candidate and needs no new dependency or port. What must
NOT happen is a second state store: R29's lesson is that the caller neutering a
parameter is as bad as no caller at all.

---

## D014-7 — `ask_day`'s answerer goes through the governed lane

**Decision.** Replace the placeholder with a call through the same governed
ollama lane the perception ladder uses, at the reasoning tier.

**Why.** No new dependency, no second endpoint, and the budget/admission story
already exists. The grounding rules, the refusal path and the "answer carries its
grounding" contract are all built and tested — only the answerer is a stub.

**Constraint carried.** FR-121: an empty range still consults NO model. That test
exists and must keep passing; it is the one that makes invention impossible
rather than merely discouraged.

---

## D014-8 — Availability is three states, not two

**Decision.** A source reports `Available`, `Occluded`, or `Ended`, and a gap
records which.

**Why.** FR-113. A minimised window, a stream that dropped, and an application
that quit are different facts. Collapsing them produces a gap that reads as a
fault when it was a user minimising something, or as quiet when the source
actually died — and 013's health model already draws exactly this distinction
between a deliberate pause and a degraded recorder (only `Degraded` is a fault).

---

## Execution model (not a technical decision, but binding)

The task crate is orchestrated and dispatched by **dev-kid**
(`/devkid.orchestrate` → `execution_plan.json`, then `/devkid.execute` with the
task watchdog). A **Fable-model agent is the GATE at every wave checkpoint**: it
reviews and it fixes what it finds before the next wave opens.

**Strictly serial**: build wave → Fable gate → wait → fix → verify → next. No
wave begins while another is under review. This is the process from 013, where a
wave built during another's review landed unreviewed and the reviewer saw a
moving tree.

Setup obligation: `dev-kid.yml` still names branch `013-dayflow-perception-waves`
and `.dk/tasks.md` still points at the 013 task file. Both are repointed to 014
in the setup phase, before any wave runs.

Mutation discipline for every gate: a mutation must PROVE IT RAN (applied,
compiled — `-D warnings` makes an orphaned binding a build failure with no result
line — and failures counted across ALL suites, since cargo stops after the first
failing binary). A surviving mutant may mean redundant code rather than a weak
test; distinguish explicitly. And check every fixture against its own claim:
thirteen times in 013 a fixture did not produce the condition its assertion
named.

## D014-9 — A per-source failure is a DROP, not a gap (found building W2/W3)

**Decision.** `Availability` maps to two different existing taxonomies, and the
choice between them is per-scope:

| scope | record | type |
|---|---|---|
| one source could not produce this interval's frame | `SampleDrop { reason: DropReason::SourceUnavailable }` | per-source (`display_id`) |
| capture stopped for the whole session | `Gap { cause: PauseCause::SourceOccluded \| SourceEnded }` | per-session |

**Why this needed deciding.** T004's DONE line said "a test proves the three
availability states are distinct in a gap record". That is not implementable as
written: `timeline::Gap` carries `session_id` and no source field, so a gap for
one of three displays claims the entire session stopped — while the other two
are still producing. 013 separated these deliberately (`SampleDrop` is
per-display; a gap is "quiet on purpose"), and collapsing them would have made a
single occluded window read as a whole-day pause.

`Availability::gap_cause()` therefore returns `Option<PauseCause>` and maps
`Available` to `None`: a healthy source warrants no gap, and writing one would
manufacture a recorded fact that never happened.

**Added:** `DropReason::SourceUnavailable`, `PauseCause::SourceOccluded`,
`PauseCause::SourceEnded`. `SourceEnded.is_automatic()` is `false` — the source
is gone, so "the condition clears" never happens and an auto-resume would spin.

**Amends:** T004's DONE line (see tasks.md, marked AMENDED).

## D014-10 — `CaptureSource` is deliberately NOT `Send`

**Decision.** The trait carries no `Send` bound.

**Why.** Platform capture handles are thread-affine — scrap's X11 capturer holds
`Rc<Server>` and raw pointers, so `DisplaySource` cannot be `Send`. A `Send`
bound on the trait excludes the most basic source kind there is, which is a
strong signal the bound is wrong rather than the implementation. The loop is a
single ticking driver that owns its sources on the thread that created them; a
source that genuinely must cross threads (a network stream with its own reader)
owns that channel internally and still satisfies the trait.

**Consequence for the loop (W4):** it cannot hand a source to a worker pool.
This is a constraint on T006, recorded here so it is not rediscovered as a
compile error.

## D014-11 — OPEN: the capture thread mutates the run under a poison-recovering lock

**Status: OPEN — flagged, not resolved.** Recorded rather than silently accepted.

`DayflowService::lock()` recovers from mutex poisoning, and its own comment says
that is sound **only** because "every mutation under this guard is a single
atomic `Option` assignment... Keep mutations here atomic, or make this return
the error."

T009's capture thread breaks that premise: it holds the guard and calls
`run.on_sample(...)`, which mutates the run's internal window set in place. A
panic mid-`on_sample` can therefore leave a run that is structurally valid but
logically inconsistent (the window advanced, the sample was not recorded), and
the recovery would serve it as healthy.

**What I tried and reverted.** Discarding the run on poison and flagging it
Degraded. That broke `a_panic_while_holding_the_lock_does_not_kill_the_service`,
an existing green test asserting the opposite guarantee. Breaking a documented,
tested invariant to satisfy a theoretical concern is the wrong trade to make
unilaterally, so the change was reverted and the concern recorded here.

**The options, none yet chosen:**
1. Capture thread owns the run; the service keeps a read-only status snapshot
   behind its own lock. Preserves both guarantees; largest change.
2. Recover, but mark the session Degraded when the poison occurred while the
   capture thread held the guard. Needs a holder tag.
3. Accept it: argue an in-place `on_sample` panic is not a real tear, and widen
   the comment to say so explicitly.

Until this is decided, the risk is: a panic inside the capture thread's tick
produces a session that reports healthy while having skipped a sample.
