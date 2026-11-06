//! Runs real files through LibRaw, so a change in its behavior or a mistake in our bindings
//! fails here. The raw file is written by the test: a small Bayer DNG with known pixels.
#![cfg(feature = "libraw")]

use std::path::{Path, PathBuf};

use reversal_core::decode::decode;
use reversal_core::dng::{self, Compression};
use reversal_core::image::{NRect, Rgb32};
use reversal_core::invert::{self, Adjust, Hints, Settings};
use reversal_core::write_positive;

/// Wider than the positive's 256-pixel tiles, so a flat patch crosses the seam at x = 256.
const WIDTH: usize = 320;
const HEIGHT: usize = 128;
const COLUMNS: usize = 4;
const ROWS: usize = 2;
/// Levels of a 14-bit sensor, written into the file and undone by LibRaw.
const BLACK_LEVEL: u16 = 256;
const WHITE_LEVEL: u16 = 16383;
/// Orange film base as linear transmittance. Red stays under the white level but above 75% of
/// it, where LibRaw's default would rescale a frame to its own brightest pixel.
const FILM_BASE: [f32; 3] = [0.85, 0.5, 0.25];
/// Green density above base of each patch, left to right, top row first: a gray scale. All
/// differ, so a flipped or rotated image can't match. The first patch is unexposed film.
const PATCH_DENSITY: [f32; COLUMNS * ROWS] = [0.0, 0.2, 0.4, 0.6, 0.8, 1.0, 1.2, 1.4];
/// Red and blue density of a neutral against green, `bend · g² + slope · g`: the dye layers
/// differ in contrast, as on real film.
const NEUTRAL_RED: [f32; 2] = [-0.15, 0.9];
const NEUTRAL_BLUE: [f32; 2] = [0.25, 0.85];
/// Pixels of each patch edge left out of checks: demosaicing blends across patch borders.
const PATCH_MARGIN: usize = 6;
/// Largest difference from the expected value, on the [0, 1] scale. Well under what any
/// white balance, gamma or rescaling step in decoding would change.
const TOLERANCE: f32 = 0.002;
/// Largest difference when reading back a positive, which isn't demosaiced: a few 16-bit steps.
const READ_BACK_TOLERANCE: f32 = 4.0 / 65535.0;
/// Largest difference of red or blue from green on a gray patch of the positive, as a share.
const MAX_CAST: f32 = 0.05;

/// Linear transmittance of a patch: T = T_base · 10^-D.
fn patch_rgb(patch: usize) -> [f32; 3] {
    let g = PATCH_DENSITY[patch];
    let density = [
        NEUTRAL_RED[0] * g * g + NEUTRAL_RED[1] * g,
        g,
        NEUTRAL_BLUE[0] * g * g + NEUTRAL_BLUE[1] * g,
    ];
    [0, 1, 2].map(|c| FILM_BASE[c] * 10f32.powf(-density[c]))
}

fn patch_at(x: usize, y: usize) -> usize {
    y * ROWS / HEIGHT * COLUMNS + x * COLUMNS / WIDTH
}

/// Pixel coordinates of a patch without its margin.
fn patch_pixels(patch: usize) -> impl Iterator<Item = (usize, usize)> {
    let (w, h) = (WIDTH / COLUMNS, HEIGHT / ROWS);
    let (left, top) = (patch % COLUMNS * w, patch / COLUMNS * h);
    (top + PATCH_MARGIN..top + h - PATCH_MARGIN)
        .flat_map(move |y| (left + PATCH_MARGIN..left + w - PATCH_MARGIN).map(move |x| (x, y)))
}

fn patch_mean(img: &Rgb32, patch: usize) -> [f32; 3] {
    let mut sum = [0.0; 3];
    let mut count = 0.0;
    for (x, y) in patch_pixels(patch) {
        let px = img.get(x, y);
        (0..3).for_each(|c| sum[c] += px[c]);
        count += 1.0;
    }
    sum.map(|s| s / count)
}

/// The gray scale: linear transmittance at each pixel.
fn gray_scale(x: usize, y: usize) -> [f32; 3] {
    patch_rgb(patch_at(x, y))
}

/// RGGB mosaic of a scene, as the sensor's 16-bit little-endian samples. Scene value 1 is the
/// white level; a sensor can read somewhat above it.
fn mosaic_bytes(scene: impl Fn(usize, usize) -> [f32; 3]) -> Vec<u8> {
    let range = (WHITE_LEVEL - BLACK_LEVEL) as f32;
    let mut bytes = Vec::with_capacity(WIDTH * HEIGHT * 2);
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let channel = [[0, 1], [1, 2]][y % 2][x % 2];
            let level = scene(x, y)[channel] * range + BLACK_LEVEL as f32;
            bytes.extend_from_slice(&(level.round() as u16).to_le_bytes());
        }
    }
    bytes
}

/// One TIFF tag: id, type, count and little-endian value bytes.
fn tag(id: u16, kind: u16, count: u32, value: Vec<u8>) -> (u16, u16, u32, Vec<u8>) {
    (id, kind, count, value)
}

