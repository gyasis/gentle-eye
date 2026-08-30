# Contract: CLI

Subcommands on the existing `src/bin/gentle-eye.rs`. Every subcommand prints **valid JSON on
stdout** (matching the existing CLI convention), diagnostics on stderr, and exits non-zero on
failure.

> **Revised 2026-08-29** to match what shipped — see the Divergences table in
> `mcp-tools.md`, which applies to every surface (one engine, FR-027).

```
gentle-eye dayflow start    [--mode session|daemon] [--displays 0,1]
gentle-eye dayflow stop
gentle-eye dayflow status
gentle-eye dayflow timeline [--from <RFC3339>] [--to <RFC3339>] [--standup]
gentle-eye dayflow standup  [--from ...] [--to ...]
gentle-eye dayflow ask      "what was I doing at 2pm?" [--from ...] [--to ...]
```

- `--mode` defaults to `session` — an unbounded daemon must be asked for by name.
- `--from`/`--to` default to **today so far** (midnight → now), the same resolver every
  surface uses.
- `timeline` returns `entries` **and** `gaps` (recorded pause intervals with their causes).
- `standup` and `timeline --standup` are two spellings of one body and return the identical
  digest (`digest` + rendered `text`).
- Not implemented (recorded in `mcp-tools.md` Divergences): `--max-duration-minutes`,
  `--segment-minutes`, `--session-id`, `--display-id`.

Payloads are identical to the MCP tools of the same name — one engine behind all surfaces
(FR-027), so the CLI is a transport, not a second implementation.

**Exit codes**: `0` success; non-zero on failure with a typed error on stderr. Note that
`status` returning `degraded` is a **successful call** and exits `0` — the degradation is in
the payload. Conflating "the tool failed" with "the recorder is unhealthy" would make the
liveness signal unreadable from a shell script.
