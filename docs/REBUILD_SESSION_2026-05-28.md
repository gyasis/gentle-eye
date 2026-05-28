# gentle-eye Rebuild Session — 2026-05-27/28

This session advanced the post-wipe rebuild and **dogfooded dev-kid** on it
(findings fed back into dev-kid's orchestrator — see that repo's
`docs/architecture/SENTINEL_ORCHESTRATOR_REWORK_2026-05-28.md`). Tracking PRD:
`gentle_eye_devkid_dogfood_2026-05-26` (`~/dev/prd/scratch/`).

## 1. Recovery is exhausted — what was deterministically recovered

The local `[Tool:]` "container" files in `src/` were **build-process logs**
(repeated `cargo check`, gdb-symbol dumps, even browser-cache JSON), **not**
source. The real Write-captured source lived in `recovered/path_line/` +
`RECOVERY/sessions/_seg*.md`; `tooldump_parser.py` reconstructs it, and
`recovered_review/` already held the complete deterministic yield. No further
recovery passes are warranted.

### Files promoted (junk/stub → clean recovered), each with a `.raw.<epoch>` backup

| File | Before | After |
|---|---|---|
| `src/storage/metadata.rs` | 57 KB cargo-log junk | 15 KB real (44 defs) |
| `src/storage/mod.rs` | 17.9 KB browser-cache JSON+certs | 515 B real |
| `src/mcp/tools.rs` | 29 KB gdb-symbol junk | 13 KB real (28 defs) |
| `src/mcp/mod.rs` | 1.6 KB | 1.9 KB |
| `src/models/mod.rs` | 39 KB `[Tool:]` container | 10.7 KB real (32 defs) |
| `src/contracts/mod.rs` | 12 KB junk | 1.3 KB real |
| `src/contracts/traits.rs` | 12.6 KB | 13.9 KB (49 defs) |
| `src/config/loader.rs` | 13.6 KB junk | 3.2 KB real |

(`src/models/analysis.rs` was promoted earlier from its `.synthesized.rs`.)
The two "big but sparse" ambiguous files (`mcp/tools.rs`, `storage/mod.rs`) were
resolved **deterministically** by inspection — no LLM/Gemini spend needed.

## 2. Keystone: `lib.rs` rebuilt (Structuralist)

`src/lib.rs` was junk; rebuilt as a **table-of-contents** crate root (decided via
Claude×Gemini paired-debate):

- 9 `pub mod` declarations, **no crate-root re-exports** (re-exports live in each
  submodule's `mod.rs`, where the binary uses them; a lib-root `pub use` would be
  an unused-import **hard error** under the `-D warnings` gate).
- **No `#![deny(warnings)]`** — redundant with `.cargo/config.toml` and brittle
  across rustc versions.
- All modules `pub` so the binary + integration tests (separate crates) see them.

### Module-layout dedup

Rust forbids both `X.rs` and `X/mod.rs`. The stray flat files
`src/{config,contracts,models}.rs` (older/smaller/stub) were backed up to
`.raw.<epoch>` in favor of the richer dir-modules (which also have submodules).

## 3. `.cargo/config.toml` fixed

The recovered `.cargo/config.toml` had a leaked markdown code-fence (` ```toml `)
on line 1 + another file's text at the bottom, so cargo couldn't parse the
manifest and bailed before compiling anything. Stripped to the real config
(`[build] rustflags = ["-D","warnings"]`, `[alias] lint = …`); backup at
`.cargo/config.toml.raw.<epoch>`.

**Baseline after the fix:** all dependencies compile (the recovered `Cargo.toml`
— `scrap 0.5`, `rmcp 0.1`, `reqwest`, `tokio`, … — is correct). Remaining work is
the genuine never-written stubs.

## 4. `tasks.md` regenerated — Hybrid Cascade

`tasks.md` was reorganized into dependency-ordered waves with per-task `> DONE:`
completion criteria (Triple-Zero: no `todo!()`/`unimplemented!()`, `cargo check`
clean under `-D warnings`, trait-conformance):

- **Wave 0 — Skeleton (hand-authored):** `contracts/errors.rs`, `analysis/mod.rs`,
  `security/mod.rs` (clears `E0583`/`E0433`).
- **Wave 1 — Contracts/traits → 2 — Leaf logic → 3 — Heavy lifters (storage,
  FFmpeg encoder) → 4 — Providers + MCP server** (ma-loop targets).
- **Wave 5 — Verify clean → 6 — Gates** (cargo check / test / clippy).

> **TODO before re-orchestrate:** add `[S]` markers to the real+compilable
> file-points in `tasks.md` so dev-kid's new sentinel-point predicate places
> sentinels correctly (skeleton waves must stay unmarked). See the dev-kid rework
> doc.

## 5. Status

`tasks.md`: recovery/keystone tasks complete; the never-written-stub build is the
remaining dogfood. The dev-kid orchestrator was reworked (a/b/c) based on findings
here and will be exercised end-to-end on the next orchestrate+execute run.
