fn main() {
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
