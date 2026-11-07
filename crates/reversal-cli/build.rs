// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Stamps the binary with the commit it was built from, to tell builds of one version apart.
//! The commit must stay out of DNGs, so one version's conversions stay bit-identical.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    let text = String::from_utf8(output.stdout).ok()?;
    (output.status.success() && !text.trim().is_empty()).then(|| text.trim().to_string())
}

/// Whether git would answer for this workspace, not for a repo it happens to be unpacked in.
fn in_own_repo() -> bool {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let top = git(&["rev-parse", "--show-toplevel"]);
    match (root.canonicalize(), top.map(std::fs::canonicalize)) {
        (Ok(root), Some(Ok(top))) => root == top,
        _ => false,
    }
}

fn main() {
    let own_repo = in_own_repo();
    for name in ["HEAD", "logs/HEAD"] {
        if let Some(path) = git(&["rev-parse", "--git-path", name]).filter(|_| own_repo) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    println!("cargo:rerun-if-changed=build.rs");
    // Empty when built from a source archive, which has no git history.
    let commit = git(&["rev-parse", "--short=7", "HEAD"])
        .filter(|_| own_repo)
        .unwrap_or_default();
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let label = if commit.is_empty() {
        version
    } else {
        format!("{version} ({commit})")
    };
    println!("cargo:rustc-env=REVERSAL_COMMIT={commit}");
    println!("cargo:rustc-env=REVERSAL_VERSION_LABEL={label}");
}
