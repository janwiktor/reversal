//! Interleaved floating-point RGB image buffer.

/// Linear RGB image, interleaved, row-major. Values are relative to sensor white (1.0).
#[derive(Clone, Debug)]
pub struct Rgb32 {
    pub width: usize,
    pub height: usize,
    pub data: Vec<f32>,
}

/// Pixel rectangle: rows `top..bottom`, columns `left..right`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Rect {
    pub top: usize,
    pub bottom: usize,
    pub left: usize,
    pub right: usize,
}

impl Rect {
    pub fn scale(self, factor: usize) -> Rect {
        Rect {
            top: self.top * factor,
            bottom: self.bottom * factor,
            left: self.left * factor,
            right: self.right * factor,
        }
    }

    pub fn clamp(self, width: usize, height: usize) -> Rect {
        Rect {
            top: self.top.min(height),
            bottom: self.bottom.min(height).max(self.top.min(height)),
            left: self.left.min(width),
            right: self.right.min(width).max(self.left.min(width)),
        }
    }
}

/// Rectangle in normalized image coordinates (0..1, origin top-left), so the same
/// region applies to the full scan and to any downsampled copy of it.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NRect {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl NRect {
    pub const FULL: NRect = NRect {
        x0: 0.0,
        y0: 0.0,
        x1: 1.0,
        y1: 1.0,
    };

    /// Centered rectangle covering `share` (0.1…1) of each side.
    pub fn centered(share: f32) -> NRect {
        let margin = (1.0 - share.clamp(0.1, 1.0)) / 2.0;
        NRect {
            x0: margin,
            y0: margin,
            x1: 1.0 - margin,
            y1: 1.0 - margin,
        }
    }

    /// Pixel rectangle in an image of the given size (at least 1×1).
    pub fn to_pixels(self, width: usize, height: usize) -> Rect {
        let (w, h) = (width as f32, height as f32);
        let x0 = self.x0.min(self.x1).clamp(0.0, 1.0);
        let x1 = self.x0.max(self.x1).clamp(0.0, 1.0);
        let y0 = self.y0.min(self.y1).clamp(0.0, 1.0);
        let y1 = self.y0.max(self.y1).clamp(0.0, 1.0);
        let left = ((x0 * w).round() as usize).min(width.saturating_sub(1));
        let top = ((y0 * h).round() as usize).min(height.saturating_sub(1));
        Rect {
            top,
            bottom: ((y1 * h).round() as usize).clamp(top + 1, height.max(top + 1)),
            left,
            right: ((x1 * w).round() as usize).clamp(left + 1, width.max(left + 1)),
        }
    }

    pub fn from_pixels(r: Rect, width: usize, height: usize) -> NRect {
        let (w, h) = (width as f32, height as f32);
        NRect {
            x0: r.left as f32 / w,
            y0: r.top as f32 / h,
            x1: r.right as f32 / w,
            y1: r.bottom as f32 / h,
        }
    }
}

impl Rgb32 {
    /// Rotate by `degrees` (clockwise positive) about the center, same canvas size,
    /// bilinear sampling; corners outside the source are filled from the nearest edge.
    pub fn straighten(&self, degrees: f32) -> Rgb32 {
        if degrees.abs() < 1e-3 {
            return self.clone();
        }
        let (w, h) = (self.width, self.height);
        let (s, c) = degrees.to_radians().sin_cos();
        let (cx, cy) = ((w as f32 - 1.0) / 2.0, (h as f32 - 1.0) / 2.0);
        let mut out = Rgb32::new(w, h);
        for y in 0..h {
            for x in 0..w {
                // Inverse rotation: where does this output pixel come from?
                let (dx, dy) = (x as f32 - cx, y as f32 - cy);
                let sx = (c * dx + s * dy + cx).clamp(0.0, (w - 1) as f32);
                let sy = (-s * dx + c * dy + cy).clamp(0.0, (h - 1) as f32);
                let (x0, y0) = (sx.floor() as usize, sy.floor() as usize);
                let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
                let (fx, fy) = (sx - x0 as f32, sy - y0 as f32);
                let (a, b, cc, d) = (
                    self.get(x0, y0),
                    self.get(x1, y0),
                    self.get(x0, y1),
                    self.get(x1, y1),
                );
                let o = (y * w + x) * 3;
                for k in 0..3 {
                    let top = a[k] + (b[k] - a[k]) * fx;
                    let bot = cc[k] + (d[k] - cc[k]) * fx;
                    out.data[o + k] = top + (bot - top) * fy;
                }
            }
        }
        out
    }
}

