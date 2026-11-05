//! Lossless JPEG (ITU T.81 process 14, "LJPEG-92") encoder for DNG tiles.
//!
//! 16-bit samples, interleaved components, predictor 1 (left), one Huffman table
//! optimized per tile. Sufficient for DNG `Compression = 7`.

const PRECISION: u32 = 16;

/// Difference category (SSSS) of a lossless JPEG difference.
#[inline]
fn category(diff: i32) -> u8 {
    if diff == 0 {
        0
    } else if diff == 32768 {
        16
    } else {
        (32 - diff.unsigned_abs().leading_zeros()) as u8
    }
}

/// Modulo-2^16 difference, mapped to -32767..=32768 as T.81 requires.
#[inline]
fn difference(x: u16, pred: u16) -> i32 {
    let d = (x as i32 - pred as i32).rem_euclid(65536);
    if d > 32768 {
        d - 65536
    } else {
        d
    }
}

/// Visit every sample's difference in scan order.
fn for_each_diff(
    samples: &[u16],
    width: usize,
    height: usize,
    comps: usize,
    mut f: impl FnMut(i32),
) {
    let row_len = width * comps;
    for y in 0..height {
        for x in 0..width {
            for c in 0..comps {
                let i = y * row_len + x * comps + c;
                let pred = if x > 0 {
                    samples[i - comps]
                } else if y > 0 {
                    samples[i - row_len]
                } else {
                    1u16 << (PRECISION - 1)
                };
                f(difference(samples[i], pred));
            }
        }
    }
}

/// Huffman code lengths per symbol (0..=16), limited to 16 bits (T.81 Annex K.2).
fn code_lengths(freq: &[u32; 17]) -> [u8; 17] {
    // One extra reserved symbol so no code is all ones.
    let mut f: Vec<u64> = freq.iter().map(|&v| v as u64).collect();
    f.push(1);
    let n = f.len();
    let mut size = vec![0u32; n];
    let mut others: Vec<Option<usize>> = vec![None; n];
    // Least frequent non-zero symbol, ties to the larger index (K.2).
    let pick = |f: &[u64], skip: Option<usize>| {
        let mut best: Option<usize> = None;
        for i in 0..f.len() {
            if f[i] == 0 || Some(i) == skip {
                continue;
            }
            if best.is_none_or(|b| f[i] <= f[b]) {
                best = Some(i);
            }
        }
        best
    };
    while let Some(c1) = pick(&f, None) {
        let Some(c2) = pick(&f, Some(c1)) else { break };
        f[c1] += f[c2];
        f[c2] = 0;
        let mut i = c1;
        size[i] += 1;
        while let Some(o) = others[i] {
            i = o;
            size[i] += 1;
        }
        others[i] = Some(c2);
        let mut i = c2;
        size[i] += 1;
        while let Some(o) = others[i] {
            i = o;
            size[i] += 1;
        }
    }
    // Count codes per length, then limit to 16 bits (Figure K.3).
    let mut bits = [0u32; 40];
    for &s in &size {
        if s > 0 {
            bits[s as usize] += 1;
        }
    }
    let mut i = 39;
    while i > 16 {
        while bits[i] > 0 {
            let mut j = i - 2;
            while bits[j] == 0 {
                j -= 1;
            }
            bits[i] -= 2;
            bits[i - 1] += 1;
            bits[j + 1] += 2;
            bits[j] -= 1;
        }
        i -= 1;
    }
    // Remove the reserved symbol from the longest length.
    let mut i = 16;
    while bits[i] == 0 {
        i -= 1;
    }
    bits[i] -= 1;
    // Assign lengths: most frequent symbols get the shortest codes.
    let mut order: Vec<usize> = (0..17).filter(|&s| freq[s] > 0).collect();
    order.sort_by(|&a, &b| freq[b].cmp(&freq[a]).then(a.cmp(&b)));
    let mut lengths = [0u8; 17];
    let mut it = order.into_iter();
    for (len, &count) in bits.iter().enumerate().take(17).skip(1) {
        for _ in 0..count {
            if let Some(s) = it.next() {
                lengths[s] = len as u8;
            }
        }
    }
    lengths
}

struct Table {
    bits: [u8; 16],
    values: Vec<u8>,
    code: [u16; 17],
    len: [u8; 17],
}

fn build_table(lengths: &[u8; 17]) -> Table {
    let mut bits = [0u8; 16];
    let mut values = Vec::new();
    for len in 1..=16u8 {
        for (s, &l) in lengths.iter().enumerate() {
            if l == len {
                bits[len as usize - 1] += 1;
                values.push(s as u8);
            }
        }
    }
    // Canonical codes (T.81 Annex C).
    let (mut code, mut codes, mut lens) = (0u16, [0u16; 17], [0u8; 17]);
    let mut k = 0;
    for len in 1..=16u8 {
        for _ in 0..bits[len as usize - 1] {
            codes[values[k] as usize] = code;
            lens[values[k] as usize] = len;
            code += 1;
            k += 1;
        }
        code <<= 1;
    }
    Table {
        bits,
        values,
        code: codes,
        len: lens,
    }
}

