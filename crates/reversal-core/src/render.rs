//! Display rendering: tone curve + Rec.2020 → sRGB. Used for UI previews and the
//! previews embedded in the DNG. Raw editors render the DNG themselves; this only
//! approximates them with the same tone curve the DNG's main profile carries.

use crate::color;
use crate::image::Rgb32;

/// Tone curve for the contrast slider (−100…+100, 0 = normal) as (x, y) pairs on
/// linear values in [0, 1]: an S-curve in a gamma-2.2 domain.
pub fn tone_curve(contrast: f32, points: usize) -> Vec<(f32, f32)> {
    // Slope parameter 1.5 at 0; ×/÷ 2.6 at the ends.
    let contrast = 1.5 * 2.6f32.powf(contrast.clamp(-100.0, 100.0) / 100.0);
    let norm = (contrast * 0.5).tanh();
    (0..points)
        .map(|i| {
            let x = i as f32 / (points - 1) as f32;
            let p = x.powf(1.0 / 2.2);
            let q = 0.5 * (1.0 + (contrast * (p - 0.5)).tanh() / norm);
            (x, q.clamp(0.0, 1.0).powf(2.2))
        })
        .collect()
}

fn interp(curve: &[(f32, f32)], x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    let i = curve
        .partition_point(|&(cx, _)| cx < x)
        .clamp(1, curve.len() - 1);
    let ((x0, y0), (x1, y1)) = (curve[i - 1], curve[i]);
    y0 + (y1 - y0) * (x - x0) / (x1 - x0).max(1e-9)
}

fn srgb_encode(v: f32) -> f32 {
    if v <= 0.003_130_8 {
        12.92 * v
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

/// Render a scene-linear Rec.2020 image (frame white ≈ 1) to 8-bit sRGB, interleaved.
pub fn to_srgb8(img: &Rgb32, curve: &[(f32, f32)]) -> Vec<u8> {
    let m = color::REC2020_TO_SRGB;
    // 4096-entry lookup for curve + transfer function.
    const N: usize = 4096;
    let lut: Vec<u8> = (0..N)
        .map(|i| (srgb_encode(interp(curve, i as f32 / (N - 1) as f32)) * 255.0 + 0.5) as u8)
        .collect();
    img.pixels()
        .flat_map(|p| {
            let s = color::apply(&m, p);
            s.map(|v| lut[((v.clamp(0.0, 1.0)) * (N - 1) as f32) as usize])
        })
        .collect()
}

/// Encode 8-bit sRGB pixels as a baseline JPEG, always without chroma subsampling
/// (the DNG's preview IFD declares this).
pub fn jpeg(pixels: &[u8], width: usize, height: usize, quality: u8) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = jpeg_encoder::Encoder::new(&mut out, quality);
    encoder.set_sampling_factor(jpeg_encoder::SamplingFactor::R_4_4_4);
    encoder
        .encode(
            pixels,
            width as u16,
            height as u16,
            jpeg_encoder::ColorType::Rgb,
        )
        .expect("JPEG encoding of an in-memory buffer");
    out
}

/// JPEG of a positive (image white = 1) as the DNG opens: same lift, same tone curve.
pub fn preview_jpeg(positive: &Rgb32, contrast: f32, quality: u8) -> Vec<u8> {
    let mut lifted = positive.clone();
    lifted
        .data
        .iter_mut()
        .for_each(|v| *v *= crate::dng::DISPLAY_LIFT);
    let px = to_srgb8(&lifted, &tone_curve(contrast, 64));
    jpeg(&px, lifted.width, lifted.height, quality)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tone_curve_is_monotonic_and_pinned() {
        for g in [-100.0, 0.0, 100.0] {
            let c = tone_curve(g, 64);
            assert!(c.windows(2).all(|w| w[1].1 >= w[0].1));
            assert!(c[0].1.abs() < 1e-6 && (c[63].1 - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn jpeg_has_no_chroma_subsampling_at_any_quality() {
        for quality in [50, 88, 90] {
            let j = jpeg(&[128; 16 * 16 * 3], 16, 16, quality);
            let sof = j.windows(2).position(|m| m == [0xFF, 0xC0]).unwrap();
            // Marker, length, precision, height, width, count, then (id, sampling, table).
            assert_eq!(j[sof + 11], 0x11);
        }
    }

    #[test]
    fn more_contrast_steepens_the_curve() {
        let soft = tone_curve(-100.0, 65);
        let hard = tone_curve(100.0, 65);
        // above mid-grey the hard curve is brighter, below it darker
        assert!(hard[56].1 > soft[56].1 && hard[8].1 < soft[8].1);
    }
}
