# Contract: MCP Tools

Five tools added to the existing `rmcp` server (`src/mcp/{server,tools}.rs`). Inputs and
outputs are `schemars`-derived JSON Schemas, consistent with the tools already registered.

## `start_dayflow`

| field | type | required | notes |
|---|---|---|---|
| `mode` | `"session" \| "daemon"` | no (default `daemon`) | |
| `max_duration_minutes` | integer | no | session mode only; capped by config |
| `segment_minutes` | integer | no | overrides config for this run (FR-034) |
| `displays` | array of integers | no | default: all attached (FR-029) |

**Returns**: `session_id`, `day`, `mode`, `segment_seconds`, `displays_active`, `started_at`.
Starting on a day that already has a session **rejoins** it (FR-033) and returns the existing
`session_id` — it does not open a second timeline for the day.

## `stop_dayflow`

Input: `session_id` (optional; defaults to the active session).
**Returns**: `session_id`, `stopped_at`, `chunks_written`, `entries_written`.
Closes and accounts for the in-progress segment rather than discarding it (FR-005).

## `dayflow_status`

Input: none.
**Returns**: `state` (`healthy` | `paused` | `off` | `degraded` | `stopped`), `pause_cause`
(when paused), plus the full `DayflowLiveness` block — `chunks_written`, `last_chunk_at`,
`last_summary_at`, `segment_seconds`, `displays_active`.

**The contract that matters**: a caller MUST be able to distinguish *healthy*, *paused*, *off*
and *degraded* from this response alone, without touching the filesystem (FR-006). Every
liveness number is read from the segment ledger and the timeline table — never from an
in-memory flag the daemon keeps about itself.

## `get_timeline`

| field | type | required | notes |
|---|---|---|---|
| `from` / `to` | RFC3339 timestamp | yes | |
| `display_id` | integer | no | filter |
| `standup` | boolean | no | returns the digest shape (FR-028) |

**Returns**: entries ordered by `start_time`, each with time range, `app`, `activity`,
`category`, `summary`, and nullable region provenance (`region_id`, `bbox`,
`parent_region_id`, `display_id`, `reading_order`). Also returns `gaps` — the pause intervals
overlapping the range, each with its cause, so a gap reads as a recorded fact rather than as
missing data.

Parameters are bound, never interpolated (FR-017).

## `ask_day`

Input: `question` (string), optional `from`/`to` (default: today).
**Returns**: `answer`, plus the `entries` it was grounded on.

**The contract that matters**: the answer is grounded **strictly** in stored entries. When the
queried range holds no entries the tool states it has no record — it does not answer from the
model's own knowledge (FR-018). An answer with an empty `entries` array and non-empty prose is
a contract violation.
