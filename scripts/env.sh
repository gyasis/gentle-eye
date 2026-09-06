#!/usr/bin/env bash
# gentle-eye build environment — ONE place, for ALL work in this repo.
#
#   source scripts/env.sh          # resolve + export
#   source scripts/env.sh --check  # resolve + report, exit non-zero if broken
#
# Everything is DETECTED, never hardcoded: paths differ per machine and a wrong
# pinned value is worse than an unset one. Nothing here needs sudo, and nothing
# touches the system OpenCV that other packages link against.
#
# Why this file exists: the two vars below are required to build
# `--features tracking`, and without them the failures point somewhere
# misleading (see docs/CV_BUILD.md). They were rediscovered by hand once; that
# is once too many.

_ge_repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
_ge_fail=0
_ge_say() { printf '  %-22s %s\n' "$1" "$2"; }
_ge_err() { printf '  %-22s %s\n' "$1" "MISSING — $2" >&2; _ge_fail=1; }

# ── 1. llvm-config ────────────────────────────────────────────────────────────
# Ubuntu ships llvm-config-NN, not a plain llvm-config; clang-sys looks for the
# unsuffixed name and reports "could not execute llvm-config", which reads as
# "LLVM is missing". Pick the highest available.
if [ -z "${LLVM_CONFIG_PATH:-}" ]; then
  _ge_llvm="$(command -v llvm-config 2>/dev/null)"
  [ -z "$_ge_llvm" ] && _ge_llvm="$(ls -1 /usr/lib/llvm-*/bin/llvm-config 2>/dev/null | sort -V | tail -1)"
  [ -z "$_ge_llvm" ] && _ge_llvm="$(ls -1 /usr/bin/llvm-config-* 2>/dev/null | sort -V | tail -1)"
  [ -n "$_ge_llvm" ] && export LLVM_CONFIG_PATH="$_ge_llvm"
fi
if [ -n "${LLVM_CONFIG_PATH:-}" ] && [ -x "${LLVM_CONFIG_PATH}" ]; then
  _ge_say "llvm-config" "$LLVM_CONFIG_PATH ($("$LLVM_CONFIG_PATH" --version 2>/dev/null))"
else
  _ge_err "llvm-config" "apt install llvm-dev  (or set LLVM_CONFIG_PATH)"
fi

# ── 2. C++ standard headers ───────────────────────────────────────────────────
# clang may resolve to a GCC toolchain whose headers are NOT installed (it picks
# 12 here while only 11 exists), so the binding generator dies on
# "'limits' file not found" inside cvdef.h — which reads as a broken OpenCV.
# Pick the highest /usr/include/c++/NN that actually CONTAINS <limits>.
if [ -z "${CPLUS_INCLUDE_PATH:-}" ]; then
  _ge_cxx=""
  for d in $(ls -1d /usr/include/c++/* 2>/dev/null | sort -V -r); do
    [ -f "$d/limits" ] && { _ge_cxx="$d"; break; }
  done
  if [ -n "$_ge_cxx" ]; then
    _ge_ver="$(basename "$_ge_cxx")"
    _ge_arch="/usr/include/$(uname -m)-linux-gnu/c++/$_ge_ver"
    _ge_paths="$_ge_cxx"
    [ -d "$_ge_arch" ] && _ge_paths="$_ge_paths:$_ge_arch"
    [ -d "$_ge_cxx/backward" ] && _ge_paths="$_ge_paths:$_ge_cxx/backward"
    export CPLUS_INCLUDE_PATH="$_ge_paths"
  fi
fi
if [ -n "${CPLUS_INCLUDE_PATH:-}" ]; then
  _ge_say "c++ headers" "${CPLUS_INCLUDE_PATH%%:*} (+$(( $(tr -cd ':' <<<"$CPLUS_INCLUDE_PATH" | wc -c) )) more)"
else
  _ge_err "c++ headers" "apt install libstdc++-dev  (no /usr/include/c++/*/limits)"
fi

# ── 3. OpenCV — prefer the user-prefix 5.x build, else the system ─────────────
# NEVER install over the system OpenCV: other Ubuntu packages link against it.
_ge_ocv5="$HOME/.local/opt/opencv5/lib/pkgconfig"
if [ -d "$_ge_ocv5" ] && ls "$_ge_ocv5"/opencv*.pc >/dev/null 2>&1; then
  export PKG_CONFIG_PATH="$_ge_ocv5${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
  export LD_LIBRARY_PATH="$HOME/.local/opt/opencv5/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
fi
_ge_ocv_mod=""
for pc in opencv5 opencv4; do
  _ge_ocv_mod="$(pkg-config --modversion "$pc" 2>/dev/null)" && { _ge_ocv_pc="$pc"; break; }
done
if [ -n "$_ge_ocv_mod" ]; then
  _ge_which="system"
  case "$(pkg-config --variable=prefix "$_ge_ocv_pc" 2>/dev/null)" in
    "$HOME"/*) _ge_which="user-prefix" ;;
  esac
  _ge_say "opencv" "$_ge_ocv_pc $_ge_ocv_mod ($_ge_which)"
else
  _ge_err "opencv" "apt install libopencv-dev libopencv-contrib-dev, or build 5.x (docs/CV_BUILD.md)"
fi

# ── 4. Repo-local Python venv for the imaging/CV scripts ──────────────────────
# Kept INSIDE the repo (.venv, gitignored) so this repo owns its environment
# rather than depending on something in ~/.local that another project may move.
if [ -x "$_ge_repo/.venv/bin/python" ]; then
  export GE_PY="$_ge_repo/.venv/bin/python"
  _ge_say "python venv" ".venv ($("$GE_PY" -c 'import cv2;print("cv2",cv2.__version__)' 2>/dev/null || echo 'no cv2'))"
else
  _ge_say "python venv" "absent — scripts/setup-venv.sh to create (optional)"
fi

export GE_ENV_READY=1
if [ "$_ge_fail" -ne 0 ]; then
  echo "  -> environment INCOMPLETE (see above); --features tracking will fail" >&2
else
  echo "  -> ready:  cargo check --features tracking"
fi
unset _ge_llvm _ge_cxx _ge_ver _ge_arch _ge_paths _ge_ocv5 _ge_ocv_mod _ge_ocv_pc _ge_which _ge_say _ge_err
[ "${1:-}" = "--check" ] && return "$_ge_fail" 2>/dev/null || true
