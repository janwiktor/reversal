# This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

# Gathers the licenses of what's inside an app into "$licenses". Sourced after libraw.sh, by the Mac
# app's and the iPad app's builds. Needs root, licenses, rust_package and rust_target.

# From scratch: a file that was gathered under another name mustn't stay.
rm -rf "$licenses"
mkdir -p "$licenses"
cp "$root/LICENSE" "$licenses/Reversal MPL-2.0.txt"
# LibRaw offers two licenses. It's the CDDL in every build, which lets the app go through app stores.
# Named for what they are: the web UI's About lists these files by name.
cp "$source_dir/LICENSE.CDDL" "$licenses/LibRaw CDDL-1.0.txt"
cp "$source_dir/COPYRIGHT" "$licenses/LibRaw copyright.txt"
{
  echo "LibRaw $LIBRAW_VERSION is used under the CDDL 1.0, see LibRaw CDDL-1.0.txt."
  echo "Its source code is attached to each release of Reversal, and is at"
  echo "https://www.libraw.org/data/LibRaw-$LIBRAW_VERSION.tar.gz"
} >"$licenses/LibRaw.txt"
echo "This product includes DNG technology under license by Adobe." >"$licenses/DNG.txt"
(cd "$root" && cargo tree -p "$rust_package" -e normal --target "$rust_target" --prefix none --format '{p}' --locked |
  python3 "$root/packaging/macos/notices.py") >"$licenses/Rust crates.txt"
