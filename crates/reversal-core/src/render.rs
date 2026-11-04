//! Display rendering: linear Rec.2020 → tone curve in ProPhoto → sRGB. Used for UI previews
//! and the previews embedded in the DNG. Raw editors render the DNG themselves; this only
//! approximates them with the same tone curve the DNG's main profile carries.

use serde::{Deserialize, Serialize};

use crate::color;
use crate::dng;
use crate::image::Rgb32;
use crate::invert;

/// Sample count of the tone curves written to the DNG and used for previews.
const CURVE_POINTS: usize = 129;
/// Reach of the shoulder: frame white opens at 0.8 and the data ends at this display value.
const STORED_REACH: f32 = 4.0; // = 2^STORE_EV
/// How far the blacks slider moves the black point, in the gamma-2.2 domain.
const BLACKS_RANGE: f32 = 0.12;
/// Strength of the shadows and highlights bumps; below 1 the curve can't fold over.
const BUMP_RANGE: f32 = 0.9;
/// Saturation the Vivid profile adds, in its look table.
const VIVID_SATURATION: f32 = 1.15;

/// Rendering styles carried as DNG profiles; a raw editor can switch between them.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Look {
    /// The standard rendering: an S-curve, with highlights clipping as on a print.
    #[default]
    Neutral,
    /// Low contrast, with the whole stored range rolled off below white.
    Flat,
    /// More contrast and saturation.
    Vivid,
}

impl Look {
    pub const ALL: [Look; 3] = [Look::Neutral, Look::Flat, Look::Vivid];

    pub fn profile_name(self) -> &'static str {
        match self {
            Look::Neutral => "Reversal Neutral",
            Look::Flat => "Reversal Flat",
            Look::Vivid => "Reversal Vivid",
        }
    }

    /// Slope parameter of the S-curve.
    fn slope(self) -> f32 {
        match self {
            Look::Neutral => 1.5,
            Look::Flat => 0.4,
            Look::Vivid => 2.2,
        }
    }

    /// Where the whites slider starts from.
    fn whites(self) -> f32 {
        match self {
            Look::Flat => -100.0,
            _ => 0.0,
        }
    }

    fn saturation(self) -> f32 {
        match self {
            Look::Vivid => VIVID_SATURATION,
            _ => 1.0,
        }
    }
}

/// Tone sliders, −100…+100 with 0 = the look as it comes. They only shape the profile's
/// curve: the stored image doesn't change, so a raw editor can still reach all of it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Tone {
    pub contrast: f32,
    /// − crushes the black point, + lifts it.
    pub blacks: f32,
    pub shadows: f32,
    pub highlights: f32,
    /// − rolls off highlights to bring back what would clip, + clips sooner.
    pub whites: f32,
}

fn unit(slider: f32) -> f32 {
    slider.clamp(-100.0, 100.0) / 100.0
}

/// Whites control, −1…+1, of a look and tone: below 0 the curve needs the stored range.
fn white_amount(look: Look, tone: Tone) -> f32 {
    unit(look.whites() + tone.whites)
}

/// Display value of linear `l` before the S-curve. At −1 an extended Reinhard shoulder takes
/// `STORED_REACH` to white; at 0 nothing happens; at +1 the top is pushed into clipping.
fn shoulder(l: f32, amount: f32) -> f32 {
    if amount < 0.0 {
        let reach2 = STORED_REACH * STORED_REACH;
        let rolled = l * (1.0 + l / reach2) / (1.0 + l);
        l + -amount * (rolled - l)
    } else {
        l * (1.0 + amount * l)
    }
}

/// Exposure the profile adds to `BaselineExposure`: the curve then sees the whole stored
/// range instead of only what reaches display white.
fn exposure_offset(look: Look, tone: Tone) -> f32 {
    if white_amount(look, tone) < 0.0 {
        -dng::STORE_EV
    } else {
        0.0
    }
}

