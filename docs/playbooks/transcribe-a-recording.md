# Turn a recording into a transcript

**When:** a lecture, a screen-shared call, a long session — recorded, and the
text on screen is what you want out of it.

Three primitives that **measure and decide nothing**. You own every threshold,
because how much of a recording is new depends on the material: scrolling text
changes every frame, slides do not. Contract:
`specs/015-screen-transcription/contracts/primitives.md`.

## Steps

```bash
# 1. Frames, each with ffmpeg's timestamp and a sharpness score. Needs ffmpeg.
gentle-eye frames --video lesson.mkv --out ./frames --fps 2 --dedup medium > frames.json
#    -> {count, frames:[{index, timestamp_s, path, sharpness}]}

# 2. Read the sharpest frame of each distinct screen. Blur predicts failure,
#    and this filter costs no model call. The rule is YOURS — this one takes
#    the top quarter by sharpness.
jq -r '.frames | sort_by(-.sharpness) | .[: (length/4|ceil)] | .[].path' frames.json |
while read -r f; do
  gentle-eye read-text --image "$f" | jq -r .text > "$f.txt"
done

# 3. Score every reading BEFORE trusting it. Never judge by length.
for t in ./frames/*.txt; do
  printf '%s ' "$t"; gentle-eye quality "$t" | jq -c .
done
#    -> {compression_ratio, unique_line_ratio, unique_token_ratio}
#       real content sits ~0.5-0.7 on compression; a looped reader collapses to ~0.01

# 4. Merge the readings that passed YOUR gate into one document, in time order.
: > doc.txt
for t in $(jq -r '.frames | sort_by(.timestamp_s) | .[].path' frames.json); do
  [ -s "$t.txt" ] || continue
  gentle-eye merge-text --similarity 0.85 doc.txt "$t.txt" | jq -r .merged > doc.next.txt
  mv doc.next.txt doc.txt
done
```

`--similarity` is **required**: at `1.0` two readings of one imperfect line
never match and a paragraph is emitted once per frame that showed it. `0.9`
tolerates about one edit per ten characters; the right value is a property of
the reader's error rate, which only you know.

## Picking the rate and the dedup

| Material | Invocation | Why |
|---|---|---|
| slides, static documents | `--fps 1 --dedup medium` | a handful of screens; sample lightly |
| scrolling text, live typing | `--fps 2 --dedup gentle` | nearly every frame is genuinely new |
| one image every N seconds, no dedup | `--fps 1/N --dedup none` | fractional rates work (`0.1` = every 10 s, verified) — but the period must fit inside the recording, or ffmpeg emits nothing |

**Do not reach for `--dedup aggressive` to get "one frame per slide".** It
keeps only wholesale screen changes: a slide whose *title* changed reads as a
near-duplicate and is dropped. Measured on a six-slide clip where only a title
patch changed — `aggressive` kept 1 of 120, `medium` and `gentle` kept 6.

## What proves it worked

- `frames.json` has `count > 0` and its `timestamp_s` values are spaced the
  way you asked. **An empty extraction is now an ERROR, not `count: 0`** — the rate's period was longer
  than the clip — read it before reading `frames`.
- Each kept reading's `unique_line_ratio` is high (real content is nearly all
  distinct lines); a reading near `0.0` is a reader that broke down — drop it,
  do not merge it.
- `doc.txt` reads as **one** document: no paragraph repeated per frame.

## When it goes wrong

| Symptom | Meaning |
|---|---|
| `frames` errors naming `ffmpeg` | the binary is missing; nothing was extracted, and it said so |
| `no frames were extracted ... the sampling period is Ns and the recording is Ms long`, exit 1 | the `--fps` period is longer than the recording; raise the rate. It states the arithmetic rather than returning an empty success |
| material you saw is missing | dedup too aggressive, or your sharpness floor rejected readable frames |
| the same paragraph repeats through `doc.txt` | `--similarity` too tight — overlapping readings are not being recognised as overlapping |
| a reading is enormous and repetitive | the reader looped; its `quality` scores show it — do not merge it |
| you computed a time as `index / fps` | wrong under any dedup — the index counts survivors. Use `timestamp_s` |
