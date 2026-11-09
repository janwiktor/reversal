// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Minimal writer for positive LinearRaw DNGs.
//!
//! Layout: IFD0 = 8-bit RGB thumbnail + DNG-level tags (color, profiles, XMP);
//! SubIFD 0 = the 16-bit LinearRaw image; SubIFD 1 = an sRGB JPEG preview.
//! Frame white is stored at `STORE_SCALE`, so highlights above it survive as recoverable
//! headroom, and each profile's tone curve maps the whole stored range to the display.
//! `BaselineExposure` stays 0: renderers treat data above display white differently, so the
//! curves leave none.

use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

use crate::color::{self, ILLUMINANT_D65};
use crate::image::Rgb32;
use crate::ljpeg;
use crate::render;

pub const UNIQUE_CAMERA_MODEL: &str = "Reversal Positive";
/// Stored value of frame white, about 2.3 stops under full scale.
pub const STORE_SCALE: f32 = 0.2;
/// Largest positive value (image white = 1) the 16-bit data can hold.
pub const STORED_MAX: f32 = 1.0 / STORE_SCALE;
const TILE: usize = 256;
const PREVIEW_LONG_SIDE: usize = 1024;
const THUMB_LONG_SIDE: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compression {
    None,
    LosslessJpeg,
}

/// A DNG camera profile.
#[derive(Clone, Debug)]
pub struct Profile {
    pub name: String,
    /// Linear (x, y) pairs over the stored range.
    pub tone_curve: Vec<(f32, f32)>,
    /// Saturation applied by the profile's look table; 1 = none.
    pub saturation: f32,
}

pub struct DngOptions<'a> {
    pub compression: Compression,
    /// Main profile: what the file opens with (and what Apple Photos applies).
    pub profile: Profile,
    /// Further profiles selectable in a raw editor's profile browser.
    pub extra_profiles: Vec<Profile>,
    /// XMP packet (UTF-8), e.g. Reversal settings and `crs:WhiteBalance`.
    pub xmp: Option<&'a str>,
    pub software: &'a str,
}

enum Val {
    Byte(Vec<u8>),
    Ascii(String),
    Short(Vec<u16>),
    Long(Vec<u32>),
    Rational(Vec<f64>),
    SRational(Vec<f64>),
    Float(Vec<f32>),
}

struct Tag(u16, Val);

