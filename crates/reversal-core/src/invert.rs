// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Density-model inversion.
//!
//! Per channel c, with transmittance T relative to the film base:
//!   D  = -log10(T / T_base)                      density above base
//!   E  = equivalent green density of D           per-frame gray curve for red and blue
//!   E' = crosstalk and toe corrections of E      fixed, in density
//!   L  = 10^((E' - anchor) / gamma + offsets)    scene-linear positive
//! A neutral scene patch lands on the gray curve, so its three E are equal and it stays
//! neutral at every density. `anchor` sets exposure from the frame's highlights and
//! average density; `offsets` hold the exposure and color balance sliders. A small
//! saturation step follows.
//!
//! Black and white film has one silver image, which blocks every color about equally, so it
//! needs no gray curves or crosstalk: the three densities above base are averaged into one
//! `E`, and the positive is that one value in all three channels.
//!
//! Geometry: scan → straighten → crop → invert → mirror → quarter turns.
//! Regions (crop, analysis area, base sample) are normalized scan coordinates.

use serde::{Deserialize, Serialize};

use crate::image::{for_each_part, NRect, Rgb32};
use crate::render::{Look, Tone};

/// Film gamma assumed when turning density into scene log exposure; sets scene contrast.
pub const GAMMA_REF: f32 = 0.7;
/// Version of the conversion model. Bump when the inversion, `BASE_SATURATION`, the
/// DNG's stored scale or the tone curve change, since old settings then render differently.
pub const MODEL_VERSION: u32 = 3;
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
/// Share of each side of the scan, centered, measured when nobody draws an area: clear of
/// holders and rebates on a framed scan. Tested at 70–95% against reference scans; 95%
/// takes in holders.
pub const DEFAULT_AREA_PERCENT: f32 = 85.0;

/// A film holder blocks the light, so it reads far denser than film: above this green density,
/// connected to the scan's edge. Picture this dense sits inside the frame, and at 1.0 real picture
/// joined the holder on two of 44 tested frames.
const HOLDER_DENSITY: f32 = 1.2;
/// Grid the holder is searched on, in analysis pixels. Each cell reads its median, so grain can't
/// break up the holder.
const HOLDER_CELL: usize = 4;
/// Margin around the holder, as a share of the scan's long side: its soft edge passes through
/// every density and would otherwise mislead the gray fit.
const HOLDER_MARGIN: f32 = 0.013;
/// Share of each side, centered, that no holder reaches. Dense cells in there are picture that
/// happens to touch the edge, as on a scan cropped into the picture, so nothing counts as holder.
const HOLDER_CLEAR_CENTER: f32 = 0.5;

/// Starting gray curves (bend, slope) for red and blue: the average across rolls.
/// Each frame's fit is pulled toward these by `GRAY_PULL`.
const GRAY_PRIOR: [[f32; 2]; 2] = [[-0.2008, 0.9564], [0.1883, 0.8340]];
const GRAY_PULL: f32 = 0.003;
/// Pixels within this density of the curve count as gray. The band narrows over the
/// iterations, so a frame dominated by one color can't capture the fit early on.
const GRAY_BANDS: [f32; 5] = [0.15, 0.10, 0.07, 0.05, 0.04];
/// Below this green density the film toe and any error in the base dominate.
const GRAY_MIN_DENSITY: f32 = 0.15;
const GRAY_BIN_EDGES: [f32; 19] = [
    0.15, 0.2, 0.25, 0.3, 0.35, 0.4, 0.45, 0.5, 0.55, 0.6, 0.7, 0.8, 0.9, 1.0, 1.1, 1.2, 1.3, 1.45,
    1.7,
];
const GRAY_MIN_BIN_PIXELS: usize = 40;
/// Bins count equally: each density range is its own evidence for gray, however many
/// pixels of it a scene happens to contain.
const GRAY_BIN_WEIGHT: f32 = 0.25;

/// Density crosstalk between the dye layers and the sensor, for a D5100 and one light
/// source: rows give new E_r - E_g, E_b - E_g and the green shift from the old differences.
/// Neutrals have zero differences, so they pass unchanged.
const CROSSTALK: [[f32; 2]; 3] = [[1.1532, 0.3195], [0.0973, 1.3271], [-0.0460, -0.1033]];

/// Film base search: cells along the scan's long side, and how much a cell may vary (10th to
/// 90th percentile, as a share of its median) and still count as even.
const BASE_CELLS_LONG_SIDE: usize = 150;
const BASE_CELL_SPREAD: f32 = 0.05;
/// Clear film lies within this share of a high percentile of the even cells' levels.
const BASE_LEVEL_PERCENTILE: f32 = 98.0;
const BASE_LEVEL_RANGE: f32 = 0.93;
/// Share of each side counted as the film's edge, and how much of a side a patch must span
/// there, or inside the scan, to be taken for clear film.
const BASE_EDGE_BAND: f32 = 0.15;
const BASE_MIN_EDGE_SHARE: f32 = 0.08;
const BASE_MIN_GAP_SHARE: f32 = 0.5;
/// A frame's base this far from its roll's, as a share in any channel, deserves a look: frames
/// of a roll scanned alike stayed within 2%, unless something was misread.
const ROLL_BASE_TOLERANCE: f32 = 0.06;

/// Film type detection works on cells of this many analysis pixels a side: grain and lens color
/// fringes average out, scene color doesn't.
const FILM_CELL: usize = 4;
/// Density bins of the detection's median gray curve, from `FILM_MIN_DENSITY` up, and the cells a
/// bin needs to count. They reach below the base: when no clear film shows, the base is a guess
/// from the picture, and on black and white that moves every channel alike.
const FILM_MIN_DENSITY: f32 = -1.0;
const FILM_BIN_WIDTH: f32 = 0.05;
const FILM_BIN_COUNT: usize = 60;
const FILM_MIN_BIN_CELLS: usize = 40;
/// Bins a frame must fill to show how its channels rise together: a few flat patches can't.
const FILM_MIN_BINS: usize = 6;
/// Black and white when 90% of cells lie this close to the frame's own gray curve, in density.
/// On 220 scans from two cameras, black and white reached 0.011 and color started at 0.0197.
const FILM_BW_SPREAD: f32 = 0.02;
/// ...and red density rises with green at a slope this close to 1: silver blocks every color
/// about equally. Black and white read 1.000–1.024, color 0.67–0.93, also on near-gray scenes.
const FILM_BW_RED_SLOPE: f32 = 0.05;

/// Exposure anchor = mean of the 99th percentile and the average of E_g, plus this.
const ANCHOR_OFFSET: f32 = 0.1027;
/// Shared film toe, expanded near the base: E' = E - amount · exp(-max(E, floor) / scale).
const TOE_AMOUNT: f32 = 0.3;
const TOE_SCALE: f32 = 0.35;
const TOE_FLOOR: f32 = -0.3;

/// Which kind of negative a frame is, and so which conversion it gets.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Film {
    /// Color negative: three dye layers under an orange mask.
    #[default]
    Color,
    /// Black and white negative: one silver image.
    Bw,
}

