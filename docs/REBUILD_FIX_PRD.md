# gentle-eye — Post-Rebuild Fix PRD

> Authored 2026-05-09, immediately after the recovery rebuild commit
> (`8a1ca9c · Rebuild gentle-eye from April 2026 recovery archive`).
>
> Purpose: capture **what's done**, **what's missing**, and **the sequenced
> path to a working `cargo build` and a runnable MCP server**. When you come
> back later, this is the file to read first.

---

## 1 · What gentle-eye is (recap)

A **Rust + MCP server for LLM-agent screen and video understanding.**
Designed to give Claude / other LLM agents access to:

- **Screen capture** — display + region selection, PNG/JPEG output
- **Video understanding** — Gemini-backed video analysis pipeline
- **Recording loop** — long-running capture → encode → analysis pipeline

**Architecture (from session record):**

```
gentle-eye/
├── src/
│   ├── bin/gentle-eye.rs           ← server entrypoint (tokio, tracing)
│   ├── mcp/{server,tools}.rs        ← MCP stdio protocol surface
│   ├── capture/{display,screen}.rs  ← capture abstractions
│   ├── config/                      ← typed config (loader + types)
│   ├── models/config.rs             ← config data models
│   └── startup.rs                   ← server init + prerequisites checks
├── modules/rust-record/             ← (NOT YET REBUILT)
│   ├── video-capture/               ← Rust video capture lib
│   └── region-selector-ui/          ← UI for selecting screen regions
├── specs/001-mcp-screen-tools/      ← (NOT YET REBUILT) SpecKit feature spec
├── benches/{capture_performance, mcp_response_time}.rs  ← (NOT YET REBUILT)
└── docs/                            ← rebuild guidance + recovery handoff
```

Active development was Dec 21–22, 2025 (2 days only — early-stage when wiped).

**Inspiration / reference project:** `Dayflow` (mentioned in
`dev_research/dayflow_analysis.md` per session record).

---

## 2 · What we did in this rebuild

### Recovery work (prior sessions)

| Step | Outcome |
|---|---|
| OpenSearch ontology mining | 944 MB / 3,245 files in `RECOVERY/` (gitignored) |
| Page-merge reconstruction | 9 `.rs` files extracted to `recovered_source/src/` |
| Cross-project verification | 0 collisions across all 9 files (grep audit) |
| Dependency mining | 23 unique crates surfaced from 3,078 sessions |
| Architecture mining | `REBUILD_*.md` docs (FILESYSTEM, ARCHITECTURE, etc.) |

### This rebuild commit (`8a1ca9c`)

| File | Status |
|---|---|
| `LICENSE`, `.gitignore` | ← from origin/main (boilerplate) |
| `README.md` | rewritten — documents the rebuild origin |
| `Cargo.toml` | **provisional** — synthesized from `REBUILD_DEPENDENCIES.md` + `use`-statement counts. Edition 2024. |
| `src/bin/gentle-eye.rs` | recovered (4.6 KB) — uses `tokio`, `anyhow`, `tracing`, calls `gentle_eye::mcp::GentleEyeServer` |
| `src/capture/display.rs` | recovered (19.7 KB) |
| `src/capture/screen.rs.partial` | recovered (2.0 KB) — **partial, won't compile** |
| `src/config.rs` | recovered (5.1 KB) |
| `src/config/mod.rs` | recovered (15.8 KB) |
| `src/mcp/tools.rs` | recovered (13.0 KB) |
| `src/mcp/server.rs.partial` | recovered (1.0 KB) — **partial, won't compile** |
| `src/models/config.rs` | recovered (20.8 KB) |
| `src/startup.rs` | recovered (3.4 KB) |
| `docs/GENTLE_EYE_PRD.md` | promoted from `RECOVERY/rebuild/` |
| `docs/REBUILD_*.md` (×7) | promoted from `RECOVERY/rebuild/` |
| `docs/ONTOLOGY.md` | promoted from `RECOVERY/rebuild/` |

### Recovery archive preserved on disk (gitignored)

- `RECOVERY/` — 945 MB, 3,245 files (sessions, specstory, structure analysis)
- `recovered_source/` — 160 KB, 10 files (the source extraction outputs)

---

## 3 · What we did NOT do — the gap list

### Source completeness

