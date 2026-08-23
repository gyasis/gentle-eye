# gentle-eye — Dayflow Mode tasks (dev-kid dogfood plan)

Generated **2026-05-28** from PRD `gentle_eye_dayflow_mode_2026-05-28` (§5 design,
§7 phased plan) + the locked architecture decisions (below). This is the **clean
greenfield feature** to properly dogfood dev-kid orchestrate/execute + the
Integration Sentinel + the ma-loop — the gentle-eye rebuild that preceded it
devolved into manual recover→debate→synthesize because the work wasn't a
well-specified forward build. Dayflow IS.

Branch: `002-dayflow-mode` (off `main`). Sentinel test: `cargo check` (see dev-kid.yml).
Local tier: Mac ollama LAN; escalation per `ralph-tiers.json`.

## Locked decisions (2026-05-28)

| # | Decision | Choice |
|---|---|---|
| D1 | Map-Reduce chunk-summarizer | **Rust-native port** of videolocr `process_video_chunks_with_gemini` (rolling `CONTEXT SUMMARY FOR NEXT CHUNK`). No Python dep — fully dogfoodable. |
| D2 | Default vision provider | **Gemini** (native-video, motion-aware). Ollama = privacy fallback. |
| D3 | Record model | **BOTH** — session-based (explicit 2h/5h sessions) **and** a continuous daemon. Same chunk→summarize→timeline pipeline underneath. |
| D4 | Summarization timing | **Real-time** — a background task summarizes each 15-min chunk as the session proceeds. |
| D5 | Retention | **3-tier** (Hot raw → Warm shrunk → Cold timeline-only) + disk-budget evict guard. Timeline is the permanent artifact; raw video is scaffolding. fps + retention windows are duration-aware/configurable ("depends on the kind of videos"). |

## Conventions

- `[P]` = parallelizable within its wave.
- `[S]` = **sentinel checkpoint**: on completion the crate is compilable AND this
  task is a complete, runnable file with a clear objective (all its deps are green
  → safely fixable in isolation). The orchestrator places a sentinel here.
  Skeleton / pure-declaration tasks get **no** `[S]`.
- Every task carries a **`> DONE:`** completion criterion the sentinel/ma-loop checks.

Reuse — do NOT rebuild (existing, green): `capture/service.rs` (`CaptureService:
RecordingService`), `capture/{frame_rate,encoder,screen,memory,stream}.rs`,
`analysis/{gemini,ollama,ocr,traits}.rs` (`VisionProvider`), `storage/{manager,
database,metadata}.rs` (`StorageManager`), `mcp/{server,tools}.rs`,
`bin/gentle-eye.rs` (CLI), `api.rs` (HTTP), `contracts/traits.rs`.

---

## Wave 0 — Foundation: module + models + contracts (skeleton, NO sentinel)

Pure declarations to clear `E0583`/`E0433` so later waves compile incrementally.

- [x] T200 [P] `src/dayflow/mod.rs` — module root: `pub mod {models, engine, summarizer, timeline, retention, daemon}` + re-exports. Wire `pub mod dayflow;` into `src/lib.rs`.
      `> DONE:` lib.rs declares `dayflow`; `cargo check` resolves the module tree (stubs allowed).
- [x] T201 [P] `src/dayflow/models.rs` — `TimelineEntry { recording_id: Uuid, start_time, end_time: DateTime<Utc>, category: ActivityCategory, app: String, activity: String, summary: String }`, `ActivityCategory` enum (Coding/Docs/Comms/Browsing/Meeting/Idle/Other), `ChunkSummary`, `RollingContext`, `DayflowSession`, `DayflowStatus`. Derive serde + schemars.
      `> DONE:` types compile; `TimelineEntry` derives `FromRow`-compatible fields (match the Wave 4 SQLite columns exactly).
- [x] T202 [P] `src/dayflow/errors.rs` (or extend `contracts/errors.rs`) — `DayflowError` enum + `From` convs + `mcp_error_code()` mapping (mirror `GentleEyeError`).
      `> DONE:` `DayflowError` maps into `GentleEyeError`; compiles.
- [x] T203 Extend `src/config/mod.rs` — `DayflowConfig { chunk_minutes: 15, record_fps: 0.5, default_provider: Gemini, retention: RetentionConfig, disk_budget_bytes }`; sane serde defaults.
      `> DONE:` `DayflowConfig::default()` round-trips through the config loader; compiles.

## Wave 1 — fps heuristic (PRD P0 · quick · gate-green) `[S]`

