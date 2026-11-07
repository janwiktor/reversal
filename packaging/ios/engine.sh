#!/bin/bash
# Builds the engine for one Apple SDK into dist/build/ios/<sdk>/lib, for the app to link: LibRaw
# and the reversal-ios crate. Xcode runs this before compiling; by hand: engine.sh iphonesimulator
set -euo pipefail

# Xcode's script environment finds neither cargo nor pkg-config by itself.
export PATH="$HOME/.cargo/bin:/opt/homebrew/opt/rustup/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"

root="$(cd "$(dirname "$0")/../.." && pwd -P)"
sdk="${1:-${PLATFORM_NAME:-iphonesimulator}}"
export IPHONEOS_DEPLOYMENT_TARGET="${IPHONEOS_DEPLOYMENT_TARGET:-17.0}"
case "$sdk" in
  iphoneos) rust_target=aarch64-apple-ios; clang_target="arm64-apple-ios$IPHONEOS_DEPLOYMENT_TARGET" ;;
  iphonesimulator) rust_target=aarch64-apple-ios-sim; clang_target="arm64-apple-ios$IPHONEOS_DEPLOYMENT_TARGET-simulator" ;;
  *) echo "Unknown SDK: $sdk" >&2; exit 1 ;;
esac
export DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
sysroot="$(xcrun --sdk "$sdk" --show-sdk-path)"
out="$root/dist/build/ios/$sdk"

# Xcode's settings are for the app. Left in place they'd reach the tools built for this Mac along
# the way, such as the crates' build scripts.
unset SDKROOT LIBRARY_SEARCH_PATHS HEADER_SEARCH_PATHS FRAMEWORK_SEARCH_PATHS LD_RUNPATH_SEARCH_PATHS

# LibRaw: the same version and options as the Mac app, for iOS. Sets prefix.
flags="-target $clang_target -isysroot $sysroot"
build="$out"
LIBRAW_CONFIGURE_EXTRA=(--host=aarch64-apple-darwin "CC=$(xcrun --sdk "$sdk" -f clang) $flags"
  "CXX=$(xcrun --sdk "$sdk" -f clang++) $flags")
# shellcheck source=../macos/libraw.sh
. "$root/packaging/macos/libraw.sh"

PKG_CONFIG_ALLOW_CROSS=1 PKG_CONFIG_PATH="$prefix/lib/pkgconfig" CARGO_TARGET_DIR="$out/target" \
  cargo build --release --locked --manifest-path "$root/Cargo.toml" -p reversal-ios --target "$rust_target"

mkdir -p "$out/lib"
cp "$out/target/$rust_target/release/libreversal_ios.a" "$prefix/lib/libraw_r.a" "$out/lib/"
