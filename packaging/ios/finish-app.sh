#!/bin/bash
# This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

# The last step of building the app. Gives it its version from the workspace and a build number from
# git, as the Mac app, and the licenses that engine.sh gathered.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd -P)"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)"
build_number="$(git -C "$root" rev-list --count HEAD 2>/dev/null || echo 0)"
plist="$TARGET_BUILD_DIR/$INFOPLIST_PATH"
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $version" "$plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleVersion $build_number" "$plist"

resources="$TARGET_BUILD_DIR/$UNLOCALIZED_RESOURCES_FOLDER_PATH"
rm -rf "$resources/Licenses"
cp -R "$root/dist/build/ios/$PLATFORM_NAME/Licenses" "$resources/Licenses"