- [x] T210 [S] Duration-aware fps heuristic in `capture/frame_rate.rs` (≤30s→5–30; 30s–15min→1; 15min–hours→0.2–0.5) + **document it** in the `start_recording` MCP tool description AND `docs/FPS_AND_DAYFLOW.md`.
      `> DONE:` heuristic fn returns 0.2–0.5 for dayflow-tier durations (unit test); `docs/FPS_AND_DAYFLOW.md` exists; tool desc mentions the table; `cargo check` green.

## Wave 2 — Long recording + on-the-fly 15-min chunking (PRD P1) `[S]`

- [ ] T220 Extend `capture/encoder.rs` / `capture/service.rs` for **segmented capture** — ffmpeg segment muxer emitting 15-min chunk files on the fly (mirror videolocr `download_and_split_video`). Chunk index + wall-clock start/end recorded.
      `> DONE:` a long capture produces sequential `chunk_NNN.mp4` files at the configured boundary.
- [x] T221 [P] Chunk manifest — `ChunkRef { index, path, start_wall, end_wall }` emitted per segment; `MemoryMonitor` integration (file-based encoder under pressure, reuse `capture/memory.rs`).
      `> DONE:` manifest enumerates all chunks of a session; memory-pressure path exercised in a test.
- [x] T222 [S] Chunking integration — a simulated/short 35-min-equivalent capture yields 3 chunks with correct boundaries.
      `> DONE:` deterministic test asserts N chunks + monotonic non-overlapping time ranges; `cargo check` + test green.

## Wave 3 — Rust-native Map-Reduce summarizer (PRD P2 · D1) `[S]`

- [x] T230 `src/dayflow/summarizer.rs` — `ChunkSummarizer` trait + impl over `VisionProvider`: `summarize_chunk(chunk: &ChunkRef, prior: &RollingContext) -> Result<ChunkSummary, DayflowError>`. Default provider = **Gemini** (D2), native-video `analyze_video`; Ollama frame+OCR fallback.
      `> DONE:` summarize_chunk returns a structured `ChunkSummary`; deterministic test with a **stub VisionProvider** passes.
- [x] T231 [P] Map step — port the rolling `CONTEXT SUMMARY FOR NEXT CHUNK`: each chunk receives the prior chunk's `RollingContext`; context threads forward.
      `> DONE:` test proves chunk N's prompt embeds chunk N-1's rolling context.
- [x] T232 [P] Reduce step — combine per-chunk `ChunkSummary` → a session digest.
      `> DONE:` reduce over 3 stub summaries yields a coherent digest; test.
- [x] T233 [S] Map-Reduce end-to-end (stub provider) — 3 chunks → 3 structured entries with threaded context + 1 digest.
      `> DONE:` integration test green; `cargo check` + clippy `-D warnings` = 0.

## Wave 3A — Two-tier perception ladder: OCR base + VLM escalation (Amendment A · supersedes D2) `[S]`

Measured 2026-08-23 on a live 1920x1080 ATEM frame through the Atelier governor.
Dayflow is an OCR-THROUGHPUT problem, not a vision-understanding problem: at 0.5 fps
all day, a general VLM per chunk cannot keep up. Spend the vision model only on meaning.

- [ ] T300 [P] `analysis/` — add an OCR-tier provider over `deepseek-ocr:3.3b` (governed lane, no `think` tag). Text extraction ONLY; must not be routed through the VLM path.
      `> DONE:` a frame returns text in under 5s warm; unit test with a stub provider; `cargo check` green.
- [ ] T301 [P] Region-crop BEFORE OCR — feed the OCR tier full-res crops from the region cascade (`regions/`), never a downscaled full frame. Reuse `target` / `measure_target`; no new detection code (see `docs/REGION_ENGINE.md` §0).
      `> DONE:` a two-pane frame yields per-pane text in correct reading order; a test asserts the full-frame column-scramble does NOT occur.
- [ ] T302 [S] Tier router — OCR tier answers text queries; escalate to the VLM tier (`ornith-1.5-9b`) only for semantic/relational questions (category, "what was happening"). Escalation must be explicit and logged, never implicit.
      `> DONE:` a text query resolves without loading a VLM; a semantic query escalates once; both paths covered by tests; `cargo check` + clippy `-D warnings` = 0.

## Wave 4 — Timeline store + real-time scheduler (PRD P3 · D4) `[S]`