| Gap | Severity | Notes |
|---|---|---|
| `src/capture/screen.rs.partial` | **blocker** for build | 2 KB partial — needs reconstruction from `RECOVERY/sessions/` discussions or rewrite from `docs/REBUILD_KEY_CODE.md` |
| `src/mcp/server.rs.partial` | **blocker** for build | 1 KB partial — `bin/gentle-eye.rs` references `GentleEyeServer` from this module; must be reconstructed |
| `src/lib.rs` | **blocker** for build | Library crate root never captured. `bin/gentle-eye.rs` does `use gentle_eye::mcp::GentleEyeServer;` so a `lib.rs` exposing `pub mod mcp; pub mod capture; pub mod config;` etc. is required |
| Module wiring (`mod.rs` in `capture/`, `mcp/`, `models/`) | **blocker** | Need to re-derive submodule trees from the structs/fns in the recovered `.rs` files |
| `modules/rust-record/{video-capture,region-selector-ui}/` | not blocker (sub-crate) | Only mentioned, no captures. May not be needed for first MCP-only build |

### Build / run

| Gap | What's needed |
|---|---|
| `cargo check` has never been run | First check will surface real missing crates / wrong versions in the provisional `Cargo.toml` |
| Gemini API integration | `docs/REBUILD_API_INTEGRATIONS.md` is 571 bytes — thin. Needs supplementation from session record. |
| MCP protocol compliance | `mcp/tools.rs` references handlers (`take_screenshot`, `get_recording_status`, `get_vision_provider_info`) — confirm against current MCP spec |
| Storage layer | `rusqlite` is in deps but no `src/storage/` files recovered. Schema likely in `_panic_dump.md` or session record. |
| Configuration loading | `config/mod.rs` + `models/config.rs` exist but pathway from CLI / env to loaded struct not yet validated |

### Spec & PRD

| Gap | What's needed |
|---|---|
| `specs/001-mcp-screen-tools/spec.md` | Re-derive from `docs/REBUILD_OVERVIEW.md` + sessions |
| `specs/001-mcp-screen-tools/plan.md` | Distill from session discussions — the design phase happened Dec 22 |
| `specs/001-mcp-screen-tools/tasks.md` | Generate from spec once spec.md exists |
| `specs/001-mcp-screen-tools/{research,data-model,quickstart}.md` | Lower priority — fill after spec stabilises |
| `memory-bank/` | All files only mentioned. Recreate `projectbrief.md`, `CLAUDE.md` from session record. |

### Tests / benches

| Gap | What's needed |
|---|---|
| No `tests/` directory | Recreate from `_critical/` activity reports (which list test-file mentions) |
| `benches/{capture_performance,mcp_response_time}.rs` | Skeleton only — write against the real capture / MCP code once it builds |

---

## 4 · Sequenced fix plan

### Phase A — get to `cargo check` (≤ 1 day of focused work)

1. **Write `src/lib.rs`** declaring the public module surface:
   ```rust
   pub mod capture;
   pub mod config;
   pub mod mcp;
   pub mod models;
   pub mod startup;
   ```
2. **Write `src/capture/mod.rs`, `src/mcp/mod.rs`, `src/models/mod.rs`** —
   submodule declarations matching the actual `.rs` files present.
3. **Run `cargo check`** — read the errors, fix iteratively:
   - Wrong / missing crate versions in `Cargo.toml`
   - Type mismatches across recovered files (each `.rs` was edited at different
     points in time; some types may have drifted)
   - Missing trait impls
4. **Stub the two `.partial` files** to compile:
   - `src/capture/screen.rs` — minimum surface to satisfy `display.rs` callers
   - `src/mcp/server.rs` — `pub struct GentleEyeServer; impl GentleEyeServer { pub async fn new() -> Result<Self>; pub async fn serve_stdio(self) -> Result<()>; pub fn config(&self) -> &Config; }`
5. **Get to `cargo check` clean** before any other work.

### Phase B — get to `cargo build --bin gentle-eye` (1–2 days)

6. Flesh out `mcp::server::GentleEyeServer` — the actual MCP loop, tool dispatch, JSON-RPC handling.
7. Flesh out `mcp::tools` handlers — `take_screenshot`, `get_recording_status`, `get_vision_provider_info`. The signatures exist in `tools.rs` already.
8. Write the storage layer (`src/storage/mod.rs` + schema) — schema clues are in `recovered_source/_panic_dump.md` (recordings table, analysis_requests, analysis_results).
9. Wire Gemini API client — see `docs/REBUILD_API_INTEGRATIONS.md` for the surface, look at sessions for actual call shape.

