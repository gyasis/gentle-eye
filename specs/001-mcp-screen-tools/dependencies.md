# gentle-eye — Authoritative Dependency / Tech-Stack Lock

**GROUND TRUTH** — recovered from a real `Cargo.toml` **Write** capture (37 tool_uses,
100% coverage) via the dr v2 multi-hop pipeline on 2026-05-26.
Source: `~/recovery/gentle-eye_v2/reconstructed_full/Cargo.toml`.

This **supersedes** both the incomplete in-place `Cargo.toml` and the earlier
spec-derived draft of this file. The spec/PRD-derived guesses were wrong in
build-breaking ways (see "Corrections" below) — proof that byte-recovery beats
spec-rebuild for dependency fidelity.

---

## Main crate `gentle-eye` (edition **2021**)

`description = "MCP server for screen recording and AI video analysis"`

### [dependencies]
| Crate | Version / features | Group |
|---|---|---|
| `rmcp` | `0.1`, features `["server","macros"]` | MCP server |
| `tokio` | `1`, features `["full"]` | async runtime |
| `async-trait` | `0.1` | — |
| `scrap` | `0.5` | **screen capture (NOT scap)** |
| `rusqlite` | `0.31`, features `["bundled"]` | storage |
| `reqwest` | `0.12`, features `["json"]` | vision providers (Gemini/Ollama) |
| `serde` | `1`, features `["derive"]` | serialization |
| `serde_json` | `1` | — |
| `toml` | `0.8` | config |
| `config` | `0.14` | config |
| `directories` | `5` | config paths |
| `thiserror` | `1` | error handling |
| `anyhow` | `1` | — |
| `base64` | `0.22` | utilities |
| `mime_guess` | `2` | — |
| `tempfile` | `3` | — |
| `uuid` | `1`, features `["v4","serde"]` | recording IDs |
| `chrono` | `0.4`, features `["serde"]` | timestamps |
| `tracing` | `0.1` | logging |
| `tracing-subscriber` | `0.3`, features `["env-filter"]` | logging |

### [dev-dependencies]
| Crate | Version |
|---|---|
| `mockall` | `0.12` |
| `tempfile` | `3` |

### [[bin]]
`name = "gentle-eye"`, `path = "src/bin/gentle-eye.rs"`

---

## Corrections vs the earlier spec-derived draft (why byte-recovery mattered)

- `thiserror` is **1**, not 2.0 (the 2.0 came from the *separate* rust-record workspace).
- **No** `schemars`, **no** `image` — both were inferred from PRD §10; ground truth omits them.
- **Added** (missed by the spec draft): `config 0.14`, `directories 5`, `mime_guess 2`.
- **Removed** (spec-draft phantoms not in ground truth): `futures`, `regex`, `bytes`, `url`,
  `hostname`, `clap`/`argh`, `criterion`, `serial_test`, `tokio-test`, `ratatui`.
- `rmcp = 0.1`, `reqwest = 0.12` (exact pins, not "latest").
- Capture crate is **`scrap 0.5`** — confirmed.

> Caveat: this is the latest *captured* Cargo.toml. If the project added crates after the
> last capture, they'll surface as unresolved-import compile errors — which is precisely
> what the dev-kid sentinel + micro-agent dogfood loop will catch and fix.

---

## `modules/rust-record/` workspace (SEPARATE — do not merge into main crate)

Dayflow-derived multi-crate workspace, edition 2021: `video-capture` (lib) +
`region-selector` (CLI) + `region-selector-ui` (Slint GUI, `slint-build = =1.13.1`).
Its OWN deps (`thiserror 2.0`, `dirs 5.0`, `hostname 0.4`, `slint`, …) belong to that
workspace only — conflating them into the main crate was the source of the earlier errors.

---

## Runtime / external technologies (PRD §11)

- **Vision providers:** Gemini (`gemini-2.0-flash`, default) + **Ollama** (`http://localhost:11434`) over `reqwest 0.12`.
- **Config:** `gentle-eye.toml` (recording / vision / security / mcp) via `config 0.14` + `toml 0.8`; paths via `directories 5`.
- **Env vars:** `GEMINI_API_KEY`, `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `GOOGLE_API_KEY`.
- **Storage:** SQLite via `rusqlite 0.31` (bundled).
- **License-lock note:** `library_license_rust_analysis.md` — avoid AGPL (OmniParser); prefer Apache-2.0/MIT.
