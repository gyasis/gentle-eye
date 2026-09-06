#!/usr/bin/env bash
# Create the REPO-LOCAL python venv (.venv) for the imaging/CV scripts.
# Repo-local on purpose: this repo owns its environment, rather than depending
# on something under ~/.local that another project may move or upgrade.
set -euo pipefail
cd "$(dirname "$0")/.."
if [ -x .venv/bin/python ]; then echo ".venv exists"; else
  if command -v uv >/dev/null 2>&1; then uv venv --python 3.12 .venv
  else python3 -m venv .venv; fi
fi
PY=.venv/bin/python
if command -v uv >/dev/null 2>&1; then
  uv pip install --python "$PY" "opencv-contrib-python==5.0.0.93" numpy
else
  "$PY" -m pip install -q --upgrade pip && "$PY" -m pip install "opencv-contrib-python==5.0.0.93" numpy
fi
"$PY" - <<'PY'
import cv2
print("cv2", cv2.__version__)
missing = [m for m in ("ximgproc","xphoto","quality","dnn_superres","intensity_transform")
           if not hasattr(cv2, m)]
print("contrib OK" if not missing else f"MISSING contrib modules: {missing}")
PY
echo "-> source scripts/env.sh to pick it up"
