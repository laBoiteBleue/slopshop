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

/// A 2D projective map (a homography, ADR 0038): `(x, y) ↦ ((a·x + c·y + e) / w,
/// (b·x + d·y + f) / w)` with `w = g·x + h·y + i`. Straight lines stay straight, parallel ones
/// need not. The affine maps are the special case `g = h = 0`, `i = 1`; every operation on them
/// computes exactly what [`Affine`]'s does, so that documents without a projective layer keep
/// their values to the bit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Projective {
    m: [f64; 9],
}

impl Default for Projective {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl From<Affine> for Projective {
    fn from(t: Affine) -> Self {
        Projective {
            m: [t.a, t.b, t.c, t.d, t.e, t.f, 0.0, 0.0, 1.0],
        }
    }
}

impl Projective {
    pub const IDENTITY: Projective = Projective {
        m: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0],
    };

    pub const fn translation(x: f64, y: f64) -> Self {
        Projective {
            m: [1.0, 0.0, 0.0, 1.0, x, y, 0.0, 0.0, 1.0],
        }
    }

    /// The nine numbers `[a, b, c, d, e, f, g, h, i]`.
    pub const fn to_array(self) -> [f64; 9] {
        self.m
    }

    /// The map of the nine numbers `[a, b, c, d, e, f, g, h, i]`, normalized so that `i` is 1
    /// (the same map); `None` when `i` is 0, not finite, or a number is not.
    pub fn from_array(m: [f64; 9]) -> Option<Self> {
        let i = m[8];
        if !m.iter().all(|v| v.is_finite()) || i == 0.0 {
            return None;
        }
        let m = if i == 1.0 { m } else { m.map(|v| v / i) };
        Some(Projective { m })
    }

    /// The affine map, when this is one.
    pub fn as_affine(self) -> Option<Affine> {
        let [a, b, c, d, e, f, g, h, i] = self.m;
        (g == 0.0 && h == 0.0 && i == 1.0).then_some(Affine { a, b, c, d, e, f })
    }

    pub fn is_affine(self) -> bool {
        self.as_affine().is_some()
    }

    pub fn is_identity(self) -> bool {
        self == Self::IDENTITY
    }

    pub fn is_finite(self) -> bool {
        self.m.iter().all(|v| v.is_finite())
    }

    /// `self` then `after`: the map `p ↦ after(self(p))`.
    pub fn then(self, after: Projective) -> Projective {
        if let (Some(t), Some(u)) = (self.as_affine(), after.as_affine()) {
            return t.then(u).into();
        }
        // Column-major 3 × 3 matrices: [a b g; c d h; e f i] as columns of x, y, 1.
        let [a1, b1, c1, d1, e1, f1, g1, h1, i1] = self.m;
        let [a2, b2, c2, d2, e2, f2, g2, h2, i2] = after.m;
        // after · self, rows (x', y', w') of after applied to the columns of self.
        let row = |r: [f64; 3], col: [f64; 3]| r[0] * col[0] + r[1] * col[1] + r[2] * col[2];
        let (rx, ry, rw) = ([a2, c2, e2], [b2, d2, f2], [g2, h2, i2]);
        let (cx, cy, c1_) = ([a1, b1, g1], [c1, d1, h1], [e1, f1, i1]);
        let m = [
            row(rx, cx),
            row(ry, cx),
            row(rx, cy),
            row(ry, cy),
            row(rx, c1_),
            row(ry, c1_),
            row(rw, cx),
            row(rw, cy),
            row(rw, c1_),
        ];
        let i = m[8];
        Projective {
            m: if i != 0.0 && i.is_finite() {
                m.map(|v| v / i)
            } else {
                m
            },
        }
    }

    /// `w` at (`x`, `y`): positive on the side of the horizon line the layer lies on.
    pub fn w(self, x: f64, y: f64) -> f64 {
        self.m[6] * x + self.m[7] * y + self.m[8]
    }

    /// The image of (`x`, `y`); not finite on the horizon line.
    pub fn apply(self, x: f64, y: f64) -> (f64, f64) {
        if let Some(t) = self.as_affine() {
            return t.apply(x, y);
        }
        let [a, b, c, d, e, f, ..] = self.m;
        let w = self.w(x, y);
        ((a * x + c * y + e) / w, (b * x + d * y + f) / w)
    }

