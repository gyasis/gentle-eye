# Dayflow operations — the runbook

How to drive Dayflow from each surface, what every configuration knob does, and how to
diagnose the states you will actually meet. Concepts and rationale live in
[`DAYFLOW.md`](DAYFLOW.md); known gaps in [`DAYFLOW_LIMITATIONS.md`](DAYFLOW_LIMITATIONS.md).

Build and test with the repo toolchain wrapper, not plain cargo:

```bash
./.tooling/bin/cargo test --quiet
```

> **Read this first**: the CLI's `start`/`stop`/`status` are per-process today — each
> invocation builds its own in-memory service, so a session started by `dayflow start`
> dies with that process. `timeline`, `standup` and `ask` genuinely share state across
> processes through SQLite. Cross-process sessions arrive with the daemon capture loop.
> Details in the limitations ledger.

## CLI

All subcommands print pretty JSON on stdout, diagnostics on stderr, and exit non-zero on
failure. **A `degraded` status still exits 0** — the degradation is in the payload,
because a non-zero exit would make every shell script treat a recoverable state as a
crash.

```bash
# Start (default mode: session; default displays: [0])
gentle-eye dayflow start [--mode session|daemon] [--displays 0,1]
# → {"session_id": "…"}

# Stop the running session
gentle-eye dayflow stop
# → {"windows_closed": 3}

# Status (also the default when no subcommand is given)
gentle-eye dayflow status
# → the DayflowStatus payload shown in DAYFLOW.md §How to read a status payload

# Timeline over a range; omitted range = "today so far" (midnight → now, UTC)
gentle-eye dayflow timeline --from 2026-08-26T09:00:00Z --to 2026-08-26T17:00:00Z
# → {"from": "…", "to": "…", "entries": [ {id, recording_id, start_time, end_time,
#     category, app, activity, summary, provenance}, … ]}

# Standup digest — two spellings, one implementation
gentle-eye dayflow standup [--from … --to …]
gentle-eye dayflow timeline --standup
# → {"digest": {from, to, recorded_seconds, attributed_seconds, span_seconds,
#     categories:[{category, seconds, percent, entries, activities}]},
#    "text": "human-readable rendering"}

# Ask a day-level question, grounded strictly on stored entries
gentle-eye dayflow ask "what was I doing at 2pm?" [--from … --to …]
# → {"answer": "…", "grounding": [entries]}
# Empty range → {"answer": "No activity was recorded for that period.", "grounding": []}
```

Notes:

- Flag values are consumed as two-token pairs, so
  `dayflow ask --from 2026-08-26T09:00:00Z "what did I do"` works — the timestamp is not
  mistaken for the question.
- The `ask` answerer is currently a stub on every surface (see limitations): only the
  empty-range refusal path behaves as advertised; a non-empty range returns a placeholder
  plus the grounding prompt.
- The contract file (`specs/013-dayflow-perception-waves/contracts/cli.md`) also names
  `--max-duration-minutes`, `--segment-minutes`, `--session-id` and `--display-id`; those
  flags are **not implemented yet** — mode and displays are the accepted start options
  today, and stop always targets the active session.

## HTTP

Bound to **`127.0.0.1` only, not configurable** — the timeline is a record of everything
the user looked at, so exposing it on a routable address is not a decision left to a
config file. Start/stop are POST (a GET that changed state could be triggered by any
speculative localhost fetch — a browser preconnect, a link checker); reads are GET.

| method | path | query parameters | returns |
|---|---|---|---|
| POST | `/dayflow/start` | `mode=session\|daemon`, `displays=0,1` | `{"session_id": "…"}` |
| POST | `/dayflow/stop` | — | `{"windows_closed": N}` |
| GET | `/dayflow/status` | — | the `DayflowStatus` payload |
| GET | `/dayflow/timeline` | `from=`, `to=` (RFC3339; both optional → today so far) | `{from, to, entries}` |
| GET | `/dayflow/standup` | `from=`, `to=` | the `Standup` digest |
| GET | `/dayflow/ask` | `question=` (required), `from=`, `to=` | `{answer, grounding}` |

```bash
curl -s -X POST 'http://127.0.0.1:PORT/dayflow/start?mode=session&displays=0'
curl -s 'http://127.0.0.1:PORT/dayflow/status'
curl -s 'http://127.0.0.1:PORT/dayflow/timeline?from=2026-08-26T09:00:00Z&to=2026-08-26T17:00:00Z'
curl -s 'http://127.0.0.1:PORT/dayflow/ask?question=what%20was%20I%20doing%20at%202pm'
curl -s -X POST 'http://127.0.0.1:PORT/dayflow/stop'
```

