# The user drew on their screen — read what they meant

**When:** someone says "I marked it", "see what I circled", "look at what I
drew". They pointed instead of describing, because pointing was faster.

## Steps

```bash
# 1. Find their markup. Newest first.
gentle-eye redpen-list --limit 3

# 2. Read it — the marks arrive at the model AS TEXT, so direction is
#    understood, not merely seen.
gentle-eye redpen-analyze --prompt "what am I being pointed at, and what should I change?"
```

## What proves it worked

The answer refers to **what was marked**, not to the screenshot in general. A
reply describing the whole window means the marks were not used.

## Read this before acting on it

- **Check the capture is RECENT.** `redpen-list` returns whatever is on disk;
  the newest may be months old. Acting on stale markup as if it were current
  direction is worse than asking.
- Strokes arrive as `pen` / `arrow` / `box` with normalised coordinates and a
  colour. An arrow has a direction — that is usually the whole message.
- Never launch the drawing UI yourself. `redpen` is the human's tool; these
  commands only **discover** what they already drew.
- To answer in kind — show *them* which box you mean — use the agent's half of
  the loop: `gentle-eye annotate --image <png> --out marked.png --box x,y,w,h
  --label "this one"`. Pixel coordinates, not normalised. Check `labels_drawn`
  in its output: boxes always draw, labels need a TrueType font
  (`GENTLE_EYE_FONT` overrides the search).