    /// The determinant of the 3 × 3 matrix (of the linear part for an affine map).
    pub fn determinant(self) -> f64 {
        if let Some(t) = self.as_affine() {
            return t.determinant();
        }
        let [a, b, c, d, e, f, g, h, i] = self.m;
        a * (d * i - f * h) - c * (b * i - f * g) + e * (b * h - d * g)
    }

    /// The inverse map; `None` when the map is not invertible (or not finite).
    pub fn inverse(self) -> Option<Projective> {
        if let Some(t) = self.as_affine() {
            return t.inverse().map(Into::into);
        }
        let det = self.determinant();
        if !det.is_finite() || det == 0.0 {
            return None;
        }
        let [a, b, c, d, e, f, g, h, i] = self.m;
        // The adjugate of [a c e; b d f; g h i], divided by the determinant.
        let inv = [
            (d * i - f * h) / det,
            (f * g - b * i) / det,
            (e * h - c * i) / det,
            (a * i - e * g) / det,
            (c * f - e * d) / det,
            (e * b - a * f) / det,
            (b * h - d * g) / det,
            (c * g - a * h) / det,
            (a * d - c * b) / det,
        ];
        Projective::from_array(inv)
    }

    /// The bounding box `[x0, y0, x1, y1]` of the image of the rectangle `[x0, y0, x1, y1]`:
    /// its four corners' (exact for a quad, which a homography keeps convex), or the whole
    /// plane when part of the rectangle reaches the horizon line.
    pub fn map_rect(self, rect: [f64; 4]) -> [f64; 4] {
        if let Some(t) = self.as_affine() {
            return t.map_rect(rect);
        }
        let [x0, y0, x1, y1] = rect;
        let corners = [(x0, y0), (x1, y0), (x0, y1), (x1, y1)];
        if corners.iter().any(|&(x, y)| self.w(x, y) <= 0.0) {
            return [
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
                f64::INFINITY,
                f64::INFINITY,
            ];
        }
        let mut out = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for (x, y) in corners.map(|(x, y)| self.apply(x, y)) {
            out = [out[0].min(x), out[1].min(y), out[2].max(x), out[3].max(y)];
        }
        out
    }

    /// The same map with every number within 1e-9 of a whole number made whole (see
    /// [`Affine::snapped`]).
    pub fn snapped(self) -> Projective {
        Projective {
            m: self.m.map(|v| {
                let whole = v.round();
                if (v - whole).abs() < 1e-9 { whole } else { v }
            }),
        }
    }

    /// Whether a layer whose content spans `bounds` (`[x0, y0, x1, y1]`) may have this transform
    /// (ADR 0018, 0038): an affine one as [`Affine::is_valid_layer_transform`] says; a projective
    /// one with its numbers below 1e9, invertible, and every point of `bounds` on the near side of
    /// the horizon line (`w` at least 1e-6 at each corner, so at every point: `w` is linear).
    pub fn is_valid_layer_transform(self, bounds: [f64; 4]) -> bool {
        if let Some(t) = self.as_affine() {
            return t.is_valid_layer_transform();
        }
        let [x0, y0, x1, y1] = bounds;
        self.m.iter().all(|v| v.abs() < 1e9)
            && self.determinant().abs() >= 1e-9
            && [(x0, y0), (x1, y0), (x0, y1), (x1, y1)]
                .iter()
                .all(|&(x, y)| self.w(x, y) >= 1e-6)
    }

    /// See [`Affine::is_pixel_exact`]; never for a projective map.
    pub fn is_pixel_exact(self) -> bool {
        self.as_affine().is_some_and(Affine::is_pixel_exact)
    }

    /// See [`Affine::integer_translation`]; never for a projective map.
    pub fn integer_translation(self) -> Option<(i64, i64)> {
        self.as_affine().and_then(Affine::integer_translation)
    }

    /// The local linear part at (`x`, `y`): the Jacobian `[∂x'/∂x, ∂y'/∂x, ∂x'/∂y, ∂y'/∂y]`
    /// (as `[a, b, c, d]` of an affine map), constant for an affine map.
    pub fn jacobian(self, x: f64, y: f64) -> [f64; 4] {
        if let Some(t) = self.as_affine() {
            return [t.a, t.b, t.c, t.d];
        }
        let [a, b, c, d, e, f, g, h, _] = self.m;
        let w = self.w(x, y);
        let (u, v) = (a * x + c * y + e, b * x + d * y + f);
        let w2 = w * w;
        [
            (a * w - u * g) / w2,
            (b * w - v * g) / w2,
            (c * w - u * h) / w2,
            (d * w - v * h) / w2,
        ]
    }