- [x] T240 `storage/database.rs` migration — `timeline_entries` table matching `TimelineEntry` columns EXACTLY (idempotent runner, `init_in_memory` for tests).
      `> DONE:` migration creates the table; in-memory test inserts+selects a row.
- [x] T241 `src/dayflow/timeline.rs` — `TimelineStore` trait + impl over `StorageManager`'s connection: `insert_entry`, `query_range(from,to)`, `count`. Injection-safe.
      `> DONE:` in-memory round-trip test (insert → query_range returns ordered entries).
- [ ] T242 Real-time scheduler — background tokio task that, every `chunk_minutes` during an active session, summarizes the just-closed chunk (T230) and writes its `TimelineEntry` (T241). Channel-based, concurrency-safe with the capture loop.
      `> DONE:` a running session with stub provider produces timeline entries live (not only after stop); test with a fast clock.
- [ ] T243 [S] `ask_day(question) -> String` — grounds Q&A on `query_range` entries (timeline as context to the provider).
      `> DONE:` `ask_day` over seeded entries returns a grounded answer (stub provider); `cargo check` + tests green.

## Wave 4A — Structural timeline: regions carry the layout, OCR carries the text (Amendment A) `[S]`

Cropping gives correct text but discards layout. Structure must come from the region
cascade (`Region.parent` + `bbox` IS the layout tree), NOT from the OCR model. The
schema is the expensive thing to reverse, so it carries geometry from day one even
while only text is populated.

- [ ] T310 `storage/database.rs` — additive migration extending `timeline_entries` with `region_id`, `bbox` (x,y,w,h), `parent_region_id`, all nullable. Idempotent; must not rewrite the T240 migration.
      `> DONE:` migration is additive and re-runnable; existing rows survive; in-memory insert+select round-trips the new columns.
- [ ] T311 [S] Join OCR text to the region tree on `RegionId` — reading order COMPUTED from bbox geometry, never requested from the model. `TimelineEntry` gains its region provenance.
      `> DONE:` a two-pane capture yields entries whose parent/bbox reconstruct the on-screen layout; ordering test is deterministic; `cargo check` + tests green.

## Wave 5 — Both record models: session + daemon (PRD P4 · D3) `[S]`

- [ ] T250 `src/dayflow/engine.rs` — `DayflowEngine` trait + impl: `start_session(opts)` / `stop_session(id)` / `status()`. Session mode honors a max duration (2h/5h/etc, capped per config).
      `> DONE:` session start→stop drives Wave 2–4 pipeline; max-duration cap honored in a test.
- [ ] T251 `src/dayflow/daemon.rs` — continuous daemon: long-lived background capture, auto-rolling sessions/segments across the day, lifecycle (start/stop/status) + persisted state/pid.
      `> DONE:` daemon starts, auto-segments continuously, reports status, stops cleanly; test with a short interval.
- [ ] T252 [S] Both modes share one pipeline — session + daemon both feed chunk→summarize→timeline.
      `> DONE:` parametrized test runs the pipeline in both modes; `cargo check` + tests green.

## Wave 5A — Daemon liveness must be provable, not reported (Amendment A) `[S]`

A long-lived capture daemon is the textbook false-green failure: it reports "started,
healthy" while writing nothing, and a whole day of empty timeline looks fine until the
next morning. See `~/.claude/antipatterns/background-jobs.md` (AP-BG2): verify the
ARTIFACT, never the exit code or the status flag.

- [ ] T330 Daemon status must carry evidence — `chunks_written`, `last_chunk_at`, `last_summary_at`. "Alive" and "alive but producing nothing" MUST be distinguishable by a caller.
      `> DONE:` a daemon that captures zero chunks reports a distinguishable degraded status; test asserts the healthy path advances `chunks_written`.
- [ ] T331 [S] Model residency vs governor idle-unload — cold load measured at 10.3s vs 2.6s warm, so a 15-min chunk cadence will always hit cold. Either keep the OCR tier resident or budget the reload; the choice must be explicit and configured.
      `> DONE:` a multi-chunk run records per-chunk latency incl. any reload; config knob documented; `cargo check` + tests green.

## Wave 6 — Tiered retention: save → shrink → archive + disk guard (D5) `[S]`

- [ ] T260 [P] `src/dayflow/retention.rs` — `RetentionConfig { hot_grace_hours, warm_days, disk_budget_bytes }` + tier state machine (Hot/Warm/Cold).
      `> DONE:` tier transitions computed from age + summarized-flag; unit test.
