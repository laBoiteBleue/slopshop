//! Tone curves of the Curves adjustment (ADR 0020): points on Photoshop's 0–255 scale, a smooth
//! curve through them (a natural cubic spline), and the lookup table both compositors
//! interpolate, so that the CPU reference and the GPU give the same values.

/// Points of a curve at most (Photoshop's limit).
pub const CURVE_POINTS: usize = 16;

/// Entries of a curve's lookup table, evenly spaced over `[0, 1]`.
pub const CURVE_LUT: usize = 1024;

/// A tone curve: 2 to [`CURVE_POINTS`] points `(input, output)` on 0–255, inputs strictly
/// increasing. Before the first point and after the last, the output stays flat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Curve {
    points: [[u8; 2]; CURVE_POINTS],
    len: u8,
}

impl Curve {
    /// The straight line from black to white: no change.
    pub const IDENTITY: Curve = {
        let mut points = [[0; 2]; CURVE_POINTS];
        points[1] = [255, 255];
        Curve { points, len: 2 }
    };

    /// The curve through `points`; `None` unless there are 2 to [`CURVE_POINTS`] of them with
    /// strictly increasing inputs.
    pub fn new(points: &[[u8; 2]]) -> Option<Curve> {
        if !(2..=CURVE_POINTS).contains(&points.len())
            || points.windows(2).any(|w| w[0][0] >= w[1][0])
        {
            return None;
        }
        let mut curve = Curve {
            points: [[0; 2]; CURVE_POINTS],
            len: points.len() as u8,
        };
        curve.points[..points.len()].copy_from_slice(points);
        Some(curve)
    }

    pub fn points(&self) -> &[[u8; 2]] {
        &self.points[..usize::from(self.len)]
    }

    /// Whether the curve changes nothing: its points on the diagonal, from black to white.
    pub fn is_identity(&self) -> bool {
        let p = self.points();
        p.iter().all(|[i, o]| i == o) && p[0] == [0, 0] && p[p.len() - 1] == [255, 255]
    }

    /// The curve's output at `x` (in `[0, 1]`), exactly: the natural cubic spline through the
    /// points, flat outside them, clamped to `[0, 1]`.
    pub fn value(&self, x: f64) -> f64 {
        self.spline().value(x)
    }

    /// The lookup table: the curve at `i / (CURVE_LUT − 1)`.
    pub fn lut(&self) -> Vec<f32> {
        let spline = self.spline();
        (0..CURVE_LUT)
            .map(|i| spline.value(i as f64 / (CURVE_LUT - 1) as f64) as f32)
            .collect()
    }

    fn spline(&self) -> Spline {
        let points: Vec<(f64, f64)> = self
            .points()
            .iter()
            .map(|&[i, o]| (f64::from(i) / 255.0, f64::from(o) / 255.0))
            .collect();
        Spline::new(points)
    }
}

/// A curve's lookup table at `v`, linearly interpolated, `v` clamped to `[0, 1]` (NaN reads as
/// 0). The GPU renderer does the same in f32.
pub fn lookup(lut: &[f32], v: f64) -> f64 {
    let v = if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
    let t = v * (lut.len() - 1) as f64;
    let i = (t as usize).min(lut.len() - 2);
    let (a, b) = (f64::from(lut[i]), f64::from(lut[i + 1]));
    a + (b - a) * (t - i as f64)
}

/// A natural cubic spline: the points, and the second derivative at each.
struct Spline {
    points: Vec<(f64, f64)>,
    second: Vec<f64>,
}