Status codes: `200` success (**including a degraded recorder** — degradation is in the
body, not the transport); `400` malformed range/timestamp or missing question; `405` on
the wrong method; `409` stopping when nothing runs; `500` on an internal failure. A panic
while handling a request degrades that one request, not the surface, and a per-connection
5-second read timeout stops a silent client from freezing the loop — but the server is
single-threaded, so N silent clients still serialise into ~5N seconds of stall
(a recorded mitigation, not a fix; localhost-only is what makes it survivable).

**Timestamp trap**: in a query string, `+` decodes to a space — correct form-encoding, and
fatal for an RFC3339 offset: `2026-08-26T00:00:00+00:00` arrives as `… 00:00` and is
refused loudly (with the mangled value quoted) rather than leniently parsed into some
other instant. Send `Z`-form timestamps, or escape the plus as `%2B`.

Divergences from `contracts/http.md`, deliberate or pending: start parameters travel in
the query string (the body is ignored); the standup is its own route rather than a
`standup=` flag on `/dayflow/timeline`; `display_id` filtering and per-`session_id` stop
are not implemented; timeline responses carry `entries` only — recorded pauses are not
yet joined in as `gaps` (see limitations).

## MCP tools

Five tools on the existing `rmcp` server, thin adapters over the same service:
`start_dayflow` (`mode?`, `displays?`), `stop_dayflow`, `dayflow_status`, `get_timeline`
(`from?`, `to?`), `ask_day` (`question`, `from?`, `to?`). Payloads match the CLI and HTTP
shapes. Two contract properties worth restating: `dayflow_status` must let a caller
distinguish healthy / paused / off / degraded from the response alone, and `ask_day` with
an empty range states it has no record rather than answering from the model's own
knowledge — an answer with an empty `grounding` array and confident prose is a contract
violation.

## Configuration — `dayflow.*`

All fields have serde defaults; an absent section means the defaults below. The segment
and sampling values are validated **when a Dayflow session or daemon starts**, deliberately
not at library config load (FR-034b) — a stale `dayflow.*` value must never block a user
who is only recording a ten-second clip.

### Core

| knob | default | what it does |
|---|---|---|
| `segment_seconds` | `900` (15 min) | The segment/summarization/liveness interval (FR-034). Hard floor **300 s**, ceiling **3600 s**, recommended band 600–900 s. Changeable mid-day; applies from the next boundary; never re-times existing entries. |
| `chunk_minutes` | `15` | Legacy interval key, kept so old config files parse. `segment_seconds` wins; this is consulted only when `segment_seconds` is `0`. |
| `intent` | `activity` | `activity` (track what you were doing) or `content` (keep the on-screen material verbatim, aggregated). Either/or, chosen per run. |
| `default_provider` | `gemini` | Summarization provider name; `ollama` for the local path. Cloud is opt-in, never required. |
| `record_fps` | `0.5` | Frame rate for the optional timelapse artifact only — not the sampling rate. |

### `sampling`

| knob | default | what it does |
|---|---|---|
| `day_interval_seconds` | `180` | Seconds between samples in daemon (all-day) mode. Must be **≥** `focused_interval_seconds` — the unattended mode has to be the cheap one, and the validator refuses the inversion. |
| `focused_interval_seconds` | `60` | Seconds between samples in a focused session. |
| `skip_unchanged` | `true` | Delta-skip (D11): don't perceive a sample whose screen is unchanged. The largest saving in the design. |

Floor 10 s, ceiling 3600 s on both intervals. A segment must hold at least two samples,
so `segment_seconds ≥ 2 × day_interval_seconds` is also enforced.

### `delta` — the content gate

