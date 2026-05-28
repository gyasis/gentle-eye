# gentle-eye — rebuild tasks (Hybrid Cascade, dev-kid lightweight mode)

Format: `- [ ] T###: <verb> <what> affecting \`single/primary/path\``
**ONE path per task** — multi-path tasks blow up dev-kid's dep inference (dogfood 2026-05-26).
Mark complete with `[x]` between waves.

**Completion standard ("Triple-Zero", every task):** (1) zero `todo!()`/`unimplemented!()`,
(2) `cargo check` clean under `-D warnings` (no unused imports/vars), (3) trait-conformance —
if the file implements a `contracts`/`analysis` trait, ALL methods are implemented.
The per-task `> DONE:` line adds file-specific criteria so the sentinel has a real
definition of done (this is the predicate that prevents the SENTINEL-T002-class halt).

**Order = dependency cascade** (paired-debate 2026-05-28): skeleton → contracts → leaf logic
→ heavy lifters → providers/wiring → verify → gates. `SENTINEL-T###` wrappers are regenerated
by `/devkid.orchestrate`, not hand-listed here.

---

## Completed — recovery promotes + keystone (file-level, deterministic)

- [x] T010: Promote synthesized HIGH-conf version affecting `src/models/mod.rs`
- [x] T011: Fill stub with synthesized content affecting `src/models/analysis.rs`
- [x] T012: Verify already-clean local affecting `src/models/config.rs`
- [x] T020: Resolve stray flat-module dup (backed up `.raw`) affecting `src/contracts.rs`
- [x] T021: Choose clean recovered over noisy local affecting `src/contracts/mod.rs`
- [x] T022: Promote 13.9KB REVIEW-UPGRADE affecting `src/contracts/traits.rs`
- [x] T031: Diff and pick clean version affecting `src/storage/metadata.rs`
- [x] T032: Re-wire storage module affecting `src/storage/mod.rs`
- [x] T041: Promote 1917B clean version affecting `src/mcp/mod.rs`
- [x] T042: Promote clean recovered over gdb-junk affecting `src/mcp/tools.rs`
- [x] T080: Rebuild Structuralist pub-mod crate root affecting `src/lib.rs`

## Wave 0 — Skeleton (HAND-AUTHORED, not ma-loop)

Clears `E0583`/`E0433` so the tree resolves; trivial glue, poor ma-loop value.

- [x] T023: Author error enum (thiserror) from data-model affecting `src/contracts/errors.rs`
  > DONE: `Error` enum covers IO/Capture/Storage/Analysis/Config/Mcp variants; `#[derive(Debug, thiserror::Error)]`; usable via `?` from sibling modules.
- [x] T062: Declare submodules + re-exports, clear E0765 junk affecting `src/analysis/mod.rs`
  > DONE: `pub mod {config,traits,gemini,ollama};` + any re-export the code imports; no leftover `[Tool:]` bytes; parses clean.
- [x] T050: Declare submodules affecting `src/security/mod.rs`
  > DONE: `pub mod {path_validator,rate_limiter,uuid_validator};`; parses clean.

## Wave 1 — Contracts & interfaces (ma-loop)

The "laws" the rest of the code conforms to.

- [x] T024 [S]: Author storage trait affecting `src/contracts/storage.rs`
  > DONE: trait for recording/analysis persistence (save/get/list); object-safe if mcp stores `Box<dyn>`; matches data-model.md field names.
- [x] T061 [S]: Author VisionProvider trait affecting `src/analysis/traits.rs`
  > DONE: `VisionProvider` trait (analyze(request) -> AnalysisResult, provider_info); async per rmcp usage; gemini.rs + ollama.rs will impl it.
- [x] T060 [S]: Author analysis config (VisionConfig) affecting `src/analysis/config.rs`
  > DONE: `VisionConfig` struct selecting provider + creds/endpoint; `#[derive(Serialize,Deserialize)]`; consumed by AppConfig.

## Wave 2 — Leaf logic (ma-loop, parallelizable)

