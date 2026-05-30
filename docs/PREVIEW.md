# gentle-eye — preview pane

Preview what gentle-eye captured (image or video) and, optionally, a **live**
view of what's being captured — OBS-style. Source PRD:
`gentle_eye_preview_pane_2026-05-30`.

Two parts:
- **Live preview (default OFF)** — a real-time view of the active target's
  source, *honoring the active crop* (preview == program).
- **Post-capture review** — preview what was just captured, with loop / autoclose.

## Design: supply-chain-minimal by default

In an agent-driven world, **dependency trust surface is a first-class
criterion** (malicious crates, hijacked maintainers, poisoned updates). So the
**default build adds ZERO new crates** — it reuses the already-installed,
OS-managed `ffmpeg`/`ffplay` plus a hand-rolled `std::net` server. Heavier
renderers are **opt-in, off by default, not built unless requested**.

| Backend | Gate | New deps | Role |
|---|---|---|---|
| **ffplay** (subprocess) | default | **0** (reuses ffmpeg) | file preview (loop/autoclose) + live (rawvideo pipe / stream URL) |
| **`std::net` HTTP gallery** | default | **0** (hand-rolled, not `tiny_http`) | remote/headless review: `<video>` with **Range/206**, 127.0.0.1-only, idle self-close, SSH-aware |
| **winit + softbuffer** | `--features richwindow` (OFF) | ~75 pure-Rust crates (not built by default) | pure-Rust window, agent-controlled multi-monitor placement |
| **opencv highgui** | `--features tracking` (OFF) | system libopencv-dev (already pulled if tracking on) | a *free reuse* if you already enabled opencv tracking — never added for preview alone |

There is **no countdown** (dropped as friction). A native window can't render
over SSH — so the HTTP gallery is the headless fallback (physics, not a choice).

## CLI

```bash
gentle-eye preview [FILE] [--loop once|forever] [--seconds N]
#   no FILE → the most-recent capture. ffplay, OS-open fallback.
#   --seconds N → show for N s then auto-close (image or video).

gentle-eye preview --gallery [--port N]
#   zero-dep std::net media gallery in the browser; <video> seeks via HTTP Range.
#   127.0.0.1 only; idle self-shutdown (~5 min). Over SSH it prints:
#     ssh -L <port>:127.0.0.1:<port> <this-host>   then open http://127.0.0.1:<port>/

gentle-eye preview --live
#   live preview of the ACTIVE target (default off). Display → cropped rawvideo
#   piped to ffplay; Stream → ffplay on the relay URL + crop filter.
```

## The pure-Rust window (opt-in)

`cargo build --features richwindow` adds a `winit`+`softbuffer` backend (≈75
pure-Rust crates) for a native window with real agent-controlled multi-monitor
placement + scale-factor. It is **not** built by default — confirm with
`cargo tree -e no-dev | grep winit` (empty on a default build).

## opencv reuse

If you've already enabled `--features tracking` (opencv, see issue #1), opencv's
`highgui` (`imshow`/`moveWindow`) can be reused as a preview window — a free
backend for that build. gentle-eye never pulls opencv just to preview.
