//! Gradients of the Gradient Map adjustment: color stops along `[0, 1]`, the colors in between
//! interpolated linearly, and the lookup tables both compositors read, so that the CPU
//! reference and the GPU give the same values (as Curves', [`crate::curve`]).

use crate::curve::CURVE_LUT;

/// Stops of a gradient at most (an adjustment stays small: it is copied with every step of a
/// composite).
pub const GRADIENT_STOPS: usize = 16;

/// Where a stop can be: `0..=GRADIENT_LOCATIONS` across the gradient (Photoshop's 4096 steps).
pub const GRADIENT_LOCATIONS: u16 = 4096;

/// A color at a place of the gradient: `location` in `0..=GRADIENT_LOCATIONS`, the color
/// sRGB-encoded, 8 bits per channel (as Photoshop's gradients).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GradientStop {
    pub location: u16,
    pub color: [u8; 3],
}

/// 2 to [`GRADIENT_STOPS`] stops, their locations never decreasing (two stops at one place make
/// a hard edge). Before the first stop and after the last, the color stays that stop's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gradient {
    stops: [GradientStop; GRADIENT_STOPS],
    len: u8,
}

const BLACK: GradientStop = GradientStop {
    location: 0,
    color: [0; 3],
};

impl Gradient {
    /// Black to white (Photoshop's default Gradient Map, from the default colors).
    pub const BLACK_TO_WHITE: Gradient = {
        let mut stops = [BLACK; GRADIENT_STOPS];
        stops[1] = GradientStop {
            location: GRADIENT_LOCATIONS,
            color: [255; 3],
        };
        Gradient { stops, len: 2 }
    };

    /// The gradient of `stops`; `None` unless there are 2 to [`GRADIENT_STOPS`] of them, in
    /// range, their locations never decreasing.
    pub fn new(stops: &[GradientStop]) -> Option<Gradient> {
        let valid = (2..=GRADIENT_STOPS).contains(&stops.len())
            && stops.iter().all(|s| s.location <= GRADIENT_LOCATIONS)
            && stops.windows(2).all(|w| w[0].location <= w[1].location);
        if !valid {
            return None;
        }
        let mut gradient = Gradient {
            stops: [BLACK; GRADIENT_STOPS],
            len: stops.len() as u8,
        };
        gradient.stops[..stops.len()].copy_from_slice(stops);
        Some(gradient)
    }

    pub fn stops(&self) -> &[GradientStop] {
        &self.stops[..usize::from(self.len)]
    }

    /// The same gradient from the other end (Photoshop's Reverse).
    pub fn reversed(&self) -> Gradient {
        let mut stops: Vec<GradientStop> = self
            .stops()
            .iter()
            .rev()
            .map(|s| GradientStop {
                location: GRADIENT_LOCATIONS - s.location,
                color: s.color,
            })
            .collect();
        // Already in order; kept stable for stops at one place.
        stops.sort_by_key(|s| s.location);
        // Invariant: the same number of stops, in range and in order.
        Gradient::new(&stops).unwrap_or(*self)
    }

    /// The sRGB-encoded color at `t` (in `[0, 1]`): linear between the stops around it.
    pub fn color(&self, t: f64) -> [f64; 3] {
        let t = if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) };
        let at = t * f64::from(GRADIENT_LOCATIONS);
        let stops = self.stops();
        let unit = |c: [u8; 3]| c.map(|v| f64::from(v) / 255.0);
        let first = stops[0];
        if at <= f64::from(first.location) {
            return unit(first.color);
        }
        for pair in stops.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if at <= f64::from(b.location) {
                let span = f64::from(b.location - a.location);
                let k = if span > 0.0 {
                    (at - f64::from(a.location)) / span
                } else {
                    1.0
                };
                let (ca, cb) = (unit(a.color), unit(b.color));
                return [0, 1, 2].map(|i| ca[i] + (cb[i] - ca[i]) * k);
            }
        }
        unit(stops[stops.len() - 1].color)
    }

    /// The lookup tables, red, green and blue: the color at `i / (CURVE_LUT − 1)`.
    pub fn luts(&self) -> [Vec<f32>; 3] {
        let colors: Vec<[f64; 3]> = (0..CURVE_LUT)
            .map(|i| self.color(i as f64 / (CURVE_LUT - 1) as f64))
            .collect();
        [0, 1, 2].map(|c| colors.iter().map(|color| color[c] as f32).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stop(location: u16, color: [u8; 3]) -> GradientStop {
        GradientStop { location, color }
    }

    #[test]
    fn colors_are_interpolated_between_the_stops() {
        let g = Gradient::new(&[stop(1024, [255, 0, 0]), stop(3072, [0, 0, 255])]).unwrap();
        // Flat before the first stop and after the last.
        assert_eq!(g.color(0.0), [1.0, 0.0, 0.0]);
        assert_eq!(g.color(1.0), [0.0, 0.0, 1.0]);
        let mid = g.color(0.5);
        assert!((mid[0] - 0.5).abs() < 1e-9 && (mid[2] - 0.5).abs() < 1e-9);
        // Two stops at one place: a hard edge.
        let edge = Gradient::new(&[
            stop(0, [0; 3]),
            stop(2048, [0; 3]),
            stop(2048, [255; 3]),
            stop(4096, [255; 3]),
        ])
        .unwrap();
        assert_eq!(edge.color(0.49), [0.0; 3]);
        assert_eq!(edge.color(0.51), [1.0; 3]);
    }

    #[test]
    fn stops_are_validated_and_reversed() {
        assert!(Gradient::new(&[stop(0, [0; 3])]).is_none());
        assert!(Gradient::new(&[stop(10, [0; 3]), stop(5, [0; 3])]).is_none());
        assert!(Gradient::new(&[stop(0, [0; 3]), stop(5000, [0; 3])]).is_none());
        assert!(Gradient::new(&[stop(0, [0; 3]); GRADIENT_STOPS + 1]).is_none());
        let g =
            Gradient::new(&[stop(0, [10; 3]), stop(1000, [20; 3]), stop(4096, [30; 3])]).unwrap();
        assert_eq!(
            g.reversed().stops(),
            [stop(0, [30; 3]), stop(3096, [20; 3]), stop(4096, [10; 3])]
        );
        assert_eq!(g.reversed().reversed(), g);
    }

    #[test]
    fn the_lookup_tables_follow_the_colors() {
        let luts = Gradient::BLACK_TO_WHITE.luts();
        assert_eq!(luts[0].len(), CURVE_LUT);
        assert_eq!(luts[1][0], 0.0);
        assert_eq!(luts[2][CURVE_LUT - 1], 1.0);
    }
}