struct BitWriter {
    out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl BitWriter {
    fn put(&mut self, value: u32, bits: u32) {
        if bits == 0 {
            return;
        }
        self.acc = (self.acc << bits) | (value as u64 & ((1u64 << bits) - 1));
        self.n += bits;
        while self.n >= 8 {
            self.n -= 8;
            let byte = (self.acc >> self.n) as u8;
            self.out.push(byte);
            if byte == 0xFF {
                self.out.push(0x00);
            }
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            let pad = 8 - self.n;
            self.put((1 << pad) - 1, pad);
        }
        self.out
    }
}

fn segment(out: &mut Vec<u8>, marker: u8, body: &[u8]) {
    out.extend_from_slice(&[0xFF, marker]);
    out.extend_from_slice(&((body.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(body);
}

/// Encode `samples` (row-major, `comps` interleaved components) as lossless JPEG.
pub fn encode(samples: &[u16], width: usize, height: usize, comps: usize) -> Vec<u8> {
    assert_eq!(samples.len(), width * height * comps);
    let mut freq = [0u32; 17];
    for_each_diff(samples, width, height, comps, |d| {
        freq[category(d) as usize] += 1
    });
    let table = build_table(&code_lengths(&freq));

    let mut out = vec![0xFF, 0xD8]; // SOI
    let mut sof = vec![PRECISION as u8];
    sof.extend_from_slice(&(height as u16).to_be_bytes());
    sof.extend_from_slice(&(width as u16).to_be_bytes());
    sof.push(comps as u8);
    for c in 0..comps {
        sof.extend_from_slice(&[c as u8, 0x11, 0]);
    }
    segment(&mut out, 0xC3, &sof);
    let mut dht = vec![0x00];
    dht.extend_from_slice(&table.bits);
    dht.extend_from_slice(&table.values);
    segment(&mut out, 0xC4, &dht);
    let mut sos = vec![comps as u8];
    for c in 0..comps {
        sos.extend_from_slice(&[c as u8, 0x00]);
    }
    sos.extend_from_slice(&[1, 0, 0]); // predictor 1, Se = 0, Ah/Al = 0
    segment(&mut out, 0xDA, &sos);

    let mut w = BitWriter { out, acc: 0, n: 0 };
    for_each_diff(samples, width, height, comps, |d| {
        let s = category(d);
        w.put(table.code[s as usize] as u32, table.len[s as usize] as u32);
        if s > 0 && s < 16 {
            let extra = if d < 0 { d - 1 } else { d };
            w.put(extra as u32, s as u32);
        }
    });
    let mut out = w.finish();
    out.extend_from_slice(&[0xFF, 0xD9]); // EOI
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal decoder for the subset we write, to check round trips.
    fn decode(data: &[u8], width: usize, height: usize, comps: usize) -> Vec<u16> {
        let dht = data.windows(2).position(|w| w == [0xFF, 0xC4]).unwrap() + 5;
        let bits: Vec<u8> = data[dht..dht + 16].to_vec();
        let nvals: usize = bits.iter().map(|&b| b as usize).sum();
        let values = &data[dht + 16..dht + 16 + nvals];
        let mut lengths = [0u8; 17];
        let mut k = 0;
        for (l, &n) in bits.iter().enumerate() {
            for _ in 0..n {
                lengths[values[k] as usize] = l as u8 + 1;
                k += 1;
            }
        }
        let t = build_table(&lengths);
        let sos = data.windows(2).position(|w| w == [0xFF, 0xDA]).unwrap();
        let sos_len = u16::from_be_bytes([data[sos + 2], data[sos + 3]]) as usize;
        let mut bytes = Vec::new();
        let mut i = sos + 2 + sos_len;
        while !(data[i] == 0xFF && data[i + 1] == 0xD9) {
            bytes.push(data[i]);
            i += if data[i] == 0xFF { 2 } else { 1 };
        }
        let (mut pos, mut out) = (0usize, vec![0u16; width * height * comps]);
        let bit = |pos: &mut usize| {
            let b = (bytes[*pos / 8] >> (7 - *pos % 8)) & 1;
            *pos += 1;
            b as u32
        };
        let row_len = width * comps;
        for idx in 0..out.len() {
            let (mut code, mut len) = (0u32, 0u8);
            let s = loop {
                code = (code << 1) | bit(&mut pos);
                len += 1;
                if let Some(s) = (0..17).find(|&s| t.len[s] == len && t.code[s] as u32 == code) {
                    break s as u8;
                }
            };
            let d: i32 = match s {
                0 => 0,
                16 => 32768,
                _ => {
                    let mut v = 0u32;
                    for _ in 0..s {
                        v = (v << 1) | bit(&mut pos);
                    }
                    if v < (1 << (s - 1)) {
                        v as i32 - (1 << s) + 1
                    } else {
                        v as i32
                    }
                }
            };
            let (x, y) = ((idx % row_len) / comps, idx / row_len);
            let pred = if x > 0 {
                out[idx - comps]
            } else if y > 0 {
                out[idx - row_len]
            } else {
                32768
            };
            out[idx] = (pred as i32 + d).rem_euclid(65536) as u16;
        }
        out
    }

    #[test]
    fn roundtrip_random_and_extremes() {
        let (w, h, c) = (37, 23, 3);
        let mut seed = 12345u32;
        let samples: Vec<u16> = (0..w * h * c)
            .map(|i| {
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                match i % 97 {
                    0 => 0,
                    1 => 65535,
                    _ => ((seed >> 8) % 4096 + 20000) as u16,
                }
            })
            .collect();
        let enc = encode(&samples, w, h, c);
        assert_eq!(decode(&enc, w, h, c), samples);
        assert!(enc.len() < samples.len() * 2);
    }

    #[test]
    fn roundtrip_flat() {
        let samples = vec![1000u16; 16 * 16 * 3];
        assert_eq!(decode(&encode(&samples, 16, 16, 3), 16, 16, 3), samples);
    }

    #[test]
    fn code_lengths_fit_16_bits() {
        let mut freq = [0u32; 17];
        for (i, f) in freq.iter_mut().enumerate() {
            *f = 1 << i.min(30); // very skewed
        }
        assert!(code_lengths(&freq).iter().all(|&l| l <= 16));
    }
}
