//! Density-model inversion.
//!
//! Per channel c, with transmittance T relative to the film base:
//!   D  = -log10(T / T_base)                      density above base
//!   E  = equivalent green density of D           per-frame grey curve for red and blue
//!   E' = crosstalk and toe corrections of E      fixed, in density
//!   L  = 10^((E' - anchor) / gamma + offsets)    scene-linear positive
//! A neutral scene patch lands on the grey curve, so its three E are equal and it stays
//! neutral at every density. `anchor` sets exposure from the frame's highlights and
//! average density; `offsets` hold the exposure and colour balance sliders. A small
//! saturation step follows.
//!
//! Geometry: scan → straighten → crop → invert → mirror → quarter turns.
//! Regions (crop, analysis area, base sample) are normalised scan coordinates.

use serde::{Deserialize, Serialize};

use crate::image::{for_each_part, NRect, Rgb32};
use crate::render::{Look, Tone};

/// Film gamma assumed when turning density into scene log exposure; sets scene contrast.
pub const GAMMA_REF: f32 = 0.7;
/// Version of the conversion model. Bump when the inversion, `BASE_SATURATION`, the
/// DNG's stored scale or the tone curve change, since old settings then render differently.
pub const MODEL_VERSION: u32 = 2;
/// Built-in saturation (linear, around Rec.2020 luminance). Matched the chroma of
/// reference scans (22 frames, 6 rolls).
pub const BASE_SATURATION: f32 = 1.25;
/// Slider ±100 ranges.
pub const EXPOSURE_EV_RANGE: f32 = 3.0;
pub const BALANCE_EV_RANGE: f32 = 0.6;
/// Factor the analysis image is downsampled by.
const ANALYSIS_FACTOR: usize = 4;
const REC2020_LUMA: [f32; 3] = [0.2627, 0.6780, 0.0593];
const LOG10_2: f32 = std::f32::consts::LOG10_2;

/// Pixels at or above this are light through holes or gaps, not film.
const CLIPPED: f32 = 0.9;
/// Share of each side of the scan, centred, measured when nobody draws an area: clear of
/// holders and rebates on a framed scan. Tested at 70–95% against reference scans; 95%
/// takes in holders.
pub const DEFAULT_AREA_PERCENT: f32 = 85.0;

/// Starting grey curves (bend, slope) for red and blue: the average across rolls.
/// Each frame's fit is pulled toward these by `GREY_PULL`.
const GREY_PRIOR: [[f32; 2]; 2] = [[-0.2008, 0.9564], [0.1883, 0.8340]];
const GREY_PULL: f32 = 0.003;
/// Pixels within this density of the curve count as grey. The band narrows over the
/// iterations, so a frame dominated by one colour can't capture the fit early on.
const GREY_BANDS: [f32; 5] = [0.15, 0.10, 0.07, 0.05, 0.04];
/// Below this green density the film toe and any error in the base dominate.
const GREY_MIN_DENSITY: f32 = 0.15;
const GREY_BIN_EDGES: [f32; 19] = [
    0.15, 0.2, 0.25, 0.3, 0.35, 0.4, 0.45, 0.5, 0.55, 0.6, 0.7, 0.8, 0.9, 1.0, 1.1, 1.2, 1.3, 1.45,
    1.7,
];
const GREY_MIN_BIN_PIXELS: usize = 40;
/// Bins count equally: each density range is its own evidence for grey, however many
/// pixels of it a scene happens to contain.
const GREY_BIN_WEIGHT: f32 = 0.25;

/// Density crosstalk between the dye layers and the sensor, for a D5100 and one light
/// source: rows give new E_r - E_g, E_b - E_g and the green shift from the old differences.
/// Neutrals have zero differences, so they pass unchanged.
const CROSSTALK: [[f32; 2]; 3] = [[1.1532, 0.3195], [0.0973, 1.3271], [-0.0460, -0.1033]];

/// Exposure anchor = mean of the 99th percentile and the average of E_g, plus this.
const ANCHOR_OFFSET: f32 = 0.1027;
/// Shared film toe, expanded near the base: E' = E - amount · exp(-max(E, floor) / scale).
const TOE_AMOUNT: f32 = 0.3;
const TOE_SCALE: f32 = 0.35;
const TOE_FLOOR: f32 = -0.3;

