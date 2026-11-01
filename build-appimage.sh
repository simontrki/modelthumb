#!/usr/bin/env bash
# Builds dist/3D-Thumbnailer-x86_64.AppImage
# Needs: cargo, curl, unzip, dpkg-deb (or ar+tar), and optionally cargo-zigbuild for a glibc-2.17 binary.
set -euo pipefail
cd "$(dirname "$0")"
ROOT=$PWD
BUILD=$ROOT/build
APPDIR=$BUILD/AppDir
GMSH_VERSION=${GMSH_VERSION:-4.15.2}
mkdir -p "$BUILD/dl" "$ROOT/dist"

echo "==> compiling"
if command -v cargo-zigbuild >/dev/null; then
  cargo zigbuild --release --target x86_64-unknown-linux-gnu.2.17
  BIN=target/x86_64-unknown-linux-gnu/release/3dthumb
else
  echo "   (cargo-zigbuild not found: binary will need the build host's glibc or newer)"
  cargo build --release
  BIN=target/release/3dthumb
fi

echo "==> fetching libgmsh $GMSH_VERSION (STEP/IGES/BREP via OpenCASCADE) from PyPI"
WHL=$BUILD/dl/gmsh-$GMSH_VERSION.whl
if [ ! -s "$WHL" ]; then
  URL=$(curl -fsSL "https://pypi.org/pypi/gmsh/$GMSH_VERSION/json" |
        python3 -c 'import json,sys; print(next(u["url"] for u in json.load(sys.stdin)["urls"] if "manylinux" in u["filename"] and "x86_64" in u["filename"]))')
  curl -fL -o "$WHL" "$URL"
fi

echo "==> fetching fallback X11/GLU libs (Ubuntu 20.04 builds, glibc>=2.14)"
DEBS=(
  libg/libglu/libglu1-mesa_9.0.1-1build1_amd64.deb
  libx/libxcursor/libxcursor1_1.2.0-2_amd64.deb
  libx/libxinerama/libxinerama1_1.1.4-2_amd64.deb
  x/xft/libxft2_2.3.3-0ubuntu1_amd64.deb
  libx/libxfixes/libxfixes3_5.0.3-2_amd64.deb
  libx/libxrender/libxrender1_0.9.10-1_amd64.deb
)
for d in "${DEBS[@]}"; do
  f=$BUILD/dl/$(basename "$d")
  [ -s "$f" ] || curl -fsSL -o "$f" "http://archive.ubuntu.com/ubuntu/pool/main/$d"
done

echo "==> assembling AppDir"
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/lib/fallback" "$BUILD/x"
install -m755 "$BIN" "$APPDIR/usr/bin/3dthumb"
unzip -o -q -j "$WHL" "gmsh-$GMSH_VERSION.data/data/lib/libgmsh.so*" -d "$APPDIR/usr/lib/"
for d in "${DEBS[@]}"; do
  rm -rf "$BUILD/x"/*; dpkg-deb -x "$BUILD/dl/$(basename "$d")" "$BUILD/x"
  find "$BUILD/x" -name 'lib*.so.[0-9]' -exec cp -L {} "$APPDIR/usr/lib/fallback/" \;
done
cp assets/3dthumb.png "$APPDIR/3dthumb.png"
cp assets/3dthumb.png "$APPDIR/.DirIcon"
cat > "$APPDIR/3dthumb.desktop" <<'DESK'
[Desktop Entry]
Type=Application
Name=3D Thumbnailer
Comment=Previews for STL, 3MF, STEP, IGES, OBJ, PLY and glTF files in the file manager
Exec=3dthumb
Icon=3dthumb
Terminal=false
Categories=Graphics;3DGraphics;Utility;
DESK
cat > "$APPDIR/AppRun" <<'RUN'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
export APPDIR="${APPDIR:-$HERE}"
exec "$HERE/usr/bin/3dthumb" "$@"
RUN
chmod +x "$APPDIR/AppRun"

echo "==> packing AppImage"
TOOL=$BUILD/dl/appimagetool-x86_64.AppImage
[ -s "$TOOL" ] || curl -fL -o "$TOOL" https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage
chmod +x "$TOOL"
# --appimage-extract-and-run: works without FUSE (containers, CI)
ARCH=x86_64 "$TOOL" --appimage-extract-and-run --no-appstream "$APPDIR" "$ROOT/dist/3D-Thumbnailer-x86_64.AppImage"
ls -lh "$ROOT/dist/3D-Thumbnailer-x86_64.AppImage"
