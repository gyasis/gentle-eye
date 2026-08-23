# Quickstart: Dayflow

**Feature**: `013-dayflow-perception-waves`

How to run Dayflow and — more importantly — how to prove it is actually working. This feature's
characteristic failure is a recorder that reports healthy while producing nothing, so every
check below reads an **artifact**, never a status flag alone.

## Prerequisites

| requirement | check |
|---|---|
| `ffmpeg` on PATH | `ffmpeg -version` |
| Governed model lane reachable | the lane's own status endpoint answers |
| Both perception models present on the lane | list the lane's models |
| Machine-local config exists | `~/.config/gentle-eye/config.toml` — **not in the repo**; a fresh machine needs it recreated |
| Build | `./.tooling/bin/cargo check` |

The config points the two perception tiers at the governed lane. Without it the tiers fall back
to defaults that are not what this feature specifies.

## Run it

```bash
# continuous daemon over every attached display, 15-minute segments
gentle-eye dayflow start

# explicit bounded session, 30-minute segments, one display
gentle-eye dayflow start --mode session --max-duration-minutes 120 \
                         --segment-minutes 30 --displays 0

gentle-eye dayflow status
gentle-eye dayflow timeline --from 2026-08-23T09:00:00Z --to 2026-08-23T18:00:00Z
gentle-eye dayflow ask "what was I doing at 2pm?"
gentle-eye dayflow stop
```

## Verify it — the checks that actually prove something

### 1. Segments exist on disk (never trust "started")

```bash
ls -la ~/.local/share/gentle-eye/recordings/<session>/display-0/
# expect chunk_0000.mp4, chunk_0001.mp4 … growing one per interval
```

Then confirm they are **non-empty and playable** — a zero-byte file is the exact false-green
this check exists to catch:

```bash
for f in chunk_*.mp4; do printf '%s %s ' "$f" "$(stat -c%s "$f")"; \
  ffprobe -v error -show_entries format=duration -of csv=p=0 "$f"; done
```

For a first smoke test use a very short interval (`--segment-minutes` set low, or the seconds
override in config) so you are not waiting 15 minutes to learn it does not work.

### 2. Status distinguishes the four states

```bash
gentle-eye dayflow status | jq '{state, pause_cause, chunks_written, last_chunk_at, last_summary_at}'
```

- **healthy** — `chunks_written` advances between two calls an interval apart
- **paused** — lock the screen; `state:"paused"`, `pause_cause:"locked"`, and `chunks_written`
  stops advancing *without* going degraded
- **off** — after `dayflow stop`
- **degraded** — running, but `last_chunk_at` older than two intervals

If locking the screen shows `degraded` rather than `paused`, FR-032 is broken — a deliberate
pause is being reported as a fault.

### 3. Entries appear DURING the session, not after

```bash
# with the session still running:
gentle-eye dayflow timeline --from <session start> --to <now> | jq 'length'
```

Non-zero before you ever call `stop` is the requirement (FR-014). Zero-until-stop means the
scheduler is batching, which defeats the whole real-time design.

### 4. Text work never touches the vision tier

```bash
grep -i escalat ~/.local/share/gentle-eye/logs/dayflow.log
```

A normal recording interval should produce **no** escalation lines. Every escalation that does
appear must name its reason (FR-007/010). Escalations on ordinary text extraction mean the
router is misrouting and the cost model is void.

### 5. Local-only by default

```bash
sudo ss -tnp | grep gentle-eye   # expect only the governed lane; nothing off-box
```

Zero off-box perception requests with default config (SC-009).

### 6. Retention shrinks without losing the timeline

```bash
du -sh ~/.local/share/gentle-eye/recordings/<session>/   # before and after
gentle-eye dayflow timeline --from <start> --to <end> | jq 'length'  # must be unchanged
```

Bytes fall; entry count does not. If an entry count ever drops after eviction, FR-024 is
broken and the permanent artifact is being treated as scaffolding.

## Tests

```bash
./.tooling/bin/cargo test                      # unit + integration, stub provider
./.tooling/bin/cargo clippy --all-targets -- -D warnings
./.tooling/bin/cargo test --test dayflow_live -- --ignored   # real capture + real models
```

The `#[ignore]` live suite is the only thing that exercises real ffmpeg, real displays and real
models. **A green `cargo test` does not mean Dayflow works** — it means the logic is right
against stubs. The live suite and the manual checks above are what certify the feature.

## Troubleshooting

| symptom | likely cause |
|---|---|
| Segments are one long file, not many | the segment muxer arguments are not in effect (research R1 — the `-force_key_frames` behaviour is the UNVERIFIED item) |
| Every segment takes ~10s longer than expected | the text tier is being unloaded between intervals — residency policy not applied (R5) |
| Text is present but columns are interleaved | perception ran on a full frame instead of a region crop (FR-011) |
| `chunks_written` frozen but state is healthy | liveness is reading a flag instead of the segment ledger — the exact bug FR-006 exists to prevent |
| Second display missing from the timeline | its pipeline failed to start; check `displays_active` against the attached count |
| Nothing recorded all morning | check `pause_cause` first — an idle threshold set too low will pause a reading-heavy morning |