/// Manual overrides for the automatic analysis. `None` = the automatic choice.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Hints {
    /// Image area used to measure density range and balance.
    pub analysis: Option<NRect>,
    /// Size of the centred image area when `analysis` isn't drawn, in percent of each side.
    pub area_percent: Option<f32>,
    /// Unexposed film (rebate) to read the film base from.
    pub base: Option<NRect>,
    /// A film base measured elsewhere, as for a whole roll. `base` wins over this.
    pub base_rgb: Option<[f32; 3]>,
}

/// Red or blue density as a function of green density for a neutral:
/// `D = bend · g² + slope · g` up to `top`, a straight tangent above, through the base.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct GreyCurve {
    pub bend: f32,
    pub slope: f32,
    pub top: f32,
}

impl GreyCurve {
    fn value(&self, g: f32) -> f32 {
        (self.bend * g + self.slope) * g
    }

    /// Green density of a neutral with density `d` in this channel.
    #[inline]
    pub fn equivalent(&self, d: f32) -> f32 {
        // A falling curve must stay invertible: stop short of its turning point.
        let top = if self.bend < 0.0 {
            self.top.min(0.9 * -self.slope / (2.0 * self.bend))
        } else {
            self.top
        };
        let rate = |g: f32| (1.0 / (self.bend * g + self.slope).max(1e-3)).clamp(0.6, 1.6);
        let end = self.value(top);
        if d < 0.0 {
            d * rate(top / 20.0)
        } else if d > end {
            top + (d - end) * rate(1.9 * top)
        } else {
            // Root of the quadratic, in the form that stays stable as bend → 0.
            let root = (self.slope * self.slope + 4.0 * self.bend * d)
                .max(0.0)
                .sqrt();
            2.0 * d / (self.slope + root).max(1e-6)
        }
    }
}

/// Automatic estimate for one frame.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Estimate {
    /// Linear camera RGB of the unexposed film base.
    pub base_rgb: [f32; 3],
    /// Where the base was read from.
    pub base_area: NRect,
    /// Part of the scan the grey curves and exposure were measured on.
    pub area: NRect,
    /// Grey curves of the red and blue layers.
    pub grey: [GreyCurve; 2],
    /// Density of `E'` that maps to linear 1.0.
    pub anchor: f32,
}

/// User adjustments. Sliders run −100…+100 with 0 = the automatic result.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Adjust {
    pub exposure: f32,
    pub contrast: f32,
    /// Tone curve sliders, see [`Tone`].
    pub blacks: f32,
    pub shadows: f32,
    pub highlights: f32,
    pub whites: f32,
    /// Profile the DNG opens with.
    pub look: Look,
    /// + warmer (yellow), − cooler (blue).
    pub temperature: f32,
    /// + magenta, − green.
    pub tint: f32,
    /// On top of [`BASE_SATURATION`]; −100 = black and white.
    pub saturation: f32,
    /// Fine rotation in degrees, clockwise positive.
    pub straighten: f32,
    /// Output crop; `None` keeps the whole scan.
    pub crop: Option<NRect>,
    /// Mirror left-right (emulsion-up scans), applied before `rotate`.
    pub flip_h: bool,
    /// Clockwise quarter turns.
    pub rotate: u8,
}

impl Default for Adjust {
    fn default() -> Self {
        Adjust {
            exposure: 0.0,
            contrast: 0.0,
            blacks: 0.0,
            shadows: 0.0,
            highlights: 0.0,
            whites: 0.0,
            look: Look::default(),
            temperature: 0.0,
            tint: 0.0,
            saturation: 0.0,
            straighten: 0.0,
            crop: None,
            flip_h: false,
            rotate: 0,
        }
    }
}

impl Adjust {
    /// Per-channel log2 gains from exposure, temperature and tint.
    fn gains_ev(&self) -> [f32; 3] {
        let e = self.exposure / 100.0 * EXPOSURE_EV_RANGE;
        let t = self.temperature / 100.0 * BALANCE_EV_RANGE;
        let g = self.tint / 100.0 * BALANCE_EV_RANGE;
        [e + t + g / 2.0, e - g, e - t + g / 2.0]
    }

    pub fn tone(&self) -> Tone {
        Tone {
            contrast: self.contrast,
            blacks: self.blacks,
            shadows: self.shadows,
            highlights: self.highlights,
            whites: self.whites,
        }
    }

    pub fn saturation_gain(&self) -> f32 {
        BASE_SATURATION * (1.0 + self.saturation.clamp(-100.0, 100.0) / 100.0)
    }
}