    /// The affine map that agrees with this one at (`x`, `y`) to the first order.
    pub fn local_affine(self, x: f64, y: f64) -> Affine {
        if let Some(t) = self.as_affine() {
            return t;
        }
        let [a, b, c, d] = self.jacobian(x, y);
        let (px, py) = self.apply(x, y);
        Affine {
            a,
            b,
            c,
            d,
            e: px - a * x - c * y,
            f: py - b * x - d * y,
        }
    }

    /// The map sending the corners of the rectangle `[x0, y0, x1, y1]` (top left, top right,
    /// bottom right, bottom left) to `quad`, in that order; `None` when three of the points are
    /// on one line (or the rectangle is empty).
    pub fn from_rect_to_quad(rect: [f64; 4], quad: [(f64, f64); 4]) -> Option<Projective> {
        let [x0, y0, x1, y1] = rect;
        let (w, h) = (x1 - x0, y1 - y0);
        if !(w > 0.0 && h > 0.0) {
            return None;
        }
        // The unit square to the quad (Heckbert's square-to-quad), after the rectangle to the
        // unit square.
        let [(qx0, qy0), (qx1, qy1), (qx2, qy2), (qx3, qy3)] = quad;
        let (sx, sy) = (qx0 - qx1 + qx2 - qx3, qy0 - qy1 + qy2 - qy3);
        let (dx1, dy1) = (qx1 - qx2, qy1 - qy2);
        let (dx2, dy2) = (qx3 - qx2, qy3 - qy2);
        let den = dx1 * dy2 - dx2 * dy1;
        if den == 0.0 || !den.is_finite() {
            return None;
        }
        let g = (sx * dy2 - dx2 * sy) / den;
        let hh = (dx1 * sy - sx * dy1) / den;
        let square = Projective::from_array([
            qx1 - qx0 + g * qx1,
            qy1 - qy0 + g * qy1,
            qx3 - qx0 + hh * qx3,
            qy3 - qy0 + hh * qy3,
            qx0,
            qy0,
            g,
            hh,
            1.0,
        ])?;
        let to_square = Affine::translation(-x0, -y0).then(Affine::scale(1.0 / w, 1.0 / h));
        let map = Projective::from(to_square).then(square);
        (map.is_finite() && map.determinant() != 0.0).then_some(map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projective_maps_are_affine_ones_exactly_when_affine() {
        let t = Affine::rotation(0.3)
            .then(Affine::scale(2.0, -0.5))
            .then(Affine::translation(7.0, -3.0));
        let u = Affine::translation(0.25, 9.0).then(Affine::rotation(-1.1));
        let (pt, pu) = (Projective::from(t), Projective::from(u));
        assert_eq!(pt.as_affine(), Some(t));
        assert_eq!(pt.then(pu).as_affine(), Some(t.then(u)));
        assert_eq!(pt.inverse().and_then(Projective::as_affine), t.inverse());
        assert_eq!(pt.apply(3.0, -8.5), t.apply(3.0, -8.5));
        assert_eq!(
            pt.map_rect([1.0, 2.0, 30.0, 40.0]),
            t.map_rect([1.0, 2.0, 30.0, 40.0])
        );
        assert_eq!(pt.determinant(), t.determinant());
        assert_eq!(
            Projective::translation(3.0, 4.0).integer_translation(),
            Some((3, 4))
        );
        assert!(Projective::IDENTITY.is_identity());
        assert_eq!(Projective::default(), Projective::IDENTITY);
    }

    /// A perspective: the rectangle 0..100 × 0..50 to a quad narrower at the top.
    fn keystone() -> Projective {
        Projective::from_rect_to_quad(
            [0.0, 0.0, 100.0, 50.0],
            [(20.0, 0.0), (80.0, 0.0), (100.0, 50.0), (0.0, 50.0)],
        )
        .unwrap()
    }

    #[test]
    fn a_rectangle_goes_to_the_quad_asked() {
        let p = keystone();
        assert!(!p.is_affine());
        let close = |(x, y): (f64, f64), (ex, ey): (f64, f64)| {
            assert!(
                (x - ex).abs() < 1e-9 && (y - ey).abs() < 1e-9,
                "({x}, {y}) vs ({ex}, {ey})"
            );
        };
        close(p.apply(0.0, 0.0), (20.0, 0.0));
        close(p.apply(100.0, 0.0), (80.0, 0.0));
        close(p.apply(100.0, 50.0), (100.0, 50.0));
        close(p.apply(0.0, 50.0), (0.0, 50.0));
        // Straight lines stay straight: the middle of the bottom edge stays on it.
        assert!((p.apply(50.0, 50.0).1 - 50.0).abs() < 1e-9);
        // A parallelogram is an affine map.
        let skew = Projective::from_rect_to_quad(
            [0.0, 0.0, 10.0, 10.0],
            [(5.0, 0.0), (15.0, 0.0), (10.0, 10.0), (0.0, 10.0)],
        )
        .unwrap();
        let back = skew.apply(10.0, 0.0);
        close(back, (15.0, 0.0));
        assert!(skew.to_array()[6].abs() < 1e-12 && skew.to_array()[7].abs() < 1e-12);
        // Three corners on one line: none.
        assert_eq!(
            Projective::from_rect_to_quad(
                [0.0, 0.0, 10.0, 10.0],
                [(0.0, 0.0), (5.0, 0.0), (10.0, 0.0), (0.0, 10.0)],
            ),
            None
        );
    }

    #[test]
    fn projective_inverses_and_compositions_undo() {
        let p = keystone();
        let q = Projective::from(Affine::rotation(0.4)).then(p);
        let back = q.then(q.inverse().unwrap());
        for (x, y) in [(0.0, 0.0), (37.0, 12.5), (99.0, 49.0)] {
            let (bx, by) = back.apply(x, y);
            assert!((bx - x).abs() < 1e-9 && (by - y).abs() < 1e-9);
        }
        let (x, y) = q.apply(10.0, 20.0);
        let (ex, ey) = p.apply(
            Affine::rotation(0.4).apply(10.0, 20.0).0,
            Affine::rotation(0.4).apply(10.0, 20.0).1,
        );
        assert!((x - ex).abs() < 1e-9 && (y - ey).abs() < 1e-9);
    }

    #[test]
    fn bounds_and_validity_know_the_horizon() {
        let p = keystone();
        let b = p.map_rect([0.0, 0.0, 100.0, 50.0]);
        for (v, e) in b.iter().zip([0.0, 0.0, 100.0, 50.0]) {
            assert!((v - e).abs() < 1e-9);
        }
        assert!(p.is_valid_layer_transform([0.0, 0.0, 100.0, 50.0]));
        // Far enough on the side where `w` falls, a rectangle crosses the horizon line.
        let h = p.to_array()[7];
        let far = if h < 0.0 {
            [0.0, 0.0, 100.0, 1e6]
        } else {
            [0.0, -1e6, 100.0, 50.0]
        };
        assert!(!p.is_valid_layer_transform(far));
        assert!(p.map_rect(far)[0].is_infinite());
        assert!(Projective::from_array([1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0]).is_none());
        let scaled = Projective::from_array([2.0, 0.0, 0.0, 2.0, 4.0, 6.0, 0.0, 0.0, 2.0]).unwrap();
        assert_eq!(scaled.as_affine(), Some(Affine::translation(2.0, 3.0)));
    }

    #[test]
    fn the_jacobian_is_the_local_linear_part() {
        let p = keystone();
        let (x, y, eps) = (30.0, 20.0, 1e-5);
        let [a, b, c, d] = p.jacobian(x, y);
        let (px, py) = p.apply(x, y);
        let (qx, qy) = p.apply(x + eps, y);
        let (rx, ry) = p.apply(x, y + eps);
        for (got, want) in [
            (a, (qx - px) / eps),
            (b, (qy - py) / eps),
            (c, (rx - px) / eps),
            (d, (ry - py) / eps),
        ] {
            assert!((got - want).abs() < 1e-4, "{got} vs {want}");
        }
        let local = p.local_affine(x, y);
        let (lx, ly) = local.apply(x, y);
        assert!((lx - px).abs() < 1e-9 && (ly - py).abs() < 1e-9);
        assert_eq!(
            Projective::from(Affine::scale(2.0, 3.0)).jacobian(5.0, 5.0),
            [2.0, 0.0, 0.0, 3.0]
        );
    }

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
