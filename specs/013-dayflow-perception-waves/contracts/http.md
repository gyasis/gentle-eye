# Contract: HTTP

Endpoints on the existing hand-rolled loopback server (`src/dayflow/http.rs`). No new
dependency, no new framework, loopback binding unchanged.

> **Revised 2026-08-29** to match what shipped. All input travels in the **query string**
> (the hand-rolled server does not parse JSON bodies); the original draft's body-based
> `start`/`ask` was never built. Shared divergences: `mcp-tools.md`.

| method | path | query | returns |
|---|---|---|---|
| POST | `/dayflow/start` | `?mode=session|daemon&displays=0,1` | `{session_id}` |
| POST | `/dayflow/stop` | — | `{windows_closed}` |
| GET | `/dayflow/status` | — | state + liveness block |
| GET | `/dayflow/timeline` | `?from=&to=` | `entries` + `gaps` |
| GET | `/dayflow/standup` | `?from=&to=` | the categorized digest |
| GET | `/dayflow/ask` | `?question=&from=&to=` | answer + grounding entries |

- `mode` defaults to `session`; `from`/`to` default to today so far — the same resolver as
  the MCP and CLI surfaces (FR-027).
- `ask` is a **GET**: it mutates nothing, and only `start`/`stop` accept POST (anything else
  answers `405`). Use `%2B`-escaping or the `Z` timestamp form — a raw `+` in a query string
  is a space.
- Not implemented: `display_id` filter, per-session stop, per-run
  `max_duration_minutes`/`segment_minutes` overrides (see `mcp-tools.md` Divergences).

**Status codes**: `200` on success; `400` on a malformed range, unparseable timestamp, or a
start/ask refused for a stated reason; `405` on the wrong method; `409` on stopping with no
active session. A `degraded` recorder still returns **`200`** — degradation is reported in
the body, not as a transport error, for the same reason the CLI exits `0`.