/// A frame's measurements and adjustments; serialised into the DNG's XMP. They reproduce
/// the conversion only under the same `model`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Settings {
    /// [`MODEL_VERSION`] these settings were made with; 0 = before versioning.
    #[serde(default)]
    pub model: u32,
    pub estimate: Estimate,
    #[serde(default)]
    pub hints: Hints,
    #[serde(default)]
    pub adjust: Adjust,
}

impl Settings {
    pub fn new(estimate: Estimate, adjust: Adjust) -> Self {
        Settings {
            model: MODEL_VERSION,
            estimate,
            hints: Hints::default(),
            adjust,
        }
    }
}

fn percentile(values: &mut [f32], p: f32) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let k = ((p / 100.0) * (values.len() - 1) as f32).round() as usize;
    let (_, v, _) = values.select_nth_unstable_by(k, |a, b| a.total_cmp(b));
    *v
}

fn median(values: &mut [f32]) -> f32 {
    percentile(values, 50.0)
}

#[inline]
fn density_above_base(v: f32, base: f32) -> f32 {
    -(v / base).max(1e-6).log10()
}

fn channel_medians(img: &Rgb32) -> [f32; 3] {
    [0, 1, 2].map(|c| median(&mut img.pixels().map(|p| p[c]).collect::<Vec<_>>()).max(1e-4))
}

/// Clipped pixels and their surroundings, which catch glare from holes and gaps.
fn near_clipped(small: &Rgb32) -> Vec<bool> {
    let (w, h) = (small.width, small.height);
    let reach = w.max(h) / 200;
    let mut out: Vec<bool> = small
        .pixels()
        .map(|p| p[0].max(p[1]).max(p[2]) >= CLIPPED)
        .collect();
    for horizontal in [true, false] {
        let src = out.clone();
        let (lines, len) = if horizontal { (h, w) } else { (w, h) };
        for line in 0..lines {
            let at = |i: usize| {
                if horizontal {
                    line * w + i
                } else {
                    i * w + line
                }
            };
            let mut until = 0;
            // Forward sweep marks what follows a clipped pixel, backward what precedes it.
            for i in 0..len {
                if src[at(i)] {
                    until = i + reach + 1;
                }
                out[at(i)] |= i < until;
            }
            let mut until = len;
            for i in (0..len).rev() {
                if src[at(i)] {
                    until = i.saturating_sub(reach);
                }
                out[at(i)] |= i >= until;
            }
        }
    }
    out
}

/// Film base: the brightest pixels away from clipped light are the rebate.
fn detect_base(small: &Rgb32) -> ([f32; 3], NRect) {
    let lum: Vec<f32> = small.pixels().map(|p| (p[0] + p[1] + p[2]) / 3.0).collect();
    let mut unclipped: Vec<bool> = near_clipped(small).iter().map(|&n| !n).collect();
    if unclipped.iter().filter(|&&u| u).count() < 1000 {
        unclipped = small
            .pixels()
            .map(|p| p[0].max(p[1]).max(p[2]) < 0.98)
            .collect();
    }
    let mut lum_ok: Vec<f32> = lum
        .iter()
        .zip(&unclipped)
        .filter(|(_, &u)| u)
        .map(|(&l, _)| l)
        .collect();
    let thr = percentile(&mut lum_ok, 97.0);
    let picked = |i: usize| unclipped[i] && lum[i] >= thr;
    let mut base = [0.0f32; 3];
    for (c, b) in base.iter_mut().enumerate() {
        let mut vals: Vec<f32> = small
            .pixels()
            .enumerate()
            .filter(|(i, _)| picked(*i))
            .map(|(_, p)| p[c])
            .collect();
        *b = median(&mut vals).max(1e-4);
    }
    // For display: the strip (rows or columns) holding most picked pixels, i.e. one rebate.
    let (w, h) = (small.width, small.height);
    let mut rows = vec![0usize; h];
    let mut cols = vec![0usize; w];
    for i in (0..w * h).filter(|&i| picked(i)) {
        rows[i / w] += 1;
        cols[i % w] += 1;
    }
    // Contiguous run around the peak where counts stay above half of it.
    let band = |v: &[usize]| {
        let (peak, &max) = v
            .iter()
            .enumerate()
            .max_by_key(|(_, &c)| c)
            .unwrap_or((0, &0));
        let (mut a, mut b) = (peak, peak + 1);
        while a > 0 && v[a - 1] * 2 > max {
            a -= 1;
        }
        while b < v.len() && v[b] * 2 > max {
            b += 1;
        }
        (a, b, max)
    };
    let (r0, r1, rmax) = band(&rows);
    let (c0, c1, cmax) = band(&cols);
    // A horizontal rebate strip fills its rows more fully than a vertical one fills columns.
    let rect = if rmax as f32 / w as f32 >= cmax as f32 / h as f32 {
        crate::image::Rect {
            top: r0,
            bottom: r1,
            left: 0,
            right: w,
        }
    } else {
        crate::image::Rect {
            top: 0,
            bottom: h,
            left: c0,
            right: c1,
        }
    };
    let area = if rmax.max(cmax) > 0 {
        NRect::from_pixels(rect, w, h)
    } else {
        NRect::FULL
    };
    (base, area)
}

