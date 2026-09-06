#!/usr/bin/env bash
# Build OpenCV 5.0.0 + contrib into a USER prefix. Never touches system 4.5.4,
# which other Ubuntu packages link against. No sudo anywhere.
set -uo pipefail
SRC="$HOME/.local/src"; PREFIX="$HOME/.local/opt/opencv5"
mkdir -p "$SRC" && cd "$SRC" || exit 1

for r in opencv opencv_contrib; do
  if [ ! -d "$SRC/$r" ]; then
    echo "== cloning $r 5.0.0 (shallow)"
    git clone --depth 1 --branch 5.0.0 "https://github.com/opencv/$r.git" "$SRC/$r" \
      >/dev/null 2>&1 || { echo "CLONE FAILED: $r"; exit 1; }
  else
    echo "== $r already cloned"
  fi
done

cd "$SRC/opencv" || exit 1
# Only the modules the screen-recovery work actually needs -- a full build is
# enormous and most of it is dead weight here.
MODS="core,imgproc,imgcodecs,calib3d,video,photo,features2d,flann,dnn,ximgproc,xphoto,quality,dnn_superres,intensity_transform"
echo "== configure -> $PREFIX"
cmake -B build -G Ninja \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_INSTALL_PREFIX="$PREFIX" \
  -DOPENCV_EXTRA_MODULES_PATH="$SRC/opencv_contrib/modules" \
  -DBUILD_LIST="$MODS" \
  -DBUILD_opencv_python3=OFF -DBUILD_opencv_python_bindings_generator=OFF \
  -DBUILD_opencv_java=OFF -DBUILD_TESTS=OFF -DBUILD_PERF_TESTS=OFF \
  -DBUILD_EXAMPLES=OFF -DBUILD_DOCS=OFF -DWITH_GTK=OFF -DWITH_QT=OFF \
  -DOPENCV_GENERATE_PKGCONFIG=ON \
  > /tmp/ocv5_configure.log 2>&1
if [ $? -ne 0 ]; then
  echo "CONFIGURE FAILED -- tail:"; tail -25 /tmp/ocv5_configure.log; exit 2
fi
echo "== configured OK"
echo "== building (ninja, $(nproc) cores)"
ninja -C build > /tmp/ocv5_build.log 2>&1
if [ $? -ne 0 ]; then
  echo "BUILD FAILED -- tail:"; tail -25 /tmp/ocv5_build.log; exit 3
fi
ninja -C build install > /tmp/ocv5_install.log 2>&1 || { echo "INSTALL FAILED"; tail -15 /tmp/ocv5_install.log; exit 4; }
echo "== installed"
find "$PREFIX" -name "opencv*.pc" 2>/dev/null
PKG_CONFIG_PATH="$PREFIX/lib/pkgconfig" pkg-config --modversion opencv5 2>/dev/null \
  || PKG_CONFIG_PATH="$PREFIX/lib/pkgconfig" pkg-config --modversion opencv4 2>/dev/null \
  || echo "(no .pc found -- check OPENCV_GENERATE_PKGCONFIG)"
echo "__DONE__"