### Phase C — first run + first MCP call (1 day)

10. `cargo run --bin gentle-eye` from a terminal with `RUST_LOG=info,gentle_eye=debug`.
11. Configure a Claude Code MCP entry pointing at the binary (`~/.config/claude-code/mcp.json` or wherever).
12. Issue `take_screenshot` from Claude. Verify capture lands somewhere.
13. Iterate on bugs surfaced by real use.

### Phase D — recording + Gemini analysis loop (variable)

14. Implement the recording loop (capture → encode → SQLite row → analysis queue).
15. Wire Gemini video analysis (file upload → poll → result row).
16. End-to-end test: continuous capture → analysis → MCP query.

### Phase E — modules / extras (post-MVP)

17. Rebuild `modules/rust-record/video-capture/` (separate sub-crate, recording lib).
18. Rebuild `modules/rust-record/region-selector-ui/` (Slint-based UI for region selection — `slint-build = "1.13.1"` is in the recovered deps).
19. Rebuild SpecKit `specs/001-mcp-screen-tools/` artifacts.
20. Rebuild `memory-bank/` documents.
21. Push back to GitHub with a real release.

---

## 5 · Install / build plan (when Phase B completes)

**Prerequisites (from `INSTALL.md` mentions in session record):**

- Rust toolchain (stable, edition 2024)
- A platform-supported screen capture backend (macOS: Core Graphics; Linux: pipewire / x11; Windows: DXGI)
- Gemini API key — `GEMINI_API_KEY` env var
- SQLite (bundled via `rusqlite` feature, no system install required)

**Install:**

```bash
# clone
git clone git@github.com:gyasis/gentle-eye.git
cd gentle-eye

# verify build
cargo check
cargo build --release

# config
cp .env.example .env       # ← create this; set GEMINI_API_KEY
mkdir recordings           # ← capture / DB target

# run
RUST_LOG=info,gentle_eye=debug cargo run --bin gentle-eye --release
```

**Wire to Claude Code (MCP):**

Add to your Claude Code MCP config (path varies by OS; consult Claude Code docs):

```json
{
  "mcpServers": {
    "gentle-eye": {
      "command": "/path/to/gentle-eye/target/release/gentle-eye",
      "args": [],
      "env": { "GEMINI_API_KEY": "...", "RUST_LOG": "info" }
    }
  }
}
```

**Verify:** in a Claude session, ask "take a screenshot of my display" — Claude should call the MCP tool, you should see a PNG land in `recordings/` (or wherever configured).

---

## 6 · References

- `docs/REBUILD_OVERVIEW.md` — top-level architecture summary
- `docs/REBUILD_ARCHITECTURE.md` — detailed module layout
- `docs/REBUILD_DEPENDENCIES.md` — crate inventory mined from sessions
- `docs/REBUILD_FILESYSTEM.md` — full intended directory tree (10,747 paths)
- `docs/REBUILD_KEY_CODE.md` — key code patterns / signatures
- `docs/REBUILD_API_INTEGRATIONS.md` — external API surfaces
- `docs/GENTLE_EYE_PRD.md` — product requirements (reconstructed)
- `docs/ONTOLOGY.md` — modules / structs / traits / fns inventory
- `RECOVERY/_VERDICT.md` (gitignored) — original recovery verdict
- `RECOVERY/sessions/` (gitignored) — 3,134 jsonls, the deep mining substrate

---

## 7 · Status checklist (update as you go)

- [x] Local rebuild committed (`8a1ca9c`)
- [ ] Pushed to GitHub (PR or direct, your call)
- [ ] `src/lib.rs` written
- [ ] `cargo check` passes
- [ ] `cargo build --bin gentle-eye` passes
- [ ] First successful MCP `take_screenshot` from Claude
- [ ] Storage layer wired
- [ ] Gemini analysis loop wired
- [ ] `modules/rust-record/` rebuilt
- [ ] SpecKit `specs/001-mcp-screen-tools/` rebuilt
- [ ] First public release pushed to GitHub
