//! Reversal conversion engine.
//!
//! Camera scan of a negative → linear camera RGB ([`decode`]) → density-model
//! inversion ([`invert`]) → positive, scene-linear LinearRaw DNG ([`dng`]).
//! No UI or host APIs here: the CLI, web UI and future plugin are thin adapters.

pub mod color;
#[cfg(feature = "libraw")]
pub mod decode;
pub mod dng;
pub mod image;
pub mod invert;
pub mod ljpeg;
pub mod render;

use std::ffi::OsString;
use std::io;
use std::path::Path;

use image::Rgb32;
use invert::Settings;

/// XMP packet stored in every positive: conversion settings and model version, plus `crs:WhiteBalance="As Shot"` so raw editors start from our neutral.
pub fn xmp_packet(source_name: &str, settings: &Settings) -> String {
    let json = serde_json::to_string(settings).expect("settings serialise");
    let esc = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('"', "&quot;")
    };
    format!(
        r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
<rdf:Description rdf:about="" xmlns:reversal="https://github.com/janwiktor/reversal/ns/1.0/"
 xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" crs:WhiteBalance="As Shot"
 reversal:Source="{}" reversal:Settings="{}"/>
</rdf:RDF></x:xmpmeta>"#,
        esc(source_name),
        esc(&json)
    )
}

/// Profiles written into every positive: the contrast chosen at conversion is
/// the main profile; Neutral (the raw editor's own default curve) is selectable.
pub fn profiles(contrast: f32) -> (dng::Profile, Vec<dng::Profile>) {
    let main = dng::Profile {
        name: "Reversal Print".into(),
        tone_curve: Some(render::tone_curve(contrast, 64)),
    };
    let neutral = dng::Profile {
        name: "Reversal Neutral".into(),
        tone_curve: None,
    };
    (main, vec![neutral])
}

/// Convert a decoded scan and write the DNG; returns the file size in bytes.
pub fn write_positive(
    out: &Path,
    source_name: &str,
    scan: &Rgb32,
    settings: &Settings,
    compression: dng::Compression,
    software: &str,
) -> io::Result<u64> {
    let positive = invert::convert(scan, settings);
    let (profile, extra_profiles) = profiles(settings.adjust.contrast);
    let xmp = xmp_packet(source_name, settings);
    let opts = dng::DngOptions {
        compression,
        profile,
        extra_profiles,
        xmp: Some(&xmp),
        software,
    };
    dng::write(out, &positive, &opts)?;
    Ok(std::fs::metadata(out)?.len())
}

/// Write through a temporary file in the same folder, so a failed write can't leave
/// a truncated file in place of a good one.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?;
    let mut tmp_name = OsString::from(".");
    tmp_name.push(name);
    tmp_name.push(format!(".{}.tmp", std::process::id()));
    let tmp = path.with_file_name(tmp_name);
    let result = std::fs::write(&tmp, bytes).and_then(|_| std::fs::rename(&tmp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_atomic_replaces_and_leaves_no_temp_file() {
        let dir = std::env::temp_dir().join(format!("reversal-atomic-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.dng");
        write_atomic(&path, b"old").unwrap();
        write_atomic(&path, b"new").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        // Failure keeps the old file and cleans up.
        assert!(write_atomic(&dir.join("missing/a.dng"), b"x").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn write_positive_writes_a_dng_with_settings() {
        let dir = std::env::temp_dir().join(format!("reversal-positive-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("p.dng");
        let mut scan = Rgb32::new(64, 48);
        for (i, v) in scan.data.iter_mut().enumerate() {
            *v = 0.05 + (i % 89) as f32 / 100.0;
        }
        let settings = Settings::new(
            invert::estimate(&scan, &invert::Hints::default()),
            invert::Adjust::default(),
        );
        let n = write_positive(
            &path,
            "s.NEF",
            &scan,
            &settings,
            dng::Compression::None,
            "t",
        )
        .unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes.len() as u64, n);
        assert_eq!(&bytes[0..4], b"II*\0");
        let needle = format!("&quot;model&quot;:{}", invert::MODEL_VERSION);
        assert!(bytes.windows(needle.len()).any(|w| w == needle.as_bytes()));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