impl Rgb32 {
    pub fn new(width: usize, height: usize) -> Self {
        Rgb32 {
            width,
            height,
            data: vec![0.0; width * height * 3],
        }
    }

    pub fn pixels(&self) -> impl Iterator<Item = [f32; 3]> + '_ {
        self.data.as_chunks::<3>().0.iter().copied()
    }

    #[inline]
    pub fn get(&self, x: usize, y: usize) -> [f32; 3] {
        let i = (y * self.width + x) * 3;
        [self.data[i], self.data[i + 1], self.data[i + 2]]
    }

    /// Box-filter downsample by an integer factor (drops partial edge blocks).
    /// Each side keeps at least one pixel, so very thin images stay valid.
    pub fn downsample(&self, factor: usize) -> Rgb32 {
        if factor <= 1 || self.data.is_empty() {
            return self.clone();
        }
        let (w, h) = ((self.width / factor).max(1), (self.height / factor).max(1));
        let mut out = Rgb32::new(w, h);
        let norm = 1.0 / (factor * factor) as f32;
        for y in 0..h {
            for x in 0..w {
                let mut acc = [0.0f32; 3];
                for dy in 0..factor {
                    let row = (y * factor + dy).min(self.height - 1) * self.width;
                    for dx in 0..factor {
                        let i = (row + (x * factor + dx).min(self.width - 1)) * 3;
                        acc[0] += self.data[i];
                        acc[1] += self.data[i + 1];
                        acc[2] += self.data[i + 2];
                    }
                }
                let o = (y * w + x) * 3;
                out.data[o..o + 3].copy_from_slice(&[acc[0] * norm, acc[1] * norm, acc[2] * norm]);
            }
        }
        out
    }

    /// Downsample so the long side is at most `long_side` pixels.
    pub fn fit(&self, long_side: usize) -> Rgb32 {
        let factor = self.width.max(self.height).div_ceil(long_side.max(1));
        self.downsample(factor.max(1))
    }

    pub fn crop(&self, r: Rect) -> Rgb32 {
        let r = r.clamp(self.width, self.height);
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        let mut out = Rgb32::new(w, h);
        for y in 0..h {
            let src = ((r.top + y) * self.width + r.left) * 3;
            out.data[y * w * 3..(y + 1) * w * 3].copy_from_slice(&self.data[src..src + w * 3]);
        }
        out
    }

    /// Mirror left-right (emulsion-up scans).
    pub fn flip_h(&self) -> Rgb32 {
        let mut out = self.clone();
        for y in 0..self.height {
            for x in 0..self.width {
                let s = (y * self.width + x) * 3;
                let d = (y * self.width + (self.width - 1 - x)) * 3;
                out.data[d..d + 3].copy_from_slice(&self.data[s..s + 3]);
            }
        }
        out
    }

    /// Rotate clockwise by `quarter_turns` × 90°.
    pub fn rotate_cw(&self, quarter_turns: u8) -> Rgb32 {
        match quarter_turns % 4 {
            0 => self.clone(),
            2 => {
                let mut out = self.clone();
                for (d, s) in out
                    .data
                    .as_chunks_mut::<3>()
                    .0
                    .iter_mut()
                    .zip(self.data.as_chunks::<3>().0.iter().rev())
                {
                    d.copy_from_slice(s);
                }
                out
            }
            q => {
                let (w, h) = (self.width, self.height);
                let mut out = Rgb32::new(h, w);
                for y in 0..h {
                    for x in 0..w {
                        // 90° cw: (x, y) -> (h-1-y, x); 270° cw: (x, y) -> (y, w-1-x)
                        let (nx, ny) = if q == 1 {
                            (h - 1 - y, x)
                        } else {
                            (y, w - 1 - x)
                        };
                        let s = (y * w + x) * 3;
                        let d = (ny * h + nx) * 3;
                        out.data[d..d + 3].copy_from_slice(&self.data[s..s + 3]);
                    }
                }
                out
            }
        }
    }
}

