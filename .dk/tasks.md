# gentle-eye — preview pane (live + post-capture) tasks (dev-kid dogfood plan)

Generated **2026-05-30** from PRD `gentle_eye_preview_pane_2026-05-30` (§2 locked
design). OBS-style preview in two parts. **Supply-chain-minimal: the DEFAULT build
adds ZERO new crates** (reuses the already-installed ffmpeg + a hand-rolled
`std::net` server). The pure-Rust window is **opt-in, off by default**. **No
countdown** (dropped as friction).

Branch: `006-preview-pane` (off `main`). Sentinel test: `./.tooling/bin/cargo check
--message-format=short` (see `dev-kid.yml`). ma-loop fallback floor = `mixed-budget`
(OpenAI+Gemini; Mac-LAN ollama skipped per user).

## Locked decisions (paired-debate, 2026-05-30)

| # | Decision | Choice |
|---|---|---|
| P1 | Default renderer | **ffplay subprocess** (reuses ffmpeg — 0 new crates). OS-open (`xdg-open`/`open`) fallback. |
| P2 | Headless/remote | hand-rolled **`std::net` HTTP gallery** (0 crates, NOT tiny_http), `<video>` + **Range/206**, 127.0.0.1, idle self-close. Native window can't render over SSH → this is the fallback. |
| P3 | Rich window | **opt-in `richwindow` feature** (winit+softbuffer, 75 crates) — **NOT built by default**. Agent-controlled multi-monitor placement. |
| P4 | Countdown | **none** (removed). |
| P5 | opencv-highgui | documented **reuse** backend under the existing `tracking` feature — NOT built for preview. |

## Conventions

- `[P]` = parallelizable within its wave.
- `[S]` = **sentinel checkpoint**: crate compilable + this task is a complete runnable
  file. Run `./.tooling/bin/cargo check` (+ tests at gate waves) here. Skeleton tasks get no `[S]`.
- Every task carries a **`> DONE:`** completion criterion.

Reuse — do NOT rebuild (existing, green): `capture/stream.rs` (`build_ffmpeg_args`,
`probe_dimensions`, `write_bgra_png`, ffmpeg subprocess pattern), `capture/screen.rs`
(`ScreenCapturer` → BGRA frames, for live), `capture/display.rs` (display catalogue
→ monitor geometry for placement), `target/{store,geometry,crop}` (active target →
live preview honors the crop), `storage/manager.rs` (recordings dir), `bin/gentle-eye.rs`
(CLI dispatch + `flag`/`parse_region` helpers), `contracts/errors.rs` (`GentleEyeError`).

---

## Wave 0 — module skeleton + capture discovery (skeleton, NO sentinel)

- [x] T400 [P] `src/preview/mod.rs` — `pub mod {discover, player, gallery, live, renderer}` + re-exports; wire `pub mod preview;` into `src/lib.rs`. Stub files for not-yet-filled submodules.
      `> DONE:` lib.rs declares `preview`; `cargo check` resolves the module tree (stubs allowed).
- [x] T401 [P] `src/preview/errors.rs` — `PreviewError` enum (`Io`, `NotFound`, `NoCaptures`, `Spawn`, `Http`) + `From<PreviewError> for GentleEyeError` + `mcp_error_code()` (mirror `TargetError`).
      `> DONE:` maps into `GentleEyeError`; compiles.
- [x] T402 [P] `src/preview/discover.rs` — `Capture { path, kind: Image|Video, modified }` + `recent_captures(root, limit) -> Vec<Capture>` (walk the recordings dir, classify by extension png/jpg vs mp4/mkv, sort by mtime desc). `latest_capture(root)`.
      `> DONE:` against a temp dir seeded with mixed files, returns them newest-first with correct kinds; unit test.

## Wave 1 — PV1: `preview [FILE]` (default, zero-dep) `[S]`

- [x] T410 `src/preview/player.rs` — `PlaybackOpts { loop_mode: Once|Forever|None, autoclose_secs: Option<u64> }`; `ffplay_args(path, kind, &PlaybackOpts) -> Vec<String>` (`-autoexit`, `-loop`, image `-t`/`-loop 1`) + `open_with_player(path, opts)` that spawns ffplay, falling back to OS-open (`xdg-open`/`open`) when ffplay is absent.
      `> DONE:` arg-builder unit tests cover image vs video + each loop/autoclose combo; ffplay-absent path selects OS-open; `cargo check` + tests green.
- [x] T411 [P] CLI `preview [FILE] [--loop once|forever] [--seconds N]` in `bin/gentle-eye.rs` — no FILE → `latest_capture` (T402); calls T410. JSON status out.
      `> DONE:` `preview` with no file resolves the most-recent capture; flags map to `PlaybackOpts`; prints valid JSON.
- [x] T412 [S] PV1 integration — discovery + arg-building exercised end-to-end (no live spawn needed).
      `> DONE:` integration test (seed temp recordings, assert chosen file + ffplay args); `cargo check` + tests green.

## Wave 2 — PV2: `preview --gallery` (default, zero-dep `std::net` + Range) `[S]`

- [x] T420 `src/preview/gallery.rs` — hand-rolled `std::net::TcpListener` GET server: parse the request line, route `/` (gallery HTML) and `/media/<name>` (serve a capture). **Path-traversal-safe**: resolve under the recordings root, reject `..`/absolute escapes. Bind **127.0.0.1** only.
      `> DONE:` `/media/../../etc/passwd` (and encoded variants) are rejected with 403/404; only files under root serve; unit test on the path-resolver.
