# Recovery audit — `gentle-eye`

_Generated: 2026-05-13T13:25:23_  
_Sources:_  
- `recovered/path_line/` (582 files) — project-cwd path-line slicer
- `recovered/` dr_reconstructed (0 files) — dr tool_use synthesis

## Totals

| Category | Count |
|---|---|
| dependency_noise | 502 |
| user_code | 75 |
| session_meta | 5 |

| Status vs local | Count |
|---|---|
| NEW | 574 |
| REVIEW | 5 |
| UPGRADE | 3 |

## User-code recoveries (top 25 by size)

| Size | Status | Source | Rel path |
|---:|---|---|---|
| 957,431 | NEW | path_line | `src/storage/mod.rs` |
| 557,460 | UPGRADE | path_line | `src/mcp/tools.rs` |
| 465,672 | NEW | path_line | `scripts/launch.sh` |
| 267,947 | NEW | path_line | `src/mcp/server.rs` |
| 238,035 | NEW | path_line | `modules/rust-record/video-capture/src/memory.rs` |
| 192,011 | NEW | path_line | `src/capture/mod.rs` |
| 184,475 | NEW | path_line | `screen_agent_research.md` |
| 57,387 | NEW | path_line | `src/storage/metadata.rs` |
| 38,946 | NEW | path_line | `src/models/mod.rs` |
| 38,786 | UPGRADE | path_line | `src/config/mod.rs` |
| 35,729 | NEW | path_line | `docs/TOOLS.md` |
| 17,711 | NEW | path_line | `src/lib.rs` |
| 17,687 | NEW | path_line | `specs/001-mcp-screen-tools/tasks.md` |
| 17,177 | NEW | path_line | `library_license_rust_analysis.md` |
| 13,601 | REVIEW | path_line | `src/models/config.rs` |
| 13,568 | NEW | path_line | `src/config/loader.rs` |
| 12,640 | NEW | path_line | `src/contracts/traits.rs` |
| 12,321 | NEW | path_line | `src/contracts/mod.rs` |
| 12,182 | NEW | path_line | `specs/001-mcp-screen-tools/data-model.md` |
| 10,130 | NEW | path_line | `src/storage/manager.rs` |
| 10,113 | NEW | path_line | `dayflow_analysis.md` |
| 6,059 | UPGRADE | path_line | `README.md` |
| 5,672 | NEW | path_line | `modules/rust-record/video-capture/tests/service_integration.rs` |
| 5,434 | NEW | path_line | `docs/gentle-eye.postman_collection.json` |
| 3,964 | NEW | path_line | `specs/001-mcp-screen-tools/plan.md` |

_+50 more user-code files (see `recovered/`)._

## Session/meta material (top 10 by size — buried code may live here)

| Size | Source | Rel path |
|---:|---|---|
| 298,213 | path_line | `.specstory/history/2025-12-22_12-27-34Z-use-context7-to-research.md` |
| 9,823 | path_line | `.specstory/history/2025-12-21_14-09-02Z-explore-the-folder-home.md` |
| 1,688 | path_line | `.specify/memory/constitution.md` |
| 692 | path_line | `.git/cursor/crepe/d637ab24d8938c71e641c62a642d7299b149e25c/metadata.json` |
| 127 | path_line | `.specstory/history/2025-12-22_12-40-22Z-design-a-detailed-implementation.md` |

## What to do next

1. `recovered/` and `recovered/path_line/` are NEVER promoted automatically — they're staged for review.
2. For each `UPGRADE`/`NEW` row above, run a quick `diff` vs local then copy if the content is real.
3. Files >500KB are SUSPICIOUSLY LARGE for source code — the path-line slicer often grabs trailing concatenated content from later paths. Inspect before promoting.
4. `junk_dup_name` rows (e.g. `main.pymain.py.py`) are slicing artifacts — discard them; the clean name has the canonical content.
5. To rebuild from spec rather than file-by-file, invoke the `disaster-recovery-rebuild-from-spec` skill.

## Rollback

No files have been promoted into the live project tree — only staged in `recovered/`. Delete `recovered/` to undo.


