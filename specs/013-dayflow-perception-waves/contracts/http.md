# Contract: HTTP

Endpoints on the existing hand-rolled server in `src/api.rs`. No new dependency, no new
framework, loopback binding unchanged.

| method | path | body / query | returns |
|---|---|---|---|
| POST | `/dayflow/start` | `{mode?, max_duration_minutes?, segment_minutes?, displays?}` | session descriptor |
| POST | `/dayflow/stop` | `{session_id?}` | stop summary |
| GET | `/dayflow/status` | — | state + liveness block |
| GET | `/dayflow/timeline` | `?from=&to=&display_id=&standup=` | entries + gaps |
| POST | `/dayflow/ask` | `{question, from?, to?}` | answer + grounding entries |

Payloads are identical to the MCP tools (FR-027).

**Status codes**: `200` on success; `400` on a malformed range or unparseable timestamp; `404`
for an unknown `session_id`; `409` when starting a session that conflicts with an active one on
a different day. A `degraded` recorder still returns **`200`** — degradation is reported in the
body, not as a transport error, for the same reason the CLI exits `0`.