/// Tone curve as (x, y) pairs on linear values in [0, 1], for a profile with
/// `exposure_offset`: shoulder, black point, S-curve in a gamma-2.2 domain, then shadows
/// and highlights bumps. x is spaced quadratically for the shadows.
pub fn tone_curve(look: Look, tone: Tone, points: usize) -> Vec<(f32, f32)> {
    let amount = white_amount(look, tone);
    let domain = 2f32.powf(-exposure_offset(look, tone));
    // Contrast: ×/÷ 2.6 at the ends.
    let slope = look.slope() * 2.6f32.powf(unit(tone.contrast));
    let norm = (slope * 0.5).tanh();
    let black = -unit(tone.blacks) * BLACKS_RANGE;
    let (shadows, highlights) = (
        unit(tone.shadows) * BUMP_RANGE,
        unit(tone.highlights) * BUMP_RANGE,
    );
    (0..points)
        .map(|i| {
            let t = i as f32 / (points - 1) as f32;
            let x = t * t;
            let p = shoulder(x * domain, amount).clamp(0.0, 1.0).powf(1.0 / 2.2);
            let p = ((p - black) / (1.0 - black)).clamp(0.0, 1.0);
            let q = 0.5 * (1.0 + (slope * (p - 0.5)).tanh() / norm);
            let q = q.clamp(0.0, 1.0);
            let q = q + shadows * q * (1.0 - q) * (1.0 - q) + highlights * q * q * (1.0 - q);
            (x, q.clamp(0.0, 1.0).powf(2.2))
        })
        .collect()
}

