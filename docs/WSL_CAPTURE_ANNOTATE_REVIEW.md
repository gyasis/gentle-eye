# Recipe: capture → annotate → review on WSL2 (Windows-visible)

The reliable, reproducible workflow for driving an X11 app, capturing it, having a
human mark it up, and getting an AI read of the marks — all on WSL2 with the GUI
visible on the Windows desktop.

## TL;DR

```bash
# headless capture (agent-driven), annotate window pops on Windows:
scripts/capture-annotate-review.sh https://www.cnn.com /tmp/shot.png

# OR watch the browser live on Windows while capturing:
CAPDISP=:100 scripts/capture-annotate-review.sh https://www.cnn.com
```

## The two-display rule (do not swap — this is the whole lesson)

| Role | Display | Why |
|---|---|---|
| **Capture** | **Xvfb `:99`** (headless) or **Xephyr `:100`** (watchable on Windows) | Both have a real, **root-grabbable** framebuffer. |
| **Annotate GUI** (`redpen`) | **WSLg `:0`** only | egui needs working GL; in WSL2 GL works only on `:0` (D3D12/WSLg). |

### Why not capture WSLg `:0` directly?
`:0` is **rootless XWayland** — `XGetImage` on the root returns **`BadMatch`**. You can
*see* apps on `:0` but cannot root-grab it. Capture must use Xvfb/Xephyr.

### Why can't `redpen` (egui) run on Xephyr `:100`?
Xephyr does **not** pass a GPU device (DRI3) to nested EGL clients. Symptom:
`libEGL: failed to get driver name for fd -1` → `NoGlutinConfigs` → `WinitEventLoop
ExitFailure`. Verified across plain Xephyr, `-glamor`, and `GALLIUM_DRIVER=d3d12`
with the `render`+`video` groups granted and `d3d12_dri.so` present. **Do not retry —
run the redpen GUI on `:0`.** (`redpen --input <png>` loads a capture, so it never
needs to capture `:0`.)

## Prerequisites (one-time)

```bash
# build (rustls TLS, no openssl; annotate verb included)
cargo build --release --bin gentle-eye
cargo build --release --features ui --bin redpen     # egui GUI annotator

sudo apt-get install -y xvfb xserver-xephyr x11-utils \
  libxcb1-dev libxcb-shm0-dev libxcb-randr0-dev libx11-dev
sudo usermod -aG render,video "$USER"   # WSLg GL on :0; re-login (or `sg`) to apply
export GEMINI_API_KEY=...                # for the review step (gemini-3.5-flash default)
```

## Gotchas baked into the script

- **Paint readiness = white-fraction < 70%**, NOT "max channel > 30". A blank white
  loading page is `255`, so the naive max check trips instantly and you capture nothing.
  See `scripts/whitefrac.py`.
- **snap chromium** processes are named `chrome` (not `chromium`) — `pkill -x chrome`.
  Never `pkill -f` a pattern that appears in your own command line (it kills your shell).
- **Headless agent-only path:** use `:99`. Only use `:100` (Xephyr) when a human wants
  to watch the app render on the Windows desktop.

## Loop stages (what the script runs)

1. ensure capture display (`:99` Xvfb or `:100` Xephyr)
2. launch the X11 app (chromium by default) on the capture display
3. wait for real content (white-fraction gate)
4. `gentle-eye screenshot --display 0` → PNG
5. `redpen --input PNG` on `:0` — human draws (P/A/B, colors 1-4), **Enter** saves to
   `~/.gentle-eye/redpen/`
6. `gentle-eye redpen-analyze` → vision AI reads the marks (where you pointed / circled)

For a fully agent-driven (no-human) markup instead of redpen, use the headless
`gentle-eye annotate --image IN --out OUT --box x,y,w,h --label TEXT` verb.
