# Contract: CLI

Subcommands on the existing `src/bin/gentle-eye.rs`. Every subcommand prints **valid JSON on
stdout** (matching the existing CLI convention), diagnostics on stderr, and exits non-zero on
failure.

```
gentle-eye dayflow start   [--mode session|daemon] [--max-duration-minutes N]
                           [--segment-minutes N] [--displays 0,1]
gentle-eye dayflow stop    [--session-id ID]
gentle-eye dayflow status
gentle-eye dayflow timeline --from <RFC3339> --to <RFC3339> [--display-id N] [--standup]
gentle-eye dayflow ask     "what was I doing at 2pm?" [--from ...] [--to ...]
```

Payloads are identical to the MCP tools of the same name — one engine behind all surfaces
(FR-027), so the CLI is a transport, not a second implementation.

**Exit codes**: `0` success; non-zero on failure with a typed error on stderr. Note that
`status` returning `degraded` is a **successful call** and exits `0` — the degradation is in
the payload. Conflating "the tool failed" with "the recorder is unhealthy" would make the
liveness signal unreadable from a shell script.
