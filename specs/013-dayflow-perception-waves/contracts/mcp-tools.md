# Contract: MCP Tools

Five tools added to the existing `rmcp` server (`src/mcp/{server,tools}.rs`). Inputs and
outputs are `schemars`-derived JSON Schemas, consistent with the tools already registered.

> **Revised 2026-08-29** to match what shipped. The original draft promised parameters no
> surface implements; a contract asserting an uncallable interface is a second source of
> truth. What was dropped is recorded in **Divergences** at the bottom — deliberately, not
> silently.

## `start_dayflow`

| field | type | required | notes |
|---|---|---|---|
| `mode` | `"session" \| "daemon"` | no (default `session`) | |
| `displays` | array of integers | no | default: display 0 |

**Default mode is `session`, on purpose.** An unqualified start is an attended, bounded act
(D10 frames sessions as the explicit start/stop granularity); an unbounded all-day daemon
must be asked for by name, never entered by accident.

**Returns**: `session_id`.
Starting while a session is running is **refused** rather than silently replacing it — a
second start that discarded the first would drop the running session's unwritten windows.

## `stop_dayflow`

Input: none — stops the active session (there is at most one).
**Returns**: `windows_closed`.
Closes and accounts for the in-progress segment rather than discarding it (FR-005).

## `dayflow_status`

Input: none.
**Returns**: `running`, `session_id`, `started_at`, `displays`, plus the full
`DayflowLiveness` block — `chunks_written`, `last_chunk_at`, `last_summary_at`,
`segment_seconds`, `displays_active`.

**The contract that matters**: a caller MUST be able to distinguish *healthy*, *paused*, *off*
and *degraded* from this response alone, without touching the filesystem (FR-006). Every
liveness number is read from the segment ledger and the timeline table — never from an
in-memory flag the daemon keeps about itself.

## `get_timeline`

| field | type | required | notes |
|---|---|---|---|
| `from` / `to` | RFC3339 timestamp | no | default: today so far (midnight → now) |
| `standup` | boolean | no | returns the digest shape (FR-028) instead of raw entries |

**Returns**: entries ordered by `start_time`, each with time range, `app`, `activity`,
`category`, `summary`, and nullable region provenance (`region_id`, `bbox`,
`parent_region_id`, `display_id`, `reading_order`). Also returns `gaps` — the pause intervals
overlapping the range, each with its cause, so a gap reads as a recorded fact rather than as
missing data. A gap's cause matters: a pause is quiet on purpose; only `degraded` is a fault
(FR-032), and conflating them would make the array worse than none.

With `standup: true`: `digest` (the categorized `Standup` shape) plus `text` (the rendered
prose) — computed by the same `DayflowService::standup` the CLI and HTTP surfaces call.

Parameters are bound, never interpolated (FR-017).

## `ask_day`

Input: `question` (string), optional `from`/`to` (default: today).
**Returns**: `answer`, plus the `entries` it was grounded on.

**The contract that matters**: the answer is grounded **strictly** in stored entries. When the
queried range holds no entries the tool states it has no record — it does not answer from the
model's own knowledge (FR-018). An answer with an empty `entries` array and non-empty prose is
a contract violation.

## Divergences from the original draft (recorded, not implemented)

| promised | status |
|---|---|
| `start_dayflow.max_duration_minutes` | not implemented on any surface. The engine supports a cap (`DayflowRun::with_max_duration`); no surface exposes it (FR-034 per-run override deferred). |
| `start_dayflow.segment_minutes` per-run override | not implemented; the interval comes from config (`segment_seconds`, `chunk_minutes` fallback). |
| `stop_dayflow.session_id` | not implemented; there is at most one active session and stop targets it. |
| `get_timeline.display_id` filter | not implemented; entries carry `display_id` in provenance, so callers can filter client-side. |
| default `mode: daemon` | changed to `session` — see `start_dayflow` above. |
