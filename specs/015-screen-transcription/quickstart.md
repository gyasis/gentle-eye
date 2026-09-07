# Quickstart — feature 015

The primitives, chained by hand. This is what the playbook automates and what
issue #17 would eventually wrap into one command.

```bash
# 1. Record (nearly free — measured ~172 kbps, ~0% CPU on a stream copy)
gentle-eye record --input rtmp://<host>/live/<key> --duration 900 --out lesson.mkv

# 2. Frames, with a sharpness score each. No cap; the dedup threshold is YOURS.
gentle-eye frames --video lesson.mkv --rate 2 --dedup medium --out ./frames
#    -> one row per kept frame: index, timestamp, path, sharpness

# 3. Read ONLY the sharpest frame of each distinct screen.
#    Blur predicts failure (M4), and this filter costs no model call.
gentle-eye read-text --image ./frames/f_0007.png

# 4. Score the reading before you trust it. Never judge by length (M3).
gentle-eye text-quality --file reading.txt
#    -> compression_ratio, unique_line_ratio, unique_token_ratio
#       degenerate readings sit ~85x below good ones on compression

# 5. Merge overlapping readings into ONE document.
gentle-eye merge-text --a doc.md --b reading.txt --similarity 0.8
```

## Choosing the thresholds

There is no correct constant — that is why they are yours:

| Material | Dedup | Why |
|---|---|---|
| Slides, static documents | aggressive | hundreds of frames, a handful of screens |
| Scrolling text | gentle | nearly every frame is genuinely new (M1: 285 of 325) |
| Mixed | medium | the default |

**How to tell it is wrong:**

- The same paragraph repeats through the document → similarity threshold too
  tight; readings that overlap are not being recognised as overlapping.
- Material you saw is missing → dedup too aggressive, or the sharpness floor is
  rejecting frames that were readable.
- A reading is enormous and repetitive → the reader broke down. Its quality
  scores will show it; do not merge it.