---

## Smart auto-promote run — 2026-05-13T13:27:45

_Epoch: `1778693264`_  

| Decision | Count |
|---|---|
| SKIP_NOT_USER_CODE | 507 |
| PROMOTE_NEW | 63 |
| SKIP_LOCAL_BIGGER | 5 |
| SKIP_OVERSIZE | 5 |
| UPGRADE | 2 |

### Promoted NEW files (63)

| Size | Rel path |
|---:|---|
| 192,011 | `src/capture/mod.rs` |
| 184,475 | `screen_agent_research.md` |
| 57,387 | `src/storage/metadata.rs` |
| 38,946 | `src/models/mod.rs` |
| 35,729 | `docs/TOOLS.md` |
| 17,711 | `src/lib.rs` |
| 17,687 | `specs/001-mcp-screen-tools/tasks.md` |
| 17,177 | `library_license_rust_analysis.md` |
| 13,568 | `src/config/loader.rs` |
| 12,640 | `src/contracts/traits.rs` |
| 12,321 | `src/contracts/mod.rs` |
| 12,182 | `specs/001-mcp-screen-tools/data-model.md` |
| 10,130 | `src/storage/manager.rs` |
| 10,113 | `dayflow_analysis.md` |
| 5,672 | `modules/rust-record/video-capture/tests/service_integration.rs` |
| 5,434 | `docs/gentle-eye.postman_collection.json` |
| 3,964 | `specs/001-mcp-screen-tools/plan.md` |
| 2,742 | `specs/001-mcp-screen-tools/contracts/traits.rs` |
| 1,646 | `memory-bank/progress.md` |
| 1,614 | `src/mcp/mod.rs` |
| 1,586 | `modules/rust-record/video-capture/src/lib.rs` |
| 1,235 | `gentle-eye.toml` |
| 1,084 | `GENTLE_EYE_VISION.md` |
| 965 | `src/analysis/ollama.rs` |
| 852 | `modules/rust-record/video-capture/src/frame_rate.rs` |
| 846 | `src/capture/frame_rate.rs` |
| 725 | `src/mcp/errors.rs` |
| 631 | `src/analysis/gemini.rs` |
| 593 | `src/capture/service.rs` |
| 574 | `specs/001-mcp-screen-tools/research.md` |

_... +33 more._

### Upgraded files (2)

| New size | Old size | Δ bytes | Rel path |
|---:|---:|---:|---|
| 38,786 | 15,798 | +22,988 | `src/config/mod.rs` |
| 6,059 | 3,669 | +2,390 | `README.md` |

### Rollback

```bash
cd ~/Documents/code/gentle-eye
for f in $(find . -name '*.raw.1778693264'); do
  mv "$f" "${f%.raw.1778693264}"
done
```


---

## Format-validated trim + promote — 2026-05-13T13:37:48

_Epoch: `1778693867`_  
_Source: `recovered_validated/` (Gemini-pattern + format-validation trimmed)_

| Decision | Count |
|---|---|
| PROMOTE_NEW | 2 |
| UPGRADE | 1 |
| SKIP_local_bigger | 0 |

### Promoted NEW (validated trims)

| Size | Rel |
|---:|---|
| 465,672 | `scripts/launch.sh` |
| 17,906 | `src/storage/mod.rs` |

### Upgraded (validated > local)

| New | Old | Δ | Rel |
|---:|---:|---:|---|
| 29,004 | 12,999 | +16,005 | `src/mcp/tools.rs` |


---

## Format-validated trim + promote — 2026-05-13T17:27:53

_Epoch: `1778707673`_  
_Source: `recovered_validated/` (Gemini-pattern + format-validation trimmed)_

| Decision | Count |
|---|---|
| PROMOTE_NEW | 0 |
| UPGRADE | 0 |
| SKIP_local_bigger | 3 |

### Skipped (local already bigger)

- `scripts/launch.sh` (validated 465,672 vs local 465,672)
- `src/mcp/tools.rs` (validated 29,004 vs local 29,004)
- `src/storage/mod.rs` (validated 17,906 vs local 17,906)
