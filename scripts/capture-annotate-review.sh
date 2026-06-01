#!/usr/bin/env bash
# gentle-eye: capture → annotate → review, reliable on WSL2.
#
# DISPLAY ROLES (the hard-won lesson — do not swap these):
#   • CAPTURE display = Xvfb :99 (headless) OR Xephyr :100 (watch the app on Windows).
#       Both have a real, root-grabbable framebuffer. WSLg :0 CANNOT be root-grabbed
#       (XGetImage → BadMatch), so never capture :0.
#   • ANNOTATE GUI (redpen) = WSLg :0 ONLY. egui needs working GL; in WSL2 that exists
#       only on :0 (D3D12/WSLg). Xephyr does NOT pass a GPU device (DRI3) to nested
#       EGL clients, so redpen cannot render on :99/:100 — proven, do not retry.
#
# USAGE:
#   capture-annotate-review.sh [URL] [OUT.png]
#   CAPDISP=:100 capture-annotate-review.sh https://news.ycombinator.com   # watch on Windows
#   APP="DISPLAY=%D some-x11-app" ...                                       # any X11 app, not just chromium
#
# ENV:
#   CAPDISP   capture display (default :99 headless; use :100 to watch via Xephyr)
#   PROVIDER  vision provider for review (default gemini; or ollama)
set -uo pipefail

ROOT="$HOME/dev/gentle-eye"
GE="$ROOT/target/release/gentle-eye"
REDPEN="$ROOT/target/release/redpen"
WF="$ROOT/scripts/whitefrac.py"
CAPDISP="${CAPDISP:-:99}"
PROVIDER="${PROVIDER:-gemini}"
URL="${1:-https://www.cnn.com}"
OUT="${2:-/tmp/ge_capture.png}"
PROF="/tmp/ge_prof_${CAPDISP//:/}"

say(){ printf '\033[36m▸ %s\033[0m\n' "$*"; }

# 1. Ensure the capture display exists.
if ! xdpyinfo -display "$CAPDISP" >/dev/null 2>&1; then
  if [ "$CAPDISP" = ":100" ]; then
    say "starting Xephyr :100 (window on Windows desktop)"
    setsid env DISPLAY=:0 Xephyr :100 -screen 1600x900 -resizeable -ac >/tmp/ge_xephyr.log 2>&1 </dev/null &
  else
    say "starting Xvfb $CAPDISP (headless)"
    setsid Xvfb "$CAPDISP" -screen 0 1600x900x24 >/tmp/ge_xvfb.log 2>&1 </dev/null &
  fi
  timeout 12 bash -c "until xdpyinfo -display $CAPDISP >/dev/null 2>&1; do :; done" \
    || { echo "capture display $CAPDISP failed to start"; exit 1; }
fi

# 2. Launch the app (chromium by default).
say "launching app on $CAPDISP → $URL"
setsid env DISPLAY="$CAPDISP" /snap/bin/chromium \
  --no-sandbox --disable-gpu --disable-dev-shm-usage \
  --user-data-dir="$PROF" --no-first-run --disable-features=Translate \
  --window-size=1600,900 --start-fullscreen "$URL" >/tmp/ge_chrome_${CAPDISP//:/}.log 2>&1 </dev/null &

# 3. Wait for REAL content (white fraction < 70%) — NOT "max channel > 30",
#    which trips instantly on a blank white loading page (255).
say "waiting for real content to paint…"
timeout 60 bash -c "until [ \"\$(python3 $WF $CAPDISP 2>/dev/null || echo 100)\" -lt 70 ] 2>/dev/null; do :; done" \
  && say "content present (white $(python3 "$WF" "$CAPDISP")%)" \
  || echo "  (warn: still mostly white after 60s — capturing anyway)"

# 4. Capture with gentle-eye.
say "capturing → $OUT"
DISPLAY="$CAPDISP" GENTLE_EYE_DISPLAY=0 "$GE" screenshot --out "$OUT" --display 0 >/dev/null \
  || { echo "capture failed"; exit 1; }

# 5. Annotate: redpen GUI on WSLg :0 (the only display with working GL).
#    Draw with mouse (P pen / A arrow / B box, colors 1-4), press Enter to save.
say "opening redpen annotator on :0 — draw, then press ENTER (Esc cancels)"
DISPLAY=:0 "$REDPEN" --input "$OUT"

# 6. Review: agent ingests your marks and sends them to vision AI.
say "reviewing your annotation via $PROVIDER"
DISPLAY=:0 GENTLE_EYE_PROVIDER="$PROVIDER" "$GE" redpen-analyze --provider "$PROVIDER"
