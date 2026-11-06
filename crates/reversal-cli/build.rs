//! Stamps the binary with the commit it was built from, to tell builds of one version apart.
//! The commit must stay out of DNGs, so one version's conversions stay bit-identical.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    let text = String::from_utf8(output.stdout).ok()?;
    (output.status.success() && !text.trim().is_empty()).then(|| text.trim().to_string())
}

fn main() {
    for name in ["HEAD", "logs/HEAD"] {
        if let Some(path) = git(&["rev-parse", "--git-path", name]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    println!("cargo:rerun-if-changed=build.rs");
    // Empty when built from a source archive, which has no git history.
    let commit = git(&["rev-parse", "--short=7", "HEAD"]).unwrap_or_default();
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let label = if commit.is_empty() {
        version
    } else {
        format!("{version} ({commit})")
    };
    println!("cargo:rustc-env=REVERSAL_COMMIT={commit}");
    println!("cargo:rustc-env=REVERSAL_VERSION_LABEL={label}");
}