/// Manual overrides for the automatic analysis. `None` = the automatic choice.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Hints {
    /// Film type; `None` detects it.
    pub film: Option<Film>,
    /// Image area used to measure density range and balance.
    pub analysis: Option<NRect>,
    /// Size of the centered image area when `analysis` isn't drawn, in percent of each side.
    pub area_percent: Option<f32>,
    /// Unexposed film (rebate) to read the film base from.
    pub base: Option<NRect>,
    /// A film base measured elsewhere, as for a whole roll. `base` wins over this.
    pub base_rgb: Option<[f32; 3]>,
}

/// Red or blue density as a function of green density for a neutral:
/// `D = bend · g² + slope · g` up to `top`, a straight tangent above, through the base.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct GrayCurve {
    pub bend: f32,
    pub slope: f32,
    pub top: f32,
}

impl GrayCurve {
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
    /// Which conversion the frame gets: the detected film type, or the one chosen. Settings from
    /// before black and white were all color.
    #[serde(default)]
    pub film: Film,
    /// Linear camera RGB of the unexposed film base.
    pub base_rgb: [f32; 3],
    /// Where the base was read from.
    pub base_area: NRect,
    /// Part of the scan the gray curves and exposure were measured on.
    pub area: NRect,
    /// Gray curves of the red and blue layers. Black and white doesn't use them.
    #[serde(alias = "grey")]
    pub gray: [GrayCurve; 2],
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
    /// + warmer (yellow), − cooler (blue). Color film only, like `tint` and `saturation`.
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
    /// Exposure as a log2 gain.
    fn exposure_ev(&self) -> f32 {
        self.exposure / 100.0 * EXPOSURE_EV_RANGE
    }

