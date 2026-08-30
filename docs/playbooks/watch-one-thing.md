# Watch one thing all day, then ask about it

**When:** something needs observing over hours — a QA run, a long build, an
agent working in a terminal — and the question comes later.

## Steps

```bash
# 1. Start the daemon. It owns the session; pick EXACTLY ONE source.
gentle-eye dayflow serve --window "Firefox" &
#    …or --target <name>, --input <url>, --displays 0,1
#    Two kinds at once is refused, not resolved.

# 2. Confirm it is not just running but PRODUCING.
gentle-eye dayflow status
```

Check `liveness` in that payload, not `running`. A session can be running and
producing nothing; those are different states and the payload distinguishes them.

```bash
# 3. Later — from any shell, on any process. It attaches to the daemon.
gentle-eye dayflow timeline
gentle-eye dayflow ask "did the build ever go red?"
```

## What proves it worked

- `status.sources[]` names what you pointed it at (kind + name), and
  `availability` is `available`.
- After ~15 minutes, `dayflow timeline` returns at least one entry.
- `dayflow ask` returns prose **and** a non-empty `grounding` array.

## When it goes wrong

| Symptom | Meaning |
|---|---|
| `ask` returns `[no model configured…]` | `GE_DAYFLOW_ENDPOINT` is unset — point it at the governed lane |
| `ask` returns `[ask failed: could not reach…]` | the lane is unreachable; a cold model load is slow (~95 s) but not this |
| `status.samples_read_whole` climbing | no region sidecars — perception is reading whole frames. Expected for `--input`; on a display it means the cascade found nothing |
| answer is `No activity was recorded` | genuinely no entries in that range. It did **not** ask a model — by design |
| a second `serve` is refused | correct: one daemon per state file. Attach to the running one |
| `availability: occluded` forever on `--window` | on macOS/Wayland this means "no X11 here", not "minimised" — a known limit |
