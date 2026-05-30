# gentle-eye — Quickstart & Install (per machine)

gentle-eye is a Rust MCP server + CLI for **screen / stream capture + AI vision
analysis**. Two front-ends share one library:

- **MCP server** (`gentle-eye serve`) — exposes tools to an agent (Claude Code).
- **CLI** (`gentle-eye <subcommand>`) — prints JSON to stdout for shelling out.

> **Platform status (2026-05-30):** developed and validated on **Linux (X11)**.
> The code is cross-platform (capture via `scrap`; memory monitor via `sysinfo`;
> vision over HTTP), so it **should** run on **macOS**, but the macOS path is
> **not yet verified** — see the macOS notes + caveats below.

---

## 0. Prerequisites (all machines)

| Need | Why | Install |
|---|---|---|
| **Rust** (stable, edition 2021) | build the binary | https://rustup.rs |
| **ffmpeg** + **ffprobe** | stream capture, video encode, PNG confirmation images | see per-OS below |
| A **vision provider** | analysis: **Gemini** (default) or **Ollama** | `GEMINI_API_KEY`/`GOOGLE_API_KEY`, or a reachable Ollama host |

Optional: `opencv` is **only** needed for the deferred `tracking` feature
(`--features tracking`) — not for normal use.

---

## 1. Linux (X11) — the validated path

```bash
# deps
sudo apt-get install -y ffmpeg            # ffmpeg + ffprobe
# (X11 session required; scrap does not support Wayland — use XWayland if needed)

# build + install onto PATH (~/.local/bin is on PATH per XDG)
git clone https://github.com/gyasis/gentle-eye.git && cd gentle-eye
cargo build --release --bin gentle-eye
cp target/release/gentle-eye ~/.local/bin/gentle-eye
which gentle-eye && gentle-eye help
```

Smoke test:
```bash
export GEMINI_API_KEY=...                 # or GOOGLE_API_KEY
gentle-eye displays                       # list displays
gentle-eye read-text --image /path/to.png # OCR a PNG → JSON
```

## 2. macOS (Apple Silicon / Intel) — should work, **unverified**

```bash
# deps
brew install ffmpeg rustup-init && rustup-init -y

# build + install
git clone https://github.com/gyasis/gentle-eye.git && cd gentle-eye
cargo build --release --bin gentle-eye
cp target/release/gentle-eye ~/.local/bin/gentle-eye   # ensure ~/.local/bin is on PATH
```

**macOS-specific:**
- **Screen Recording permission (TCC):** grant the terminal/app that launches
  gentle-eye access under *System Settings → Privacy & Security → Screen
  Recording*, then restart it. ⚠️ The startup permission check is currently a
  stub that always "passes", so it won't pre-warn you — without the grant,
  screen captures come back **black**.
- **Stream capture** (ATEM/RTMP → crop → vision) is the most portable path and
  should work as soon as `ffmpeg` is present.
- **Memory-pressure eviction** works (cross-platform via `sysinfo`).
- Not yet validated end to end — please report issues.

## 3. Windows

Untested. The code has no Linux-only hard dependency (memory via `sysinfo`,
capture via `scrap` which supports Windows), but treat it as experimental.
Install `ffmpeg` and ensure it's on `PATH`.

---

## 4. Register as a Claude Code MCP server

Add to `~/.claude.json` under `mcpServers` (point `command` at the installed
binary), then **restart Claude Code** so the tools load:

```jsonc
"gentle-eye": {
  "command": "/home/<you>/.local/bin/gentle-eye",   // macOS: /Users/<you>/.local/bin/gentle-eye
  "args": ["serve"],
  "env": {
    "GENTLE_EYE_PROVIDER": "gemini",                 // or "ollama"
    "GEMINI_API_KEY": "..."                           // or OLLAMA_HOST / OLLAMA_PORT for ollama
  }
}
```

After upgrading the binary, **restart Claude Code** (the MCP server runs the
old process until it re-spawns). The CLI picks up changes immediately.

---

## 5. Configuration (env)

| Var | Purpose |
|---|---|
| `GENTLE_EYE_PROVIDER` | `gemini` (default) or `ollama` |
| `GEMINI_API_KEY` / `GOOGLE_API_KEY` | Gemini auth |
| `OLLAMA_HOST` / `OLLAMA_PORT` | Ollama endpoint (default `localhost:11434`) |
| `GENTLE_EYE_DISPLAY` | display index to capture (default 0) |
| `GENTLE_EYE_DATA` | storage base dir for recordings |

---

## 6. Next steps

- **What can it do?** → [`docs/TOOLS.md`](TOOLS.md) — the full MCP + CLI reference.
- **Region-of-interest cropping** → [`docs/TARGET.md`](TARGET.md).
- **Stream a live source (ATEM/OBS)** → [`docs/ATEM_STREAMING.md`](ATEM_STREAMING.md).
- **fps & dayflow** → [`docs/FPS_AND_DAYFLOW.md`](FPS_AND_DAYFLOW.md).
