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

## D014-11 — RESOLVED (W4 gate): the capture thread mutates the run under a poison-recovering lock

**Status: RESOLVED at the W4 gate.** The tension was real and is now closed
mechanically, not by widening the comment.

**Decision: restore the premise instead of arguing around it.** The capture
thread wraps its tick in `catch_unwind` (see `DayflowService::start_capture`),
so a panic inside `on_sample`'s multi-step mutation is caught BEFORE it can
unwind through the guard. The lock can therefore only ever be poisoned by a
single-atomic-assignment mutation site — exactly the premise `lock()`'s
recovery comment states — so the recovery stays sound and the existing green
test `a_panic_while_holding_the_lock_does_not_kill_the_service` keeps its
guarantee untouched.

**Why continuing to serve the run after a caught tick panic is sound.** Every
interruption point in `on_sample` leaves the run valid-but-undercounting:
each field write (`sample_count += 1`, `chunks_written += 1`,
`last_chunk_at = ...`, `stopped = true`) is individually atomic with respect
to unwinding, the window map is consistent between statements, and allocation
failure aborts rather than unwinds. The only loss is the in-flight
`ClosedWindow` in the panicking frame — which dies with that frame under ANY
locking scheme, so no locking design recovers it. On a caught panic the thread
halts capture deliberately (whatever panicked once will panic again next
tick) and logs loudly; the silence then crosses the staleness threshold
(`STALE_INTERVALS × segment_seconds`) and the session reports **Degraded** —
so "reports healthy while having skipped a sample" is bounded to that window,
not indefinite. `capture_running()` now checks the thread, not the handle, so
the halt is visible immediately.

**Why not option 1** (capture thread owns the run, service keeps a snapshot):
it preserves the same guarantees at the cost of re-plumbing `status`, `stop`,
`with_run` and every pause/idle surface through a snapshot channel — a
service-wide restructure that trades a solved problem for a set of new
staleness races between the snapshot and the run. Not a gate-sized change, and
no remaining risk pays for it.

Proven by `a_panicking_tick_halts_capture_without_poisoning_the_service` in
`tests/dayflow_loop.rs`: a source that panics mid-tick leaves the service
answering, the run serveable (no poison observed), capture halted, and the
dead thread's handle reaped rather than blocking the next `start_capture`.

The original record of the tension, kept for the audit trail:

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

The risk as recorded then: a panic inside the capture thread's tick produces a
session that reports healthy while having skipped a sample. (Closed by the
`catch_unwind` resolution above — the false-healthy window is bounded by the
staleness threshold and the halt is immediately visible in `capture_running`.)

## D014-12 — The governed lane drops connections on a cold model load (measured)

**Measured 2026-08-29**, live, against the Atelier governor
(`$GE_DAYFLOW_ENDPOINT`, the governed ollama lane) with three real displays:

| run | models | result |
|---|---|---|
| 1 | cold | **FAILED** — `error sending request` on `/api/generate` at `ask_day` |
| 2 | warm | **PASSED** — grounded answer over 3 entries, 277s |

The pipeline was identical in both runs and correct in both: capture, gate,
ladder and timeline all produced accurate summaries of the real screens. Only
the final `ask` call differed. A separate probe measured a **95 s** cold load of
`ornith-1.5-9b:latest` on that lane.

**Consequence for T024/T025 (the real `ask_day` answerer).** The 013 live test's
ask path is an ad-hoc `reqwest` client with a FLAT 180 s timeout and NO retry.
That shape fails exactly as observed: a single flat budget cannot distinguish a
dead endpoint from a legitimate cold load, and a transport error during a model
swap is expected behaviour on the governed lane, not an exception.

The production answerer MUST have:
1. a **split timeout** — short `connect` (~3 s, so a wrong URL fails fast) and a
   generous `read` (~120 s, so a cold load completes);
2. **one retry** on a transport error, because the first question after an idle
   period is precisely when the model is cold — the common case, not the edge.

Without both, a user's first `ask_day` of the day fails while every test passes.
Reference implementation of the split-timeout shape: `atelier-governor.md` R-AG7.

## D014-13 — W6 gate: geometry cannot see a minimised X11 window (measured)

**The defect.** `WmLocator` decided "minimised" by `bbox.w == 0 || bbox.h == 0`
over `WmProvider::windows()`. Both halves of that are wrong, and both were
measured live (2026-08-29, DISPLAY=:1, xterm + xdotool + xprop):

1. **A minimised window keeps its geometry.** Minimised: still in
   `_NET_CLIENT_LIST`, geometry unchanged (184×69 at the same position),
   `WM_STATE` = Iconic, and `_NET_WM_STATE_HIDDEN` set. X11 retains an
   iconified window's last rectangle, so the zero-area test never fires.
2. **`windows()` filters zero-area anyway** (`continue` on `width==0`), so the
   branch was doubly unreachable — dead code shaped like a safety check.

