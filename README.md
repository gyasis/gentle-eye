# gentle-eye

> Rust + MCP server for LLM-agent screen and video understanding.

## Status

**Recovered project — partially rebuilt from session archives (2026-05-09).**

The original local source was wiped during the April 2026 disk incident. The
GitHub repo had only a boilerplate first commit. This repository is a rebuild
assembled from:

- `recovered_source/src/` — 9 `.rs` files extracted from Claude Code session
  jsonls (page-merged Read tool_results, verified against project keywords)
- `RECOVERY/rebuild/REBUILD_*.md` — design + dependency + architecture docs
  mined from 3,078 session files
- `docs/` (this repo) — those rebuild docs, promoted into the live tree
- `Cargo.toml` (this repo) — best-effort reconstruction from
  `REBUILD_DEPENDENCIES.md` + `use`-statement analysis. Original `Cargo.toml`
  was never captured by Claude's Read/Write tool, so this is **provisional**:
  expect to fix versions and missing crates on first `cargo check`.

The recovery archive lives in `RECOVERY/` and `recovered_source/` (both
gitignored — kept on disk for reference, not committed).

## What this is

A Rust-language MCP server that gives LLM agents (Claude, others) access to
**screen capture** and **video understanding** primitives. The video-analysis
backend is Gemini-flavoured; the recording layer is split as a sub-project
under `modules/rust-record/` (not yet rebuilt).

Architecture sketch (from session record):

- `src/bin/gentle-eye.rs` — server entry; tracing + tokio + `GentleEyeServer`
- `src/mcp/{server,tools}.rs` — MCP stdio protocol surface; `take_screenshot`,
  `get_recording_status`, `get_vision_provider_info`, etc.
- `src/capture/{display,screen}.rs` — display + screen capture abstractions
- `src/config/` and `src/models/config.rs` — typed configuration
- `src/startup.rs` — server init + prerequisites checking

## Build (provisional)

```bash
cargo check         # expect missing-crate errors first run
cargo run --bin gentle-eye
```

The first `cargo check` will surface what's missing or version-mismatched in
`Cargo.toml`. Adjust dependencies based on real `use` errors. Many of the
recovered `.rs` files reference modules (e.g. `mcp::GentleEyeServer`) whose
implementation files were never captured — those modules will need to be
re-derived from `docs/REBUILD_KEY_CODE.md` and `docs/REBUILD_ARCHITECTURE.md`.

## Rebuild guidance

The `docs/REBUILD_*.md` files are the practical handoff:

| Doc | What's in it |
|---|---|
| `REBUILD_OVERVIEW.md` | Top-down summary — read first |
| `REBUILD_ARCHITECTURE.md` | Module layout + responsibilities |
| `REBUILD_DEPENDENCIES.md` | Crates with version specs (mined from sessions) |
| `REBUILD_FILESYSTEM.md` | Intended directory tree (10,747 paths catalogued) |
| `REBUILD_KEY_CODE.md` | Key code patterns / signatures discovered |
| `REBUILD_API_INTEGRATIONS.md` | External APIs (Gemini, MCP) |
| `REBUILD_PRD.md` | Product requirements distilled from sessions |
| `GENTLE_EYE_PRD.md` | Original PRD reconstruction |
| `ONTOLOGY.md` | Module / struct / trait / fn inventory |

## What's missing

- `modules/rust-record/{video-capture,region-selector-ui}/` — the recording
  sub-project; only mentioned in sessions, no captures
- `specs/001-mcp-screen-tools/` — SpecKit spec docs (spec/plan/tasks/research/
  data-model/quickstart) — only mentioned, no captures
- `memory-bank/` — mentioned, no captures
- Any `.rs` file whose Read or Write tool_use was never made through Claude
  Code — gentle-eye was edited primarily in Cursor / external buffers, so
  most source did not pass through Claude's tool pipeline

## License

MIT — see `LICENSE`.
