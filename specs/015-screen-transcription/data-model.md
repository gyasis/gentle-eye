# Data model — feature 015

Nothing is persisted. The primitives are stateless; the **recording** is the only
durable artifact, and everything else is derived from it and regenerable.

## FrameRow

One kept frame from a recording.

| Field | Meaning |
|---|---|
| `index` | position in the kept sequence |
| `timestamp` | offset into the recording |
| `path` | where the frame was written |
| `sharpness` | focus measure; **comparable within this recording only** |

Kept frames are those that survived near-duplicate suppression at the caller's
threshold. The rows do not say which frames were dropped — the caller sets the
threshold and can re-run to see more.

## TextQuality

The information content of one reading. Deliberately has **no** `length` field:
including it would invite the character-ceiling mistake M3 rules out.

| Field | Good (measured) | Degenerate (measured) |
|---|---|---|
| `compression_ratio` | 0.610 | 0.0072 |
| `unique_line_ratio` | 0.955 | 0.003 |
| `unique_token_ratio` | 0.781 | 0.004 |

No verdict field. The caller judges.

## Merge result

The merged text, plus enough for the caller to know what happened:

| Field | Meaning |
|---|---|
| `text` | the merged document |
| `overlap_lines` | how many lines were recognised as shared |

`overlap_lines == 0` on input that plainly overlaps is the signature of a
threshold set too tight — visible rather than silent.

## Reading (the caller's own record)

Not a type the tool owns, but the shape the playbook keeps: a frame, its
sharpness, its reading, that reading's quality scores, and whether the caller
accepted it. **Rejected readings stay in the record.** A transcript that drops
them cannot say how complete it is — and M2 measured 24% rejection on real
material.
