# Building the `tracking` (OpenCV) feature

The default build needs **no system libraries** — that rule is in `Cargo.toml` and
`AGENTS.md` and it still holds. This file is only about `--features tracking`.

## It builds today, and needs NO installs

Verified 2026-09-06 on Ubuntu 22.04: `cargo check --features tracking` finishes in
~1m15s against the **system OpenCV 4.5.4**, with `libopencv-dev` and
`libopencv-contrib-dev` already present. Two environment variables are required, and
without them the failure messages point somewhere misleading:

```bash
export LLVM_CONFIG_PATH=/usr/bin/llvm-config-14
export CPLUS_INCLUDE_PATH="/usr/include/c++/11:/usr/include/x86_64-linux-gnu/c++/11:/usr/include/c++/11/backward"
cargo check --features tracking
```

### Why each is needed (both cost real time to diagnose)

**`LLVM_CONFIG_PATH`** — Ubuntu ships `llvm-config-14`, not a plain `llvm-config`.
`clang-sys` looks for the unsuffixed name, fails, and reports
*"could not execute llvm-config"*. That reads like LLVM is missing. It is not.

**`CPLUS_INCLUDE_PATH`** — clang-14 resolves its toolchain to **GCC 12**
(`/usr/bin/../lib/gcc/x86_64-linux-gnu/12/../../../../include/c++`) but only **GCC 11**
headers are installed (`/usr/include/c++/11`). The binding generator then dies with
`fatal error: 'limits' file not found` while parsing `opencv2/core/cvdef.h`. That reads
like a broken OpenCV install. It is not — it is a clang/gcc version mismatch.

## Modules available at 4.5.4

All of the ones the screen-recovery work needs are bound:

`ximgproc` `xphoto` `quality` `dnn_superres` `intensity_transform`
`imgproc` `calib3d` `video` `photo` `core`

## The one command

```bash
source scripts/env.sh          # DETECTS everything; prints what it resolved
cargo check --features tracking
```

`scripts/env.sh` detects rather than hardcodes — a pinned path is wrong on every
other machine, which is worse than an unset one. It resolves the highest
`llvm-config`, the highest `/usr/include/c++/NN` that actually contains `<limits>`,
and prefers a user-prefix OpenCV 5 over the system install. It reports each choice and
fails loud with the apt line that would fix it. `--check` returns non-zero if
incomplete. Anything already exported in your shell wins.

## OpenCV 5 — BUILT AND VERIFIED (2026-09-06)

OpenCV **5.0.0** + contrib is built into `~/.local/opt/opencv5` (81 MB, user prefix, no
sudo, system 4.5.4 untouched). `scripts/env.sh` picks it up automatically via
`PKG_CONFIG_PATH`.

Verified: `cargo check --features tracking` finishes in **39s** against
`opencv5 5.0.0`, with opencv-rust 0.100.1. cmake 3.22.1 configured it without complaint.

Rebuild it with `scripts/build-opencv5.sh` (shallow clone of `opencv` +
`opencv_contrib` at tag 5.0.0, trimmed `BUILD_LIST`, Ninja).

### Modules bound against 5.0.0

`ximgproc` `xphoto` `quality` `dnn_superres` `intensity_transform`
`imgproc` `video` `photo` `core` `dnn` `flann` `imgcodecs`

### API change: `calib3d` is GONE in OpenCV 5

It was renamed to **`geometry`** (`opencv2/geometry.hpp`). Asking for `calib3d` in
`BUILD_LIST` silently yields nothing — it is not a build failure, so it looks like an
omission. Nothing here needs it: the stacker's primitives are
`getPerspectiveTransform` / `warpPerspective` (**imgproc**), `findTransformECC`
(**video**), `phaseCorrelate` (**imgproc**).

### Falling back to 4.5.4

Still fully supported and needs no build at all — the system `libopencv-dev` +
`libopencv-contrib-dev` are already installed and bind every module listed above.
`scripts/env.sh` uses them automatically when the 5.x prefix is absent. Measured there:
`cargo check --features tracking` in ~1m15s.

The Python wheel (`opencv-contrib-python`) is **not** a substitute for either — it ships
bundled `.so` files with no C++ headers or `.pc` file, so nothing can link against it.
It is only for the repo-local `.venv` (`scripts/setup-venv.sh`), which pins the same
5.0.0 line so Rust and Python agree.

## The dnn_superres caveat

`dnn_superres` is bound and usable, but it is deliberately **not** a default anywhere in
this repo. Learned upscaling invents detail; on text it produces confident wrong glyphs.
Multi-frame stacking recovers *real* detail from sub-pixel jitter and is the honest
path. If `dnn_superres` output is ever surfaced, label it as model-generated and never
treat it as evidence.