/// Estimate film base, image area, density points and balance from a full-res scan.
pub fn estimate(scan: &Rgb32, hints: &Hints) -> Estimate {
    estimate_on(&analysis_image(scan), hints)
}

/// The downsampled copy estimates are computed on; callers may cache it.
pub fn analysis_image(scan: &Rgb32) -> Rgb32 {
    scan.downsample(ANALYSIS_FACTOR)
}

fn in_band(px: &[f32; 3], grey: &[GreyCurve; 2], band: f32) -> bool {
    (px[0] - grey[0].value(px[1])).abs() < band && (px[2] - grey[1].value(px[1])).abs() < band
}

/// Weighted fit of `bend, slope` to (x, y), pulled toward `prior`.
fn fit_pulled(points: &[(f32, f32)], prior: [f32; 2]) -> [f32; 2] {
    let w2 = GREY_BIN_WEIGHT * GREY_BIN_WEIGHT;
    let (mut aa, mut ab, mut bb, mut ra, mut rb) = (GREY_PULL, 0.0, GREY_PULL, 0.0, 0.0);
    for &(x, y) in points {
        let (a, b) = (x * x, x);
        let residual = y - (prior[0] * a + prior[1] * b);
        aa += w2 * a * a;
        ab += w2 * a * b;
        bb += w2 * b * b;
        ra += w2 * a * residual;
        rb += w2 * b * residual;
    }
    let det = aa * bb - ab * ab;
    [
        prior[0] + (bb * ra - ab * rb) / det,
        prior[1] + (aa * rb - ab * ra) / det,
    ]
}

/// Grey curves of one frame from density-above-base pixels, without any reference: repeatedly
/// take the pixels near the current curves, and fit the median of each density bin.
fn fit_grey(density: &[[f32; 3]]) -> [GreyCurve; 2] {
    let dense: Vec<[f32; 3]> = density
        .iter()
        .filter(|p| p[1] > GREY_MIN_DENSITY)
        .copied()
        .collect();
    let curve = |c: [f32; 2], top: f32| GreyCurve {
        bend: c[0],
        slope: c[1],
        top,
    };
    let mut grey = GREY_PRIOR.map(|c| curve(c, 1.0));
    for band in GREY_BANDS {
        let mut bins = vec![Vec::new(); GREY_BIN_EDGES.len() - 1];
        for p in dense.iter().filter(|p| in_band(p, &grey, band)) {
            let i = GREY_BIN_EDGES.partition_point(|&e| e <= p[1]);
            if (1..GREY_BIN_EDGES.len()).contains(&i) {
                bins[i - 1].push(*p);
            }
        }
        let medians: Vec<[f32; 3]> = bins
            .iter()
            .filter(|b| b.len() >= GREY_MIN_BIN_PIXELS)
            .map(|b| [0, 1, 2].map(|c| median(&mut b.iter().map(|p| p[c]).collect::<Vec<_>>())))
            .collect();
        if medians.len() < 4 {
            break;
        }
        let points = |c: usize| medians.iter().map(|m| (m[1], m[c])).collect::<Vec<_>>();
        grey = [
            curve(fit_pulled(&points(0), GREY_PRIOR[0]), 1.0),
            curve(fit_pulled(&points(2), GREY_PRIOR[1]), 1.0),
        ];
    }
    let band = GREY_BANDS[GREY_BANDS.len() - 1];
    let mut near: Vec<f32> = dense
        .iter()
        .filter(|p| in_band(p, &grey, band))
        .map(|p| p[1])
        .collect();
    let top = if near.len() > 200 {
        percentile(&mut near, 97.0)
    } else if !dense.is_empty() {
        percentile(&mut dense.iter().map(|p| p[1]).collect::<Vec<_>>(), 90.0)
    } else {
        1.0
    };
    grey.map(|g| GreyCurve { top, ..g })
}