impl Spline {
    fn new(points: Vec<(f64, f64)>) -> Spline {
        let n = points.len();
        let mut second = vec![0.0; n];
        if n > 2 {
            // The tridiagonal system of a natural spline (zero second derivative at both ends),
            // solved by elimination.
            let h: Vec<f64> = points.windows(2).map(|w| w[1].0 - w[0].0).collect();
            let slope: Vec<f64> = points
                .windows(2)
                .zip(&h)
                .map(|(w, h)| (w[1].1 - w[0].1) / h)
                .collect();
            let mut diagonal = vec![0.0; n];
            let mut rhs = vec![0.0; n];
            for i in 1..n - 1 {
                diagonal[i] = 2.0 * (h[i - 1] + h[i]);
                rhs[i] = 6.0 * (slope[i] - slope[i - 1]);
            }
            for i in 2..n - 1 {
                let m = h[i - 1] / diagonal[i - 1];
                diagonal[i] -= m * h[i - 1];
                rhs[i] -= m * rhs[i - 1];
            }
            for i in (1..n - 1).rev() {
                second[i] = (rhs[i] - h[i] * second[i + 1]) / diagonal[i];
            }
        }
        Spline { points, second }
    }

    fn value(&self, x: f64) -> f64 {
        let p = &self.points;
        let (first, last) = (p[0], p[p.len() - 1]);
        if x <= first.0 {
            return first.1;
        }
        if x >= last.0 {
            return last.1;
        }
        let k = p.partition_point(|&(px, _)| px <= x).clamp(1, p.len() - 1);
        let ((x0, y0), (x1, y1)) = (p[k - 1], p[k]);
        let h = x1 - x0;
        let (a, b) = ((x1 - x) / h, (x - x0) / h);
        let y = a * y0
            + b * y1
            + ((a * a * a - a) * self.second[k - 1] + (b * b * b - b) * self.second[k]) * h * h
                / 6.0;
        y.clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curves_need_two_to_sixteen_increasing_points() {
        assert!(Curve::new(&[[0, 0]]).is_none());
        assert!(Curve::new(&[[0, 0], [0, 255]]).is_none());
        assert!(Curve::new(&[[10, 0], [5, 255]]).is_none());
        let many: Vec<[u8; 2]> = (0..17).map(|i| [i * 15, i * 15]).collect();
        assert!(Curve::new(&many).is_none());
        assert!(Curve::new(&many[..16]).is_some());
        assert!(Curve::IDENTITY.is_identity());
        assert!(
            !Curve::new(&[[0, 0], [128, 160], [255, 255]])
                .unwrap()
                .is_identity()
        );
    }

    #[test]
    fn the_spline_goes_through_its_points_and_is_flat_outside() {
        let line = Curve::IDENTITY;
        for x in [0.0, 0.25, 0.7, 1.0] {
            assert!((line.value(x) - x).abs() < 1e-12);
        }
        let s = Curve::new(&[[30, 10], [128, 160], [220, 250]]).unwrap();
        for [i, o] in [[30u8, 10u8], [128, 160], [220, 250]] {
            assert!((s.value(f64::from(i) / 255.0) - f64::from(o) / 255.0).abs() < 1e-12);
        }
        assert_eq!(s.value(0.0), 10.0 / 255.0);
        assert_eq!(s.value(1.0), 250.0 / 255.0);
        // Smooth and rising between its points; never outside [0, 1].
        assert!(s.value(0.3) < s.value(0.4));
        let steep = Curve::new(&[[0, 0], [100, 250], [130, 5], [255, 255]]).unwrap();
        assert!((0..=100).all(|i| (0.0..=1.0).contains(&steep.value(f64::from(i) / 100.0))));
    }

    #[test]
    fn the_lookup_table_matches_the_curve() {
        let s = Curve::new(&[[0, 20], [64, 40], [190, 230], [255, 255]]).unwrap();
        let lut = s.lut();
        assert_eq!(lut.len(), CURVE_LUT);
        for i in 0..=200 {
            let x = f64::from(i) / 200.0;
            assert!((lookup(&lut, x) - s.value(x)).abs() < 1e-4, "{x}");
        }
        assert_eq!(lookup(&lut, -1.0), f64::from(lut[0]));
        assert_eq!(lookup(&lut, 2.0), f64::from(lut[CURVE_LUT - 1]));
        assert_eq!(lookup(&lut, f64::NAN), f64::from(lut[0]));
    }
}
