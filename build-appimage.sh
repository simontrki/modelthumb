#!/usr/bin/env bash
# Builds dist/modelthumb-x86_64.AppImage
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
  BIN=target/x86_64-unknown-linux-gnu/release/modelthumb
else
  echo "   (cargo-zigbuild not found: binary will need the build host's glibc or newer)"
  cargo build --release
  BIN=target/release/modelthumb
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
install -m755 "$BIN" "$APPDIR/usr/bin/modelthumb"
unzip -o -q -j "$WHL" "gmsh-$GMSH_VERSION.data/data/lib/libgmsh.so*" -d "$APPDIR/usr/lib/"
for d in "${DEBS[@]}"; do
  rm -rf "$BUILD/x"/*; dpkg-deb -x "$BUILD/dl/$(basename "$d")" "$BUILD/x"
  find "$BUILD/x" -name 'lib*.so.[0-9]' -exec cp -L {} "$APPDIR/usr/lib/fallback/" \;
done
# licenses: GPL-3.0 for modelthumb, plus the notices of everything bundled
LIC=$APPDIR/usr/share/licenses/modelthumb
mkdir -p "$LIC"
cp LICENSE THIRD-PARTY-LICENSES.md "$LIC/"
unzip -o -q -j "$WHL" "gmsh-$GMSH_VERSION.data/data/share/doc/gmsh/LICENSE.txt" -d "$BUILD/x"
mv "$BUILD/x/LICENSE.txt" "$LIC/gmsh-LICENSE.txt"
for d in "${DEBS[@]}"; do
  pkg=$(basename "$d" | cut -d_ -f1)
  rm -rf "$BUILD/x"/*; dpkg-deb -x "$BUILD/dl/$(basename "$d")" "$BUILD/x"
  cp "$BUILD/x/usr/share/doc/$pkg/copyright" "$LIC/$pkg-copyright"
done
cp assets/modelthumb.png "$APPDIR/modelthumb.png"
cp assets/modelthumb.png "$APPDIR/.DirIcon"
cat > "$APPDIR/modelthumb.desktop" <<'DESK'
[Desktop Entry]
Type=Application
Name=modelthumb
Comment=Previews for STL, 3MF, STEP, IGES, OBJ, PLY and glTF files in the file manager
Exec=modelthumb
Icon=modelthumb
Terminal=false
Categories=Graphics;3DGraphics;Utility;
DESK
cat > "$APPDIR/AppRun" <<'RUN'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
export APPDIR="${APPDIR:-$HERE}"
exec "$HERE/usr/bin/modelthumb" "$@"
RUN
chmod +x "$APPDIR/AppRun"

echo "==> packing AppImage"
TOOL=$BUILD/dl/appimagetool-x86_64.AppImage
[ -s "$TOOL" ] || curl -fL -o "$TOOL" https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage
chmod +x "$TOOL"
# --appimage-extract-and-run: works without FUSE (containers, CI)
ARCH=x86_64 "$TOOL" --appimage-extract-and-run --no-appstream "$APPDIR" "$ROOT/dist/modelthumb-x86_64.AppImage"
ls -lh "$ROOT/dist/modelthumb-x86_64.AppImage"
