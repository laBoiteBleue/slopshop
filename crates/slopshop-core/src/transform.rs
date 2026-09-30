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
