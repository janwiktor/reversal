"""Licence texts of the Rust crates in the app, from `cargo tree` output on stdin."""

import glob
import json
import os
import subprocess
import sys

wanted = set()
for line in sys.stdin:
    parts = line.split()
    if len(parts) >= 2:
        wanted.add((parts[0], parts[1].lstrip("v")))

metadata = json.loads(
    subprocess.check_output(["cargo", "metadata", "--format-version", "1", "--locked"])
)
for package in sorted(metadata["packages"], key=lambda p: p["name"]):
    if (package["name"], package["version"]) not in wanted or package["source"] is None:
        continue
    print(f"{'=' * 72}\n{package['name']} {package['version']}: {package['license']}\n")
    folder = os.path.dirname(package["manifest_path"])
    for path in sorted(glob.glob(folder + "/LICEN[CS]E*") + glob.glob(folder + "/COPYING*")):
        if os.path.isfile(path):
            print(open(path, encoding="utf-8", errors="replace").read())
