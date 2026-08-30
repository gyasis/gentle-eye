# Phase 1 Data Model: Dayflow

**Feature**: `013-dayflow-perception-waves` | **Date**: 2026-08-23

Types marked **(exists)** are already implemented and green — shown for context, and extended
only where noted. Their existing field names are preserved; nothing here renames or removes a
field that shipped.

---

## Entities

### `DayflowSession` **(exists — extend)**

One bounded or continuous recording run.

| field | type | notes |
|---|---|---|
| `id` | `Uuid` | exists |
| `recording_id` | `Uuid` | exists |
| `started_at` | `DateTime<Utc>` | exists |
| `ended_at` | `Option<DateTime<Utc>>` | exists |
| `mode` | `DayflowMode` | exists — session or daemon |
| `status` | `DayflowStatus` | exists — **extend the enum**, see state machine below |
| `day` | `NaiveDate` | **new** — the local calendar day this session belongs to. Turning capture off and on mid-day rejoins the same day (FR-033) by matching on this, not by creating a second timeline. |
| `segment_seconds` | `u32` | **new** — the interval in force *for this run*. Not read from config at query time, because config can change mid-day (FR-035). |
| `pauses` | `Vec<PauseInterval>` | **new** — the recorded gaps (FR-032). |

### `PauseInterval` **(new)**

| field | type | notes |
|---|---|---|
| `from` | `DateTime<Utc>` | when capture stopped |
| `to` | `Option<DateTime<Utc>>` | `None` while still paused |
| `cause` | `PauseCause` | `Idle` · `Locked` · `DisplaySleep` · `UserOff` |

`cause` is what lets status tell a deliberate pause from a fault (FR-006 vs FR-032) — the
distinction the whole liveness story rests on. Do not collapse it to a boolean.

### `ChunkRef` **(exists — extend)**

One segment file. Renaming is deliberately avoided: `ChunkRef` already ships and is used by
`chunking.rs` and `summarizer.rs`.

| field | type | notes |
|---|---|---|
| `index` | `usize` | exists — ffmpeg's per-run counter |
| `path` | `PathBuf` | exists |
| `start_wall` | `DateTime<Utc>` | exists — **actual**, never derived from the interval (R7) |
| `end_wall` | `DateTime<Utc>` | exists — actual |
| `display_id` | `u32` | **new** — which display produced it (FR-029) |
| `sequence` | `u64` | **new** — monotonic **within the session**, across encoder restarts. `index` restarts at 0 on every ffmpeg restart (pause, interval change, display change), so it is not a stable identity. |
| `summarized` | `bool` | **new** — the eviction guard (FR-025). A segment that failed summarization stays `false` and is retried, never reclaimed. |

**Identity is `(session_id, display_id, sequence)`.** Nothing may key a segment on `index` or
on its filename.

### `ChunkSummary` **(exists)** / `RollingContext` **(exists)**

Unchanged. `ChunkSummary` already carries `chunk_index`, `start_time`, `end_time`, `category`,
`app`, `activity`, `detail`; `RollingContext` carries the forward-threaded `summary`.

### `TimelineEntry` **(exists — extend)**

The permanent artifact. Survives every retention tier and every eviction.

| field | type | notes |
|---|---|---|
| `id` | `Uuid` | exists |
| `recording_id` | `Uuid` | exists |
| `start_time` / `end_time` | `DateTime<Utc>` | exists |
| `category` | `ActivityCategory` | exists — Coding/Docs/Comms/Browsing/Meeting/Idle/Other |
| `app` | `String` | exists |
| `activity` | `String` | exists |
| `summary` | `String` | exists |
| `region_id` | `Option<u64>` | **new, nullable** (FR-019) |
| `bbox` | `Option<(i32,i32,u32,u32)>` | **new, nullable** — x, y, w, h |
| `parent_region_id` | `Option<u64>` | **new, nullable** |
| `display_id` | `Option<u32>` | **new, nullable** (FR-029) |
| `reading_order` | `Option<u32>` | **new, nullable** — computed from geometry (FR-020), never from a model |

All five are nullable so rows written by the shipped `T240` migration survive untouched
(FR-021).

### `Region` **(exists — extend)**

`regions::Region` already has `bbox`, `parent: Option<u64>`, `source`, `granularity`, `trust`,
`role`, `label`, `provenance`, and `assign_parents` already builds the containment tree.

| field | type | notes |
|---|---|---|
| `display_id` | `u32` | **new** — the one gap multi-display introduces. Without it, two regions at the same coordinates on different screens are indistinguishable. |

### `DayflowLiveness` **(new)**

Returned by status. **Every field is derived from an artifact another process wrote** — the
ffmpeg segment manifest and the `timeline_entries` table — never from a flag the daemon sets
about itself.