- [ ] T261 **Shrink** step — after a chunk is summarized (T242), transcode raw → timelapse / N change-threshold frames + OCR text (reuse `analysis/ocr.rs`; videolocr `videoocr.py` change-extraction as blueprint), replacing the raw chunk.
      `> DONE:` post-summarize, raw chunk is replaced by a warm artifact ≤10% size; test asserts size drop + OCR text retained.
- [ ] T262 **Evict** step — disk-pressure-driven eviction mirroring `MemoryMonitor` warm/cold/evict: over budget → drop oldest raw (already summarized), then oldest warm; **never** the timeline DB.
      `> DONE:` simulated over-budget evicts raw-then-warm in age order; timeline_entries untouched; test.
- [ ] T263 [S] Retention end-to-end — summarize → shrink → (over-budget) evict, timeline preserved throughout.
      `> DONE:` integration test proves total bytes drop while `query_range` still returns every entry; `cargo check` + tests green.

## Wave 7 — Surfaces: MCP + CLI + HTTP `[S]`

- [ ] T270 `mcp/tools.rs` + `mcp/server.rs` — add `start_dayflow` / `stop_dayflow` / `get_timeline` / `ask_day` / `dayflow_status` (schemars input/output schemas + dispatch over `DayflowEngine`/`TimelineStore`).
      `> DONE:` `tools/list` shows the new tools; a `call_tool` round-trip for each returns valid JSON (stub provider).
- [ ] T271 [P] `bin/gentle-eye.rs` — CLI subcommands `dayflow start|stop|timeline|ask|status` (JSON out, reuse lib).
      `> DONE:` each subcommand prints valid JSON; `dayflow status` reflects engine state.
- [ ] T272 [P] `api.rs` — HTTP `POST /dayflow/start`, `POST /dayflow/stop`, `GET /dayflow/timeline`, `POST /dayflow/ask`, `GET /dayflow/status` (reuse lib, no new deps).
      `> DONE:` each endpoint returns correct JSON on the hand-rolled server; live curl test.
- [ ] T273 [S] All three front-ends drive dayflow against the same engine.
      `> DONE:` MCP + CLI + HTTP each start→status→timeline; `cargo check` + clippy `-D warnings` = 0.

## Wave 8 — Categories + standup view (PRD P4) `[S]`

- [ ] T280 Activity-category taxonomy applied in the summarizer prompt (D5 categories) → `TimelineEntry.category` populated.
      `> DONE:` summaries carry a category; test asserts category ∈ taxonomy.
- [ ] T281 [S] Standup/highlights view — yesterday's highlights / today's priorities / a GitHub-style activity grid, surfaced via `ask_day` + a `get_timeline --standup` shape.
      `> DONE:` `ask_day("what did I do today")` returns a categorized, time-ranged digest; `cargo check` + tests green.

## Wave 9 — Gates `[S]`

- [ ] T290 [S] `cargo test` — all unit + integration tests green.
      `> DONE:` `./.tooling/bin/cargo test` exit 0.
- [ ] T291 [S] `cargo clippy --all-targets -- -D warnings` — zero warnings.
      `> DONE:` clippy exit 0.
- [ ] T292 Live validation (ignored test) — real session → 15-min chunks → **Gemini** Map-Reduce → timeline → `ask_day`; success-criterion = a coherent queryable activity timeline ("what was I doing at 2pm?"). Plus the **local-first** path on LAN Ollama.
      `> DONE:` `cargo test --test dayflow_live -- --ignored` produces a real timeline; both provider paths exercised.

---

## Run mode: dev-kid LITE + in-session checkpoints (2026-05-28)

We run this with **dev-kid lite**: it only needs a `tasks.md` to orchestrate waves
on (`.dk/tasks.md`). dev-kid lite **dispatches** Wave N to the Developer agent
(in-session Claude) to implement, then waits for `[x]`. The **`[S]` checkpoints are
done by the in-session agent** — i.e. *you* run `cargo check` (`./.tooling/bin/cargo
check`) at each `[S]`, not the full autonomous sentinel/ma-loop. ma-loop / tier
escalation is the fallback only if you choose to invoke it on a stuck file.

