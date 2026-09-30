//! 2D affine transforms of layers (ADR 0017).
//!
//! A transform maps a layer's content space (its raster's and mask's pixel coordinates) to its
//! parent's space (the document, or the group it is in). Coordinates are in pixels, x to the
//! right and y down; the point `(x, y)` maps to `(a·x + c·y + e, b·x + d·y + f)`.

/// A 2D affine map, `(x, y) ↦ (a·x + c·y + e, b·x + d·y + f)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Affine {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

impl Default for Affine {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Affine {
    pub const IDENTITY: Affine = Affine {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    pub const fn translation(x: f64, y: f64) -> Self {
        Affine {
            e: x,
            f: y,
            ..Self::IDENTITY
        }
    }

    /// The six numbers `[a, b, c, d, e, f]`.
    pub const fn to_array(self) -> [f64; 6] {
        [self.a, self.b, self.c, self.d, self.e, self.f]
    }

    pub const fn from_array([a, b, c, d, e, f]: [f64; 6]) -> Self {
        Affine { a, b, c, d, e, f }
    }

    pub fn is_identity(self) -> bool {
        self == Self::IDENTITY
    }

    pub fn is_finite(self) -> bool {
        self.to_array().iter().all(|v| v.is_finite())
    }

    /// `self` then `after`: the map `p ↦ after(self(p))`.
    pub fn then(self, after: Affine) -> Affine {
        Affine {
            a: after.a * self.a + after.c * self.b,
            b: after.b * self.a + after.d * self.b,
            c: after.a * self.c + after.c * self.d,
            d: after.b * self.c + after.d * self.d,
            e: after.a * self.e + after.c * self.f + after.e,
            f: after.b * self.e + after.d * self.f + after.f,
        }
    }

    pub fn apply(self, x: f64, y: f64) -> (f64, f64) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }

    /// A scale by (`sx`, `sy`) about the origin.
    pub const fn scale(sx: f64, sy: f64) -> Self {
        Affine {
            a: sx,
            d: sy,
            ..Self::IDENTITY
        }
    }

    /// A rotation by `radians` about the origin (clockwise on screen: y points down).
    pub fn rotation(radians: f64) -> Self {
        let (sin, cos) = radians.sin_cos();
        Affine {
            a: cos,
            b: sin,
            c: -sin,
            d: cos,
            e: 0.0,
            f: 0.0,
        }
    }

    pub fn determinant(self) -> f64 {
        self.a * self.d - self.b * self.c
    }

    /// The inverse map; `None` when the map is not invertible (or not finite).
    pub fn inverse(self) -> Option<Affine> {
        let det = self.determinant();
        if !det.is_finite() || det == 0.0 {
            return None;
        }
        let (a, b, c, d) = (self.d / det, -self.b / det, -self.c / det, self.a / det);
        let inverse = Affine {
            a,
            b,
            c,
            d,
            e: -(a * self.e + c * self.f),
            f: -(b * self.e + d * self.f),
        };
        inverse.is_finite().then_some(inverse)
    }

    /// The bounding box `[x0, y0, x1, y1]` of the image of the rectangle `[x0, y0, x1, y1]`.
    pub fn map_rect(self, [x0, y0, x1, y1]: [f64; 4]) -> [f64; 4] {
        let corners = [(x0, y0), (x1, y0), (x0, y1), (x1, y1)].map(|(x, y)| self.apply(x, y));
        let mut out = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for (x, y) in corners {
            out = [out[0].min(x), out[1].min(y), out[2].max(x), out[3].max(y)];
        }
        out
    }

    /// The same map with every number within 1e-9 of a whole number made whole: transforms
    /// built by floating-point steps (a rotation by 90°, a scale back to 1, a move by whole
    /// pixels) stay exact.
    pub fn snapped(self) -> Affine {
        Affine::from_array(self.to_array().map(|v| {
            let whole = v.round();
            if (v - whole).abs() < 1e-9 { whole } else { v }
        }))
    }

    /// Whether a layer may have this transform (ADR 0018): finite, with magnitudes below 1e9,
    /// and invertible without collapsing (|determinant| ≥ 1e-9).
    pub fn is_valid_layer_transform(self) -> bool {
        self.to_array().iter().all(|v| v.abs() < 1e9) && self.determinant().abs() >= 1e-9
    }

