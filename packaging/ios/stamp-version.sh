#!/bin/bash
# Gives the built app its version from the workspace, and a build number from git, as the Mac app.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd -P)"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)"
build_number="$(git -C "$root" rev-list --count HEAD 2>/dev/null || echo 0)"
plist="$TARGET_BUILD_DIR/$INFOPLIST_PATH"
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $version" "$plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleVersion $build_number" "$plist"