fn rational(v: f64) -> (i64, i64) {
    let den: i64 = 1_000_000;
    let num = (v * den as f64).round() as i64;
    let g = gcd(num.unsigned_abs(), den as u64).max(1) as i64;
    (num / g, den / g)
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

impl Val {
    fn encode(&self) -> (u16, u32, Vec<u8>) {
        match self {
            Val::Byte(v) => (1, v.len() as u32, v.clone()),
            Val::Ascii(s) => {
                let mut b = s.as_bytes().to_vec();
                b.push(0);
                (2, b.len() as u32, b)
            }
            Val::Short(v) => (
                3,
                v.len() as u32,
                v.iter().flat_map(|x| x.to_le_bytes()).collect(),
            ),
            Val::Long(v) => (
                4,
                v.len() as u32,
                v.iter().flat_map(|x| x.to_le_bytes()).collect(),
            ),
            Val::Rational(v) => (
                5,
                v.len() as u32,
                v.iter()
                    .flat_map(|&x| {
                        let (n, d) = rational(x);
                        [(n as u32).to_le_bytes(), (d as u32).to_le_bytes()].concat()
                    })
                    .collect(),
            ),
            Val::SRational(v) => (
                10,
                v.len() as u32,
                v.iter()
                    .flat_map(|&x| {
                        let (n, d) = rational(x);
                        [(n as i32).to_le_bytes(), (d as i32).to_le_bytes()].concat()
                    })
                    .collect(),
            ),
            Val::Float(v) => (
                11,
                v.len() as u32,
                v.iter().flat_map(|x| x.to_le_bytes()).collect(),
            ),
        }
    }
}

/// TIFF byte buffer; offsets written into IFDs are relative to the buffer start.
struct Buf {
    data: Vec<u8>,
}

impl Buf {
    fn align(&mut self) {
        if self.data.len() % 2 == 1 {
            self.data.push(0);
        }
    }

    fn blob(&mut self, bytes: &[u8]) -> u32 {
        self.align();
        let off = self.data.len() as u32;
        self.data.extend_from_slice(bytes);
        off
    }

    /// Write an IFD with its out-of-line values right after it; returns its offset.
    fn ifd(&mut self, mut tags: Vec<Tag>) -> u32 {
        tags.sort_by_key(|t| t.0);
        self.align();
        let start = self.data.len();
        let values_at = start + 2 + 12 * tags.len() + 4;
        let mut entries = (tags.len() as u16).to_le_bytes().to_vec();
        let mut extra: Vec<u8> = Vec::new();
        for Tag(id, val) in &tags {
            let (typ, count, bytes) = val.encode();
            entries.extend_from_slice(&id.to_le_bytes());
            entries.extend_from_slice(&typ.to_le_bytes());
            entries.extend_from_slice(&count.to_le_bytes());
            if bytes.len() <= 4 {
                let mut field = bytes.clone();
                field.resize(4, 0);
                entries.extend_from_slice(&field);
            } else {
                if (values_at + extra.len()) % 2 == 1 {
                    extra.push(0);
                }
                entries.extend_from_slice(&((values_at + extra.len()) as u32).to_le_bytes());
                extra.extend_from_slice(&bytes);
            }
        }
        entries.extend_from_slice(&0u32.to_le_bytes());
        self.data.extend_from_slice(&entries);
        self.data.extend_from_slice(&extra);
        start as u32
    }
}

/// Tags of a profile, the same whether it's the main one or an extra one.
fn profile_tags(p: &Profile) -> Vec<Tag> {
    let (color_matrix, forward) = color::dng_matrices();
    let flat = |m: color::Mat3| m.iter().flatten().copied().collect::<Vec<f64>>();
    let mut tags = vec![
        Tag(50708, Val::Ascii(UNIQUE_CAMERA_MODEL.into())),
        Tag(50721, Val::SRational(flat(color_matrix))), // ColorMatrix1: XYZ -> our RGB
        Tag(50778, Val::Short(vec![ILLUMINANT_D65])),   // CalibrationIlluminant1
        Tag(50936, Val::Ascii(p.name.clone())),         // ProfileName
        Tag(50941, Val::Long(vec![3])),                 // ProfileEmbedPolicy: no restrictions
        Tag(50964, Val::SRational(flat(forward))),      // ForwardMatrix1: -> XYZ D50
        // DefaultBlackRender: None. The curves place black themselves; left on auto, renderers
        // subtract their own black level, each a different one.
        Tag(51110, Val::Long(vec![1])),
    ];
    tags.push(Tag(
        50940, // ProfileToneCurve
        Val::Float(p.tone_curve.iter().flat_map(|&(x, y)| [x, y]).collect()),
    ));
    if p.saturation != 1.0 {
        // A uniform table: no hue shift, the same saturation scale everywhere, value as is.
        let (hues, sats, vals) = (6u32, 2u32, 1u32);
        let entry = [0.0, p.saturation, 1.0];
        tags.push(Tag(50981, Val::Long(vec![hues, sats, vals]))); // ProfileLookTableDims
        tags.push(Tag(
            50982, // ProfileLookTableData
            Val::Float(entry.repeat((hues * sats * vals) as usize)),
        ));
    }
    tags
}

/// A camera profile in DCP layout ("IIRC"), for ExtraCameraProfiles.
fn profile_blob(p: &Profile) -> Vec<u8> {
    let mut buf = Buf {
        data: b"IIRC".to_vec(),
    };
    buf.data.extend_from_slice(&8u32.to_le_bytes());
    buf.ifd(profile_tags(p));
    buf.data
}

fn quantize(img: &Rgb32) -> Vec<u16> {
    img.data
        .iter()
        .map(|&v| ((v * STORE_SCALE).clamp(0.0, 1.0) * 65535.0 + 0.5) as u16)
        .collect()
}

/// Write the image data; returns the layout tags for the main IFD.
fn write_pixels(buf: &mut Buf, img: &Rgb32, compression: Compression) -> Vec<Tag> {
    let samples = quantize(img);
    let (w, h) = (img.width, img.height);
    // LibRaw misplaces the rows of a lossless JPEG tile wider than its image, and TIFF tile
    // widths come in multiples of 16, so images narrower than a tile are stored uncompressed.
    let compression = if w < TILE {
        Compression::None
    } else {
        compression
    };
    match compression {
        Compression::None => {
            let rows_per_strip = ((1 << 20) / (w * 6)).max(1);
            let (mut offsets, mut counts) = (Vec::new(), Vec::new());
            for chunk in samples.chunks(rows_per_strip * w * 3) {
                let bytes: Vec<u8> = chunk.iter().flat_map(|v| v.to_le_bytes()).collect();
                offsets.push(buf.blob(&bytes));
                counts.push(bytes.len() as u32);
            }
            vec![
                Tag(259, Val::Short(vec![1])),
                Tag(273, Val::Long(offsets)),
                Tag(278, Val::Long(vec![rows_per_strip as u32])),
                Tag(279, Val::Long(counts)),
            ]
        }
        Compression::LosslessJpeg => {
            let (mut offsets, mut counts) = (Vec::new(), Vec::new());
            let mut tile = vec![0u16; TILE * TILE * 3];
            for ty in (0..h).step_by(TILE) {
                for tx in (0..w).step_by(TILE) {
                    // Edge tiles are padded by repeating the last row/column.
                    for y in 0..TILE {
                        let sy = (ty + y).min(h - 1);
                        for x in 0..TILE {
                            let sx = (tx + x).min(w - 1);
                            let s = (sy * w + sx) * 3;
                            let d = (y * TILE + x) * 3;
                            tile[d..d + 3].copy_from_slice(&samples[s..s + 3]);
                        }
                    }
                    let jpeg = ljpeg::encode(&tile, TILE, TILE, 3);
                    offsets.push(buf.blob(&jpeg));
                    counts.push(jpeg.len() as u32);
                }
            }
            vec![
                Tag(259, Val::Short(vec![7])),
                Tag(322, Val::Long(vec![TILE as u32])),
                Tag(323, Val::Long(vec![TILE as u32])),
                Tag(324, Val::Long(offsets)),
                Tag(325, Val::Long(counts)),
            ]
        }
    }
}

/// Encode a positive (scene-linear Rec.2020, frame white ≈ 1, geometry applied) as DNG bytes.
pub fn encode(img: &Rgb32, opts: &DngOptions) -> Vec<u8> {
    let mut buf = Buf {
        data: b"II*\0\0\0\0\0".to_vec(),
    }; // IFD0 offset patched at the end

    let mut main = write_pixels(&mut buf, img, opts.compression);
    main.extend([
        Tag(254, Val::Long(vec![0])),
        Tag(256, Val::Long(vec![img.width as u32])),
        Tag(257, Val::Long(vec![img.height as u32])),
        Tag(258, Val::Short(vec![16; 3])),
        Tag(262, Val::Short(vec![34892])), // LinearRaw
        Tag(277, Val::Short(vec![3])),
        Tag(284, Val::Short(vec![1])),
        Tag(339, Val::Short(vec![1; 3])),
        Tag(50714, Val::Long(vec![0; 3])),     // BlackLevel
        Tag(50717, Val::Long(vec![65535; 3])), // WhiteLevel
    ]);
    let main_ifd = buf.ifd(main);

    // Previews: rendered with the main profile, so thumbnails match the opened file.
    let preview = img.fit(PREVIEW_LONG_SIDE);
    let preview_px = render::to_srgb8(&preview, &opts.profile);
    let jpeg = render::jpeg(&preview_px, preview.width, preview.height, 90);
    let jpeg_off = buf.blob(&jpeg);
    let preview_ifd = buf.ifd(vec![
        Tag(254, Val::Long(vec![1])),
        Tag(256, Val::Long(vec![preview.width as u32])),
        Tag(257, Val::Long(vec![preview.height as u32])),
        Tag(258, Val::Short(vec![8; 3])),
        Tag(259, Val::Short(vec![7])), // JPEG
        Tag(262, Val::Short(vec![6])), // YCbCr
        Tag(273, Val::Long(vec![jpeg_off])),
        Tag(277, Val::Short(vec![3])),
        Tag(278, Val::Long(vec![preview.height as u32])),
        Tag(279, Val::Long(vec![jpeg.len() as u32])),
        Tag(530, Val::Short(vec![1, 1])), // YCbCrSubSampling: as render::jpeg writes it
        Tag(50970, Val::Long(vec![2])),   // PreviewColorSpace: sRGB
    ]);

    let thumb = img.fit(THUMB_LONG_SIDE);
    let thumb_px = render::to_srgb8(&thumb, &opts.profile);
    let thumb_off = buf.blob(&thumb_px);

    let extra: Vec<u32> = opts
        .extra_profiles
        .iter()
        .map(|p| buf.blob(&profile_blob(p)))
        .collect();

    let mut tags = profile_tags(&opts.profile);
    tags.extend([
        Tag(254, Val::Long(vec![1])), // IFD0 = thumbnail
        Tag(256, Val::Long(vec![thumb.width as u32])),
        Tag(257, Val::Long(vec![thumb.height as u32])),
        Tag(258, Val::Short(vec![8; 3])),
        Tag(259, Val::Short(vec![1])),
        Tag(262, Val::Short(vec![2])), // RGB
        Tag(271, Val::Ascii("Reversal".into())),
        Tag(272, Val::Ascii("Positive".into())),
        Tag(273, Val::Long(vec![thumb_off])),
        Tag(274, Val::Short(vec![1])), // geometry is applied to the pixels
        Tag(277, Val::Short(vec![3])),
        Tag(278, Val::Long(vec![thumb.height as u32])),
        Tag(279, Val::Long(vec![thumb_px.len() as u32])),
        Tag(284, Val::Short(vec![1])),
        Tag(305, Val::Ascii(opts.software.into())),
        Tag(330, Val::Long(vec![main_ifd, preview_ifd])), // SubIFDs
        Tag(50706, Val::Byte(vec![1, 4, 0, 0])),          // DNGVersion
        Tag(50707, Val::Byte(vec![1, 3, 0, 0])),          // DNGBackwardVersion
        Tag(50728, Val::Rational(vec![1.0; 3])),          // AsShotNeutral
        Tag(50730, Val::SRational(vec![0.0])),            // BaselineExposure
    ]);
    if !extra.is_empty() {
        tags.push(Tag(50933, Val::Long(extra))); // ExtraCameraProfiles
    }
    if let Some(xmp) = opts.xmp {
        tags.push(Tag(700, Val::Byte(xmp.as_bytes().to_vec())));
    }
    let ifd0 = buf.ifd(tags);
    buf.data[4..8].copy_from_slice(&ifd0.to_le_bytes());
    buf.data
}

pub fn write(path: &Path, img: &Rgb32, opts: &DngOptions) -> io::Result<()> {
    crate::write_atomic(path, &encode(img, opts))
}

/// Whether the file at `path` is a positive this writer produced: a little-endian TIFF whose
/// first IFD names [`UNIQUE_CAMERA_MODEL`]. A camera's own DNG, or any other file, isn't.
/// Only a file that can't be opened is an error; one that can't be read as a TIFF is `false`.
pub fn is_reversal_positive(path: &Path) -> io::Result<bool> {
    let mut file = std::fs::File::open(path)?;
    let mut header = [0u8; 8];
    if file.read_exact(&mut header).is_err() || header[..4] != *b"II*\0" {
        return Ok(false);
    }
    let ifd = u32::from_le_bytes(header[4..8].try_into().expect("4 bytes"));
    let mut count = [0u8; 2];
    if file.seek(SeekFrom::Start(ifd as u64)).is_err() || file.read_exact(&mut count).is_err() {
        return Ok(false);
    }
    let mut entries = vec![0u8; u16::from_le_bytes(count) as usize * 12];
    if file.read_exact(&mut entries).is_err() {
        return Ok(false);
    }
    let wanted = [UNIQUE_CAMERA_MODEL.as_bytes(), b"\0"].concat();
    for entry in entries.as_chunks::<12>().0 {
        let tag = u16::from_le_bytes([entry[0], entry[1]]);
        let kind = u16::from_le_bytes([entry[2], entry[3]]);
        let len = u32::from_le_bytes(entry[4..8].try_into().expect("4 bytes")) as usize;
        if tag != 50708 || kind != 2 || len != wanted.len() {
            continue;
        }
        // An ASCII value this long sits where the entry's last four bytes point.
        let offset = u32::from_le_bytes(entry[8..12].try_into().expect("4 bytes"));
        let mut value = vec![0u8; len];
        if file.seek(SeekFrom::Start(offset as u64)).is_err()
            || file.read_exact(&mut value).is_err()
        {
            return Ok(false);
        }
        return Ok(value == wanted);
    }
    Ok(false)
}

/// Share of pixels in a positive above what the DNG can store.
pub fn clipped_share(positive: &Rgb32) -> f64 {
    let clipped = positive
        .pixels()
        .filter(|p| p.iter().any(|&v| v >= STORED_MAX))
        .count();
    clipped as f64 / (positive.width * positive.height).max(1) as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::invert::Film;

    #[test]
    fn profile_tags_carry_only_what_the_look_needs() {
        let tone = render::Tone::default();
        for film in [Film::Color, Film::Bw] {
            for look in render::Look::ALL {
                let tags = profile_tags(&render::profile(look, tone, film));
                let has = |id: u16| tags.iter().any(|t| t.0 == id);
                assert!(has(50940), "{look:?} has a tone curve");
                assert!(!has(51109), "{look:?} needs no exposure offset");
                // A look table for Hard's saturation, and to hold black and white gray.
                let table = film == Film::Bw || look == render::Look::Hard;
                assert_eq!(has(50982), table, "{film:?} {look:?}");
            }
        }
    }

    #[test]
    fn every_profile_turns_off_black_subtraction() {
        let tone = render::Tone::default();
        for look in render::Look::ALL {
            let tags = profile_tags(&render::profile(look, tone, Film::Color));
            let tag = tags
                .iter()
                .find(|t| t.0 == 51110)
                .expect("DefaultBlackRender");
            assert!(matches!(&tag.1, Val::Long(v) if v == &[1]), "{look:?}");
        }
    }

    #[test]
    fn a_written_file_turns_off_black_subtraction_in_its_main_and_extra_profiles() {
        let tone = render::Tone::default();
        let bytes = encode(
            &Rgb32::new(4, 4),
            &DngOptions {
                compression: Compression::None,
                profile: render::profile(render::Look::Neutral, tone, Film::Color),
                extra_profiles: vec![render::profile(render::Look::Flat, tone, Film::Color)],
                xmp: None,
                software: "t",
            },
        );
        let entry = |ifd: usize, tag: u16| {
            (0..u16_at(&bytes, ifd) as usize)
                .map(|i| ifd + 2 + 12 * i)
                .find(|&e| u16_at(&bytes, e) == tag)
        };
        let ifd0 = u32_at(&bytes, 4) as usize;
        let main = entry(ifd0, 51110).expect("in the main profile");
        assert_eq!(u32_at(&bytes, main + 8), 1);
        let blob = u32_at(&bytes, entry(ifd0, 50933).unwrap() + 8) as usize;
        let extra = entry(blob + u32_at(&bytes, blob + 4) as usize, 51110);
        assert_eq!(u32_at(&bytes, extra.expect("in the extra profile") + 8), 1);
    }

    #[test]
    fn written_baseline_is_zero_for_every_look() {
        let img = Rgb32 {
            width: 4,
            height: 4,
            data: vec![0.5; 48],
        };
        for look in render::Look::ALL {
            let bytes = encode(
                &img,
                &DngOptions {
                    compression: Compression::None,
                    profile: render::profile(look, render::Tone::default(), Film::Color),
                    extra_profiles: vec![],
                    xmp: None,
                    software: "t",
                },
            );
            let ifd = u32_at(&bytes, 4) as usize;
            let entry = (0..u16_at(&bytes, ifd) as usize)
                .map(|i| ifd + 2 + 12 * i)
                .find(|&e| u16_at(&bytes, e) == 50730)
                .unwrap();
            let value = u32_at(&bytes, entry + 8) as usize;
            assert_eq!(u32_at(&bytes, value), 0, "{look:?}");
        }
    }

    fn u16_at(b: &[u8], o: usize) -> u16 {
        u16::from_le_bytes([b[o], b[o + 1]])
    }
    fn u32_at(b: &[u8], o: usize) -> u32 {
        u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
    }

    #[test]
    fn writes_valid_tiff_structure() {
        let mut img = Rgb32::new(300, 200);
        img.data
            .iter_mut()
            .enumerate()
            .for_each(|(i, v)| *v = (i % 997) as f32 / 997.0);
        let opts = DngOptions {
            compression: Compression::LosslessJpeg,
            profile: render::profile(render::Look::Neutral, render::Tone::default(), Film::Color),
            extra_profiles: vec![render::profile(
                render::Look::Flat,
                render::Tone::default(),
                Film::Color,
            )],
            xmp: Some("<x/>"),
            software: "test",
        };
        let b = encode(&img, &opts);
        assert_eq!(&b[0..4], b"II*\0");
        let ifd0 = u32_at(&b, 4) as usize;
        let n = u16_at(&b, ifd0) as usize;
        let tags: Vec<u16> = (0..n).map(|i| u16_at(&b, ifd0 + 2 + 12 * i)).collect();
        assert!(tags.windows(2).all(|w| w[0] < w[1]), "tags sorted");
        for t in [330u16, 50706, 50708, 50721, 50730, 50933, 700] {
            assert!(tags.contains(&t), "missing tag {t}");
        }
        // ExtraCameraProfiles points at an "IIRC" blob.
        let e = (0..n)
            .find(|&i| u16_at(&b, ifd0 + 2 + 12 * i) == 50933)
            .unwrap();
        let off = u32_at(&b, ifd0 + 2 + 12 * e + 8) as usize;
        assert_eq!(&b[off..off + 4], b"IIRC");
    }

    #[test]
    fn preview_ifd_declares_no_chroma_subsampling() {
        let opts = DngOptions {
            compression: Compression::None,
            profile: render::profile(render::Look::Neutral, render::Tone::default(), Film::Color),
            extra_profiles: vec![],
            xmp: None,
            software: "test",
        };
        let b = encode(&Rgb32::new(64, 48), &opts);
        let entry = |ifd: usize, tag: u16| {
            (0..u16_at(&b, ifd) as usize)
                .map(|i| ifd + 2 + 12 * i)
                .find(|&e| u16_at(&b, e) == tag)
                .unwrap()
        };
        let subifds = u32_at(&b, entry(u32_at(&b, 4) as usize, 330) + 8) as usize;
        let preview = u32_at(&b, subifds + 4) as usize;
        let e = entry(preview, 530);
        assert_eq!((u16_at(&b, e + 8), u16_at(&b, e + 10)), (1, 1));
    }

    #[test]
    fn clipped_share_counts_pixels_above_stored_max() {
        let mut img = Rgb32::new(2, 2);
        img.data[0] = STORED_MAX;
        assert_eq!(clipped_share(&img), 0.25);
    }

    #[test]
    fn only_images_narrower_than_a_tile_skip_lossless_jpeg() {
        let tiff_compression = |width: usize| {
            let img = Rgb32 {
                width,
                height: 2,
                data: vec![0.5; width * 2 * 3],
            };
            let mut buf = Buf { data: Vec::new() };
            let tags = write_pixels(&mut buf, &img, Compression::LosslessJpeg);
            match tags.iter().find(|t| t.0 == 259).map(|t| &t.1) {
                Some(Val::Short(v)) => v[0],
                _ => panic!("no compression tag"),
            }
        };
        assert_eq!(tiff_compression(TILE - 1), 1);
        assert_eq!(tiff_compression(TILE), 7);
        assert_eq!(tiff_compression(TILE + 1), 7);
    }

    #[test]
    fn rationals_reduce() {
        assert_eq!(rational(1.0), (1, 1));
        assert_eq!(rational(2.0), (2, 1));
        assert_eq!(rational(-0.5), (-1, 2));
    }

    #[test]
    fn only_our_own_positives_are_recognized() {
        let dir = std::env::temp_dir().join(format!("reversal-is-ours-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let check = |name: &str, bytes: &[u8]| {
            let path = dir.join(name);
            std::fs::write(&path, bytes).unwrap();
            is_reversal_positive(&path).unwrap()
        };
        let opts = DngOptions {
            compression: Compression::None,
            profile: render::profile(render::Look::Neutral, render::Tone::default(), Film::Color),
            extra_profiles: vec![],
            xmp: None,
            software: "t",
        };
        assert!(check("ours.dng", &encode(&Rgb32::new(8, 8), &opts)));
        // A TIFF from another camera, with the same tag and another name of the same length.
        let mut other = Buf {
            data: b"II*\0\0\0\0\0".to_vec(),
        };
        let name = "X".repeat(UNIQUE_CAMERA_MODEL.len());
        let ifd = other.ifd(vec![Tag(50708, Val::Ascii(name))]);
        other.data[4..8].copy_from_slice(&ifd.to_le_bytes());
        assert!(!check("other.dng", &other.data));
        assert!(!check("raw.nef", b"not a tiff at all"));
        assert!(!check("short.dng", b"II*\0\xff\xff\xff\xff"));
        assert!(!check("empty.dng", b""));
        assert!(is_reversal_positive(&dir.join("missing.dng")).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