- [x] T421 [P] Range support — parse `Range: bytes=start-end`; reply **206** + `Content-Range`/`Accept-Ranges` + sliced body; full **200** when absent. Pure range-math fn.
      `> DONE:` unit tests: a range returns the exact byte slice + correct headers; no-range returns full 200; open-ended `bytes=N-` handled.
- [x] T422 [P] `src/preview/gallery_html` — embedded single-page HTML (no external files): lists `recent_captures` (T402), `<img>` inline, `<video controls>` for video; optional ffprobe metadata (dims/duration).
      `> DONE:` the rendered HTML references each capture via `/media/<name>` and uses `<video>` for video / `<img>` for images; unit test on the HTML builder.
- [x] T423 idle self-shutdown (~5 min no requests → exit) + SSH detect (`SSH_TTY`/`SSH_CLIENT`): local → auto-open the URL (`xdg-open`/`open`); remote → print the `ssh -L` tunnel hint.
      `> DONE:` SSH-detection branch selects the right action (unit-testable via env); idle-timeout logic unit-tested with a fast clock.
- [x] T424 [S] PV2 integration — bind an ephemeral port, real HTTP round-trips.
      `> DONE:` `GET /` → 200 listing a seeded capture; `GET /media/<f>` with a Range → 206 + correct bytes; traversal rejected; `cargo check` + tests green.

## Wave 3 — PV3: live preview (default OFF, ffplay) `[S]`

- [x] T430 `src/preview/live.rs` — `live_ffplay_cmd(source, active_target) -> Command spec`: for a **Display** source, pipe `ScreenCapturer` BGRA frames as rawvideo to ffplay stdin, **honoring the active `target` crop** (reuse `target::geometry` + `target::crop`); for a **Stream**, point ffplay at the relay URL (+ ffmpeg `crop=` when a target is active). Arg/spec is unit-tested; the pipe loop is integration-only.
      `> DONE:` the command spec for display vs stream is correct (rawvideo `-f`/`-pixel_format bgra`/`-video_size`, or stream URL + crop); honors-target asserted in a unit test; `cargo check`.
- [x] T431 [P] CLI `preview --live` (default OFF — only when invoked). Uses the active target/source.
      `> DONE:` `preview --live` builds the live command from the active target; no-op/clear message when no source; prints status.
- [x] T432 [S] PV3 — live command-building green.
      `> DONE:` unit tests for display + stream live specs; `cargo check` + tests green.

## Wave 4 — PV4: renderer trait + opt-in `richwindow` (winit+softbuffer) `[S]`

- [x] T440 `src/preview/renderer.rs` — `PreviewRenderer` trait (`show_image(path)`, `show_live(frame, dims)`, `place(monitor)`); default impl = the ffplay-backed renderer (reuse player/live). The CLI uses the trait.
      `> DONE:` trait + default ffplay impl compile; the default path goes through the trait; `cargo check`.
- [x] T441 `Cargo.toml` `[features] richwindow = ["dep:winit","dep:softbuffer"]` with **optional** `winit`/`softbuffer` deps; `#[cfg(feature="richwindow")]` window backend: open a (small, ~10%) window, blit BGRA frames, **agent-controlled placement** from the display catalogue + scale-factor.
      `> DONE:` `--features richwindow` compiles the winit backend; placement uses monitor geometry; (default build untouched).
- [x] T442 [S] Gate — `richwindow` is opt-in only.
      `> DONE:` default `./.tooling/bin/cargo check` compiles WITHOUT winit/softbuffer (absent from the default build graph — `cargo tree -e no-dev`); the opencv-highgui reuse backend is documented in `docs/PREVIEW.md`.

## Wave 5 — Gates + docs `[S]`

- [x] T450 [S] `cargo test` — all unit + integration tests green.
      `> DONE:` `./.tooling/bin/cargo test` exit 0.
- [x] T451 [S] `cargo clippy --all-targets -- -D warnings` (default features) — zero warnings.
      `> DONE:` clippy exit 0; default build confirmed 0 new crates.
- [x] T452 `docs/PREVIEW.md` (+ QUICKSTART/TOOLS updates) — the 2-part design, the feature-gated renderer table, supply-chain rationale, the `richwindow` opt-in + opencv-highgui reuse note, CLI usage.
      `> DONE:` `docs/PREVIEW.md` exists and documents the preview pane + the dep posture.

---

## Run mode: dev-kid LITE + in-session checkpoints

dev-kid lite reads this `.dk/tasks.md`. It dispatches Wave N to the in-session
Developer agent (Claude) to implement; the **`[S]` checkpoints are run by the
in-session agent** (`./.tooling/bin/cargo check`, + `cargo test`/`clippy -D warnings`
at gate waves). ma-loop / tier escalation is the fallback only on a stuck file
(floor `mixed-budget`).

- **Constitution:** Edition 2021, deps pinned. The **default build MUST add zero new
  crates** (ffplay subprocess + hand-rolled `std::net`). `winit`/`softbuffer` MUST be
  optional + feature-gated (`richwindow`, off by default). No countdown.
- **Halt-and-fix:** any dev-kid / sentinel bug = the valued finding. Stop, capture,
  fix the tool, resume.

- [ ] SENTINEL-T006: Sentinel validation for T004, T005, T006: verify implementations pass tests
- [ ] SENTINEL-T011: Sentinel validation for T007, T008, T009, T010, T011: verify implementations pass tests
- [ ] SENTINEL-T014: Sentinel validation for T012, T013, T014: verify implementations pass tests
- [ ] SENTINEL-T017: Sentinel validation for T015, T016, T017: verify implementations pass tests
- [ ] SENTINEL-T018: Sentinel validation for T018: verify implementations pass tests
- [ ] SENTINEL-T019: Sentinel validation for T019: verify implementations pass tests
