# Setup: ATEM Mini → Source Streaming → gentle-eye (vision agent)

**What this enables:** stream a live video source (the ATEM Mini's program output —
whatever HDMI is on air) into the **gentle-eye** MCP server, which grabs a frame and
has an LLM (Gemini/Ollama) **describe what's on the screen**. End goal: "stream an
input to my agent so it can grab a screenshot and talk about the app."

**Status:** ✅ Working end-to-end (verified 2026-05-28 — captured a live 761 KB frame
over native RTMP and Gemini described it: a Netflix profile screen with *Devil May
Cry* artwork).

---

## 1. Architecture (two producer paths, one consumer)

```
                        ┌─ PRODUCER A (preferred): ATEM native RTMP ────────────┐
ATEM Mini ISO  ──HDMI in──▶ program out ──Ethernet, press STREAM──▶ rtmp://THIS_BOX:7001/live/atem
                        │                                                       │
                        └─ PRODUCER B (fallback): USB UVC ──ffmpeg (atem-serve)─┘
                                                                                │
relay (Docker nginx-rtmp `atem-relay`, :7001) ◀────────────────────────────────┘
   │  re-serves: RTMP :7001 · HLS :7080 · RTSP :8554
   ▼
gentle-eye  capture-stream  ──▶  PNG frame  ──▶  gentle-eye analyze (Gemini/Ollama)  ──▶  description
   (CLI: ~/.local/bin/gentle-eye   |   MCP: wired in ~/.claude.json, needs CC restart)
```

- **Consumer** = gentle-eye (Rust MCP server). It only *reads* a stream URL.
- **Producer** = whatever puts the ATEM picture onto a stream the consumer can read.
  Path A is the ATEM's own RTMP encoder (no USB needed, ATEM anywhere on LAN).
  Path B captures the ATEM's USB-UVC webcam output locally with ffmpeg.

---

## 2. Components (what's installed, where)

| Thing | Location | Role |
|---|---|---|
| `gentle-eye` (CLI + MCP) | `~/.local/bin/gentle-eye` | consumer: capture-stream / analyze / record / read-text |
| gentle-eye MCP registration | `~/.claude.json` | exposes `capture_stream_frame` etc. as in-agent tools (needs Claude Code restart) |
| Docker relay `atem-relay` | container, `tiangolo/nginx-rtmp`, `-p 7001:1935 -p 7080:8080`, `--restart unless-stopped` | **preferred relay** — bypasses ufw |
| `atem-relay` (script) | `~/.local/bin/atem-relay` | mediamtx-based relay wrapper (alternative to Docker) + prints settings |
| `mediamtx` | `~/.local/bin/mediamtx` + cfg `~/.config/atem-relay/mediamtx.yml` | single-binary relay (host process — subject to ufw) |
| `atem-serve` (script) | `~/.local/bin/atem-serve` | USB-UVC capture → HLS (fallback path) |

This box's IPs: wired `enp2s0 <THIS_BOX_IP>` · Wi-Fi `wlo1 <THIS_BOX_WIFI>`.
The ATEM on the LAN: `<ATEM_IP>` (Blackmagic MAC `<ATEM_MAC>`), USB device `/dev/video4` (MJPEG 1080p24).

---

## 3. The working setup — step by step (PREFERRED: native RTMP via Docker)

### 3.1 Relay (one-time; auto-restarts after)

**Option 1 — `docker run`:**
```bash
docker run -d --name atem-relay --restart unless-stopped \
  -p 7001:1935 -p 7080:8080 tiangolo/nginx-rtmp
# later: docker start atem-relay   (it auto-starts on boot via the restart policy)
```

**Option 2 — Docker Compose** (file shipped at `codebook/atem-relay/docker-compose.yml`):
```bash
cd ~/Documents/code/codebook/atem-relay
docker compose up -d
docker compose logs -f atem-relay   # watch for the ATEM publishing
# (if a docker-run container named atem-relay already exists: docker rm -f atem-relay first)
```
```yaml
# codebook/atem-relay/docker-compose.yml
services:
  atem-relay:
    image: tiangolo/nginx-rtmp
    container_name: atem-relay
    restart: unless-stopped
    ports:
      - "7001:1935"   # RTMP ingest — ATEM pushes here
      - "7080:8080"   # HLS over HTTP
```

Why Docker (either option) and not the mediamtx host process: **Docker-published
ports bypass ufw** (see §5). The relay is reachable from the LAN with *no firewall rule*.

### 3.2 ATEM Mini ISO (ATEM Software Control → Output → Streaming)
| Field | Value |
|---|---|
| Service | **Custom RTMP** (may require loading a streaming XML/profile in ATEM Software Control) |
| Server | `rtmp://<THIS_BOX_IP>:7001/live`  (the **wired** IP, port **7001**) |
| Stream Key | `atem` |
| then | press the **STREAM** button — confirm it shows a live **data rate (kbps)** |

⚠️ Program "on air" (the picture) is **NOT** the same as RTMP streaming — you must
press the dedicated STREAM/broadcast button.

### 3.3 Capture + describe (works now via CLI, no restart)
```bash
gentle-eye capture-stream --url rtmp://localhost:7001/live/atem --out /tmp/frames
#   HLS alt:  http://localhost:7080/live/atem/index.m3u8
#   RTSP alt: rtsp://localhost:8554/live/atem
gentle-eye analyze --image /tmp/frames/<frame>.png \
  --prompt "Describe the app on screen and what the user is doing" --provider gemini
```
In-agent (after a Claude Code restart): the MCP tool `capture_stream_frame { "stream_url": "rtmp://localhost:7001/live/atem" }` → then `analyze_video` on the PNG.

