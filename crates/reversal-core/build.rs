// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

fn main() {
    // The probe's output isn't passed on to cargo, so say ourselves when to run it again.
    for name in [
        "PKG_CONFIG_PATH",
        "PKG_CONFIG_LIBDIR",
        "PKG_CONFIG_SYSROOT_DIR",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    if std::env::var_os("CARGO_FEATURE_LIBRAW").is_none() {
        return;
    }
    let lib = pkg_config::Config::new()
        .atleast_version("0.20")
        .cargo_metadata(false)
        .probe("libraw")
        .expect("LibRaw not found: install it (macOS: brew install libraw pkgconf)");
    for path in &lib.link_paths {
        println!("cargo:rustc-link-search=native={}", path.display());
    }
    let apple = std::env::var("CARGO_CFG_TARGET_VENDOR").as_deref() == Ok("apple");
    for name in &lib.libs {
        // libraw.pc names GNU's stdc++; Apple toolchains ship libc++ instead.
        let name = if apple && name == "stdc++" {
            "c++"
        } else {
            name.as_str()
        };
        println!("cargo:rustc-link-lib={name}");
    }
}
