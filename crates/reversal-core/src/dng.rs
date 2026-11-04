//! Minimal writer for positive LinearRaw DNGs.
//!
//! Layout: IFD0 = 8-bit RGB thumbnail + DNG-level tags (colour, profiles, XMP);
//! SubIFD 0 = the 16-bit LinearRaw image; SubIFD 1 = an sRGB JPEG preview.
//! Pixels are stored `STORE_EV` stops under with `BaselineExposure` compensating,
//! so highlights above frame white survive as recoverable headroom.

use std::io;
use std::path::Path;

use crate::color::{self, ILLUMINANT_D65};
use crate::image::Rgb32;
use crate::ljpeg;
use crate::render;

pub const UNIQUE_CAMERA_MODEL: &str = "Reversal Positive";
/// Frame white is stored at 2^-STORE_EV and lifted back by BaselineExposure.
pub const STORE_EV: f32 = 2.0;
const STORE_SCALE: f32 = 0.2; // ≈ 2^-2.32: frame white opens at 0.8 after +2 EV
/// Display value of frame white once a reader applies BaselineExposure.
pub const DISPLAY_LIFT: f32 = STORE_SCALE * 4.0; // 4 = 2^STORE_EV
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

/// A DNG camera profile: name plus optional tone curve (linear (x, y) pairs).
#[derive(Clone, Debug)]
pub struct Profile {
    pub name: String,
    pub tone_curve: Option<Vec<(f32, f32)>>,
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
    ];
    if let Some(curve) = &p.tone_curve {
        tags.push(Tag(
            50940,
            Val::Float(curve.iter().flat_map(|&(x, y)| [x, y]).collect()),
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

    // Previews: same tone curve as the main profile, so thumbnails match the opened file.
    let curve = opts
        .profile
        .tone_curve
        .clone()
        .unwrap_or_else(|| render::tone_curve(0.0, 64));
    let preview = img.fit(PREVIEW_LONG_SIDE);
    let mut lifted = preview.clone();
    lifted.data.iter_mut().for_each(|v| *v *= DISPLAY_LIFT);
    let preview_px = render::to_srgb8(&lifted, &curve);
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

    let thumb = lifted.fit(THUMB_LONG_SIDE);
    let thumb_px = render::to_srgb8(&thumb, &curve);
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
        Tag(50730, Val::SRational(vec![STORE_EV as f64])), // BaselineExposure
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
            profile: Profile {
                name: "Reversal Test".into(),
                tone_curve: Some(render::tone_curve(0.0, 16)),
            },
            extra_profiles: vec![Profile {
                name: "Reversal Neutral".into(),
                tone_curve: None,
            }],
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
            profile: Profile {
                name: "Reversal Test".into(),
                tone_curve: None,
            },
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
    fn display_lift_matches_baseline_exposure() {
        assert_eq!(DISPLAY_LIFT, STORE_SCALE * 2f32.powf(STORE_EV));
    }

    #[test]
    fn clipped_share_counts_pixels_above_stored_max() {
        let mut img = Rgb32::new(2, 2);
        img.data[0] = STORED_MAX;
        assert_eq!(clipped_share(&img), 0.25);
    }

    #[test]
    fn rationals_reduce() {
        assert_eq!(rational(1.0), (1, 1));
        assert_eq!(rational(2.0), (2, 1));
        assert_eq!(rational(-0.5), (-1, 2));
    }
}