Consequence in production: a minimised window reported `Visible(stale rect)`,
and the source **cropped the screen at the stale rectangle, recording whatever
was underneath it** — an FR-114 violation with nothing erroring anywhere.
`WindowState::Minimised` was reachable only through the test harness's
`ScriptedLocator`: green in test, inert in production (the R29/R36 shape).

**Also measured: the other-workspace case needs its own check.** A window moved
to desktop 1 (current 0) keeps geometry, does NOT get `_NET_WM_STATE_HIDDEN`
on this WM — only `_NET_WM_DESKTOP` moves. So "hidden" must be
`_NET_WM_STATE_HIDDEN` **or** (`_NET_WM_DESKTOP` ≠ current ∧ ≠ 0xFFFFFFFF).

**The fix.** `WmProvider::window_states()` — one enumeration path (`windows()`
is now built on it, R40) returning bbox + label + `showing`, from the EWMH
state, not the geometry. `WmLocator` maps `!showing` → `Minimised`. Verified
end-to-end against 14 real windows: a controlled minimise flipped `showing`
true→false with the geometry unchanged.

**What remains true.** `Gone` was always real (a killed window leaves
`_NET_CLIENT_LIST` — measured), and Err-from-the-WM → `Minimised` (retry, don't
retire) stands. So minimised vs quit IS now distinguishable in production;
before this fix only quit was.

**Sibling defects fixed at the same gate:** the two cropping sources
reintroduced the W5 `select_regions` defect one seam up (a filter that dropped
every region answered `Some(vec![])`, hiding the whole-frame read — now
clip-to-intersection with `None` when nothing overlaps, shared in
`source::clip_regions_to`); `NamedTargetSource::availability` served a stale
`Available` while its inner source was Occluded (now passes every
non-Available inner state through); and the T014 session-gap arm (see the
amended T014 DONE line).

## D014-14 — A source kind can be UNSUPPORTED on this OS, and that is not "occluded"

**Decision.** `WindowLocator` gains a fourth answer, `Unsupported`, distinct from
`Minimised` and `Gone`. A source that cannot work on this operating system says
so ONCE and fails loudly; it is never retried.

**Why.** Measured on this tree: `x11rb` is an **unconditional** dependency and
`regions/providers/wm.rs` carries **no `cfg` gate**, so it compiles on macOS
(x11rb is pure Rust) and fails at connect time. `WmLocator` maps that Err to
`Minimised` — correct for a transient X error, wrong for a platform that has no
X11 at all. Consequence today on macOS:

> `--window` starts, `x11rb::connect` fails, the source reports `Occluded`, and
> the loop retries it every tick until midnight, capturing nothing, while
> `status` says the window is *minimised*. That is a lie: the truth is there is
> no window manager of that kind here.

Compare `IdleDetector`, which is `#[cfg(target_os = "linux")]` and documented as
degrading. The difference is that idle degradation is *stated* and this one is
*disguised*.

**The seam is already right.** `WindowLocator` is a trait precisely so a
CoreGraphics (`CGWindowListCopyWindowInfo`) or Wayland implementation drops in
without touching `WindowSource` or the loop — D014-1 on the platform axis. What
is missing is only the honest fourth state.

**Applies to:** `WmLocator` (X11), and any future locator. `InputSource` is
already portable (ffmpeg), and `DisplaySource` rides scrap's own per-OS backends.

## D014-15 — ONE HTTP daemon; the CLI and MCP are CLIENTS of it

**Decision.** Dayflow's cross-process story (T022/T023) is a single HTTP server
that owns the session. The CLI and the MCP tool become thin clients of that
server rather than two parallel implementations over their own in-process
service.

**Why.** Three reasons, in the order they matter:

1. **It is the actual bug.** Today every CLI invocation builds its OWN in-memory
   `DayflowService`, so `dayflow start` in one process and `dayflow status` in
   the next are different sessions talking to nobody. That is T022's whole
   subject.
2. **MCP install overhead.** An MCP tool has to be registered per session/host;
   a CLI does not. If both are clients of one server, the zero-install path
   (CLI) and the agent path (MCP) share one implementation and one running
   session — no capability exists on one surface only.
3. **Platform reach (D014-14).** The DAEMON must be native on the machine being
   captured — capture handles are thread-affine and OS-specific. A *client* need
   not be. Splitting them means a thin client can drive a capture daemon running
   on another box, which a single fat binary cannot.

**Already half-built:** `http::bind` and `http::serve` exist in
`src/dayflow/http.rs`. What is missing is a command that launches them — there
is no `gentle-eye dayflow serve` — and the client mode for CLI/MCP.

**Parity gap to close in the same wave:** `standup` exists on the CLI and has NO
MCP tool. Under this decision, surface parity is structural rather than
remembered: both call the same server.

**Consequence for T022's persisted state:** it must persist the RESOLVED
`SourceSpec` (the W7 gate's single-enumeration invariant), not the spec as
typed — a spec re-resolved on restart can name different displays than the
session's own ordinals.
