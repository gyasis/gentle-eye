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

## OpenCV 5

`opencv-rust` 0.100 supports **4.x or 5.x**. Ubuntu 22.04 apt tops out at **4.5.4**, so
5.x requires a source build of `opencv` + `opencv_contrib`. Build it into a **user
prefix** and point `PKG_CONFIG_PATH` at it — never over the system 4.5.4, which other
packages link against:

```bash
cmake -B build -G Ninja -DCMAKE_INSTALL_PREFIX=$HOME/.local/opt/opencv5 \
      -DOPENCV_EXTRA_MODULES_PATH=../opencv_contrib/modules -DBUILD_LIST=...
export PKG_CONFIG_PATH=$HOME/.local/opt/opencv5/lib/pkgconfig:$PKG_CONFIG_PATH
```

The Python wheel (`opencv-contrib-python`) is **not** a substitute — it ships bundled
`.so` files with no C++ headers or `.pc` file, so nothing can link against it.

## The dnn_superres caveat

`dnn_superres` is bound and usable, but it is deliberately **not** a default anywhere in
this repo. Learned upscaling invents detail; on text it produces confident wrong glyphs.
Multi-frame stacking recovers *real* detail from sub-pixel jitter and is the honest
path. If `dnn_superres` output is ever surfaced, label it as model-generated and never
treat it as evidence.