/// Run `f(start, part)` on parts of `data` on all cores; parts hold whole `unit`s, and
/// `start` is the index of a part's first element. For work that treats each element on its
/// own, so the result doesn't depend on the split. Runs in place where threads can't start.
pub fn for_each_part<T: Send>(data: &mut [T], unit: usize, f: impl Fn(usize, &mut [T]) + Sync) {
    /// Below this a thread costs more than it saves.
    const MIN_UNITS: usize = 16 * 1024;
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    let units = (data.len() / unit).div_ceil(cores).max(MIN_UNITS);
    if units * unit >= data.len() {
        return f(0, data);
    }
    std::thread::scope(|scope| {
        for (i, part) in data.chunks_mut(units * unit).enumerate() {
            let f = &f;
            scope.spawn(move || f(i * units * unit, part));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn numbered(w: usize, h: usize) -> Rgb32 {
        let mut img = Rgb32::new(w, h);
        for (i, v) in img.data.iter_mut().enumerate() {
            *v = (i / 3) as f32;
        }
        img
    }

    #[test]
    fn fit_keeps_thin_images_valid() {
        let wide = Rgb32::new(5000, 3).fit(1024);
        assert_eq!((wide.width, wide.height), (1000, 1));
        let tall = numbered(2, 9).downsample(4);
        assert_eq!((tall.width, tall.height), (1, 2));
        assert!(tall.data.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn rotate_four_times_is_identity() {
        let img = numbered(3, 2);
        let r = img.rotate_cw(1).rotate_cw(1).rotate_cw(1).rotate_cw(1);
        assert_eq!(r.data, img.data);
        assert_eq!((img.rotate_cw(1).width, img.rotate_cw(1).height), (2, 3));
    }

    #[test]
    fn rotate_cw_moves_top_left_to_top_right() {
        let img = numbered(3, 2); // pixel 0 at top-left
        let r = img.rotate_cw(1);
        assert_eq!(r.get(r.width - 1, 0)[0], 0.0);
    }

    #[test]
    fn for_each_part_reaches_every_element_once_with_its_index() {
        for len in [0, 9, 3 * 100_000 + 3] {
            let mut data = vec![0usize; len];
            for_each_part(&mut data, 3, |start, part| {
                assert_eq!(part.len() % 3, 0);
                for (i, v) in part.iter_mut().enumerate() {
                    *v = start + i + 1;
                }
            });
            assert!(data.iter().enumerate().all(|(i, &v)| v == i + 1));
        }
    }

    #[test]
    fn flip_twice_is_identity() {
        let img = numbered(4, 3);
        assert_eq!(img.flip_h().flip_h().data, img.data);
        assert_eq!(img.flip_h().get(3, 0)[0], 0.0);
    }

    #[test]
    fn straighten_zero_is_identity_and_small_angle_keeps_center() {
        let img = numbered(9, 7);
        assert_eq!(img.straighten(0.0).data, img.data);
        let r = img.straighten(3.0);
        assert_eq!((r.width, r.height), (9, 7));
        assert!((r.get(4, 3)[0] - img.get(4, 3)[0]).abs() < 1e-3);
    }

    #[test]
    fn nrect_roundtrips_and_clamps() {
        let r = NRect {
            x0: 0.25,
            y0: 0.5,
            x1: 0.75,
            y1: 1.0,
        }
        .to_pixels(400, 200);
        assert_eq!(
            r,
            Rect {
                top: 100,
                bottom: 200,
                left: 100,
                right: 300
            }
        );
        let tiny = NRect {
            x0: 0.99,
            y0: 0.99,
            x1: 0.99,
            y1: 0.99,
        }
        .to_pixels(10, 10);
        assert!(tiny.right > tiny.left && tiny.bottom > tiny.top && tiny.right <= 10);
    }

    #[test]
    fn downsample_averages() {
        let mut img = Rgb32::new(2, 2);
        img.data = vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 2.0, 2.0, 2.0, 3.0, 3.0, 3.0];
        assert_eq!(img.downsample(2).data, vec![1.5, 1.5, 1.5]);
    }
}
