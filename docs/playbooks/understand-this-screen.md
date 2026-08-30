# What is on screen right now

**When:** a one-shot question. No recording, no session.

## Cheapest first

```bash
# Plain text — OCR, no model, no network.
gentle-eye read-text --display 0

# Structure — boxes in reading order, no model. Geometry, not opinion.
gentle-eye regions --depth pane --display 0

# Understanding — a VLM answers a question.
gentle-eye screenshot --out /tmp/now.png --display 0
gentle-eye analyze --image /tmp/now.png --prompt "which test is failing?"
```

Reach for the cheapest tier that answers the question. OCR and geometry cost
nothing and need no network; a model call costs a model call.

## Only one region matters?

```bash
gentle-eye target add panel --display 0 --region 0.1,0.1,0.5,0.4
gentle-eye screenshot --target panel --out /tmp/panel.png
```

The region is stored **normalised**, so it survives a resolution change; the
pixel rectangle is resolved per frame, never cached.

## What proves it worked

`read-text` returns text you can see on screen. `regions` returns boxes whose
count and shape match the layout. `analyze` answers the question asked rather
than describing the image generically.

## When it goes wrong

- `tesseract not found` — OCR needs the binary; install it or use `analyze`.
- `regions` returns one box for everything — the a11y bus is unavailable
  (common in sandboxes); the WM tier still gives window-level boxes.
- Every region reports `display_id: 0` on a multi-monitor desk. Known limit.
