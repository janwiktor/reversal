// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Reversal conversion engine.
//!
//! Camera scan of a negative → linear camera RGB ([`decode`]) → density-model
//! inversion ([`invert`]) → positive, scene-linear LinearRaw DNG ([`dng`]).
//! No UI or host APIs here: the CLI, web UI and Mac app are thin adapters.

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
use std::sync::atomic::{AtomicU64, Ordering};

use image::Rgb32;
use invert::Settings;

/// XMP packet stored in every positive: conversion settings and model version, plus `crs:WhiteBalance="As Shot"` so raw editors start from our neutral.
pub fn xmp_packet(source_name: &str, settings: &Settings) -> String {
    let conversion = Settings {
        custom_crop: None,
        ..settings.clone()
    };
    let json = serde_json::to_string(&conversion).expect("settings serialize");
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

/// Profiles written into every positive, all under the same tone sliders and for the frame's
/// film type: the chosen look is the main profile, the others are selectable in a raw editor.
pub fn profiles(settings: &Settings) -> (dng::Profile, Vec<dng::Profile>) {
    let (adjust, film) = (&settings.adjust, settings.estimate.film);
    let tone = adjust.tone();
    let others = render::Look::ALL
        .into_iter()
        .filter(|&look| look != adjust.look)
        .map(|look| render::profile(look, tone, film))
        .collect();
    (render::profile(adjust.look, tone, film), others)
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
    let (profile, extra_profiles) = profiles(settings);
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
    // Unique per write, so writers of one file can't share a half-written temporary file.
    static WRITES: AtomicU64 = AtomicU64::new(0);
    let write = WRITES.fetch_add(1, Ordering::Relaxed);
    let mut tmp_name = OsString::from(".");
    tmp_name.push(name);
    tmp_name.push(format!(".{}.{write}.tmp", std::process::id()));
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
    fn a_kept_custom_crop_stays_out_of_the_dng() {
        let estimate = invert::estimate_on(
            &Rgb32 {
                width: 1,
                height: 1,
                data: vec![0.2, 0.3, 0.4],
            },
            &invert::Hints::default(),
        );
        let plain = Settings::new(estimate, invert::Adjust::default());
        let kept = Settings {
            custom_crop: Some(image::NRect {
                x0: 0.1,
                y0: 0.2,
                x1: 0.6,
                y1: 0.7,
            }),
            ..plain.clone()
        };
        assert_eq!(xmp_packet("a.NEF", &kept), xmp_packet("a.NEF", &plain));
    }

    fn settings_for(film: invert::Film, look: render::Look) -> Settings {
        let mut estimate = invert::estimate_on(&Rgb32::new(1, 1), &invert::Hints::default());
        estimate.film = film;
        let adjust = invert::Adjust {
            look,
            ..invert::Adjust::default()
        };
        Settings::new(estimate, adjust)
    }

    #[test]
    fn the_chosen_look_is_the_main_profile_and_the_others_stay_selectable() {
        let (main, others) = profiles(&settings_for(invert::Film::Color, render::Look::Hard));
        assert_eq!(main.name, "Reversal Hard");
        let names: Vec<_> = others.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Reversal Neutral", "Reversal Flat"]);
    }

    #[test]
    fn black_and_white_gets_its_own_profiles_held_gray() {
        let (main, others) = profiles(&settings_for(invert::Film::Bw, render::Look::Flat));
        assert_eq!(main.name, "Reversal B&W Flat");
        let names: Vec<_> = others.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Reversal B&W Neutral", "Reversal B&W Hard"]);
        assert!([&main]
            .into_iter()
            .chain(&others)
            .all(|p| p.saturation == 0.0));
    }

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