| field | type | source |
|---|---|---|
| `chunks_written` | `u64` | count of manifest lines |
| `last_chunk_at` | `Option<DateTime<Utc>>` | newest segment's `end_wall` |
| `last_summary_at` | `Option<DateTime<Utc>>` | newest `timeline_entries` row |
| `segment_seconds` | `u32` | interval in force — the window for "recent" (SC-006) |
| `displays_active` | `u32` | pipelines currently capturing |
| `health` | `DayflowHealth` | derived, see below |

---

## State machines

### Session / daemon health

```
Healthy      ── idle threshold / lock / display sleep ──▶ Paused(cause)
Healthy      ── user turns capture off ─────────────────▶ Off
Paused       ── activity detected ──────────────────────▶ Healthy
Off          ── user turns capture on (same day) ───────▶ Healthy   (rejoins day, FR-033)
Healthy      ── no new segment for 2 × interval ────────▶ Degraded
Degraded     ── a segment appears ──────────────────────▶ Healthy
any          ── stop ──────────────────────────────────▶ Stopped
```

`Paused`, `Off` and `Degraded` are **three different states**, never one "not recording" flag.
Collapsing them is precisely the failure this feature exists to prevent: a pause looks
identical to a breakage, so the operator learns to ignore both.

`Degraded` is evaluated in **segment intervals**, not fixed minutes (SC-006) — a 30-minute
interval means an hour of silence before degradation, and that is correct.

### Retention tier

```
Hot (raw segment)
  │  summarized == true  AND  age > hot_grace_hours
  ▼
Warm (timelapse + retained extracted text, ≤10% of raw — SC-008)
  │  age > warm_days   OR   over disk budget
  ▼
Cold (timeline entry only — the permanent artifact)
```

**Eviction order under budget** (FR-024): summarized-raw oldest-first, then warm oldest-first.
Never a timeline entry. Never a segment with `summarized == false` (FR-025) — a summarization
failure caused by an unreachable backend must not become data loss.

---

## Schema delta

Additive, idempotent, re-runnable — it must not rewrite the `T240` migration (FR-021).

```sql
-- region provenance on the existing timeline_entries table; all nullable
ALTER TABLE timeline_entries ADD COLUMN region_id        INTEGER;
ALTER TABLE timeline_entries ADD COLUMN bbox_x           INTEGER;
ALTER TABLE timeline_entries ADD COLUMN bbox_y           INTEGER;
ALTER TABLE timeline_entries ADD COLUMN bbox_w           INTEGER;
ALTER TABLE timeline_entries ADD COLUMN bbox_h           INTEGER;
ALTER TABLE timeline_entries ADD COLUMN parent_region_id INTEGER;
ALTER TABLE timeline_entries ADD COLUMN display_id       INTEGER;
ALTER TABLE timeline_entries ADD COLUMN reading_order    INTEGER;

-- pause intervals, so a gap is a recorded fact rather than an absence of rows
CREATE TABLE IF NOT EXISTS dayflow_pauses (
  id          TEXT PRIMARY KEY,
  session_id  TEXT NOT NULL,
  from_ts     TEXT NOT NULL,
  to_ts       TEXT,
  cause       TEXT NOT NULL
);

-- segment ledger: the durable record behind liveness and eviction
CREATE TABLE IF NOT EXISTS dayflow_segments (
  session_id  TEXT NOT NULL,
  display_id  INTEGER NOT NULL,
  sequence    INTEGER NOT NULL,
  path        TEXT NOT NULL,
  start_wall  TEXT NOT NULL,
  end_wall    TEXT NOT NULL,
  summarized  INTEGER NOT NULL DEFAULT 0,
  tier        TEXT NOT NULL DEFAULT 'hot',
  bytes       INTEGER,
  PRIMARY KEY (session_id, display_id, sequence)
);

CREATE INDEX IF NOT EXISTS idx_timeline_range ON timeline_entries(start_time, end_time);
CREATE INDEX IF NOT EXISTS idx_segments_evict ON dayflow_segments(tier, summarized, end_wall);
```

`bbox` is stored as four columns rather than a blob so eviction and layout queries can filter
on geometry in SQL. Range queries stay parameter-bound (FR-017) — no string interpolation.

---

## Validation rules

- `end_wall > start_wall` for every segment; a zero- or negative-length segment is an error,
  not a row.
- Segments of one `(session, display)` never overlap; `sequence` is strictly increasing.
- **Segment durations are not assumed uniform** anywhere (R7). Any aggregation that multiplies
  a count by the configured interval is a defect.
- A `TimelineEntry` with a `region_id` must also have a `display_id` — provenance is not
  half-populated.
- `reading_order` is unique within `(display_id, parent_region_id)` for one segment's entries.
- A pause interval with `to < from` is an error; an open pause (`to = NULL`) is valid only for
  the current session.
- Clock discontinuity: if a new segment's `start_wall` precedes the previous `end_wall` (DST or
  a manual clock change), the segment is recorded with its real timestamps and flagged, rather
  than silently producing an overlapping or negative-length entry.