---

## 4. Fallback path — USB direct (no network)

```bash
atem-serve                       # auto-detects the Blackmagic device (/dev/video4) → HLS
atem-serve --device /dev/video2  # or ANY v4l2 input (Insta360, USB cam…)
atem-serve --rtmp rtmp://localhost:7001/live/atem   # or push USB into the relay
```
HLS output: `~/.local/share/gentle-eye/atem-hls/` (NOT `/tmp/atem-hls` — root-owned here).
Capture: `gentle-eye capture-stream --url ~/.local/share/gentle-eye/atem-hls/index.m3u8 --out /tmp/frames`.

---

## 5. Lessons learned (the debugging journey — READ THIS)

These are the traps we actually hit. They're the real value of this doc.

1. **Port 7001, NOT 1935.** The original relay was nginx-rtmp Docker mapped `-p 7001:1935`,
   so the ATEM is configured for `:7001`. A relay on `:1935` silently receives nothing.
   *(Recovered from the disaster-recovery archive — the user remembered the addressing was different.)*
2. **Docker-published ports BYPASS ufw.** This is the big one. The original "just worked"
   because the relay was a Docker container — Docker inserts iptables rules (DOCKER chain)
   that skip ufw's INPUT filtering. When the relay was reimplemented as a **host process**
   (mediamtx), it became subject to ufw's default-deny and the ATEM's packets were dropped.
   Fix: either `sudo ufw allow from <lan>/24 to any port 7001 proto tcp`, **or** just run
   the relay in Docker (no sudo needed).
3. **No-signal vs real content by frame size.** A no-signal/blank ATEM frame is ~**6 KB**;
   real content is **500 KB–1 MB+**. Use byte size to tell instantly whether a real source
   is on air (don't trust "1920×1080" alone — a blank frame is still 1080p).
4. **mediamtx empty config rejects ALL paths** (`path 'live/atem' is not configured`).
   It needs `paths: { all_others: }` to accept arbitrary publishers.
5. **`/tmp/atem-hls` is root-owned here** → the USB HLS path must write to a user-owned dir
   (`~/.local/share/gentle-eye/atem-hls`).
6. **The `:80` web page is a red herring** — an unrelated nginx/apache on this box. The
   stream lives on 7001/7080/8554, never :80.
7. **Verify the publisher is real, not your own test.** nginx-rtmp logs the publisher
   user-agent: `FMLE/3.0 (compatible; Lavf…)` = an **ffmpeg** test push (yours), not the ATEM.
   To confirm the ATEM: kill all ffmpeg (`pkill -9 -x ffmpeg` — use `-x`, NOT `-f ffmpeg`
   which also kills your shell), then capture again; if a frame still comes, it's the ATEM.
8. **CLI works with no restart; the MCP in-agent tools need a Claude Code restart.**

---

## 6. Verify / self-test (proves the relay independent of the ATEM)

```bash
# push the USB feed INTO the relay (stands in for the ATEM's STREAM), then capture
ffmpeg -f v4l2 -input_format mjpeg -video_size 1920x1080 -i /dev/video4 \
  -c:v libx264 -preset ultrafast -tune zerolatency -pix_fmt yuv420p -f flv \
  rtmp://localhost:7001/live/atem &
gentle-eye capture-stream --url rtmp://localhost:7001/live/atem --out /tmp/frames
pkill -9 -x ffmpeg          # stop the test push (exact-name match!)
```
A >500 KB frame = relay good. If the real ATEM still won't publish after this, the problem
is ATEM-side (STREAM not started, wrong server/key, or custom-RTMP XML not loaded).

---

## 7. Quick reference

| Item | Value |
|---|---|
| ATEM stream target | `rtmp://<THIS_BOX_IP>:7001/live`  key `atem` |
| gentle-eye capture URL | `rtmp://localhost:7001/live/atem` |
| Relay ports | RTMP 7001 · HLS 7080 · RTSP 8554 |
| Relay container | `docker start atem-relay` (auto-restarts) |
| USB device | `/dev/video4` (Blackmagic, MJPEG 1080p24) |
| Health check | `docker ps | grep atem-relay` ; `gentle-eye capture-stream --url rtmp://localhost:7001/live/atem --out /tmp/t` |

## 8. Troubleshooting

| Symptom | Likely cause | Fix |
|---|---|---|
| capture → `Input/output error` / no frame | nothing publishing to `live/atem` | start ATEM STREAM (or `atem-serve`); confirm port 7001 |
| frame is ~6 KB | ATEM on-air source is no-signal | put a real HDMI source on air on the ATEM |
| ATEM STREAM "connecting"/fails | can't reach relay | confirm `rtmp://<THIS_BOX_IP>:7001/live` + relay up; if host-process relay, `ufw allow 7001` |
| `path '…' is not configured` (mediamtx) | empty config | `paths: { all_others: }` in `~/.config/atem-relay/mediamtx.yml` |
| publisher shows `Lavf` user-agent | it's your own ffmpeg test, not the ATEM | `pkill -9 -x ffmpeg`, re-capture |

---
*Captured from the 2026-05-28 build session. Source project: `~/Documents/code/gentle-eye`.
Recovery origin: the original setup was reconstructed from the disaster-recovery archive
(`recovered-files-v2`), incl. the 2025-12-21 "llm-agent-screen-and-video-understanding" session.*
