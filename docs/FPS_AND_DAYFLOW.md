# FPS & Dayflow — how to choose a capture frame rate

gentle-eye captures the screen at a configurable frame rate. The *right* fps
depends almost entirely on **how long you intend to record** — short clips want
smooth motion, all-day "dayflow" sessions want an aggressive timelapse so the
footage stays small and the AI summary stays cheap.

This is encoded as a duration-aware heuristic in
[`capture::frame_rate::recommend_fps`](../src/capture/frame_rate.rs) and surfaced
in the `start_recording` MCP tool description so an agent picks correctly.

## The heuristic

| Expected length          | fps        | Why                                                   |
|--------------------------|------------|-------------------------------------------------------|
| ≤ ~30 s, motion matters  | **15**     | smooth playback, still a tiny file                    |
| ~30 s – 15 min, debugging| **1**      | a sequence of actions; cheap to store and analyze     |
| 15 min – ~2 h (dayflow)  | **0.5**    | timelapse; recording is chunked + Map-Reduce summarized |
| ≥ ~2 h (long dayflow)    | **0.2**    | aggressive timelapse for all-day capture              |

`recommend_fps(Duration)` returns these as `f32`. The integer
[`FrameRateController`](../src/capture/frame_rate.rs) handles the ≥1-fps
recording tiers; the **dayflow** capture path (Wave 2+) consumes the fractional
sub-1-fps values directly.

## Why these numbers

- **1 fps matches Gemini's native video sampling.** Gemini internally samples
  video at ~1 fps (~258–300 tokens/frame ⇒ ~15–20k tokens/min), so ~1M context
  ≈ ~45–50 min of video. Recording faster than the model samples just wastes
  frames and disk.
- **Long recordings MUST be chunked.** Because ~45–50 min fills the context
  window, dayflow splits recordings into 15-minute chunks and summarizes them
  Map-Reduce style with a rolling context summary (the videolocr pattern). A
  sub-1-fps timelapse keeps each chunk small while preserving the *activity*
  signal a timeline needs.
- **Short clips are about motion, not duration.** A 10-second UI interaction or
  animation needs 15+ fps to be legible; the file is tiny regardless.

## Dayflow tiers

Dayflow mode (continuous recording → 15-min chunks → per-chunk summaries →
queryable activity timeline) defaults to **0.5 fps** (`DayflowConfig.record_fps`),
dropping toward **0.2 fps** for very long all-day sessions. The timeline — not
the raw video — is the durable artifact; raw chunks are shrunk and then evicted
under the retention policy (`DayflowConfig.retention`).
