# Contract — CaptureSource

The seam that makes an input and a display co-equal (FR-110, FR-111a).

## The loop's side of the bargain

The loop calls `next_frame` on the cadence, hands the frame and
`regions_for(&frame)` to the sampler, and asks `availability()` when a frame
fails. It never matches on the source kind. **A new kind is added by writing an
implementor and nothing else** — if adding one requires editing the loop, the
contract has been broken.

## The source's side

- **`next_frame` may fail, and failure is not fatal.** The loop records a gap
  with the cause from `availability()` and continues. A source that returns
  `Ended` is not restarted; `Occluded` and `Available` are retried on the next
  tick.
- **`regions_for` returns `None` honestly.** A source with no cascade to ask must
  NOT synthesise a single whole-frame region: that would be indistinguishable
  from a real detection, and the whole-frame read would then be invisible. The
  loop counts `None` into `samples_read_whole` and status reports it (FR-103).
- **`identity()` is stable across movement.** A window dragged to another monitor
  or reopened is the same source. Position is not identity (013/R30).
- **`ordinal()` is stable for the session's lifetime.** It is the durable key's
  middle field; a source whose ordinal changed mid-session would split its own
  segments across two identities (013/R34).

## Fail-open, unchanged

Every gate downstream keeps its 013 semantics: on any error the sample is KEPT.
A source error produces a recorded gap, never a silently dropped interval —
dayflow cannot re-capture yesterday.

## What this contract does NOT cover

Frame rate negotiation, audio, and any source that cannot produce a still frame.
A source that can only stream continuously is out of scope; Dayflow samples.