    /// Per-channel log2 gains from exposure, temperature and tint.
    fn gains_ev(&self) -> [f32; 3] {
        let e = self.exposure_ev();
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

/// A frame's measurements and adjustments; serialized into the DNG's XMP. They reproduce
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
    /// The last custom crop, kept while another crop is in use so it can be picked again.
    /// It never affects the conversion, and it stays out of the DNG.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_crop: Option<NRect>,
}

impl Settings {
    pub fn new(estimate: Estimate, adjust: Adjust) -> Self {
        Settings {
            model: MODEL_VERSION,
            estimate,
            hints: Hints::default(),
            adjust,
            custom_crop: None,
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
    let clipped = small
        .pixels()
        .map(|p| p[0].max(p[1]).max(p[2]) >= CLIPPED)
        .collect();
    dilate(clipped, w, h, w.max(h) / 200)
}

/// Grows a mask by `reach` pixels in each direction, as a square.
fn dilate(mut out: Vec<bool>, w: usize, h: usize, reach: usize) -> Vec<bool> {
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

/// Pixels of the film holder, with a margin around it, on an image from [`analysis_image`]: cells
/// denser than any film, connected to the scan's edge, so dense picture inside the frame stays.
/// `None` when the scan shows no holder, or the cells reach its center and so aren't a holder.
pub fn holder_pixels(small: &Rgb32, base_rgb: [f32; 3]) -> Option<Vec<bool>> {
    let (w, h) = (small.width, small.height);
    let (cols, rows) = (w / HOLDER_CELL, h / HOLDER_CELL);
    if cols == 0 || rows == 0 {
        return None;
    }
    // Density above base as a level: green below this is denser than the limit.
    let limit = base_rgb[1] * 10f32.powf(-HOLDER_DENSITY);
    let mut greens = Vec::with_capacity(HOLDER_CELL * HOLDER_CELL);
    let mut dense = vec![false; cols * rows];
    for row in 0..rows {
        for col in 0..cols {
            greens.clear();
            for y in row * HOLDER_CELL..(row + 1) * HOLDER_CELL {
                for x in col * HOLDER_CELL..(col + 1) * HOLDER_CELL {
                    greens.push(small.get(x, y)[1]);
                }
            }
            dense[row * cols + col] = median(&mut greens) < limit;
        }
    }
    let mut reached = vec![false; cols * rows];
    let mut stack: Vec<usize> = (0..cols * rows)
        .filter(|&i| {
            let (row, col) = (i / cols, i % cols);
            dense[i] && (row == 0 || col == 0 || row + 1 == rows || col + 1 == cols)
        })
        .collect();
    while let Some(i) = stack.pop() {
        if std::mem::replace(&mut reached[i], true) {
            continue;
        }
        let (row, col) = (i / cols, i % cols);
        if row > 0 && dense[i - cols] && !reached[i - cols] {
            stack.push(i - cols);
        }
        if row + 1 < rows && dense[i + cols] && !reached[i + cols] {
            stack.push(i + cols);
        }
        if col > 0 && dense[i - 1] && !reached[i - 1] {
            stack.push(i - 1);
        }
        if col + 1 < cols && dense[i + 1] && !reached[i + 1] {
            stack.push(i + 1);
        }
    }
    if !reached.contains(&true) {
        return None;
    }
    let center = NRect::centered(HOLDER_CLEAR_CENTER).to_pixels(cols, rows);
    let in_center = (center.top..center.bottom)
        .any(|row| (center.left..center.right).any(|col| reached[row * cols + col]));
    if in_center {
        return None;
    }
    let mut mask = vec![false; w * h];
    for row in 0..rows {
        for col in 0..cols {
            if reached[row * cols + col] {
                for y in row * HOLDER_CELL..(row + 1) * HOLDER_CELL {
                    mask[y * w + col * HOLDER_CELL..y * w + (col + 1) * HOLDER_CELL].fill(true);
                }
            }
        }
    }
    let margin = (w.max(h) as f32 * HOLDER_MARGIN).round() as usize;
    Some(dilate(mask, w, h, HOLDER_CELL + margin))
}

/// The pixels of [`holder_pixels`] as rectangles, for showing what the analysis ignores.
pub fn holder_areas(small: &Rgb32, base_rgb: [f32; 3]) -> Vec<NRect> {
    let Some(mask) = holder_pixels(small, base_rgb) else {
        return Vec::new();
    };
    let (w, h) = (small.width, small.height);
    let mut done = Vec::new();
    // Runs of the row above, as (left, right, top), which continue while a row repeats them.
    let mut open: Vec<(usize, usize, usize)> = Vec::new();
    for y in 0..=h {
        let mut runs = Vec::new();
        if y < h {
            let line = &mask[y * w..(y + 1) * w];
            let mut x = 0;
            while x < w {
                if line[x] {
                    let left = x;
                    while x < w && line[x] {
                        x += 1;
                    }
                    runs.push((left, x));
                } else {
                    x += 1;
                }
            }
        }
        let mut next = Vec::with_capacity(runs.len());
        for &(left, right, top) in &open {
            if runs.contains(&(left, right)) {
                next.push((left, right, top));
            } else {
                let rect = crate::image::Rect {
                    left,
                    top,
                    right,
                    bottom: y,
                };
                done.push(NRect::from_pixels(rect, w, h));
            }
        }
        for &(left, right) in &runs {
            if !next.iter().any(|&(l, r, _)| (l, r) == (left, right)) {
                next.push((left, right, y));
            }
        }
        open = next;
    }
    done
}

/// A film base read from one scan.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BaseReading {
    /// Linear camera RGB of the film base.
    pub rgb: [f32; 3],
    /// Where it was read: one even patch, free of holes, markings and picture.
    pub area: NRect,
    /// False when no patch looked like clear film, so the value is only a best guess.
    pub found: bool,
}

/// Largest rectangle of set cells in a grid, as (left, top, right, bottom) with exclusive
/// ends: for each row, the tallest bars ending there, walked with a stack of rising bars.
fn largest_rect(cells: &[bool], cols: usize, rows: usize) -> Option<(usize, usize, usize, usize)> {
    let mut heights = vec![0usize; cols + 1];
    let mut best = None;
    let mut best_area = 0;
    let mut stack: Vec<usize> = Vec::with_capacity(cols + 1);
    for row in 0..rows {
        for col in 0..cols {
            heights[col] = if cells[row * cols + col] {
                heights[col] + 1
            } else {
                0
            };
        }
        stack.clear();
        for col in 0..=cols {
            while let Some(&top) = stack.last() {
                if heights[top] <= heights[col] {
                    break;
                }
                stack.pop();
                let left = stack.last().map_or(0, |&l| l + 1);
                let area = heights[top] * (col - left);
                if area > best_area {
                    best_area = area;
                    best = Some((left, row + 1 - heights[top], col, row + 1));
                }
            }
            stack.push(col);
        }
    }
    best
}

/// Film base: clear film is the brightest thing on a negative, and it is even. So split the
/// scan into small cells, keep the even ones near the brightest level, and read the largest
/// rectangle of them. Holes, film markings and picture detail break a cell's evenness, and
/// the holder and exposed film are darker. Film edges come first, since an even bright patch
/// inside the picture may be a deep shadow; a strip across the scan can be the gap between
/// two frames.
pub fn detect_base(small: &Rgb32) -> BaseReading {
    let (w, h) = (small.width, small.height);
    let side = (w.max(h) / BASE_CELLS_LONG_SIDE).max(2);
    let (cols, rows) = (w / side, h / side);
    let near = near_clipped(small);
    let mut even = vec![false; cols * rows];
    let mut level = vec![0.0f32; cols * rows];
    let mut values: [Vec<f32>; 3] = std::array::from_fn(|_| Vec::with_capacity(side * side));
    for row in 0..rows {
        for col in 0..cols {
            let cell = row * cols + col;
            for v in &mut values {
                v.clear();
            }
            let mut touches_clipped = false;
            for y in row * side..(row + 1) * side {
                for x in col * side..(col + 1) * side {
                    let at = y * w + x;
                    touches_clipped |= near[at];
                    for (c, v) in values.iter_mut().enumerate() {
                        v.push(small.data[at * 3 + c]);
                    }
                }
            }
            if touches_clipped {
                continue;
            }
            let (mut sum, mut flat) = (0.0, true);
            for v in &mut values {
                let mid = median(v).max(1e-4);
                flat &= percentile(v, 90.0) - percentile(v, 10.0) < BASE_CELL_SPREAD * mid;
                sum += mid;
            }
            even[cell] = flat;
            level[cell] = sum / 3.0;
        }
    }
    let mut levels: Vec<f32> = (0..cols * rows)
        .filter(|&i| even[i])
        .map(|i| level[i])
        .collect();
    if levels.is_empty() {
        return BaseReading {
            rgb: channel_medians(small),
            area: NRect::FULL,
            found: false,
        };
    }
    let bright = percentile(&mut levels, BASE_LEVEL_PERCENTILE) * BASE_LEVEL_RANGE;
    let clear: Vec<bool> = (0..cols * rows)
        .map(|i| even[i] && level[i] >= bright)
        .collect();
    let band_cols = (cols as f32 * BASE_EDGE_BAND).ceil() as usize;
    let band_rows = (rows as f32 * BASE_EDGE_BAND).ceil() as usize;
    let at_edge: Vec<bool> = (0..cols * rows)
        .map(|i| {
            let (row, col) = (i / cols, i % cols);
            clear[i]
                && (row < band_rows
                    || row + band_rows >= rows
                    || col < band_cols
                    || col + band_cols >= cols)
        })
        .collect();
    let long_enough = |(l, t, r, b): (usize, usize, usize, usize), share: f32| {
        (r - l) as f32 >= share * cols as f32 || (b - t) as f32 >= share * rows as f32
    };
    let edge = largest_rect(&at_edge, cols, rows).filter(|&r| long_enough(r, BASE_MIN_EDGE_SHARE));
    let across = largest_rect(&clear, cols, rows);
    let (rect, found) = match (edge, across) {
        (Some(rect), _) => (rect, true),
        (None, Some(rect)) => (rect, long_enough(rect, BASE_MIN_GAP_SHARE)),
        (None, None) => unreachable!("the brightest even cell is always clear"),
    };
    let pixels = crate::image::Rect {
        left: rect.0 * side,
        top: rect.1 * side,
        right: rect.2 * side,
        bottom: rect.3 * side,
    };
    BaseReading {
        rgb: channel_medians(&small.crop(pixels)),
        area: NRect::from_pixels(pixels, w, h),
        found,
    }
}

/// Estimate film base, image area, density points and balance from a full-res scan.
pub fn estimate(scan: &Rgb32, hints: &Hints) -> Estimate {
    estimate_on(&analysis_image(scan), hints)
}

/// The downsampled copy estimates are computed on; callers may cache it.
pub fn analysis_image(scan: &Rgb32) -> Rgb32 {
    scan.downsample(ANALYSIS_FACTOR)
}

fn in_band(px: &[f32; 3], gray: &[GrayCurve; 2], band: f32) -> bool {
    (px[0] - gray[0].value(px[1])).abs() < band && (px[2] - gray[1].value(px[1])).abs() < band
}

/// Weighted fit of `bend, slope` to (x, y), pulled toward `prior`.
fn fit_pulled(points: &[(f32, f32)], prior: [f32; 2]) -> [f32; 2] {
    let w2 = GRAY_BIN_WEIGHT * GRAY_BIN_WEIGHT;
    let (mut aa, mut ab, mut bb, mut ra, mut rb) = (GRAY_PULL, 0.0, GRAY_PULL, 0.0, 0.0);
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

/// Gray curves of one frame from density-above-base pixels, without any reference: repeatedly
/// take the pixels near the current curves, and fit the median of each density bin.
fn fit_gray(density: &[[f32; 3]]) -> [GrayCurve; 2] {
    let dense: Vec<[f32; 3]> = density
        .iter()
        .filter(|p| p[1] > GRAY_MIN_DENSITY)
        .copied()
        .collect();
    let curve = |c: [f32; 2], top: f32| GrayCurve {
        bend: c[0],
        slope: c[1],
        top,
    };
    let mut gray = GRAY_PRIOR.map(|c| curve(c, 1.0));
    for band in GRAY_BANDS {
        let mut bins = vec![Vec::new(); GRAY_BIN_EDGES.len() - 1];
        for p in dense.iter().filter(|p| in_band(p, &gray, band)) {
            let i = GRAY_BIN_EDGES.partition_point(|&e| e <= p[1]);
            if (1..GRAY_BIN_EDGES.len()).contains(&i) {
                bins[i - 1].push(*p);
            }
        }
        let medians: Vec<[f32; 3]> = bins
            .iter()
            .filter(|b| b.len() >= GRAY_MIN_BIN_PIXELS)
            .map(|b| [0, 1, 2].map(|c| median(&mut b.iter().map(|p| p[c]).collect::<Vec<_>>())))
            .collect();
        if medians.len() < 4 {
            break;
        }
        let points = |c: usize| medians.iter().map(|m| (m[1], m[c])).collect::<Vec<_>>();
        gray = [
            curve(fit_pulled(&points(0), GRAY_PRIOR[0]), 1.0),
            curve(fit_pulled(&points(2), GRAY_PRIOR[1]), 1.0),
        ];
    }
    let band = GRAY_BANDS[GRAY_BANDS.len() - 1];
    let mut near: Vec<f32> = dense
        .iter()
        .filter(|p| in_band(p, &gray, band))
        .map(|p| p[1])
        .collect();
    let top = if near.len() > 200 {
        percentile(&mut near, 97.0)
    } else if !dense.is_empty() {
        percentile(&mut dense.iter().map(|p| p[1]).collect::<Vec<_>>(), 90.0)
    } else {
        1.0
    };
    gray.map(|g| GrayCurve { top, ..g })
}

/// `E` of a density-above-base pixel: gray curves, then crosstalk.
#[inline]
fn equivalent_density(d: [f32; 3], gray: &[GrayCurve; 2]) -> [f32; 3] {
    let green = d[1];
    let x = gray[0].equivalent(d[0]) - green;
    let y = gray[1].equivalent(d[2]) - green;
    let g = green + CROSSTALK[2][0] * x + CROSSTALK[2][1] * y;
    [
        g + CROSSTALK[0][0] * x + CROSSTALK[0][1] * y,
        g,
        g + CROSSTALK[1][0] * x + CROSSTALK[1][1] * y,
    ]
}

/// Exposure from the highlights and the scene average together: highlights alone misjudge
/// frames with small bright lights, the average alone misjudges low-key frames.
fn fit_anchor(density: &[[f32; 3]], gray: &[GrayCurve; 2]) -> f32 {
    anchor_of(
        density
            .iter()
            .map(|&d| (equivalent_density(d, gray)[1], d[1])),
    )
}

/// The anchor from each pixel's `E` and its density above base.
fn anchor_of(levels: impl Iterator<Item = (f32, f32)>) -> f32 {
    let mut e = Vec::new();
    // Bare film says nothing about the scene.
    let (mut sum, mut n) = (0.0f64, 0usize);
    for (level, above_base) in levels {
        e.push(level);
        if above_base > 0.03 {
            sum += level as f64;
            n += 1;
        }
    }
    let mean = if n > 0 { (sum / n as f64) as f32 } else { 0.0 };
    0.5 * (percentile(&mut e, 99.0) + mean) + ANCHOR_OFFSET
}

/// Gray curves of a black and white frame, which has none: every channel is itself.
const SILVER_GRAY: [GrayCurve; 2] = [GrayCurve {
    bend: 0.0,
    slope: 1.0,
    top: 1.0,
}; 2];

/// How much each channel counts in a black and white density: in proportion to the light it
/// gets through the base. Shot noise in density falls as the signal grows, so this weighs each
/// channel by the inverse of its noise variance.
fn silver_weights(base_rgb: [f32; 3]) -> [f32; 3] {
    let base = base_rgb.map(|b| b.max(1e-6));
    let sum: f32 = base.iter().sum();
    base.map(|b| b / sum)
}

/// Silver density of a pixel: its three densities above base, weighted. Dividing by the base
/// has already taken out the base's tint, fog and the scan's white balance.
#[inline]
fn silver_density(d: [f32; 3], weights: [f32; 3]) -> f32 {
    weights[0] * d[0] + weights[1] * d[1] + weights[2] * d[2]
}

fn fit_anchor_silver(density: &[[f32; 3]], base_rgb: [f32; 3]) -> f32 {
    let weights = silver_weights(base_rgb);
    anchor_of(density.iter().map(|&d| {
        let silver = silver_density(d, weights);
        (silver, silver)
    }))
}

/// Where a frame's measurements start: its film base, and density above it on the measured area.
struct Measured {
    base_rgb: [f32; 3],
    base_area: NRect,
    /// The base detection, when no base was picked.
    detected_base: Option<BaseReading>,
    area: NRect,
    /// Each measured pixel.
    density: Vec<[f32; 3]>,
    /// Each measured cell of `FILM_CELL` pixels a side, from its mean light. Cells cut by the
    /// holder, clipped light or the area's edge are left out.
    cells: Vec<[f32; 3]>,
}

fn measure(small: &Rgb32, hints: &Hints) -> Measured {
    let (w, h) = (small.width, small.height);

    let (base_rgb, base_area, detected_base) = match hints.base {
        Some(area) => (
            channel_medians(&small.crop(area.to_pixels(w, h))),
            area,
            None,
        ),
        None => {
            let detected = detect_base(small);
            (
                hints.base_rgb.unwrap_or(detected.rgb),
                detected.area,
                Some(detected),
            )
        }
    };
    let area = hints.analysis.unwrap_or_else(|| {
        NRect::centered(hints.area_percent.unwrap_or(DEFAULT_AREA_PERCENT) / 100.0)
    });

    // A drawn area is the user's word on where the picture is, so it's measured as drawn.
    let holder = match hints.analysis {
        Some(_) => None,
        None => holder_pixels(small, base_rgb),
    };
    let r = area.to_pixels(w, h);
    let row = r.right - r.left;
    // Which pixels of the area are film: not holder, not light through holes.
    let film: Vec<bool> = (r.top..r.bottom)
        .flat_map(|y| (r.left..r.right).map(move |x| (x, y)))
        .map(|(x, y)| {
            let p = small.get(x, y);
            let held = holder.as_ref().is_some_and(|mask| mask[y * w + x]);
            !held && p[0].max(p[1]).max(p[2]) < CLIPPED
        })
        .collect();
    let is_film = |x: usize, y: usize| film[(y - r.top) * row + (x - r.left)];
    let above_base = |p: [f32; 3]| [0, 1, 2].map(|c| density_above_base(p[c], base_rgb[c]));
    let density = (r.top..r.bottom)
        .flat_map(|y| (r.left..r.right).map(move |x| (x, y)))
        .filter(|&(x, y)| is_film(x, y))
        .map(|(x, y)| above_base(small.get(x, y)))
        .collect();
    let mut cells = Vec::new();
    for top in (r.top..r.bottom.saturating_sub(FILM_CELL - 1)).step_by(FILM_CELL) {
        'cell: for left in (r.left..r.right.saturating_sub(FILM_CELL - 1)).step_by(FILM_CELL) {
            let mut sum = [0.0f32; 3];
            for y in top..top + FILM_CELL {
                for x in left..left + FILM_CELL {
                    if !is_film(x, y) {
                        continue 'cell;
                    }
                    let p = small.get(x, y);
                    for c in 0..3 {
                        sum[c] += p[c];
                    }
                }
            }
            let n = (FILM_CELL * FILM_CELL) as f32;
            cells.push(above_base(sum.map(|v| v / n)));
        }
    }
    Measured {
        base_rgb,
        base_area,
        detected_base,
        area,
        density,
        cells,
    }
}

/// How far a frame strays from black and white, from cells of density above base: how far they
/// lie from the frame's own gray curve (90th percentile, in density), and the slope of red
/// against green density along that curve. The curve runs straight between the median cells of
/// each density bin. `None` when the frame spans too little density to tell.
fn film_reading(cells: &[[f32; 3]]) -> Option<(f32, f32)> {
    let mut bins = vec![Vec::new(); FILM_BIN_COUNT];
    for cell in cells {
        let bin = ((cell[1] - FILM_MIN_DENSITY) / FILM_BIN_WIDTH).floor();
        if bin >= 0.0 && (bin as usize) < FILM_BIN_COUNT {
            bins[bin as usize].push(*cell);
        }
    }
    let knots: Vec<[f32; 3]> = bins
        .iter()
        .filter(|b| b.len() >= FILM_MIN_BIN_CELLS)
        .map(|b| [0, 1, 2].map(|c| median(&mut b.iter().map(|p| p[c]).collect::<Vec<_>>())))
        .collect();
    if knots.len() < FILM_MIN_BINS {
        return None;
    }
    // Least squares through the base: red = slope · green.
    let (rg, gg) = knots.iter().fold((0.0, 0.0), |(rg, gg), k| {
        (rg + k[0] * k[1], gg + k[1] * k[1])
    });
    let on_curve = |g: f32| {
        let i = knots
            .partition_point(|k| k[1] < g)
            .clamp(1, knots.len() - 1);
        let (a, b) = (knots[i - 1], knots[i]);
        let t = (g - a[1]) / (b[1] - a[1]).max(1e-6);
        [a[0] + (b[0] - a[0]) * t, a[2] + (b[2] - a[2]) * t]
    };
    let (lowest, highest) = (knots[0][1], knots[knots.len() - 1][1]);
    let mut off: Vec<f32> = cells
        .iter()
        .filter(|c| (lowest..=highest).contains(&c[1]))
        .map(|c| {
            let [red, blue] = on_curve(c[1]);
            (c[0] - red).hypot(c[2] - blue)
        })
        .collect();
    Some((percentile(&mut off, 90.0), rg / gg))
}

/// Film type from measured cells. Density above base makes it independent of the scan's white
/// balance. A color negative strays from its gray curve wherever the scene has color, and its
/// dye layers rise at different rates even where it hasn't; black and white does neither. A frame
/// that can't tell is color, the conversion that loses nothing.
fn film_of(cells: &[[f32; 3]]) -> Film {
    match film_reading(cells) {
        Some((spread, red_slope))
            if spread < FILM_BW_SPREAD && (red_slope - 1.0).abs() < FILM_BW_RED_SLOPE =>
        {
            Film::Bw
        }
        _ => Film::Color,
    }
}

/// What a frame reads as, whatever its settings choose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameReading {
    /// The detected film type.
    pub film: Film,
    /// Whether the scan shows clear film for the base.
    pub base_found: bool,
}

impl Measured {
    fn reading(&self, small: &Rgb32) -> FrameReading {
        FrameReading {
            film: film_of(&self.cells),
            base_found: self
                .detected_base
                .map_or_else(|| detect_base(small).found, |b| b.found),
        }
    }
}

/// What a frame on an image from [`analysis_image`] reads as, without fitting an estimate.
pub fn read_frame(small: &Rgb32, hints: &Hints) -> FrameReading {
    measure(small, hints).reading(small)
}

/// [`estimate_on`], with what the frame reads as, in one pass. `keep` is the film type to convert
/// as when `hints` doesn't choose one: a frame measured again keeps its type, so changing an area
/// or the angle can't switch its conversion. `None` takes the detected type, as a frame's first
/// measurement does.
pub fn measure_frame(small: &Rgb32, hints: &Hints, keep: Option<Film>) -> (Estimate, FrameReading) {
    let m = measure(small, hints);
    let reading = m.reading(small);
    let film = hints.film.or(keep).unwrap_or(reading.film);
    let (gray, anchor) = match film {
        Film::Color => {
            let gray = fit_gray(&m.density);
            (gray, fit_anchor(&m.density, &gray))
        }
        Film::Bw => (SILVER_GRAY, fit_anchor_silver(&m.density, m.base_rgb)),
    };
    let estimate = Estimate {
        film,
        base_rgb: m.base_rgb,
        base_area: m.base_area,
        area: m.area,
        gray,
        anchor,
    };
    (estimate, reading)
}

/// [`estimate`] on an image from [`analysis_image`], detecting the film type unless `hints`
/// chooses it.
pub fn estimate_on(small: &Rgb32, hints: &Hints) -> Estimate {
    measure_frame(small, hints, None).0
}

/// One film base for a roll from its frames' bases: stock and processing don't change from
/// frame to frame, and the median outvotes a frame whose base was misread. Gray curves stay
/// per frame, which matched finished scans better than curves pooled over a roll.
pub fn roll_base(frame_bases: &[[f32; 3]]) -> [f32; 3] {
    [0, 1, 2].map(|c| median(&mut frame_bases.iter().map(|b| b[c]).collect::<Vec<_>>()))
}

/// Whether a frame's own base is further from its roll's than one roll's frames should be.
pub fn differs_from_roll(frame_base: [f32; 3], roll: [f32; 3]) -> bool {
    (0..3).any(|c| (frame_base[c] / roll[c].max(1e-6) - 1.0).abs() > ROLL_BASE_TOLERANCE)
}

/// Scene-linear positive for one pixel of linear camera RGB, before saturation. Black and white
/// comes out the same in all three channels, with only exposure applied.
#[inline]
pub fn positive(rgb: [f32; 3], e: &Estimate, a: &Adjust) -> [f32; 3] {
    positive_with(rgb, e, gains_ev(e.film, a), silver_weights(e.base_rgb))
}

/// Per-channel gains in stops from the sliders, as a film type takes them: black and white takes
/// exposure only.
fn gains_ev(film: Film, a: &Adjust) -> [f32; 3] {
    match film {
        Film::Color => a.gains_ev(),
        Film::Bw => [a.exposure_ev(); 3],
    }
}

/// [`positive`] with its per-frame constants already worked out, for converting whole images.
#[inline]
fn positive_with(rgb: [f32; 3], e: &Estimate, gains_ev: [f32; 3], weights: [f32; 3]) -> [f32; 3] {
    let d = [0, 1, 2].map(|c| density_above_base(rgb[c], e.base_rgb[c]));
    let linear = |level: f32, gain_ev: f32| {
        let toe = TOE_AMOUNT * (-level.max(TOE_FLOOR) / TOE_SCALE).exp();
        10f32.powf((level - toe - e.anchor) / GAMMA_REF + gain_ev * LOG10_2)
    };
    match e.film {
        Film::Color => {
            let eq = equivalent_density(d, &e.gray);
            [0, 1, 2].map(|c| linear(eq[c], gains_ev[c]))
        }
        Film::Bw => [linear(silver_density(d, weights), gains_ev[1]); 3],
    }
}

/// Linear saturation around Rec.2020 luminance; negative results are clipped to 0.
#[inline]
pub fn saturate(p: [f32; 3], gain: f32) -> [f32; 3] {
    let y = p[0] * REC2020_LUMA[0] + p[1] * REC2020_LUMA[1] + p[2] * REC2020_LUMA[2];
    p.map(|v| (y + gain * (v - y)).max(0.0))
}

/// The positive to judge clipping on: the crop or measured area, with any detected holder turned
/// to clear film. The holder no longer sets the exposure, so it would count as clipped picture.
pub fn picture_for_clipping(scan: &Rgb32, s: &Settings) -> Rgb32 {
    let mut straight = scan.straighten(s.adjust.straighten);
    if s.hints.analysis.is_none() {
        if let Some(holder) = holder_pixels(&straight, s.estimate.base_rgb) {
            let (pixels, _) = straight.data.as_chunks_mut::<3>();
            for (pixel, &is_holder) in pixels.iter_mut().zip(&holder) {
                if is_holder {
                    *pixel = s.estimate.base_rgb;
                }
            }
        }
    }
    let mut level = s.clone();
    level.adjust.straighten = 0.0;
    convert_with(
        &straight,
        &level,
        Some(s.adjust.crop.unwrap_or(s.estimate.area)),
    )
}

/// The positive as it will be written: straighten, crop, invert, saturate, mirror, rotate.
/// `crop_override` replaces the output crop (e.g. `Some(NRect::FULL)` for a full view).
pub fn convert_with(scan: &Rgb32, s: &Settings, crop_override: Option<NRect>) -> Rgb32 {
    let a = &s.adjust;
    let straight = scan.straighten(a.straighten);
    let crop = crop_override.or(a.crop).unwrap_or(NRect::FULL);
    let mut img = straight.crop(crop.to_pixels(straight.width, straight.height));
    let gain = a.saturation_gain();
    let (gains, weights) = (
        gains_ev(s.estimate.film, a),
        silver_weights(s.estimate.base_rgb),
    );
    for_each_part(&mut img.data, 3, |_, part| {
        for px in part.as_chunks_mut::<3>().0 {
            *px = saturate(positive_with(*px, &s.estimate, gains, weights), gain);
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
    const CURVES: [GrayCurve; 2] = [
        GrayCurve {
            bend: -0.15,
            slope: 0.9,
            top: 1.2,
        },
        GrayCurve {
            bend: 0.25,
            slope: 0.85,
            top: 1.2,
        },
    ];

    fn est() -> Estimate {
        Estimate {
            film: Film::Color,
            base_rgb: BASE,
            base_area: NRect::FULL,
            area: NRect::FULL,
            gray: CURVES,
            anchor: 0.7,
        }
    }

    /// Film with varied neutral density in the middle of a scan, and holder on the sides.
    fn scan_in_holder(holder: bool) -> Rgb32 {
        let mut scan = Rgb32::new(400, 300);
        for y in 0..300 {
            for x in 0..400 {
                let side = !(80..320).contains(&x);
                let d = if holder && side {
                    2.2
                } else {
                    0.2 + 0.9 * (((x * 7 + y * 13) % 97) as f32 / 96.0)
                };
                let px = pixel([d, d, d]);
                scan.data[(y * 400 + x) * 3..(y * 400 + x) * 3 + 3].copy_from_slice(&px);
            }
        }
        scan
    }

    fn with_base() -> Hints {
        Hints {
            base_rgb: Some(BASE),
            ..Hints::default()
        }
    }

    #[test]
    fn a_holder_doesnt_set_the_exposure_and_a_drawn_area_overrides_it() {
        let scan = scan_in_holder(true);
        let mask = holder_pixels(&scan, BASE).unwrap();
        assert!(mask[150 * 400 + 5] && mask[150 * 400 + 395], "holder");
        assert!(!mask[150 * 400 + 200], "picture");

        // Measured around the holder, as if only the film were in the scan.
        let film = NRect::from_pixels(
            crate::image::Rect {
                left: 90,
                top: 23,
                right: 310,
                bottom: 277,
            },
            400,
            300,
        );
        let drawn = |analysis| Hints {
            analysis: Some(analysis),
            ..with_base()
        };
        let ignored = estimate_on(&scan, &with_base()).anchor;
        assert!((ignored - estimate_on(&scan, &drawn(film)).anchor).abs() < 0.02);

        // A drawn area is measured as drawn, holder included.
        assert!(estimate_on(&scan, &drawn(NRect::FULL)).anchor > ignored + 0.3);
    }

    #[test]
    fn a_scan_without_a_holder_measures_as_before() {
        let scan = scan_in_holder(false);
        assert_eq!(holder_pixels(&scan, BASE), None);
        assert_eq!(holder_areas(&scan, BASE), Vec::new());
    }

    #[test]
    fn dense_picture_is_not_a_holder() {
        // Inside the frame, away from the edge.
        let mut scan = scan_in_holder(false);
        for y in 100..200 {
            for x in 150..250 {
                let px = pixel([2.2, 2.2, 2.2]);
                scan.data[(y * 400 + x) * 3..(y * 400 + x) * 3 + 3].copy_from_slice(&px);
            }
        }
        assert_eq!(holder_pixels(&scan, BASE), None);
        // Touching the edge and running into the center: picture on a scan with no holder.
        for y in 140..160 {
            for x in 0..250 {
                let px = pixel([2.2, 2.2, 2.2]);
                scan.data[(y * 400 + x) * 3..(y * 400 + x) * 3 + 3].copy_from_slice(&px);
            }
        }
        assert_eq!(holder_pixels(&scan, BASE), None);
    }

    #[test]
    fn holder_areas_cover_exactly_the_holder_pixels() {
        let scan = scan_in_holder(true);
        let mask = holder_pixels(&scan, BASE).unwrap();
        let areas = holder_areas(&scan, BASE);
        assert!(!areas.is_empty() && areas.len() < 20);
        let mut drawn = vec![false; mask.len()];
        for area in areas {
            let r = area.to_pixels(400, 300);
            for y in r.top..r.bottom {
                drawn[y * 400 + r.left..y * 400 + r.right].fill(true);
            }
        }
        assert!(drawn == mask);
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

    #[test]
    fn a_detected_holder_isnt_judged_as_clipped_picture() {
        let scan = scan_in_holder(true);
        let hints = with_base();
        let mut s = Settings::new(estimate_on(&scan, &hints), Adjust::default());
        s.hints = hints;
        let brightest = |img: &Rgb32| img.data.iter().cloned().fold(0.0f32, f32::max);
        let holder_kept = convert_with(&scan, &s, Some(s.estimate.area));
        let judged = picture_for_clipping(&scan, &s);
        assert!(brightest(&judged) < brightest(&holder_kept));

        // A drawn area is measured as drawn, and judged as drawn.
        s.hints.analysis = Some(s.estimate.area);
        assert_eq!(picture_for_clipping(&scan, &s).data, holder_kept.data);
    }

    #[test]
    fn a_drawn_base_reads_the_straightened_scan_it_was_drawn_on() {
        let (side, off_center, degrees) = (400usize, 150.0f32, 10.0f32);
        let center = (side as f32 - 1.0) / 2.0;
        let mut scan = Rgb32::new(side, side);
        scan.data.fill(0.2);
        for y in 190..210 {
            for x in 340..360 {
                scan.data[(y * side + x) * 3..(y * side + x + 1) * 3].fill(0.8);
            }
        }
        // Where straightening moves the patch: its offset, turned by the angle.
        let (s, c) = degrees.to_radians().sin_cos();
        let (x, y) = (
            (center + c * off_center) / side as f32,
            (center + s * off_center) / side as f32,
        );
        let hints = Hints {
            base: Some(NRect {
                x0: x - 0.01,
                y0: y - 0.01,
                x1: x + 0.01,
                y1: y + 0.01,
            }),
            ..Hints::default()
        };
        assert!((estimate_on(&scan.straighten(degrees), &hints).base_rgb[1] - 0.8).abs() < 0.01);
        assert!((estimate_on(&scan, &hints).base_rgb[1] - 0.2).abs() < 0.01);
    }

    fn pixel(d: [f32; 3]) -> [f32; 3] {
        [0, 1, 2].map(|c| BASE[c] * 10f32.powf(-d[c]))
    }

    /// A neutral of green density `g` on the given curves.
    fn neutral(g: f32, gray: &[GrayCurve; 2]) -> [f32; 3] {
        [gray[0].value(g), g, gray[1].value(g)]
    }

    #[test]
    fn gray_curve_inverts_and_continues_past_its_ends() {
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
        let straight = GrayCurve {
            bend: 0.0,
            slope: 0.8,
            top: 1.0,
        };
        assert!((straight.equivalent(0.4) - 0.5).abs() < 1e-5);
    }

    #[test]
    fn falling_curve_stays_invertible_past_its_turning_point() {
        let curve = GrayCurve {
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
                pixel(neutral(i as f32 * 0.1, &e.gray)),
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
            |g: f32, e: &Estimate| positive(pixel(neutral(g, &e.gray)), e, &Adjust::default())[1];
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
        let gray = saturate([0.4, 0.4, 0.4], 2.0);
        assert!(gray.iter().all(|v| (v - 0.4).abs() < 1e-6));
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
    fn estimate_finds_base_and_measures_the_center_on_synthetic_scan() {
        let e = estimate(&synthetic_scan(), &Hints::default());
        for (got, want) in e.base_rgb.iter().zip(BASE) {
            assert!((got - want).abs() < 0.01, "base {:?}", e.base_rgb);
        }
        assert_eq!(e.area, NRect::centered(DEFAULT_AREA_PERCENT / 100.0));
        let hints = Hints {
            area_percent: Some(60.0),
            ..Hints::default()
        };
        assert_eq!(
            estimate(&synthetic_scan(), &hints).area,
            NRect::centered(0.6)
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
    fn fit_recovers_the_gray_curve_of_a_neutral_scene() {
        let e = estimate_on(&synthetic_scan(), &Hints::default());
        for (got, want) in e.gray.iter().zip(CURVES) {
            for g in [0.3, 0.6, 0.9] {
                assert!(
                    (got.value(g) - want.value(g)).abs() < 0.02,
                    "{got:?} at {g}"
                );
            }
        }
    }

    #[test]
    fn a_dominant_color_does_not_capture_the_fit() {
        // Mostly one strong color, with far fewer grays: a meadow under a small sky.
        let meadow = (0..12000).map(|i| {
            let d = neutral(0.2 + 0.9 * (i % 100) as f32 / 100.0, &CURVES);
            [d[0] + 0.25, d[1], d[2] - 0.25]
        });
        let sky = (0..2000).map(|i| neutral(0.5 + 0.65 * (i % 100) as f32 / 100.0, &CURVES));
        let gray = fit_gray(&meadow.chain(sky).collect::<Vec<_>>());
        for (got, want) in gray.iter().zip(CURVES) {
            let (got, want) = (got.value(1.0), want.value(1.0));
            assert!((got - want).abs() < 0.03, "{got} {want}");
        }
    }

    #[test]
    fn fit_falls_back_to_the_prior_without_evidence() {
        for density in [Vec::new(), vec![[0.01, 0.01, 0.01]; 500]] {
            let gray = fit_gray(&density);
            assert_eq!([gray[0].bend, gray[0].slope], GRAY_PRIOR[0]);
            assert_eq!([gray[1].bend, gray[1].slope], GRAY_PRIOR[1]);
            assert!(fit_anchor(&density, &gray).is_finite() || density.is_empty());
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
    fn largest_rect_finds_the_biggest_block() {
        #[rustfmt::skip]
        let cells = [
            true,  false, false, false,
            false, true,  true,  true,
            false, true,  true,  true,
            false, false, true,  false,
        ];
        assert_eq!(largest_rect(&cells, 4, 4), Some((1, 1, 4, 3)));
        assert_eq!(largest_rect(&[false; 4], 2, 2), None);
    }

    /// A strip of film: clear edges with holes and markings, picture between them.
    fn film_strip() -> (Rgb32, Vec<NRect>) {
        let (w, h) = (600, 400);
        let mut scan = Rgb32::new(w, h);
        let mut flaws = Vec::new();
        for y in 0..h {
            for x in 0..w {
                // Detail everywhere in the picture, which is denser than clear film.
                let detail = ((x / 3 + y / 3) % 2) as f32 * 0.5;
                let d = if (60..340).contains(&y) {
                    [0.4 + detail; 3]
                } else {
                    [0.0; 3]
                };
                let i = (y * w + x) * 3;
                scan.data[i..i + 3].copy_from_slice(&pixel(d));
            }
        }
        let mut paint = |x0: usize, y0: usize, x1: usize, y1: usize, v: [f32; 3]| {
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = (y * w + x) * 3;
                    scan.data[i..i + 3].copy_from_slice(&v);
                }
            }
            flaws.push(NRect::from_pixels(
                crate::image::Rect {
                    left: x0,
                    top: y0,
                    right: x1,
                    bottom: y1,
                },
                w,
                h,
            ));
        };
        // Sprocket holes along the top, and film markings printed along the bottom edge.
        for x in (20..600).step_by(60) {
            paint(x, 15, x + 24, 40, [1.0; 3]);
        }
        for x in (40..560).step_by(120) {
            paint(x, 350, x + 60, 375, pixel([0.7, 0.9, 1.0]));
        }
        (scan, flaws)
    }

    #[test]
    fn the_base_comes_from_even_film_clear_of_holes_and_markings() {
        let (scan, flaws) = film_strip();
        let read = detect_base(&scan);
        assert!(read.found);
        for (got, want) in read.rgb.iter().zip(BASE) {
            assert!((got - want).abs() < 0.005, "{:?}", read.rgb);
        }
        let a = read.area;
        for f in flaws {
            let apart = a.x1 <= f.x0 || f.x1 <= a.x0 || a.y1 <= f.y0 || f.y1 <= a.y0;
            assert!(apart, "{a:?} overlaps {f:?}");
        }
    }

    #[test]
    fn a_scan_without_clear_film_says_so() {
        let (w, h) = (400, 300);
        let mut scan = Rgb32::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let d = 0.2 + ((x * 7 + y * 13) % 11) as f32 * 0.08;
                let i = (y * w + x) * 3;
                scan.data[i..i + 3].copy_from_slice(&pixel([d; 3]));
            }
        }
        assert!(!detect_base(&scan).found);
    }

    #[test]
    fn a_frame_far_from_its_roll_is_noticed() {
        let roll = [0.3, 0.4, 0.25];
        assert!(!differs_from_roll([0.306, 0.396, 0.252], roll));
        assert!(differs_from_roll([0.3, 0.45, 0.25], roll));
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

    /// Pseudo-random in −1…1, the same on every run.
    fn noise(x: usize, y: usize, c: usize) -> f32 {
        let n = (x * 73_856_093) ^ (y * 19_349_663) ^ (c * 83_492_791);
        (n % 2001) as f32 / 1000.0 - 1.0
    }

    /// 400x300: clear film around a frame whose density is `scene` at (x, y), each channel then
    /// scaled by `gains`, as a scan's white balance would. Sensor noise differs per channel.
    fn scan_of(scene: impl Fn(usize, usize) -> [f32; 3], gains: [f32; 3]) -> Rgb32 {
        let (w, h) = (400, 300);
        let mut scan = Rgb32::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let inside = (40..360).contains(&x) && (30..270).contains(&y);
                let d = if inside { scene(x, y) } else { [0.0; 3] };
                let d = [0, 1, 2].map(|c| d[c] + 0.01 * noise(x, y, c));
                let px = pixel(d);
                let i = (y * w + x) * 3;
                for c in 0..3 {
                    scan.data[i + c] = px[c] * gains[c];
                }
            }
        }
        scan
    }

    /// Silver density across the frame, slightly warmer in blue than red as silver can be.
    fn silver(x: usize, y: usize) -> [f32; 3] {
        let d = 0.15 + 0.95 * (x - 40) as f32 / 320.0 + 0.08 * (y as f32 / 9.0).sin();
        [0.99 * d, d, 1.02 * d]
    }

    const SCAN_GAINS: [[f32; 3]; 3] = [[1.0; 3], [2.1, 1.0, 0.55], [0.6, 1.0, 1.8]];

    #[test]
    fn black_and_white_is_detected_whatever_the_white_balance() {
        for gains in SCAN_GAINS {
            let scan = scan_of(silver, gains);
            let e = estimate_on(&scan, &Hints::default());
            assert_eq!(e.film, Film::Bw, "{gains:?}");
            assert_eq!(read_frame(&scan, &Hints::default()).film, Film::Bw);
        }
    }

    #[test]
    fn color_film_stays_color_even_on_a_gray_scene() {
        // Only grays, but on color film's gray curves: its dye layers rise at different rates.
        for gains in SCAN_GAINS {
            let gray_scene = scan_of(
                |x, _| neutral(0.2 + 0.8 * (x - 40) as f32 / 320.0, &CURVES),
                gains,
            );
            assert_eq!(
                estimate_on(&gray_scene, &Hints::default()).film,
                Film::Color,
                "{gains:?}"
            );
        }
        // Colored stripes across a neutral scene.
        let colorful = scan_of(
            |x, y| {
                let d = neutral(0.2 + 0.8 * (x - 40) as f32 / 320.0, &CURVES);
                if y % 40 < 14 {
                    [d[0] + 0.2, d[1], d[2] - 0.2]
                } else {
                    d
                }
            },
            [1.0; 3],
        );
        assert_eq!(estimate_on(&colorful, &Hints::default()).film, Film::Color);
    }

    #[test]
    fn a_frame_that_cant_tell_stays_color() {
        let blank = scan_of(|_, _| [0.5; 3], [1.0; 3]);
        assert_eq!(estimate_on(&blank, &Hints::default()).film, Film::Color);
    }

    #[test]
    fn a_chosen_film_type_overrides_detection() {
        let bw = scan_of(silver, [1.0; 3]);
        let as_color = Hints {
            film: Some(Film::Color),
            ..Hints::default()
        };
        assert_eq!(estimate_on(&bw, &as_color).film, Film::Color);
        assert_eq!(read_frame(&bw, &as_color).film, Film::Bw);
        let as_bw = Hints {
            film: Some(Film::Bw),
            ..Hints::default()
        };
        assert_eq!(estimate_on(&synthetic_scan(), &as_bw).film, Film::Bw);
    }

    #[test]
    fn measuring_again_keeps_the_film_type_unless_the_hints_choose() {
        let bw = scan_of(silver, [1.0; 3]);
        // Measured before as color, say with another area: it stays color, and says what it reads as.
        let (e, reading) = measure_frame(&bw, &Hints::default(), Some(Film::Color));
        assert_eq!((e.film, reading.film), (Film::Color, Film::Bw));
        assert!(reading.base_found);
        assert_eq!(
            e,
            estimate_on(
                &bw,
                &Hints {
                    film: Some(Film::Color),
                    ..Hints::default()
                }
            )
        );
        let as_bw = Hints {
            film: Some(Film::Bw),
            ..Hints::default()
        };
        assert_eq!(
            measure_frame(&bw, &as_bw, Some(Film::Color)).0.film,
            Film::Bw
        );
        assert_eq!(measure_frame(&bw, &Hints::default(), None).0.film, Film::Bw);
        assert_eq!(read_frame(&bw, &as_bw), reading);
    }

    #[test]
    fn black_and_white_is_gray_and_takes_only_exposure() {
        for gains in SCAN_GAINS {
            let scan = scan_of(silver, gains);
            let s = Settings::new(estimate_on(&scan, &Hints::default()), Adjust::default());
            let out = convert(&scan, &s);
            assert!(out.pixels().all(|p| p[0] == p[1] && p[1] == p[2]));
            // Denser film is a brighter positive.
            let at = |x: usize| out.get(x, 150)[1];
            assert!(at(330) > at(200) && at(200) > at(70));

            let colored = Adjust {
                temperature: 100.0,
                tint: -60.0,
                saturation: 80.0,
                ..Adjust::default()
            };
            let px = scan.get(200, 150);
            assert_eq!(
                positive(px, &s.estimate, &colored),
                positive(px, &s.estimate, &s.adjust)
            );
            let brighter = Adjust {
                exposure: 100.0 / EXPOSURE_EV_RANGE,
                ..Adjust::default()
            };
            let ratio =
                positive(px, &s.estimate, &brighter)[0] / positive(px, &s.estimate, &s.adjust)[0];
            assert!((ratio - 2.0).abs() < 1e-3, "{ratio}");
        }
    }

    #[test]
    fn black_and_white_doesnt_depend_on_the_scans_white_balance() {
        let convert_with_gains = |gains| {
            let scan = scan_of(silver, gains);
            let s = Settings::new(estimate_on(&scan, &Hints::default()), Adjust::default());
            convert(&scan, &s)
        };
        // Each channel's noise counts a little differently, so compare the frame as a whole.
        let mean = |img: &Rgb32| img.data.iter().sum::<f32>() / img.data.len() as f32;
        let plain = mean(&convert_with_gains([1.0; 3]));
        for gains in SCAN_GAINS {
            let other = mean(&convert_with_gains(gains));
            assert!(
                (other / plain - 1.0).abs() < 0.01,
                "{gains:?}: {plain} {other}"
            );
        }
    }

    #[test]
    fn settings_from_before_black_and_white_read_as_color() {
        let mut json = serde_json::to_value(Settings::new(est(), Adjust::default())).unwrap();
        json["estimate"].as_object_mut().unwrap().remove("film");
        json["hints"].as_object_mut().unwrap().remove("film");
        json["adjust"]["look"] = "vivid".into();
        let old: Settings = serde_json::from_value(json).unwrap();
        assert_eq!(old.estimate.film, Film::Color);
        assert_eq!(old.hints.film, None);
        assert_eq!(old.adjust.look, Look::Hard);
        assert_eq!(serde_json::to_value(Film::Bw).unwrap(), "bw");
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