/// `E` of a density-above-base pixel: grey curves, then crosstalk.
#[inline]
fn equivalent_density(d: [f32; 3], grey: &[GreyCurve; 2]) -> [f32; 3] {
    let green = d[1];
    let x = grey[0].equivalent(d[0]) - green;
    let y = grey[1].equivalent(d[2]) - green;
    let g = green + CROSSTALK[2][0] * x + CROSSTALK[2][1] * y;
    [
        g + CROSSTALK[0][0] * x + CROSSTALK[0][1] * y,
        g,
        g + CROSSTALK[1][0] * x + CROSSTALK[1][1] * y,
    ]
}

/// Exposure from the highlights and the scene average together: highlights alone misjudge
/// frames with small bright lights, the average alone misjudges low-key frames.
fn fit_anchor(density: &[[f32; 3]], grey: &[GreyCurve; 2]) -> f32 {
    let mut e_g: Vec<f32> = density
        .iter()
        .map(|&d| equivalent_density(d, grey)[1])
        .collect();
    // Bare film says nothing about the scene.
    let (mut sum, mut n) = (0.0f64, 0usize);
    for (e, d) in e_g.iter().zip(density) {
        if d[1] > 0.03 {
            sum += *e as f64;
            n += 1;
        }
    }
    let mean = if n > 0 { (sum / n as f64) as f32 } else { 0.0 };
    0.5 * (percentile(&mut e_g, 99.0) + mean) + ANCHOR_OFFSET
}

/// [`estimate`] on an image from [`analysis_image`].
pub fn estimate_on(small: &Rgb32, hints: &Hints) -> Estimate {
    let (w, h) = (small.width, small.height);

    let (base_rgb, base_area) = match hints.base {
        Some(area) => (channel_medians(&small.crop(area.to_pixels(w, h))), area),
        None => {
            let (detected, base_area) = detect_base(small);
            (hints.base_rgb.unwrap_or(detected), base_area)
        }
    };
    let area = hints.analysis.unwrap_or_else(|| {
        NRect::centred(hints.area_percent.unwrap_or(DEFAULT_AREA_PERCENT) / 100.0)
    });

    let density: Vec<[f32; 3]> = small
        .crop(area.to_pixels(w, h))
        .pixels()
        .filter(|p| p[0].max(p[1]).max(p[2]) < CLIPPED)
        .map(|p| [0, 1, 2].map(|c| density_above_base(p[c], base_rgb[c])))
        .collect();
    let grey = fit_grey(&density);
    Estimate {
        base_rgb,
        base_area,
        area,
        grey,
        anchor: fit_anchor(&density, &grey),
    }
}

/// One film base for a roll from its frames' bases: stock and processing don't change from
/// frame to frame, and the median outvotes a frame whose base was misread. Grey curves stay
/// per frame, which matched finished scans better than curves pooled over a roll.
pub fn roll_base(frame_bases: &[[f32; 3]]) -> [f32; 3] {
    [0, 1, 2].map(|c| median(&mut frame_bases.iter().map(|b| b[c]).collect::<Vec<_>>()))
}

/// Scene-linear positive for one pixel of linear camera RGB, before saturation.
#[inline]
pub fn positive(rgb: [f32; 3], e: &Estimate, a: &Adjust) -> [f32; 3] {
    let gains = a.gains_ev();
    let d = [0, 1, 2].map(|c| density_above_base(rgb[c], e.base_rgb[c]));
    let eq = equivalent_density(d, &e.grey);
    [0, 1, 2].map(|c| {
        let toe = TOE_AMOUNT * (-eq[c].max(TOE_FLOOR) / TOE_SCALE).exp();
        10f32.powf((eq[c] - toe - e.anchor) / GAMMA_REF + gains[c] * LOG10_2)
    })
}

/// Linear saturation around Rec.2020 luminance; negative results are clipped to 0.
#[inline]
pub fn saturate(p: [f32; 3], gain: f32) -> [f32; 3] {
    let y = p[0] * REC2020_LUMA[0] + p[1] * REC2020_LUMA[1] + p[2] * REC2020_LUMA[2];
    p.map(|v| (y + gain * (v - y)).max(0.0))
}