/// The DNG profile of a look under the current tone sliders.
pub fn profile(look: Look, tone: Tone) -> dng::Profile {
    dng::Profile {
        name: look.profile_name().into(),
        tone_curve: tone_curve(look, tone, CURVE_POINTS),
        exposure_offset: exposure_offset(look, tone),
        saturation: look.saturation(),
    }
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

/// Render a positive (frame white = 1) to 8-bit sRGB as a raw editor would open it under
/// `profile`, interleaved. The curve applies in ProPhoto, the space raw editors use.
pub fn to_srgb8(img: &Rgb32, profile: &dng::Profile) -> Vec<u8> {
    let to_prophoto = color::rec2020_to_prophoto();
    let to_srgb = color::prophoto_to_srgb();
    let lift = dng::DISPLAY_LIFT * 2f32.powf(profile.exposure_offset);
    // Lookup for the curve, fine enough for curves steep near black.
    const N: usize = 16384;
    let lut: Vec<f32> = (0..N)
        .map(|i| interp(&profile.tone_curve, i as f32 / (N - 1) as f32))
        .collect();
    img.pixels()
        .flat_map(|p| {
            let p = invert::saturate(p.map(|v| v * lift), profile.saturation);
            let p = color::apply(&to_prophoto, p);
            let toned = p.map(|v| lut[(v.clamp(0.0, 1.0) * (N - 1) as f32) as usize]);
            let s = color::apply(&to_srgb, toned);
            s.map(|v| (srgb_encode(v.clamp(0.0, 1.0)) * 255.0 + 0.5) as u8)
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

/// JPEG of a positive (image white = 1) as the DNG opens under `profile`.
pub fn preview_jpeg(positive: &Rgb32, profile: &dng::Profile, quality: u8) -> Vec<u8> {
    let px = to_srgb8(positive, profile);
    jpeg(&px, positive.width, positive.height, quality)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENDS: [f32; 3] = [-100.0, 0.0, 100.0];

    /// Display value of linear `l` as a look's profile renders it.
    fn display(look: Look, tone: Tone, l: f32) -> f32 {
        let curve = tone_curve(look, tone, 513);
        interp(&curve, l * 2f32.powf(exposure_offset(look, tone)))
    }

    #[test]
    fn every_slider_combination_keeps_the_curve_monotonic() {
        for look in Look::ALL {
            for contrast in ENDS {
                for blacks in ENDS {
                    for shadows in ENDS {
                        for highlights in ENDS {
                            for whites in ENDS {
                                let tone = Tone {
                                    contrast,
                                    blacks,
                                    shadows,
                                    highlights,
                                    whites,
                                };
                                let c = tone_curve(look, tone, CURVE_POINTS);
                                assert!(
                                    c.windows(2).all(|w| w[1].1 >= w[0].1 - 1e-6),
                                    "{look:?} {tone:?}"
                                );
                                assert!(c.iter().all(|p| (0.0..=1.0).contains(&p.1)));
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn looks_without_slider_moves_are_pinned() {
        for look in Look::ALL {
            let c = tone_curve(look, Tone::default(), CURVE_POINTS);
            assert!(c[0].1.abs() < 1e-6, "{look:?}");
            assert!((c[CURVE_POINTS - 1].1 - 1.0).abs() < 1e-5, "{look:?}");
        }
    }

    #[test]
    fn neutral_is_the_plain_s_curve() {
        let c = tone_curve(Look::Neutral, Tone::default(), 65);
        let norm = 0.75f32.tanh();
        for &(x, y) in &c {
            let q = 0.5 * (1.0 + (1.5 * (x.powf(1.0 / 2.2) - 0.5)).tanh() / norm);
            assert!((y - q.clamp(0.0, 1.0).powf(2.2)).abs() < 1e-5);
        }
        assert_eq!(exposure_offset(Look::Neutral, Tone::default()), 0.0);
    }

    #[test]
    fn flat_keeps_the_whole_stored_range_and_neutral_doesnt() {
        let top = STORED_REACH;
        assert!(display(Look::Flat, Tone::default(), 0.9 * top) < 0.995);
        assert!(display(Look::Flat, Tone::default(), top) > 0.99);
        assert!(display(Look::Neutral, Tone::default(), 1.0) > 0.999);
    }

    #[test]
    fn lowering_whites_brings_back_clipped_highlights_and_spares_midtones() {
        let lower = Tone {
            whites: -100.0,
            ..Tone::default()
        };
        assert!(display(Look::Neutral, lower, 2.0) < 0.99);
        let (before, after) = (
            display(Look::Neutral, Tone::default(), 0.18),
            display(Look::Neutral, lower, 0.18),
        );
        assert!((before - after).abs() < 0.04, "{before} {after}");
    }

    #[test]
    fn each_slider_moves_its_own_part_of_the_curve() {
        let n = |tone: Tone, l: f32| display(Look::Neutral, tone, l);
        let d = Tone::default();
        let shadows = Tone {
            shadows: 100.0,
            ..d
        };
        assert!(n(shadows, 0.05) > n(d, 0.05) + 0.01);
        assert!((n(shadows, 0.9) - n(d, 0.9)).abs() < 0.04);
        let highlights = Tone {
            highlights: -100.0,
            ..d
        };
        assert!(n(highlights, 0.6) < n(d, 0.6) - 0.02);
        assert!((n(highlights, 0.02) - n(d, 0.02)).abs() < 0.005);
        let blacks = |blacks| Tone { blacks, ..d };
        assert!(n(blacks(-100.0), 0.01) < n(d, 0.01));
        assert!(n(blacks(100.0), 0.0) > 0.0);
        let whites = Tone { whites: 100.0, ..d };
        assert!(n(whites, 0.7) > n(d, 0.7));
    }

    #[test]
    fn vivid_is_harder_and_more_saturated_than_flat() {
        let d = Tone::default();
        assert!(display(Look::Vivid, d, 0.7) > display(Look::Flat, d, 0.7));
        assert!(display(Look::Vivid, d, 0.02) < display(Look::Flat, d, 0.02));
        assert!(profile(Look::Vivid, d).saturation > profile(Look::Neutral, d).saturation);
    }

    #[test]
    fn previews_follow_the_profile() {
        let img = Rgb32 {
            width: 1,
            height: 1,
            data: vec![0.9, 0.4, 0.2],
        };
        let px = |look| to_srgb8(&img, &profile(look, Tone::default()));
        // Vivid separates the channels further than Flat.
        let spread = |p: Vec<u8>| p[0] as i32 - p[2] as i32;
        assert!(spread(px(Look::Vivid)) > spread(px(Look::Flat)));
    }

    #[test]
    fn previews_keep_neutrals_on_the_curve() {
        let p = profile(Look::Neutral, Tone::default());
        for grey in [0.05, 0.3, 0.8] {
            let img = Rgb32 {
                width: 1,
                height: 1,
                data: vec![grey; 3],
            };
            let out = to_srgb8(&img, &p);
            let want = srgb_encode(interp(&p.tone_curve, grey * dng::DISPLAY_LIFT)) * 255.0;
            assert!(
                out.iter().all(|&v| (v as f32 - want).abs() <= 2.0),
                "{grey}"
            );
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
        let at = |contrast, l| {
            let tone = Tone {
                contrast,
                ..Tone::default()
            };
            display(Look::Neutral, tone, l)
        };
        // above mid-grey the hard curve is brighter, below it darker
        assert!(at(100.0, 0.7) > at(-100.0, 0.7) && at(100.0, 0.02) < at(-100.0, 0.02));
    }
}