    /// Whether the map sends pixel centers to pixel centers without resampling: a whole-pixel
    /// translation after a rotation by a multiple of 90° or a flip.
    pub fn is_pixel_exact(self) -> bool {
        let unit = |v: f64| v == 1.0 || v == -1.0;
        let permutes = (unit(self.a) && self.b == 0.0 && self.c == 0.0 && unit(self.d))
            || (self.a == 0.0 && unit(self.b) && unit(self.c) && self.d == 0.0);
        let whole = |v: f64| v.fract() == 0.0 && v.abs() <= f64::from(i32::MAX);
        permutes && whole(self.e) && whole(self.f)
    }

    /// The whole-pixel offset of a pure translation by whole pixels: the transforms that
    /// compositors apply without resampling. `None` for any other transform.
    pub fn integer_translation(self) -> Option<(i64, i64)> {
        let whole = |v: f64| v.fract() == 0.0 && v.abs() <= f64::from(i32::MAX);
        let linear_identity = self.a == 1.0 && self.b == 0.0 && self.c == 0.0 && self.d == 1.0;
        (linear_identity && whole(self.e) && whole(self.f))
            .then_some((self.e as i64, self.f as i64))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composition_applies_in_order() {
        let scale = Affine {
            a: 2.0,
            d: 3.0,
            ..Affine::IDENTITY
        };
        let move_by = Affine::translation(10.0, -4.0);
        // Scale then move: (1, 1) → (2, 3) → (12, -1).
        assert_eq!(scale.then(move_by).apply(1.0, 1.0), (12.0, -1.0));
        // Move then scale: (1, 1) → (11, -3) → (22, -9).
        assert_eq!(move_by.then(scale).apply(1.0, 1.0), (22.0, -9.0));
        assert!(Affine::IDENTITY.then(move_by) == move_by);
    }

    #[test]
    fn inverses_undo_and_invalid_transforms_are_refused() {
        let t = Affine::rotation(0.3)
            .then(Affine::scale(2.0, -0.5))
            .then(Affine::translation(7.0, -3.0));
        let back = t.then(t.inverse().unwrap());
        for (x, y) in [(0.0, 0.0), (10.0, -4.0)] {
            let (bx, by) = back.apply(x, y);
            assert!((bx - x).abs() < 1e-12 && (by - y).abs() < 1e-12);
        }
        assert!(t.is_valid_layer_transform());
        assert_eq!(Affine::scale(0.0, 1.0).inverse(), None);
        assert!(!Affine::scale(1e-5, 1e-5).is_valid_layer_transform());
        assert!(!Affine::translation(1e10, 0.0).is_valid_layer_transform());
        assert!(!Affine::translation(f64::NAN, 0.0).is_valid_layer_transform());
        assert_eq!(
            Affine::rotation(std::f64::consts::FRAC_PI_4).map_rect([0.0, 0.0, 2.0, 2.0])[2],
            2f64.sqrt()
        );
    }

    #[test]
    fn snapping_makes_near_whole_numbers_whole() {
        let quarter = Affine::rotation(std::f64::consts::FRAC_PI_2)
            .then(Affine::translation(3.000_000_000_01, 0.5));
        assert!(!quarter.is_pixel_exact());
        let snapped = quarter.snapped();
        assert_eq!(snapped.to_array(), [0.0, 1.0, -1.0, 0.0, 3.0, 0.5]);
        assert_eq!(Affine::rotation(0.3).snapped(), Affine::rotation(0.3));
    }

    #[test]
    fn quarter_turns_and_flips_are_pixel_exact() {
        let quarter = Affine {
            a: 0.0,
            b: 1.0,
            c: -1.0,
            d: 0.0,
            e: 5.0,
            f: 0.0,
        };
        assert!(quarter.is_pixel_exact());
        assert!(Affine::scale(-1.0, 1.0).is_pixel_exact());
        assert!(
            !Affine::scale(-1.0, 1.0)
                .then(Affine::translation(0.5, 0.0))
                .is_pixel_exact()
        );
        assert!(!Affine::rotation(0.1).is_pixel_exact());
        assert!(!Affine::scale(2.0, 2.0).is_pixel_exact());
    }

    #[test]
    fn only_whole_pixel_translations_are_exact() {
        assert_eq!(
            Affine::translation(-3.0, 7.0).integer_translation(),
            Some((-3, 7))
        );
        assert_eq!(Affine::translation(0.5, 0.0).integer_translation(), None);
        let rotated = Affine {
            b: 1.0,
            c: -1.0,
            a: 0.0,
            d: 0.0,
            ..Affine::IDENTITY
        };
        assert_eq!(rotated.integer_translation(), None);
        assert_eq!(Affine::translation(1e12, 0.0).integer_translation(), None);
        assert_eq!(
            Affine::from_array(Affine::translation(2.0, 3.0).to_array()),
            Affine::translation(2.0, 3.0)
        );
    }
}
