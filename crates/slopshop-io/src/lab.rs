//! CIE Lab (D50, as Photoshop stores colors) to and from sRGB-encoded values, for the colors of
//! adjustment layers (Photo Filter).

use slopshop_core::color::{
    D50, D65, RgbPrimaries, TransferFunction, bradford, mat_inverse, mat_mul, mat_vec,
};

/// The D50 white as XYZ (Y = 1).
const D50_XYZ: [f64; 3] = [0.9642, 1.0, 0.8249];

/// Linear sRGB → XYZ adapted to D50.
fn srgb_to_xyz_d50() -> [[f64; 3]; 3] {
    mat_mul(&bradford(D65, D50), &RgbPrimaries::REC709.to_xyz())
}

/// A Lab color (L 0–100, a and b around 0) as sRGB-encoded values, not clipped.
pub(crate) fn lab_to_srgb([l, a, b]: [f64; 3]) -> [f64; 3] {
    let f = |t: f64| {
        if t > 6.0 / 29.0 {
            t.powi(3)
        } else {
            3.0 * (6.0f64 / 29.0).powi(2) * (t - 4.0 / 29.0)
        }
    };
    let fy = (l + 16.0) / 116.0;
    let xyz = [fy + a / 500.0, fy, fy - b / 200.0];
    let xyz = [0, 1, 2].map(|i| D50_XYZ[i] * f(xyz[i]));
    // The sRGB matrix is invertible: its primaries are valid.
    let to_rgb = mat_inverse(&srgb_to_xyz_d50()).unwrap_or([[0.0; 3]; 3]);
    mat_vec(&to_rgb, xyz).map(|v| encode(v))
}

/// sRGB-encoded values as a Lab color.
pub(crate) fn srgb_to_lab(rgb: [f64; 3]) -> [f64; 3] {
    let linear = rgb.map(|v| f64::from(TransferFunction::Srgb.decode(v as f32)));
    let xyz = mat_vec(&srgb_to_xyz_d50(), linear);
    let f = |t: f64| {
        if t > (6.0f64 / 29.0).powi(3) {
            t.cbrt()
        } else {
            t / (3.0 * (6.0f64 / 29.0).powi(2)) + 4.0 / 29.0
        }
    };
    let [fx, fy, fz] = [0, 1, 2].map(|i| f(xyz[i] / D50_XYZ[i]));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

fn encode(v: f64) -> f64 {
    f64::from(TransferFunction::Srgb.encode(v as f32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn white_gray_and_colors_round_trip() {
        let close = |a: [f64; 3], b: [f64; 3], tolerance: f64| {
            a.iter().zip(b).all(|(x, y)| (x - y).abs() < tolerance)
        };
        assert!(close(srgb_to_lab([1.0; 3]), [100.0, 0.0, 0.0], 0.05));
        assert!(close(lab_to_srgb([100.0, 0.0, 0.0]), [1.0; 3], 1e-3));
        // Middle gray: L* 53.4, neutral.
        assert!(close(srgb_to_lab([0.5; 3]), [53.39, 0.0, 0.0], 0.05));
        for rgb in [[0.9, 0.5, 0.1], [0.1, 0.3, 0.8], [0.2, 0.7, 0.4]] {
            assert!(close(lab_to_srgb(srgb_to_lab(rgb)), rgb, 1e-4), "{rgb:?}");
        }
    }
}
