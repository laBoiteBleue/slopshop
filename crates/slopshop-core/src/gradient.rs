//! Gradients: color stops along `[0, 1]`, the colors in between interpolated linearly. The
//! Gradient Map adjustment reads them through lookup tables both compositors share, so that the
//! CPU reference and the GPU give the same values (as Curves', [`crate::curve`]); the Gradient
//! tool lays them across the canvas ([`GradientField`]).

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

/// How a [`GradientField`] spreads from its start to its end (Photoshop's five styles).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GradientShape {
    /// Along the line, square to it.
    Linear,
    /// Around the start, the end on its rim.
    Radial,
    /// A counterclockwise sweep (as seen on screen) around the start, from the line through the
    /// end back to it.
    Angle,
    /// Linear on both sides of the start, mirrored.
    Reflected,
    /// Out from the start in a diamond, the end one of its corners.
    Diamond,
}

impl GradientShape {
    /// Every shape, in Photoshop's order.
    pub const ALL: [GradientShape; 5] = [
        GradientShape::Linear,
        GradientShape::Radial,
        GradientShape::Angle,
        GradientShape::Reflected,
        GradientShape::Diamond,
    ];

    /// The stable identifier files and the IPC use.
    pub fn id(self) -> &'static str {
        match self {
            GradientShape::Linear => "linear",
            GradientShape::Radial => "radial",
            GradientShape::Angle => "angle",
            GradientShape::Reflected => "reflected",
            GradientShape::Diamond => "diamond",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|shape| shape.id() == id)
    }
}

/// A gradient laid across the canvas by the Gradient tool: its colors from `from` to `to`
/// (document pixels), shaped by `shape`, and an opacity going from `alpha[0]` to `alpha[1]`
/// (Photoshop's "Foreground to Transparent"). Before the start and past the end, the end
/// colors stay.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradientField {
    pub gradient: Gradient,
    pub alpha: [f32; 2],
    pub shape: GradientShape,
    pub from: [f64; 2],
    pub to: [f64; 2],
}

impl GradientField {
    /// Finite points apart, opacities in `[0, 1]`.
    pub fn is_valid(&self) -> bool {
        let finite = self.from.iter().chain(&self.to).all(|v| v.is_finite());
        let apart = self.from != self.to;
        finite && apart && self.alpha.iter().all(|a| (0.0..=1.0).contains(a))
    }

    /// Where document point (`x`, `y`) falls along the gradient, in `[0, 1]`.
    pub fn position(&self, x: f64, y: f64) -> f64 {
        let [fx, fy] = self.from;
        let (dx, dy) = (self.to[0] - fx, self.to[1] - fy);
        let length2 = dx * dx + dy * dy;
        let (vx, vy) = (x - fx, y - fy);
        // Along the line and square to it, in lengths of the line.
        let along = || (vx * dx + vy * dy) / length2;
        let across = || (dx * vy - dy * vx) / length2;
        let t = match self.shape {
            GradientShape::Linear => along(),
            GradientShape::Radial => (vx * vx + vy * vy).sqrt() / length2.sqrt(),
            GradientShape::Angle => {
                // Counterclockwise on screen (y goes down): from the line to the point.
                let turn = (dy * vx - dx * vy).atan2(vx * dx + vy * dy) / std::f64::consts::TAU;
                if turn < 0.0 { turn + 1.0 } else { turn }
            }
            GradientShape::Reflected => along().abs(),
            GradientShape::Diamond => along().abs() + across().abs(),
        };
        if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) }
    }

    /// The sRGB-encoded color (in `[0, 1]`) and the opacity at document point (`x`, `y`).
    pub fn at(&self, x: f64, y: f64) -> ([f64; 3], f64) {
        let t = self.position(x, y);
        let [a0, a1] = self.alpha.map(f64::from);
        (self.gradient.color(t), a0 + (a1 - a0) * t)
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

    #[test]
    fn a_field_lays_the_gradient_from_its_start_to_its_end() {
        let field = GradientField {
            gradient: Gradient::BLACK_TO_WHITE,
            alpha: [1.0, 0.0],
            shape: GradientShape::Linear,
            from: [10.0, 0.0],
            to: [110.0, 0.0],
        };
        assert!(field.is_valid());
        // Square to the line: the same position whatever y.
        assert_eq!(field.position(60.0, -40.0), 0.5);
        assert_eq!(field.position(0.0, 5.0), 0.0, "before the start");
        assert_eq!(field.position(500.0, 5.0), 1.0, "past the end");
        let (color, alpha) = field.at(35.0, 0.0);
        assert_eq!(color, [0.25; 3]);
        assert_eq!(alpha, 0.75);
        // Radial: by the distance to the start.
        let radial = GradientField {
            shape: GradientShape::Radial,
            ..field
        };
        assert_eq!(radial.position(10.0, 50.0), 0.5);
        assert!(
            !GradientField {
                to: [10.0, 0.0],
                ..field
            }
            .is_valid()
        );
        assert!(
            !GradientField {
                alpha: [1.5, 0.0],
                ..field
            }
            .is_valid()
        );
    }

    #[test]
    fn angle_reflected_and_diamond_follow_photoshop() {
        let field = |shape| GradientField {
            gradient: Gradient::BLACK_TO_WHITE,
            alpha: [1.0, 1.0],
            shape,
            from: [100.0, 100.0],
            to: [140.0, 100.0],
        };
        let close = |a: f64, b: f64| (a - b).abs() < 1e-12;
        // Angle: from the line toward the end, counterclockwise on screen (up first, y down).
        let angle = field(GradientShape::Angle);
        assert_eq!(
            angle.position(200.0, 100.0),
            0.0,
            "on the line, past the end"
        );
        assert!(close(angle.position(100.0, 50.0), 0.25), "above the start");
        assert!(close(angle.position(10.0, 100.0), 0.5), "behind it");
        assert!(close(angle.position(100.0, 130.0), 0.75), "below it");
        assert!(
            angle.position(200.0, 100.5) > 0.99,
            "just under the line: the sweep's end"
        );
        assert_eq!(angle.position(100.0, 100.0), 0.0, "the start itself");
        // Reflected: the linear gradient, mirrored about the start.
        let reflected = field(GradientShape::Reflected);
        assert_eq!(reflected.position(120.0, 7.0), 0.5);
        assert_eq!(reflected.position(80.0, -3.0), 0.5);
        assert_eq!(reflected.position(0.0, 0.0), 1.0);
        // Diamond: the end is a corner; halfway along each axis meets its edge.
        let diamond = field(GradientShape::Diamond);
        assert_eq!(diamond.position(140.0, 100.0), 1.0);
        assert_eq!(diamond.position(100.0, 60.0), 1.0);
        assert!(close(diamond.position(120.0, 80.0), 1.0), "on its edge");
        assert!(close(diamond.position(110.0, 90.0), 0.5));
        assert!(close(diamond.position(90.0, 105.0), 0.375));
        // Turned: the diamond turns with the line.
        let turned = GradientField {
            to: [130.0, 130.0],
            ..diamond
        };
        assert!(close(turned.position(130.0, 70.0), 1.0));
        assert!(close(turned.position(115.0, 115.0), 0.5));
    }

    #[test]
    fn shapes_have_stable_ids() {
        for shape in GradientShape::ALL {
            assert_eq!(GradientShape::from_id(shape.id()), Some(shape));
        }
        assert_eq!(GradientShape::from_id("spiral"), None);
    }
}