/// The positive as it will be written: straighten, crop, invert, saturate, mirror, rotate.
/// `crop_override` replaces the output crop (e.g. `Some(NRect::FULL)` for a full view).
pub fn convert_with(scan: &Rgb32, s: &Settings, crop_override: Option<NRect>) -> Rgb32 {
    let a = &s.adjust;
    let straight = scan.straighten(a.straighten);
    let crop = crop_override.or(a.crop).unwrap_or(NRect::FULL);
    let mut img = straight.crop(crop.to_pixels(straight.width, straight.height));
    let gain = a.saturation_gain();
    for_each_part(&mut img.data, 3, |_, part| {
        for px in part.as_chunks_mut::<3>().0 {
            *px = saturate(positive(*px, &s.estimate, a), gain);
        }
    });
    if a.flip_h {
        img = img.flip_h();
    }
    img.rotate_cw(a.rotate)
}

pub fn convert(scan: &Rgb32, s: &Settings) -> Rgb32 {
    convert_with(scan, s, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: [f32; 3] = [0.3, 0.4, 0.25];
    const CURVES: [GreyCurve; 2] = [
        GreyCurve {
            bend: -0.15,
            slope: 0.9,
            top: 1.2,
        },
        GreyCurve {
            bend: 0.25,
            slope: 0.85,
            top: 1.2,
        },
    ];

    fn est() -> Estimate {
        Estimate {
            base_rgb: BASE,
            base_area: NRect::FULL,
            area: NRect::FULL,
            grey: CURVES,
            anchor: 0.7,
        }
    }

    #[test]
    fn roll_base_outvotes_a_misread_frame_and_a_picked_base_still_wins() {
        let base = [0.3, 0.4, 0.25];
        let misread = [0.9, 0.9, 0.9];
        assert_eq!(roll_base(&[base, misread, base]), base);

        let mut scan = Rgb32::new(64, 48);
        for (i, v) in scan.data.iter_mut().enumerate() {
            *v = 0.05 + (i % 89) as f32 / 100.0;
        }
        let mut hints = Hints {
            base_rgb: Some(base),
            ..Hints::default()
        };
        assert_eq!(estimate_on(&scan, &hints).base_rgb, base);
        hints.base = Some(NRect::FULL);
        assert_ne!(estimate_on(&scan, &hints).base_rgb, base);
    }

    fn pixel(d: [f32; 3]) -> [f32; 3] {
        [0, 1, 2].map(|c| BASE[c] * 10f32.powf(-d[c]))
    }

    /// A neutral of green density `g` on the given curves.
    fn neutral(g: f32, grey: &[GreyCurve; 2]) -> [f32; 3] {
        [grey[0].value(g), g, grey[1].value(g)]
    }

    #[test]
    fn grey_curve_inverts_and_continues_past_its_ends() {
        for curve in CURVES {
            let end = curve.value(curve.top);
            for i in 1..=20 {
                let g = curve.top * i as f32 / 20.0;
                assert!((curve.equivalent(curve.value(g)) - g).abs() < 1e-4, "{g}");
            }
            let samples: Vec<f32> = (-20..=80)
                .map(|i| curve.equivalent(i as f32 * 0.03))
                .collect();
            assert!(samples.windows(2).all(|w| w[1] > w[0]));
            assert!(curve.equivalent(end + 0.5) > curve.top);
            assert!(curve.equivalent(-0.1) < 0.0);
        }
        let straight = GreyCurve {
            bend: 0.0,
            slope: 0.8,
            top: 1.0,
        };
        assert!((straight.equivalent(0.4) - 0.5).abs() < 1e-5);
    }

    #[test]
    fn falling_curve_stays_invertible_past_its_turning_point() {
        let curve = GreyCurve {
            bend: -0.6,
            slope: 0.9,
            top: 1.2,
        };
        let samples: Vec<f32> = (0..=100)
            .map(|i| curve.equivalent(i as f32 * 0.02))
            .collect();
        assert!(samples.iter().all(|v| v.is_finite()));
        assert!(samples.windows(2).all(|w| w[1] > w[0]));
    }

    #[test]
    fn neutrals_stay_neutral_at_every_density() {
        let e = est();
        for i in 1..=12 {
            let p = positive(
                pixel(neutral(i as f32 * 0.1, &e.grey)),
                &e,
                &Adjust::default(),
            );
            assert!((p[0] / p[1] - 1.0).abs() < 2e-3, "{p:?}");
            assert!((p[2] / p[1] - 1.0).abs() < 2e-3, "{p:?}");
        }
    }

    #[test]
    fn denser_film_is_brighter_and_the_anchor_sets_exposure() {
        let e = est();
        let at =
            |g: f32, e: &Estimate| positive(pixel(neutral(g, &e.grey)), e, &Adjust::default())[1];
        assert!(at(1.0, &e) > at(0.5, &e));
        assert!(at(0.5, &e) > at(0.1, &e));
        let darker = Estimate {
            anchor: e.anchor + GAMMA_REF * LOG10_2,
            ..e.clone()
        };
        assert!((at(0.8, &e) / at(0.8, &darker) - 2.0).abs() < 1e-2);
    }

    #[test]
    fn sliders_are_per_channel_gains() {
        let e = est();
        let px = [0.1, 0.1, 0.1];
        let base = positive(px, &e, &Adjust::default());
        let ev = |a: Adjust| {
            let p = positive(px, &e, &a);
            [0, 1, 2].map(|c| (p[c] / base[c]).log2())
        };
        let full_exposure = ev(Adjust {
            exposure: 100.0,
            ..Adjust::default()
        });
        assert!(full_exposure
            .iter()
            .all(|v| (v - EXPOSURE_EV_RANGE).abs() < 1e-3));
        let warm = ev(Adjust {
            temperature: 100.0,
            ..Adjust::default()
        });
        assert!(warm[0] > 0.0 && warm[2] < 0.0 && warm[1].abs() < 1e-4);
        let magenta = ev(Adjust {
            tint: 100.0,
            ..Adjust::default()
        });
        assert!(magenta[1] < 0.0 && magenta[0] > 0.0 && magenta[2] > 0.0);
    }

    #[test]
    fn saturation_keeps_neutrals_and_luminance() {
        let grey = saturate([0.4, 0.4, 0.4], 2.0);
        assert!(grey.iter().all(|v| (v - 0.4).abs() < 1e-6));
        let c = [0.5, 0.3, 0.2];
        let s = saturate(c, 1.5);
        let luma = |p: [f32; 3]| p.iter().zip(REC2020_LUMA).map(|(v, w)| v * w).sum::<f32>();
        assert!((luma(c) - luma(s)).abs() < 1e-5);
        assert!(s[0] - s[2] > c[0] - c[2]);
        let bw = Adjust {
            saturation: -100.0,
            ..Adjust::default()
        };
        assert_eq!(bw.saturation_gain(), 0.0);
    }

    /// 400x300: bright base border, frame with a neutral density gradient inside.
    fn synthetic_scan() -> Rgb32 {
        let (w, h) = (400, 300);
        let mut scan = Rgb32::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let inside = (40..360).contains(&x) && (30..270).contains(&y);
                let d = if inside {
                    neutral(0.2 + 0.8 * (x - 40) as f32 / 320.0, &CURVES)
                } else {
                    [0.0; 3]
                };
                let i = (y * w + x) * 3;
                scan.data[i..i + 3].copy_from_slice(&pixel(d));
            }
        }
        scan
    }

    #[test]
    fn estimate_finds_base_and_measures_the_centre_on_synthetic_scan() {
        let e = estimate(&synthetic_scan(), &Hints::default());
        for (got, want) in e.base_rgb.iter().zip(BASE) {
            assert!((got - want).abs() < 0.01, "base {:?}", e.base_rgb);
        }
        assert_eq!(e.area, NRect::centred(DEFAULT_AREA_PERCENT / 100.0));
        let hints = Hints {
            area_percent: Some(60.0),
            ..Hints::default()
        };
        assert_eq!(
            estimate(&synthetic_scan(), &hints).area,
            NRect::centred(0.6)
        );
    }

    #[test]
    fn estimate_is_repeatable() {
        let scan = synthetic_scan();
        assert_eq!(
            estimate(&scan, &Hints::default()),
            estimate(&scan, &Hints::default())
        );
    }

    #[test]
    fn fit_recovers_the_grey_curve_of_a_neutral_scene() {
        let e = estimate_on(&synthetic_scan(), &Hints::default());
        for (got, want) in e.grey.iter().zip(CURVES) {
            for g in [0.3, 0.6, 0.9] {
                assert!(
                    (got.value(g) - want.value(g)).abs() < 0.02,
                    "{got:?} at {g}"
                );
            }
        }
    }

    #[test]
    fn a_dominant_colour_does_not_capture_the_fit() {
        // Mostly one strong colour, with far fewer greys: a meadow under a small sky.
        let meadow = (0..12000).map(|i| {
            let d = neutral(0.2 + 0.9 * (i % 100) as f32 / 100.0, &CURVES);
            [d[0] + 0.25, d[1], d[2] - 0.25]
        });
        let sky = (0..2000).map(|i| neutral(0.5 + 0.65 * (i % 100) as f32 / 100.0, &CURVES));
        let grey = fit_grey(&meadow.chain(sky).collect::<Vec<_>>());
        for (got, want) in grey.iter().zip(CURVES) {
            let (got, want) = (got.value(1.0), want.value(1.0));
            assert!((got - want).abs() < 0.03, "{got} {want}");
        }
    }

    #[test]
    fn fit_falls_back_to_the_prior_without_evidence() {
        for density in [Vec::new(), vec![[0.01, 0.01, 0.01]; 500]] {
            let grey = fit_grey(&density);
            assert_eq!([grey[0].bend, grey[0].slope], GREY_PRIOR[0]);
            assert_eq!([grey[1].bend, grey[1].slope], GREY_PRIOR[1]);
            assert!(fit_anchor(&density, &grey).is_finite() || density.is_empty());
        }
    }

    #[test]
    fn anchor_balances_highlights_and_scene_average() {
        // Same small highlights, more of the scene dense: the frame is exposed darker.
        let low_key = |share: usize| -> Vec<[f32; 3]> {
            (0..10000)
                .map(|i| {
                    let g = match i % 100 {
                        0..=1 => 1.2,
                        n if n < share => 0.8,
                        _ => 0.1,
                    };
                    neutral(g, &CURVES)
                })
                .collect()
        };
        let dark = fit_anchor(&low_key(10), &CURVES);
        let bright = fit_anchor(&low_key(70), &CURVES);
        assert!(bright > dark + 0.1, "{dark} {bright}");
    }

    #[test]
    fn holder_edge_does_not_set_exposure() {
        let clean = synthetic_scan();
        let mut held = clean.clone();
        let (w, h) = (held.width, held.height);
        for y in 0..h {
            for x in 0..w / 25 {
                held.data[(y * w + x) * 3..(y * w + x) * 3 + 3].copy_from_slice(&[0.002; 3]);
            }
        }
        let e = estimate(&held, &Hints::default());
        let want = estimate(&clean, &Hints::default()).anchor;
        assert!((e.anchor - want).abs() < 0.05, "{} {want}", e.anchor);
    }

    #[test]
    fn glare_around_film_holes_does_not_set_the_base() {
        let mut scan = synthetic_scan();
        let (w, h) = (scan.width, scan.height);
        // A sprocket hole in the rebate with a brighter halo around it.
        for y in 0..h.min(20) {
            for x in 0..w {
                let dist = (x as i32 - 200).abs().max(y as i32 - 5);
                let v: [f32; 3] = if dist <= 3 {
                    [1.0; 3]
                } else if dist == 4 {
                    [0.6, 0.7, 0.5]
                } else {
                    continue;
                };
                scan.data[(y * w + x) * 3..(y * w + x) * 3 + 3].copy_from_slice(&v);
            }
        }
        let e = estimate(&scan, &Hints::default());
        assert!((e.base_rgb[1] - BASE[1]).abs() < 0.01, "{:?}", e.base_rgb);
    }

    #[test]
    fn hints_override_detection() {
        let scan = synthetic_scan();
        let corner = NRect {
            x0: 0.0,
            y0: 0.0,
            x1: 0.05,
            y1: 0.05,
        };
        let area = NRect {
            x0: 0.3,
            y0: 0.3,
            x1: 0.7,
            y1: 0.7,
        };
        let e = estimate(
            &scan,
            &Hints {
                analysis: Some(area),
                base: Some(corner),
                ..Hints::default()
            },
        );
        assert_eq!(e.area, area);
        assert_eq!(e.base_area, corner);
        assert!((e.base_rgb[1] - BASE[1]).abs() < 1e-3);
    }

    #[test]
    fn full_scan_is_kept_without_crop() {
        let scan = synthetic_scan();
        let s = Settings::new(estimate(&scan, &Hints::default()), Adjust::default());
        let out = convert(&scan, &s);
        assert_eq!((out.width, out.height), (400, 300));
        let rotated = convert(
            &scan,
            &Settings {
                adjust: Adjust {
                    rotate: 1,
                    ..Adjust::default()
                },
                ..s.clone()
            },
        );
        assert_eq!((rotated.width, rotated.height), (300, 400));
    }

    #[test]
    fn settings_record_the_model_version() {
        let s = Settings::new(est(), Adjust::default());
        assert_eq!(s.model, MODEL_VERSION);
        let mut json = serde_json::to_value(&s).unwrap();
        json.as_object_mut().unwrap().remove("model");
        let old: Settings = serde_json::from_value(json).unwrap();
        assert_eq!(old.model, 0);
    }
}
