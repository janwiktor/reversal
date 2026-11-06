#!/bin/bash
# Builds dist/Reversal-<version>-arm64.dmg: an ad hoc signed app with LibRaw built in.
# Needs Rust, pkgconf and Xcode (for the icon); no Homebrew libraries end up in the app.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
here="$root/packaging/macos"
build="$root/dist/build"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)"

# Sets prefix, source_dir, LIBRAW_VERSION and MACOS_TARGET.
. "$here/libraw.sh"

# Engine. Its own target folder, so this build doesn't disturb a development build.
PKG_CONFIG_PATH="$prefix/lib/pkgconfig" CARGO_TARGET_DIR="$build/target" \
  CARGO_PROFILE_RELEASE_LTO=fat CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 CARGO_PROFILE_RELEASE_STRIP=true \
  cargo build --release --locked --manifest-path "$root/Cargo.toml"

if otool -L "$build/target/release/reversal" | grep -q raw; then
  echo "The engine links a shared LibRaw, so it wouldn't run on other Macs." >&2
  exit 1
fi

# App bundle.
app="$build/Reversal.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources/Licenses"
swiftc -O -target "arm64-apple-macos$MACOS_TARGET" "$here/main.swift" -o "$app/Contents/MacOS/Reversal"
cp "$build/target/release/reversal" "$app/Contents/MacOS/reversal-engine"
sed "s/@VERSION@/$version/" "$here/Info.plist" >"$app/Contents/Info.plist"
DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}" xcrun actool "$here/Reversal.icon" \
  --compile "$app/Contents/Resources" --app-icon Reversal --platform macosx \
  --minimum-deployment-target "$MACOS_TARGET" --output-partial-info-plist "$build/icon.plist" >"$build/icon.log"
# It can fail and still exit 0.
if [ ! -f "$app/Contents/Resources/Assets.car" ] || [ ! -f "$app/Contents/Resources/Reversal.icns" ]; then
  cat "$build/icon.log" >&2
  echo "The icon didn't compile." >&2
  exit 1
fi

# Licenses of what's inside.
licenses="$app/Contents/Resources/Licenses"
cp "$source_dir/LICENSE.CDDL" "$source_dir/COPYRIGHT" "$licenses/"
{
  echo "LibRaw $LIBRAW_VERSION is used under the CDDL 1.0, see LICENSE.CDDL."
  echo "Its source code is at https://www.libraw.org/data/LibRaw-$LIBRAW_VERSION.tar.gz"
} >"$licenses/LibRaw.txt"
(cd "$root" && cargo tree -e normal --target aarch64-apple-darwin --prefix none --format '{p}' --locked |
  python3 "$here/notices.py") >"$licenses/Rust crates.txt"

# Inside out: the engine first, then the bundle that seals it.
codesign --force --sign - "$app/Contents/MacOS/reversal-engine"
codesign --force --sign - "$app"
codesign --verify --deep --strict "$app"

# Disk image, with a link to drag the app onto.
stage="$build/dmg"
rm -rf "$stage"
mkdir -p "$stage"
cp -R "$app" "$stage/"
cp "$here/Read me first.txt" "$stage/"
ln -s /Applications "$stage/Applications"
dmg="$root/dist/Reversal-$version-arm64.dmg"
rm -f "$dmg"
hdiutil create -quiet -volname "Reversal" -srcfolder "$stage" -format ULMO -fs APFS "$dmg"
echo "$dmg ($(du -h "$dmg" | cut -f1 | xargs))"
