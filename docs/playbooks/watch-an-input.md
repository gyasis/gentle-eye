# Watch something that is not on this screen

**When:** a stream, a capture card, an IP camera, a video file. Content that was
never rendered on this desktop.

This is the case that proves the source abstraction is real rather than a filter
over screen capture.

## Steps

```bash
# Needs ffmpeg on PATH.
gentle-eye dayflow serve --input rtsp://camera.local/live &
gentle-eye dayflow status
gentle-eye dayflow ask "what changed on the feed this morning?"
```

A one-shot grab, without a session:

```bash
gentle-eye capture-stream --url rtsp://camera.local/live --out /tmp/frame.png
gentle-eye analyze --image /tmp/frame.png --prompt "is anyone in frame?"
```

## What to expect that differs from a screen

- **`samples_read_whole` equals the sample count.** An input has no window
  manager to ask, so it reports **no regions — honestly** — and every frame is
  read whole. That is correct, and it is counted rather than hidden.
- A failed grab is **occluded**, not ended: an encoder restart, a flapping
  network and a waking camera all look identical to one failure. Only a
  sustained outage ends the source, so a dead URL does not retry until midnight.

## What proves it worked

`status.sources[0].kind` is `input` and its `name` is your URL; samples appear;
`ask` describes the feed's content rather than your desktop.