- At each `[S]`: run `cargo check` (and at gate waves, `cargo test` + `cargo clippy
  -D warnings`). Green → mark the wave `[x]` and advance. Red → fix in place
  (attribution-aware: if the primary error span is a *dependency* file, fix that,
  don't mangle the target), then re-check.
- **Halt-and-fix:** any dev-kid / sentinel / ma-loop bug encountered = the valued
  finding. Stop, capture, fix the TOOL, resume. (This is the whole point of the dogfood.)


---

## Amendment A (2026-08-23) — measured perception ladder, structural timeline, daemon liveness

Authored mid-build after the governor/vision work landed on `fix/ollama-governor-base-url`
(commits `6b256ab`, `d2f5192`). Originals above are untouched and remain the audit trail;
`T2xx` handles are permanent. New work uses the `T3xx` range.

### A.1 — Evidence (measured, same live ATEM frame, via the Atelier governor)

| path | warm latency | tokens | outcome |
|---|---|---|---|
| tesseract (`read-text`) | ~instant | n/a | garbled; roughly half unusable on dark-theme terminal text |
| `deepseek-ocr:3.3b`, FULL frame | 2.6s (cold 10.3s) | 444 | near-verbatim BUT columns scrambled across panes; `CLI` misread as `CI` |
| `deepseek-ocr:3.3b`, CROPPED pane | **1.6s** | **231** | correct order, `CLI` correct — faster, cheaper AND more accurate |
| `ornith-1.5-9b` (VLM) | 39s | 787 | verbatim, but ~15x the cost of the OCR tier |

Cropping won on every axis at once. This independently reproduces the
`sparse-delta-perception` (Lookout) finding of 2026-06-27 — whole-screen single reads
lose text to downscaling; a locked full-res region is more accurate AND cheaper. It also
settled a disputed digit three ways (`18m 47s`), where the downscaled full-frame read
was wrong.

### A.2 — Decision amendments

| # | Original | Amended | Why |
|---|---|---|---|
| D2 | Default vision provider = **Gemini**; Ollama = privacy fallback | **Two tiers.** Base = `deepseek-ocr:3.3b` (text). Escalation = `ornith-1.5-9b` (meaning). Cloud is opt-in, not default. | D2 predates the governed fleet (22 vision-capable models). Dayflow records the screen ALL DAY, making it the most privacy-sensitive surface in the toolkit — a cloud default is a decision, not an inheritance. |
| D6 | *(new)* | **Low-compute first.** Never spend a vision model on a job OCR can do. | Streaming throughput, not intelligence, is the binding constraint. |
| D7 | *(new)* | **Structure comes from the region cascade, text from OCR, joined on `RegionId`.** Reading order is computed from geometry. | Cropping alone discards layout. `Region.parent` + `bbox` already model it (`docs/REGION_ENGINE.md`), and it is far cheaper than asking any model. |
| D8 | *(new)* | **All model calls route through the Atelier governor** (`:8799/llm/ollama`), never raw `:11434`. | Admission + auto-unload; raw access risks the unified-memory swap-death. Enabled by `6b256ab`; thinking-model output cleaned by `d2f5192`. |

### A.3 — Sequencing (unchanged tasks, corrected expectations)

`T220` (ffmpeg segment muxer) and `T242` (real-time scheduler) are **still open** despite
their waves reading as complete — 13 of 32 boxes are ticked overall. Nothing in Wave 5 is
testable until `T220` emits real chunk files, so run the waves in file order rather than
jumping to Wave 5.

### A.4 — Open, deliberately not decided here

Whether `deepseek-ocr` can emit structured output directly (markdown / grounding
bboxes) is **untested**; if it can, part of T311 may collapse into T300. `marker` was
considered and set aside: its one advantage over the OCR tier is reading order, which
T301 already solves with a bounding box, and it would add a multi-model Python pipeline
inside a Rust daemon.

## Dev-kid LITE wiring prerequisites (do BEFORE `/devkid.orchestrate`)

1. `git switch -c 002-dayflow-mode` (off `main`).
2. `dev-kid.yml` → `branch: 002-dayflow-mode`.
3. Mirror THIS file to where lite reads it: `.dk/tasks.md` (lite's working copy).
   Run `dev-kid spec-resolve` and verify `.dk/tasks.md` matches this file with the
   `[S]` markers intact (the 2026-05-28 finding: a root-level `tasks.md` is NOT in
   the resolver chain — use the branch-resolved path / `.dk/tasks.md`).
4. `/devkid.preflight` (Mac ollama tier1 reachable, no localhost fallback) — only
   needed for the ma-loop fallback tier; lite dispatch itself just needs the tasks.md.
5. `/devkid.orchestrate` → `/devkid.execute` wave-by-wave; you implement + checkpoint each `[S]`.