| knob | default | what it does |
|---|---|---|
| `enabled` | `true` | Apply the gate at all. |
| `strategy` | `either` | `magnitude` \| `proportion` \| `either` \| `both`. See DAYFLOW.md — each single strategy has a measured blind spot. |
| `gate_width` | `240` | Downscale width for the comparison buffer (Lookout `GATE_WIDTH`). |
| `magnitude_threshold` | `6.0` | Mean-abs-diff above which the screen counts as changed (Lookout `GATE_CHANGE`; alias `change_threshold` accepted). |
| `proportion_threshold` | `0.4` | Fraction of pixels that must differ (videolocr). |
| `pixel_tolerance` | `2` | A pixel counts as changed only past this delta — absorbs downscale resampling jitter that would otherwise trip the gate on nearly every sample. |
| `content_std` | `8.0` | Greyscale std below which the frame is blank/uniform and skipped. |
| `on_drop` | `fail` | `fail` (stop the run — development posture) or `record` (log, count, continue — production posture for unattended runs). The drop is recorded either way. |
| `dedup_text` | `true` | Also dedupe at the text level: normalised OCR lines already seen are not re-stored even when pixels moved. |

### `displays`

`all` (default) · `primary` · `only: [0, 2]` (positional — brittle across replug) ·
`named: ["primary", "portrait", "ultrawide", "<label>"]` (identity-based, survives
replugging). A selection that resolves to nothing is an **error**, never a silent empty
set — a recorder that quietly records nothing all day is the false green this feature is
built to avoid.

### `idle`

| knob | default | what it does |
|---|---|---|
| `enabled` | `true` | Pause capture while the user is idle; resume on activity (FR-030/031). A paused interval is an explicit gap, never a degraded reading. |
| `threshold_seconds` | `300` | Idle time before pausing. |
| `hysteresis_seconds` | `30` | Dwell on **both** transitions so brief inactivity cannot thrash the recorder into a burst of tiny segments. |

Idle comes from the X11 MIT-SCREEN-SAVER idle counter (verified monotonic). Lock
detection was deliberately descoped — the X saver `state` field is unusable under GNOME
(it reports 3, outside the documented range); if lock-pausing is ever wanted, the working
signals are `org.gnome.ScreenSaver` over D-Bus or logind `LockedHint`, never the X field.
A platform with no idle backend degrades to "never idle" (records continuously), never to
a permanent pause.

### `perception`

| knob | default | what it does |
|---|---|---|
| `endpoint` | `http://127.0.0.1:11434` | Base URL of the governed model lane. **A neutral placeholder** — the real host is machine-local and supplied by config file or environment, never committed. |
| `api_path` | `/api/generate` | MUST stay `/api/generate` for the text tier. `/api/chat` wraps the prompt in a chat template that bleeds role markers into the OCR output. |
| `text_model` | `deepseek-ocr:latest` | The cheap OCR tier that handles nearly all volume. |
| `text_prompt` | `Free OCR.` | **Pinned.** Verbose variants collapse the model into a 43-second repetition loop that returns garbage with a 200. Do not embellish. |
| `grounding_prompt` | `<image>\n<\|grounding\|>Convert the document to markdown.` | Returns per-block bounding boxes at ~6× the latency. A deliberate per-call choice, never the default. |
| `reason_model` | `ornith-1.5-9b:latest` | The visual-reasoning tier, spent once per segment on explicit escalation only. |
| `residency` | `on_demand` | `resident` \| `on_demand` \| `off`. See DAYFLOW.md §Residency — measured burst behaviour makes `on_demand` ~0.4% overhead at the default cadence. Note: `resident` currently cannot be expressed end-to-end (limitations). |
| `max_regions_per_segment` | `12` | Hard cap on regions perceived per segment per display — bounds work at the source so the rate budget is a safety net, not the shaper. |

**Budget arithmetic to respect**: Dayflow's perception traffic has its own rate-limit
budget derived from the sampling interval, display count and region cap, measured over a
10-minute window (a per-minute window cannot even express the 3-minute cadence). A
segment's perception burst is roughly `samples × regions + 1` calls fired back-to-back at
segment close. A configuration whose burst exceeds its budget is **refused up front** with
a message naming the arithmetic and the three knobs that fix it — because the alternative,
discovered by derivation, was a silent livelock: the segment refused mid-burst, requeued
by retry-never-drop, retrying forever while starving every other segment.

### `retention`

| knob | default | what it does |
|---|---|---|
| `hot_grace_hours` | `48` | How long summarized raw samples are kept before they become shrinkable by age. Note: budget pressure can shrink a **summarized** window of any age — the only hard floor is the `summarized` flag itself. |
| `warm_days` | `14` | How long a shrunk (warm) timelapse is kept before it is evictable by age. |
| `disk_budget_bytes` | `21474836480` (20 GiB) | Over this, evict: all reclaimable raw oldest-first, then warm oldest-first. Never a timeline entry, never an unsummarized segment. |

## Troubleshooting

### Nothing in the timeline

