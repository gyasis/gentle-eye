# gentle-eye — Wave 2+ rebuild tasks (dogfood plan)

Generated **2026-05-26** from the recovery gap map (`RECOVERY_AUDIT.md`,
`recovered_review/REVIEW_REPORT.md`, GENTLE_EYE_PRD.md §8) + the 70 TodoWrite
sequences SIO surfaced from session segments.

Branch: `001-mcp-screen-tools`. Tech-stack lock: `specs/001-mcp-screen-tools/dependencies.md`.
Sentinel test: `cargo check`. Local tier: qwen3-coder:30b on Mac ollama via LAN.

`[P]` = can run in parallel within its wave.

---

## Wave 0 — Spec backfill (FOUNDATIONAL · blocks /devkid.orchestrate)

- [ ] T001 [P] Backfill `specs/001-mcp-screen-tools/spec.md` from `docs/GENTLE_EYE_PRD.md` §§1–7 (vision · personas · architecture · 11 MCP tools · functional · non-functional · data model).
- [ ] T002 [P] Backfill `specs/001-mcp-screen-tools/analysis-report.md` with current rebuild snapshot (file map · gaps · dogfood status).
- [x] T003 [P] `.specify/memory/constitution.md` mirrored from `memory-bank/shared/.constitution.md` (this commit).

## Wave 1 — Models layer

- [ ] T010 Promote `src/models/mod.rs` from `recovered_review/src/models/mod.rs.synthesized.rs` (HIGH conf). Verify `Recording` / `RecordingStatus` / `EncoderMode` match downstream imports.
- [x] T011 `src/models/analysis.rs` — PROMOTED 2026-05-26 (was 27B stub → 542 lines, MED conf).
- [ ] T012 `src/models/config.rs` — already clean local; assert byte-equivalent to synthesized.

## Wave 2 — Contracts layer

- [ ] T020 `src/contracts.rs` (27B stub) — decide: collapse into `src/contracts/mod.rs` OR populate as re-export shim.
- [ ] T021 `src/contracts/mod.rs` — diff noisy 12KB local vs `recovered_review/src/contracts/mod.rs` (1.3KB clean); pick.
- [ ] T022 [P] `src/contracts/traits.rs` — promote `recovered_review/src/contracts/traits.rs` (REVIEW-UPGRADE: 13.9KB > 12.6KB local).
- [ ] T023 [P] `src/contracts/errors.rs` (1B EMPTY) — author from `data-model.md` + sibling errors pattern.
- [ ] T024 [P] `src/contracts/storage.rs` (27B stub) — author storage trait.

## Wave 3 — Storage layer

- [ ] T030 `src/storage/database.rs` (1B EMPTY) — author SQLite migration runner per `rusqlite = "0.31"` bundled.
- [ ] T031 `src/storage/metadata.rs` — diff noisy 57KB local vs recovered 15KB; pick.
- [ ] T032 `src/storage/mod.rs` — re-wire after T030/T031.

## Wave 4 — MCP server

- [ ] T040 **`src/mcp/server.rs` (MISSING)** — author `rmcp 0.1` server entrypoint wiring the 7 MCP tools (`start_recording` / `stop_recording` / `get_recording_status` / `analyze_video` / `list_recordings` / `cancel_recording` / `get_vision_provider_info`) per `specs/001-mcp-screen-tools/contracts/mcp-tools.json`.
- [ ] T041 `src/mcp/mod.rs` — promote `recovered_review/src/mcp/mod.rs` (1917B vs 1614B clean).
- [ ] T042 `src/mcp/tools.rs` — keep clean local 29KB; assert.

## Wave 5 — Security layer (4 EMPTY 1-BYTE FILES — all to author)

- [ ] T050 [P] `src/security/mod.rs` — module declarations + re-exports.
- [ ] T051 [P] `src/security/path_validator.rs` — validate `file_path` against `allowed_video_paths` (per `gentle-eye.toml` `[security]`).
- [ ] T052 [P] `src/security/rate_limiter.rs` — enforce `analyze_rate_limit_per_minute = 10`.
- [ ] T053 [P] `src/security/uuid_validator.rs` — validate `recording_id` UUID v4.

## Wave 6 — Analysis layer

- [ ] T060 [P] `src/analysis/config.rs` (1B) — author wiring `VisionConfig`.
- [ ] T061 [P] `src/analysis/traits.rs` (1B) — author `VisionProvider` trait.
- [ ] T062 `src/analysis/mod.rs` (27B) — wire `gemini.rs` + `ollama.rs` + traits.
- [ ] T063 `src/analysis/gemini.rs` — complete from current 631B partial per spec.
- [ ] T064 `src/analysis/ollama.rs` — complete from current 965B partial per spec.

## Wave 7 — Capture layer cleanup

- [ ] T070 `src/capture/mod.rs` — review massive 192KB local; deduplicate if mention-aggregation, keep if real.
- [ ] T071 `src/capture/screen.rs` (336B stub) — complete per spec (uses `scrap::Capturer`).
- [ ] T072 [P] `src/capture/encoder.rs` (27B) — author FFmpeg pipe encoder.
- [ ] T073 [P] `src/capture/memory.rs` (1B) — author memory-pressure monitor.
- [ ] T074 `src/capture/frame_rate.rs` — verify clean (846B).
- [ ] T075 `src/capture/service.rs` — verify clean (593B).

## Wave 8 — Lib + Bin

- [ ] T080 `src/lib.rs` — reconstruct as clean `pub mod` declarations matching the `src/` tree (drop the 17KB JSON-noise).
- [ ] T081 `src/bin/gentle-eye.rs` — verify clean local 4.6KB binary entry.

## Wave 9 — Modules workspace (separate Dayflow-derived)

- [ ] T090 `modules/rust-record/video-capture/src/memory.rs` (MISSING) — author.
- [ ] T091 [P] `modules/rust-record/video-capture/src/capture.rs` (27B) — author.
- [ ] T092 [P] `modules/rust-record/video-capture/src/encoder.rs` (27B) — author.

## Wave 10 — End-to-end gates

- [ ] T100 `cargo check` — whole workspace compiles clean.
- [ ] T101 `cargo test` — unit tests green.
- [ ] T102 `cargo clippy -- -D warnings` — no warnings.
- [ ] T103 Manual: gentle-eye MCP server starts; one `start_recording → stop_recording → analyze_video` round-trip succeeds end-to-end.

---

## Dogfood gates (operate at every wave checkpoint)

- Sentinel runs `cargo check` after each wave; failing tasks go back to micro-agent (tier escalation per `ralph-tiers.json`).
- Atelier monitors Mac ollama warm/cold for the local tier (qwen3-coder:30b / gemma3:27b / deepseek-r1:32b).
- **Halt-and-fix:** any dev-kid / sentinel / micro-agent / atelier bug = the valued finding. Stop, capture, fix the TOOL, resume.
