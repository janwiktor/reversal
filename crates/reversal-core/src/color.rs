//! Color-space matrices for the output DNG (linear Rec.2020) and previews (sRGB).

pub type Mat3 = [[f64; 3]; 3];

/// Linear Rec.2020 → CIE XYZ, D65 white.
pub const REC2020_TO_XYZ: Mat3 = [
    [0.636958, 0.144617, 0.168881],
    [0.262700, 0.677998, 0.059302],
    [0.000000, 0.028073, 1.060985],
];

/// Bradford chromatic adaptation D65 → D50 (the DNG profile connection space).
pub const BRADFORD_D65_TO_D50: Mat3 = [
    [1.0478112, 0.0228866, -0.0501270],
    [0.0295424, 0.9904844, -0.0170491],
    [-0.0092345, 0.0150436, 0.7521316],
];

/// Linear Rec.2020 → linear sRGB (both D65); out-of-gamut values may be negative.
pub const REC2020_TO_SRGB: Mat3 = [
    [1.660491, -0.587641, -0.072850],
    [-0.124550, 1.132900, -0.008349],
    [-0.018151, -0.100579, 1.118730],
];

/// CIE XYZ D50 → linear ProPhoto RGB, the space raw editors apply profile curves in.
pub const XYZ_D50_TO_PROPHOTO: Mat3 = [
    [1.3459433, -0.2556075, -0.0511118],
    [-0.5445989, 1.5081673, 0.0205351],
    [0.0, 0.0, 1.2118128],
];

/// CIE XYZ D50 → linear sRGB, with Bradford adaptation to D65.
pub const XYZ_D50_TO_SRGB: Mat3 = [
    [3.1338561, -1.6168667, -0.4906146],
    [-0.9787684, 1.9161415, 0.033454],
    [0.0719453, -0.2289914, 1.4052427],
];

/// EXIF LightSource code for D65, used as the DNG calibration illuminant.
pub const ILLUMINANT_D65: u16 = 21;

pub fn mul(a: &Mat3, b: &Mat3) -> Mat3 {
    let mut m = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            m[i][j] = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    m
}

#[allow(clippy::needless_range_loop)] // cofactor indices read clearer as loops
pub fn invert(m: &Mat3) -> Mat3 {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    let mut r = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            let (a, b) = ((j + 1) % 3, (j + 2) % 3);
            let (c, d) = ((i + 1) % 3, (i + 2) % 3);
            r[i][j] = (m[a][c] * m[b][d] - m[a][d] * m[b][c]) / det;
        }
    }
    r
}

#[inline]
pub fn apply(m: &Mat3, p: [f32; 3]) -> [f32; 3] {
    [0, 1, 2]
        .map(|i| (m[i][0] * p[0] as f64 + m[i][1] * p[1] as f64 + m[i][2] * p[2] as f64) as f32)
}

/// Linear Rec.2020 → linear ProPhoto, through the DNG's own forward matrix.
pub fn rec2020_to_prophoto() -> Mat3 {
    mul(&XYZ_D50_TO_PROPHOTO, &dng_matrices().1)
}

/// Linear ProPhoto → linear sRGB; out-of-gamut values may be negative.
pub fn prophoto_to_srgb() -> Mat3 {
    mul(&XYZ_D50_TO_SRGB, &invert(&XYZ_D50_TO_PROPHOTO))
}

/// DNG ColorMatrix1 (XYZ → "camera" = our Rec.2020 data) and ForwardMatrix1
/// (white-balanced camera → XYZ D50), row-major.
pub fn dng_matrices() -> (Mat3, Mat3) {
    (
        invert(&REC2020_TO_XYZ),
        mul(&BRADFORD_D65_TO_D50, &REC2020_TO_XYZ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inverse_roundtrips() {
        let i = mul(&REC2020_TO_XYZ, &invert(&REC2020_TO_XYZ));
        for (r, row) in i.iter().enumerate() {
            for (c, v) in row.iter().enumerate() {
                assert!((v - if r == c { 1.0 } else { 0.0 }).abs() < 1e-9);
            }
        }
    }

    #[test]
    fn forward_matrix_maps_white_to_d50() {
        let (_, fwd) = dng_matrices();
        let xyz = apply(&fwd, [1.0, 1.0, 1.0]);
        assert!(
            (xyz[0] - 0.9642).abs() < 2e-3
                && (xyz[1] - 1.0).abs() < 2e-3
                && (xyz[2] - 0.8249).abs() < 2e-3
        );
    }

    #[test]
    fn prophoto_round_trip_keeps_white_and_matches_the_direct_matrix() {
        let m = mul(&prophoto_to_srgb(), &rec2020_to_prophoto());
        let w = apply(&m, [1.0, 1.0, 1.0]);
        assert!(w.iter().all(|v| (v - 1.0).abs() < 2e-3));
        let (via, direct) = (
            apply(&m, [0.6, 0.3, 0.1]),
            apply(&REC2020_TO_SRGB, [0.6, 0.3, 0.1]),
        );
        assert!(via.iter().zip(direct).all(|(a, b)| (a - b).abs() < 3e-3));
    }

    #[test]
    fn rec2020_to_srgb_keeps_white() {
        let w = apply(&REC2020_TO_SRGB, [1.0, 1.0, 1.0]);
        assert!(w.iter().all(|v| (v - 1.0).abs() < 1e-3));
    }
}
pub fn  badly_formatted( ) {}

#[cfg(test)]
mod ci_failure_probe {
    #[test]
    fn fails_on_purpose() {
        assert_eq!(1 + 1, 3);
    }
}
