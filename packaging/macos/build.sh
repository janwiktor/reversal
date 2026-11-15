#!/bin/bash
# This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

# Builds dist/Reversal-<version>-arm64.dmg: the app with LibRaw built in.
# Needs Rust, pkgconf and Xcode (for the icon); no Homebrew libraries end up in the app.
#
# Signing. With none of these set, the app is ad hoc signed, which is enough to run it on this Mac.
#   SIGNING_IDENTITY   A Developer ID Application certificate in the keychain, by name or hash:
#                      signs the app with the hardened runtime, and the disk image.
#   Then notarizes the disk image and staples the ticket, with either
#   NOTARY_PROFILE     a profile saved with `xcrun notarytool store-credentials`, or
#   NOTARY_KEY, NOTARY_KEY_ID, NOTARY_ISSUER
#                      an App Store Connect API key: the .p8 file's path, its key ID and issuer ID.
#   REQUIRE_NOTARIZED=1 stops the build when it wouldn't be notarized.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd -P)"
here="$root/packaging/macos"
build="$root/dist/build"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)"

# Rises with every commit on main, and anyone building a commit gets the same number. Needs the
# whole history; a source archive has none and builds as 0.
build_number=0
# Only this repo's history counts, not that of a repo the archive happens to be unpacked in.
if [ "$(git -C "$root" rev-parse --show-toplevel 2>/dev/null || true)" = "$root" ]; then
  if [ "$(git -C "$root" rev-parse --is-shallow-repository)" = true ]; then
    echo "A shallow clone can't count commits for the build number: fetch the whole history." >&2
    exit 1
  fi
  build_number="$(git -C "$root" rev-list --count HEAD)"
fi

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
sed -e "s/@VERSION@/$version/" -e "s/@BUILD@/$build_number/" "$here/Info.plist" >"$app/Contents/Info.plist"
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
rust_package=reversal-cli
rust_target=aarch64-apple-darwin
# shellcheck source=licenses.sh
. "$here/licenses.sh"

identity="${SIGNING_IDENTITY:-}"
notary=()
if [ -n "${NOTARY_PROFILE:-}" ]; then
  notary=(--keychain-profile "$NOTARY_PROFILE")
elif [ -n "${NOTARY_KEY:-}" ]; then
  notary=(--key "$NOTARY_KEY" --key-id "${NOTARY_KEY_ID:?NOTARY_KEY needs NOTARY_KEY_ID}" \
    --issuer "${NOTARY_ISSUER:?NOTARY_KEY needs NOTARY_ISSUER}")
fi
if [ "${REQUIRE_NOTARIZED:-0}" = 1 ] && { [ -z "$identity" ] || [ ${#notary[@]} -eq 0 ]; }; then
  echo "REQUIRE_NOTARIZED is set, but SIGNING_IDENTITY or the notary credentials are missing." >&2
  exit 1
fi
if [ -z "$identity" ] && [ ${#notary[@]} -gt 0 ]; then
  echo "Notarizing needs SIGNING_IDENTITY: an ad hoc signed app can't be notarized." >&2
  exit 1
fi

# Inside out: the engine first, then the bundle that seals it. Notarizing needs the hardened
# runtime and a secure timestamp; the app needs no entitlements under it.
if [ -n "$identity" ]; then
  sign=(codesign --force --sign "$identity" --options runtime --timestamp)
else
  sign=(codesign --force --sign -)
fi
"${sign[@]}" "$app/Contents/MacOS/reversal-engine"
"${sign[@]}" "$app"
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

signing="ad hoc signed"
if [ -n "$identity" ]; then
  codesign --force --sign "$identity" --timestamp "$dmg"
  signing="signed with $(codesign -dvv "$dmg" 2>&1 | sed -n 's/^Authority=//p' | head -1)"
fi
# Notarizing the disk image covers the app in it. The ticket is stapled to the disk image; once the
# app is copied out, Gatekeeper looks the app's ticket up online.
if [ ${#notary[@]} -gt 0 ]; then
  xcrun notarytool submit "$dmg" "${notary[@]}" --wait --timeout 30m --output-format json \
    >"$build/notary.json" || true
  status="$(plutil -extract status raw "$build/notary.json" 2>/dev/null || echo unknown)"
  if [ "$status" != Accepted ]; then
    cat "$build/notary.json" >&2
    id="$(plutil -extract id raw "$build/notary.json" 2>/dev/null || true)"
    [ -n "$id" ] && xcrun notarytool log "$id" "${notary[@]}" >&2 || true
    echo "Notarization ended with status $status." >&2
    exit 1
  fi
  xcrun stapler staple -q "$dmg"
  xcrun stapler validate -q "$dmg"
  signing="$signing, notarized"
fi
echo "Version $version, build $build_number, $signing"
echo "$dmg ($(du -h "$dmg" | cut -f1 | xargs))"