Work down this list — the causes are ordered from expected to faulty:

1. **Is anything wired to real hardware yet?** The engine, daemon and retention rules are
   built and tested, but the capture loop that drives them from a live screen is still
   owed (see limitations). A `dayflow start` today exercises the lifecycle, not the
   pixels. If you expected live entries, this is the reason.
2. **Is the recorder paused or off?** `dayflow status` → `liveness.health`. `paused` and
   `off` are quiet on purpose; the timeline legitimately has nothing for those intervals.
3. **Is it degraded?** See the next section.
4. **Is the range wrong?** An omitted range means "today so far" *in UTC* — midnight UTC,
   not local midnight. Ask with explicit `--from/--to` if you live far from Greenwich.
5. **Are summaries failing?** Capture and summarization are decoupled: segments whose
   summarization fails stay `summarized = false` and are retried while capture continues.
   `last_chunk_at` advancing while `last_summary_at` lags means the perception backend is
   unreachable — check the governed lane, then let the retry drain the queue.

### Status says `degraded`

`degraded` means: running, not paused, and **no sample taken for two segment intervals**.
It is the only fault state, and it is judged from produced artifacts, so it does not lie
about intent.

- Check `frames_dropped` in the same payload — a climbing count with WARN logs naming the
  display and expected-vs-actual bytes points at capture failing (permission revoked,
  display gone, encoder failure).
- Check the interval: at a long `segment_seconds` the detection window is long too —
  degraded-detection is defined in segment intervals, not minutes, so a 1-hour interval
  means up to two hours of silence before the fault is visible. That is a reason to stay
  in the 10–15 minute band.
- A fresh start or a resume is *not* degraded for its first window — the staleness clock
  runs from `producing_since`. If you see degraded immediately after starting, the clock
  wiring is broken; report it.
- Remember the exit-code rule: monitors must alert on `health == "degraded"` in the
  payload, not on HTTP status or process exit — those stay 200/0 by design.

### The disk is filling

1. **Is anything sweeping at all?** No daemon loop calls retention on a schedule yet
   (limitations) — `plan`/`shrink`/`reclaim` are built and correct, but until the capture
   loop lands they run only when driven. A filling disk with a green suite is consistent
   with "nobody is calling the sweeper".
2. **Read the plan, including its refusals.** The planner returns a decision for every
   segment with a reason for each one it did *not* touch. The important refusal is
   `NotSummarized`: unsummarized segments are **never** evicted, whatever the budget, so a
   perception-backend outage shows up as disk growth. The fix is to restore the backend
   and let the summarize-retry drain — not to force-delete the only record of those hours.
3. **Check the budget against reality.** `disk_budget_bytes` defaults to 20 GiB; segment
   byte sizes from the synthetic probes are misleadingly small, so budget from a real
   capture's numbers.
4. If everything is unsummarized and the budget is blown, the designed answer is to stop
   capturing — freeing "just something" is exactly the emergency path retention refuses
   to have.

### The model is cold every segment

Expected magnitude first: the text tier's cold load is **3.74 s** and a segment pays it
**once**, at the head of its burst — about 0.4% of a 15-minute cadence. If that is what
you are seeing, it is the designed `on_demand` behaviour and not worth 7.4 GB of a shared
box to remove.

If per-call latency is far above warm (~0.18 s measured; ~1.6 s on a real cropped frame):

- **Co-tenancy, not cold load, is the usual culprit.** With another large model contending
  on the same machine the identical call measured **17.8 s/frame** — 10× worse. Check
  what else the governed lane is serving before touching Dayflow's config.
- Setting `residency: resident` is currently aspirational: the computed `keep_alive` has
  no channel to the provider call yet, so the effective policy is always `on_demand`
  (limitations). Don't burn time wondering why `resident` changed nothing.
- The model's own idle-unload window is ~50 s and **per-model** — observing a different
  model resident for hours tells you nothing about this one. That misreading caused a
  full design reversal once already (R5→R27).

### A question comes back with the wrong hour, or refused

- Refused with a quoted mangled value like `2026-08-26T00:00:00 00:00`: the `+` in your
  timestamp was form-decoded to a space. Use `Z`-form or `%2B`.
- Grounded on the wrong-looking entries: range queries are **overlap**-based on purpose —
  an activity that began at 1:50 and ran through 2:00 is part of the answer to "what was I
  doing at 2pm". Start-time containment was the original implementation and it denied
  records the store contained.