fn shorts(id: u16, values: &[u16]) -> (u16, u16, u32, Vec<u8>) {
    let bytes = values.iter().flat_map(|v| v.to_le_bytes()).collect();
    tag(id, 3, values.len() as u32, bytes)
}

fn long(id: u16, value: u32) -> (u16, u16, u32, Vec<u8>) {
    tag(id, 4, 1, value.to_le_bytes().to_vec())
}

fn ascii(id: u16, text: &str) -> (u16, u16, u32, Vec<u8>) {
    let bytes = [text.as_bytes(), &[0]].concat();
    tag(id, 2, bytes.len() as u32, bytes)
}

/// Numerators over 1000, as RATIONAL (`kind` 5) or SRATIONAL (`kind` 10).
fn thousandths(id: u16, kind: u16, values: &[i32]) -> (u16, u16, u32, Vec<u8>) {
    let bytes = values
        .iter()
        .flat_map(|v| [v.to_le_bytes(), 1000i32.to_le_bytes()].concat())
        .collect();
    tag(id, kind, values.len() as u32, bytes)
}

/// A minimal Bayer DNG. It carries a color matrix and an as-shot white balance like a camera's,
/// so a decode that applied either would come out with different values.
fn raw_dng(scene: impl Fn(usize, usize) -> [f32; 3]) -> Vec<u8> {
    const HEADER_LEN: usize = 8;
    let pixels = mosaic_bytes(scene);
    let mut tags = vec![
        long(254, 0), // NewSubfileType: main image
        shorts(256, &[WIDTH as u16]),
        shorts(257, &[HEIGHT as u16]),
        shorts(258, &[16]),    // BitsPerSample
        shorts(259, &[1]),     // Compression: none
        shorts(262, &[32803]), // PhotometricInterpretation: CFA
        ascii(271, "Reversal"),
        ascii(272, "Test Sensor"),
        long(273, HEADER_LEN as u32), // StripOffsets: pixels follow the header
        shorts(274, &[1]),            // Orientation: as stored
        shorts(277, &[1]),            // SamplesPerPixel
        long(278, HEIGHT as u32),     // RowsPerStrip
        long(279, pixels.len() as u32),
        shorts(284, &[1]),                    // PlanarConfiguration
        shorts(33421, &[2, 2]),               // CFARepeatPatternDim
        tag(33422, 1, 4, vec![0, 1, 1, 2]),   // CFAPattern: RGGB
        tag(50706, 1, 4, vec![1, 4, 0, 0]),   // DNGVersion
        tag(50707, 1, 4, vec![1, 1, 0, 0]),   // DNGBackwardVersion
        ascii(50708, "Reversal Test Sensor"), // UniqueCameraModel
        shorts(50714, &[BLACK_LEVEL]),        // BlackLevel
        shorts(50717, &[WHITE_LEVEL]),        // WhiteLevel
        thousandths(
            50721, // ColorMatrix1: XYZ to camera
            10,
            &[900, -300, -100, -400, 1200, 200, -100, 200, 600],
        ),
        thousandths(50728, 5, &[500, 1000, 700]), // AsShotNeutral
        shorts(50778, &[21]),                     // CalibrationIlluminant1: D65
    ];
    tags.sort_by_key(|t| t.0);

    let ifd_offset = HEADER_LEN + pixels.len();
    let mut values_offset = ifd_offset + 2 + 12 * tags.len() + 4;
    let mut file = b"II*\0".to_vec();
    file.extend_from_slice(&(ifd_offset as u32).to_le_bytes());
    file.extend_from_slice(&pixels);
    file.extend_from_slice(&(tags.len() as u16).to_le_bytes());
    let mut values = Vec::new();
    for (id, kind, count, value) in &tags {
        file.extend_from_slice(&id.to_le_bytes());
        file.extend_from_slice(&kind.to_le_bytes());
        file.extend_from_slice(&count.to_le_bytes());
        if value.len() <= 4 {
            let mut inline = value.clone();
            inline.resize(4, 0);
            file.extend_from_slice(&inline);
        } else {
            file.extend_from_slice(&(values_offset as u32).to_le_bytes());
            values.extend_from_slice(value);
            values_offset += value.len();
            if value.len() % 2 == 1 {
                values.push(0);
                values_offset += 1;
            }
        }
    }
    file.extend_from_slice(&0u32.to_le_bytes()); // no next IFD
    file.extend_from_slice(&values);
    file
}

/// A folder of its own for one test, removed when the test ends.
struct TempDir(PathBuf);