Depend on contracts; nothing depends on them.

- [x] T051 [S]: Author path validator affecting `src/security/path_validator.rs`
  > DONE: rejects traversal/symlink-escape; returns `contracts::Error`; unit tests for `../` and absolute-escape cases.
- [x] T052 [S]: Author rate limiter affecting `src/security/rate_limiter.rs`
  > DONE: per-tool token-bucket/window limiter; thread-safe; returns Error on breach.
- [x] T073 [S]: Author memory-pressure monitor affecting `src/capture/memory.rs`
  > DONE: reports/guards against unified-memory pressure before capture; no platform-pinned paths.
- [x] T053: Author UUID v4 validator affecting `src/security/uuid_validator.rs`
  > DONE: built + ma-loop-proven 2026-05-27 (re-validated at Wave 6 gate).

## Wave 3 — Heavy lifters (ma-loop, SEQUENTIAL — external crates, more iters)

- [x] T030 [S]: Author SQLite migration runner affecting `src/storage/database.rs`
  > DONE: rusqlite bundled; creates recordings + analysis tables matching data-model.md; idempotent migrations; impls `contracts::storage` trait.
- [x] T072 [S]: Author FFmpeg pipe encoder affecting `src/capture/encoder.rs`
  > DONE: pipes frames → ffmpeg → MP4; configurable fps; no Triton/x86 assumptions.
- [ ] T090 [S]: Author memory monitor affecting `modules/rust-record/video-capture/src/memory.rs`
  > DONE: workspace-member builds standalone; mirrors src/capture/memory intent.
- [ ] T091 [S]: Author capture impl affecting `modules/rust-record/video-capture/src/capture.rs`
  > DONE: scrap 0.5 capture; member crate cargo-check clean.
- [ ] T092 [S]: Author encoder impl affecting `modules/rust-record/video-capture/src/encoder.rs`
  > DONE: member-crate encoder; cargo-check clean.

## Wave 4 — Providers + MCP wiring (ma-loop)

Most complex; implement traits + carry MCP schema derives.

- [x] T063 [S]: Complete Gemini provider affecting `src/analysis/gemini.rs`
  > DONE: impls `VisionProvider`; reqwest call to Gemini; all trait methods; no hallucinated API names (check against traits.rs).
- [x] T064 [S]: Complete Ollama provider affecting `src/analysis/ollama.rs`
  > DONE: impls `VisionProvider`; targets OLLAMA_HOST; all trait methods.
- [x] T071 [S]: Complete screen capture using scrap::Capturer affecting `src/capture/screen.rs`
  > DONE: scrap 0.5 `Display`/`Capturer` API; frame grab path; cargo-check clean.
- [x] T040 [S]: Author rmcp 0.1 server entrypoint (11 tools) affecting `src/mcp/server.rs`
  > DONE: `GentleEyeServer` dispatching the 11 PRD tools; every *Input/*Output `#[derive(Serialize,Deserialize,JsonSchema)]`; re-exported by mcp/mod.rs.

## Wave 5 — Verify clean (inspection, no rewrite unless broken)

- [x] T070: Verify/dedup 192KB local module affecting `src/capture/mod.rs`
  > DONE: declares submodules; no duplicate definitions; cargo-check clean.
- [x] T074: Verify clean module affecting `src/capture/frame_rate.rs`
- [x] T075: Verify clean module affecting `src/capture/service.rs`
- [x] T076: Verify clean module affecting `src/storage/manager.rs`
- [x] T077: Verify clean module affecting `src/startup.rs`
- [x] T081: Verify clean binary entry affecting `src/bin/gentle-eye.rs`

## Wave 6 — End-to-end gates

- [x] T100: Whole workspace compiles clean affecting `Cargo.toml`
  > DONE: `cargo check` exit 0 under `-D warnings` (real exit code, not shell-wrapper-masked).
- [x] T101: Unit + integration tests green affecting `Cargo.toml`
- [x] T102: Zero clippy warnings affecting `Cargo.toml`
