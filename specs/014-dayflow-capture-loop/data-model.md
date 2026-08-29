# Data Model — 014 Dayflow Capture Loop

New and changed entities only. Everything from 013 is unchanged unless named.

## CaptureSource (new, trait)

What a session watches. The feature's central abstraction: the loop knows only
that a source yields frames.

| Operation | Returns | Notes |
|---|---|---|
| `next_frame(now)` | `Result<RawFrame>` | BGRA + dimensions — exactly what `Sampler` already consumes |
| `regions_for(&frame)` | `Option<Vec<Region>>` | `None` when the kind has no cascade to ask (D014-3) |
| `availability()` | `Availability` | three states, not two (D014-8) |
| `identity()` | `SourceIdentity` | durable across the thing moving or reopening (FR-112) |
| `ordinal()` | `u32` | the source slot; occupies the existing `display_id` position (D014-2) |

### Implementors

| Kind | Frames from | Regions | Ordinal |
|---|---|---|---|
| `DisplaySource` | `capture::display` (scrap 0.5) | the existing cascade | display index — today's behaviour, byte-identical |
| `WindowSource` | display capture cropped to the window | cascade, scoped to the window | source slot |
| `TargetSource` | the named target's source + region | cascade, scoped to the region | source slot |
| `InputSource` | a stream/capture device URL | `None` — no window manager to ask | source slot |

## Availability (new enum)

`Available` · `Occluded` · `Ended`.

Three, because a minimised window, a dropped stream and a quit application are
different facts (FR-113). Mirrors the existing health model's insistence that a
deliberate pause is not a fault.

## SourceIdentity (new)

Durable identity of a source, stable across the thing moving or being re-created
(FR-112). Derived from what the source IS — a window's application and title, a
stream's URL, a display's index — never from screen position, which changes when
a window is dragged.

This mirrors `Region::identity` (013/R30: "identity is where a thing IS, not
where it sits in a vector") and uses a specified hash, not `DefaultHasher`, for
the same reason R31 gives: an id written to disk that a toolchain upgrade can
rebind is not stable.

## SessionSpec (new)

What a session was asked to watch: one or more sources plus the existing
intent/cadence/segment configuration. Persisted in `DaemonState` so a restart
resumes the same sources (FR-108).

## Changed

| Entity | Change | Migration |
|---|---|---|
| `display_id` (segments, chunks, regions, provenance, sample filenames) | REDEFINED as source ordinal | **None.** Same type, same position, same values for display sources (D014-2) |
| `Gap` | gains the source and the availability cause | additive, nullable |
| `SegmentLatency::samples_read_whole` | already exists; now SURFACED in status | none — reporting only (FR-103) |
| `DayflowStatus` | gains the session's sources and their availability | additive (FR-115) |
| vision provider request | gains optional `keep_alive` | signature question, settled in the design task (D014-4) |
| `scheduler::entry_from` | receives regions, writes real provenance | replaces `provenance: None` (FR-106) |