impl TempDir {
    fn new(test: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("reversal-{test}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }

    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn assert_close(actual: [f32; 3], expected: [f32; 3], tolerance: f32, what: &str) {
    for c in 0..3 {
        assert!(
            (actual[c] - expected[c]).abs() <= tolerance,
            "{what}, channel {c}: {actual:?}, expected {expected:?}"
        );
    }
}

fn decode_scene(dir: &TempDir) -> Rgb32 {
    decode(&dir.write("scene.dng", &raw_dng(gray_scale))).unwrap()
}

#[test]
fn decode_returns_linear_camera_rgb_without_white_balance() {
    let dir = TempDir::new("decode-values");
    let scan = decode_scene(&dir);
    assert_eq!((scan.width, scan.height), (WIDTH, HEIGHT));
    assert_eq!(scan.data.len(), WIDTH * HEIGHT * 3);
    for patch in 0..PATCH_DENSITY.len() {
        for (x, y) in patch_pixels(patch) {
            assert_close(
                scan.get(x, y),
                patch_rgb(patch),
                TOLERANCE,
                &format!("patch {patch} at ({x}, {y})"),
            );
        }
    }
}

#[test]
fn decode_is_deterministic() {
    let dir = TempDir::new("decode-repeat");
    assert_eq!(decode_scene(&dir).data, decode_scene(&dir).data);
}

#[test]
fn decoded_negative_converts_to_a_positive_dng_that_reads_back() {
    let dir = TempDir::new("pipeline");
    let scan = decode_scene(&dir);
    let unexposed = NRect {
        x0: 0.0,
        y0: 0.0,
        x1: 0.25,
        y1: 0.5,
    };
    let hints = Hints {
        base: Some(unexposed),
        ..Hints::default()
    };
    let estimate = invert::estimate(&scan, &hints);
    assert_close(estimate.base_rgb, FILM_BASE, 0.02, "film base");

    let settings = Settings::new(estimate, Adjust::default());
    let positive = invert::convert(&scan, &settings);
    assert_eq!((positive.width, positive.height), (WIDTH, HEIGHT));

    // Removing the mask and fitting the gray curves keeps the gray scale neutral throughout.
    for patch in 0..PATCH_DENSITY.len() {
        let [r, g, b] = patch_mean(&positive, patch);
        assert!(
            (r - g).abs() < MAX_CAST * g && (b - g).abs() < MAX_CAST * g,
            "patch {patch} isn't neutral: {:?}",
            [r, g, b]
        );
    }
    // More density in the negative means a brighter positive.
    for patch in 1..PATCH_DENSITY.len() {
        let (thin, dense) = (
            patch_mean(&positive, patch - 1),
            patch_mean(&positive, patch),
        );
        for c in 0..3 {
            assert!(
                dense[c] > thin[c],
                "patch {patch}, channel {c}: {dense:?} isn't brighter than {thin:?}"
            );
        }
    }

    // Both ways of storing the positive decode to the same values through LibRaw, pixel for
    // pixel, so damage at a tile seam shows.
    for (name, compression) in [
        ("positive.dng", Compression::None),
        ("positive-ljpeg.dng", Compression::LosslessJpeg),
    ] {
        assert_reads_back(&dir, name, &scan, &settings, compression, &positive);
    }

    // Narrower than a tile, as a small crop can be.
    let mut cropped = settings.clone();
    cropped.adjust.crop = Some(NRect {
        x0: 0.0,
        y0: 0.0,
        x1: 0.6,
        y1: 1.0,
    });
    let narrow = invert::convert(&scan, &cropped);
    assert!(narrow.width < 256);
    assert_reads_back(
        &dir,
        "narrow-ljpeg.dng",
        &scan,
        &cropped,
        Compression::LosslessJpeg,
        &narrow,
    );
}

/// Writes the positive and checks that LibRaw returns its stored values at every pixel.
fn assert_reads_back(
    dir: &TempDir,
    name: &str,
    scan: &Rgb32,
    settings: &Settings,
    compression: Compression,
    positive: &Rgb32,
) {
    let out = dir.0.join(name);
    write_positive(&out, "scene.dng", scan, settings, compression, "test").unwrap();
    let back = decode(&out).unwrap();
    assert_eq!(
        (back.width, back.height),
        (positive.width, positive.height),
        "{name}"
    );
    for y in 0..back.height {
        for x in 0..back.width {
            let expected = positive.get(x, y).map(|v| (v / dng::STORED_MAX).min(1.0));
            assert_close(
                back.get(x, y),
                expected,
                READ_BACK_TOLERANCE,
                &format!("{name} at ({x}, {y})"),
            );
        }
    }
}

#[test]
fn a_missing_file_is_an_error() {
    let dir = TempDir::new("decode-missing");
    let err = decode(&dir.0.join("nothing.dng")).unwrap_err();
    assert!(err.to_string().contains("open"), "{err}");
}

#[test]
fn a_file_that_is_not_a_raw_is_an_error() {
    let dir = TempDir::new("decode-garbage");
    let path = dir.write("notes.dng", &[b'x'; 4096]);
    let err = decode(&path).unwrap_err();
    assert!(err.to_string().contains("open"), "{err}");
}

#[test]
fn a_path_with_a_nul_byte_is_an_error() {
    let err = decode(Path::new("bad\0name.dng")).unwrap_err();
    assert!(err.to_string().contains("NUL"), "{err}");
}
