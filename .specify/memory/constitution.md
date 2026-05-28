# gentle-eye constitution

Rules dev-kid sentinel enforces at every wave checkpoint.

## Rust standards (LOCKED — ground-truth-recovered)

- Edition **2021** (not 2024).
- All deps pinned in `Cargo.toml` — no unspecified versions.
- Capture via **`scrap = "0.5"`** (not `scap`).
- MCP server via **`rmcp = "0.1"`** with features `["server","macros"]`.
- Errors: **`thiserror = "1"`** for typed; **`anyhow = "1"`** for app errors.
- Vision providers via **`reqwest = "0.12"`** with `["json"]`.
- Config via **`config = "0.14"`** + **`toml = "0.8"`** + **`directories = "5"`**.

## Code quality

- `pub` items get a `///` doc comment.
- Each module has `#[cfg(test)] mod tests` covering constructors + validation.
- No hardcoded secrets; env vars only (`GEMINI_API_KEY`, etc.).
- No `unwrap()` outside tests; use `?` + `thiserror` types.

## Security (from `gentle-eye.toml` + PRD §6)

- `analyze_rate_limit_per_minute = 10`, enforced by `src/security/rate_limiter.rs`.
- All paths validated by `src/security/path_validator.rs` against `allowed_video_paths`.
- Recording IDs are UUID v4, validated by `src/security/uuid_validator.rs`.

## Build gates

- **`cargo check --message-format=short`** must pass at every wave checkpoint.
- `cargo clippy -- -D warnings` before any merge to main.
- `cargo test` green at Wave 10.

## Dogfood discipline (session decision Q3 2026-05-26)

- Halt-and-fix: tool bug = the valued finding. Stop, capture, fix the TOOL.
- Path-seed over session-gating (R-DR9).
- Check existing tools before reinventing (R-DR10).
- Tool dumps are data, not noise (R-DR11).
- `coverage_pct=100 COMPLETE` can be empty — verify byte size (R-DR12).
