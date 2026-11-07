# This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

# Builds the LibRaw that goes in the Mac app, in "$build", unless it's already there. Sourced
# by build.sh and by CI, so the tests run against the same library the app ships with.
# Sets prefix, source_dir, LIBRAW_VERSION and MACOS_TARGET.

LIBRAW_VERSION=0.22.3
LIBRAW_SHA256=789f3c33466210a1a0907f747a97c715d21bbf4af712f257fcbbd90111b227e4
MACOS_TARGET=13.0
export MACOSX_DEPLOYMENT_TARGET="$MACOS_TARGET"
mkdir -p "$build"

# LibRaw, static and with only what the conversion uses: the Homebrew library only runs on
# the macOS it was built for. Color management is ours and OpenMP isn't needed.
prefix="$build/libraw-install-$LIBRAW_VERSION"
source_dir="$build/LibRaw-$LIBRAW_VERSION"
if [ ! -f "$prefix/lib/pkgconfig/libraw.pc" ]; then
  tarball="$build/LibRaw-$LIBRAW_VERSION.tar.gz"
  [ -f "$tarball" ] || curl -fsSL -o "$tarball" "https://www.libraw.org/data/LibRaw-$LIBRAW_VERSION.tar.gz"
  echo "$LIBRAW_SHA256  $tarball" | shasum -a 256 -c -
  rm -rf "$source_dir" "$prefix"
  tar -xzf "$tarball" -C "$build"
  (cd "$source_dir" &&
    ./configure --prefix="$prefix" --disable-shared --enable-static --disable-openmp \
      --disable-lcms --disable-jpeg --enable-zlib --disable-examples >/dev/null &&
    make -j"$(sysctl -n hw.ncpu)" install >/dev/null)
fi
