//! Resampling of transformed layers (ADR 0018): an elliptical weighted average (EWA) with a
//! Jinc-windowed Jinc kernel ("EWA Lanczos sharp"), clamped against ringing, from the pyramid
//! level that matches the scale.
//!
//! Everything that depends only on the layer's transform and the output scale (the level, the
//! map to its texels, the ellipse) is computed once per layer in [`Resampling`]; the GPU renderer
//! receives the same numbers and the same kernel table ([`weight_table`]), and runs the same
//! loop as [`Resampling::sample`].
//!
//! A layer in perspective (ADR 0038) has no constant ellipse: [`Resampling::placed`] reads one
//! level for the whole layer, the finest any of its corners needs, and each sample's ellipse comes
//! from the map's Jacobian there, its extent capped at [`MAX_EXTENT`] texels (where the layer
//! recedes most, a sample reads at most that many).
//!
//! Coordinates: texel (i, j) of a level covers `[i, i + 1) × [j, j + 1)` in that level's texel
//! space, its center is at (i + ½, j + ½); document pixel (x, y) is sampled at its center too.

use std::sync::OnceLock;

use crate::geom::Size;
use crate::transform::{Affine, Projective};

/// Radius of the kernel, in texels (of an unstretched ellipse): the third zero of Jinc.
pub const RADIUS: f64 = 3.238_315_484_166_236;
/// Robidoux's sharpening of the EWA Lanczos kernel: the Jinc is narrowed by this factor.
pub const BLUR: f64 = 0.981_250_564_426_935_6;
/// First zero of Jinc: the window's lobe is stretched to end at [`RADIUS`].
const JINC_FIRST_ZERO: f64 = 1.219_669_891_266_504_5;
/// Entries of the kernel table, over r² from 0 to [`RADIUS`]².
pub const TABLE_SIZE: usize = 1024;
/// Texels within this r² of the sample point bound the result (anti-ringing): the 2×2 texels
/// around it when enlarging, and those under the output pixel when reducing.
pub const ANTIRING_R2: f64 = 2.0;
/// Most texels between the sample point and the edge of the ellipse's bounding box, per axis:
/// strongly anisotropic reductions read a coarser level instead.
pub const MAX_EXTENT: f64 = 8.0;

/// Bessel function of the first kind, order 1, for |x| ≤ 12 (power series; cancellation loses
/// about 4 of f64's 16 digits there).
fn bessel_j1(x: f64) -> f64 {
    let q = -(x * x) / 4.0;
    let mut term = x / 2.0;
    let mut sum = term;
    for k in 1..40 {
        term *= q / f64::from(k * (k + 1));
        sum += term;
        if term.abs() < 1e-17 * sum.abs() {
            break;
        }
    }
    sum
}

/// `2·J1(πx) / (πx)`: 1 at 0, first zero at [`JINC_FIRST_ZERO`].
fn jinc(x: f64) -> f64 {
    if x == 0.0 {
        return 1.0;
    }
    let px = std::f64::consts::PI * x;
    2.0 * bessel_j1(px) / px
}

/// The kernel at distance² `r2` (in texels of an unstretched ellipse); 0 beyond [`RADIUS`].
pub fn kernel(r2: f64) -> f64 {
    let r = r2.max(0.0).sqrt();
    if r >= RADIUS {
        return 0.0;
    }
    jinc(r / BLUR) * jinc(r * JINC_FIRST_ZERO / RADIUS)
}

/// The kernel sampled at `TABLE_SIZE` evenly spaced r², from 0 to [`RADIUS`]², as `f32` (the
/// values both compositors interpolate).
pub fn weight_table() -> &'static [f32; TABLE_SIZE] {
    static TABLE: OnceLock<[f32; TABLE_SIZE]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let step = RADIUS * RADIUS / (TABLE_SIZE - 1) as f64;
        std::array::from_fn(|i| kernel(i as f64 * step) as f32)
    })
}

/// The kernel at `r2` from the table, linearly interpolated (what the GPU computes).
pub fn table_weight(table: &[f32; TABLE_SIZE], r2: f64) -> f64 {
    let f = r2 / (RADIUS * RADIUS) * (TABLE_SIZE - 1) as f64;
    if f.is_nan() || f >= (TABLE_SIZE - 1) as f64 {
        return 0.0;
    }
    let i = f.max(0.0) as usize;
    let t = f - i as f64;
    let (w0, w1) = (f64::from(table[i]), f64::from(table[i + 1]));
    w0 + (w1 - w0) * t
}

/// How a sample is computed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Filter {
    /// The texel under the sample point, as is: quarter turns, flips and whole-pixel moves,
    /// sampled at document pixels (ADR 0018).
    Nearest,
    /// EWA: texel offsets `(du, dv)` from the sample point weigh `kernel(r²)` with
    /// `r² = q[0]·du² + q[1]·du·dv + q[2]·dv²`, over the box of half-size `extent` texels.
    Ewa { q: [f64; 3], extent: [f64; 2] },
}

/// How to sample one image placed in the document by a transform, for output pixels of a given
/// size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Resampling {
    /// The pyramid level read.
    pub level: usize,
    /// Document point → texel coordinates of `level`.
    pub to_texel: Affine,
    pub filter: Filter,
    /// Document → image (level 0) pixels.
    inverse: Affine,
    /// Document pixels per output pixel, at least 1.
    scale: f64,
    /// A placement in perspective (ADR 0038); `to_texel` and `filter` are then those at the
    /// image's center, for reference only.
    perspective: Option<Perspective>,
}

/// A layer placed by a projective map: what [`Resampling`] computes at each sample.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Perspective {
    /// Document point → texel coordinates of the level read.
    pub to_texel: Projective,
    /// Document → image (level 0) pixels.
    inverse: Projective,
    /// The image's size (level 0).
    size: Size,
}

impl Resampling {
    /// Sampling an image of `levels` pyramid levels, placed by `transform`, for output pixels
    /// of `scale` document pixels (1 for export; the view's otherwise: a zoomed-in view samples
    /// document pixels, like export). `None` when the transform is not invertible.
    pub fn new(transform: Affine, scale: f64, levels: usize) -> Option<Self> {
        let inverse = transform.inverse()?;
        let scale = at_least_one(scale);
        let (major, minor) = axes(inverse, scale);
        let coarsest = levels.saturating_sub(1);
        let mut level = if minor > 1.0 {
            (minor.log2().floor() as usize).min(coarsest)
        } else {
            0
        };
        while level < coarsest && major / f64::from(1u32 << level) * RADIUS > MAX_EXTENT {
            level += 1;
        }
        Some(Self::build(inverse, scale, level))
    }

    /// Sampling an image of `size` (level 0) and `levels` pyramid levels placed by any
    /// `transform`: [`Self::new`] for an affine one; for a projective one (ADR 0038), the level
    /// is the finest the image's corners need, and each sample's ellipse comes from the
    /// Jacobian there. `None` when the transform is not invertible.
    pub fn placed(transform: Projective, scale: f64, levels: usize, size: Size) -> Option<Self> {
        if let Some(t) = transform.as_affine() {
            return Self::new(t, scale, levels);
        }
        let inverse = transform.inverse()?;
        let scale = at_least_one(scale);
        let (w, h) = (f64::from(size.width), f64::from(size.height));
        let minor = [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)]
            .map(|(x, y)| transform.apply(x, y))
            .into_iter()
            .map(|(x, y)| axes(inverse.local_affine(x, y), scale).1)
            .fold(f64::INFINITY, f64::min);
        let coarsest = levels.saturating_sub(1);
        let level = if minor > 1.0 && minor.is_finite() {
            (minor.log2().floor() as usize).min(coarsest)
        } else {
            0
        };
        Some(Self::build_perspective(inverse, scale, level, size))
    }

    /// The same sampling from another pyramid level (the renderer coarsens levels when tiles do
    /// not fit its cache).
    pub fn at_level(self, level: usize) -> Self {
        match self.perspective {
            Some(p) => Self::build_perspective(p.inverse, self.scale, level, p.size),
            None => Self::build(self.inverse, self.scale, level),
        }
    }

    /// The placement in perspective, if it is one (ADR 0038).
    pub fn perspective(&self) -> Option<&Perspective> {
        self.perspective.as_ref()
    }

    /// Document pixels per output pixel (at least 1): what each sample's ellipse is made for.
    pub fn scale(&self) -> f64 {
        self.scale
    }

    /// The document area where samples read any texel of the level read (`width` × `height`
    /// texels): the whole plane for a layer in perspective whose filter's reach crosses the
    /// horizon line. `None` when the map is not invertible.
    pub fn document_reach(&self, width: u32, height: u32) -> Option<[f64; 4]> {
        let (w, h) = (f64::from(width), f64::from(height));
        if let Some(p) = &self.perspective {
            let m = MAX_EXTENT;
            return Some(p.to_texel.inverse()?.map_rect([-m, -m, w + m, h + m]));
        }
        let [eu, ev] = match self.filter {
            Filter::Nearest => [0.5, 0.5],
            Filter::Ewa { extent, .. } => extent,
        };
        Some(
            self.to_texel
                .inverse()?
                .map_rect([-eu, -ev, w + eu, h + ev]),
        )
    }

    fn build_perspective(inverse: Projective, scale: f64, level: usize, size: Size) -> Self {
        let factor = f64::from(1u32 << level.min(31));
        let to_texel = inverse.then(Affine::scale(1.0 / factor, 1.0 / factor).into());
        // The image's center in the document, where the reference numbers are taken.
        let center = inverse.inverse().map_or((0.0, 0.0), |forward| {
            forward.apply(f64::from(size.width) / 2.0, f64::from(size.height) / 2.0)
        });
        let local = to_texel.local_affine(center.0, center.1);
        Self {
            level,
            to_texel: local,
            filter: ewa(local, scale),
            inverse: inverse.local_affine(center.0, center.1),
            scale,
            perspective: Some(Perspective {
                to_texel,
                inverse,
                size,
            }),
        }
    }

    /// Where document point (`x`, `y`) falls in the level's texels, and the filter there; `None`
    /// beyond the horizon line of a layer in perspective (nothing of it shows there).
    fn at(&self, x: f64, y: f64) -> Option<((f64, f64), Filter)> {
        match &self.perspective {
            None => Some((self.to_texel.apply(x, y), self.filter)),
            Some(p) => {
                if p.to_texel.w(x, y) <= 0.0 {
                    return None;
                }
                let local = p.to_texel.local_affine(x, y);
                Some((p.to_texel.apply(x, y), ewa(local, self.scale)))
            }
        }
    }

    fn build(inverse: Affine, scale: f64, level: usize) -> Self {
        let factor = f64::from(1u32 << level.min(31));
        let to_texel = inverse.then(Affine::scale(1.0 / factor, 1.0 / factor));
        let filter = if scale == 1.0 && level == 0 && inverse.is_pixel_exact() {
            Filter::Nearest
        } else {
            ewa(to_texel, scale)
        };
        Self {
            level,
            to_texel,
            filter,
            inverse,
            scale,
            perspective: None,
        }
    }

    /// The part of the image (level-0 pixels, `[x0, y0, x1, y1]`) that samples within the
    /// document area `[x0, y0, x1, y1]` read.
    pub fn source_area(&self, area: [f64; 4]) -> [f64; 4] {
        let factor = f64::from(1u32 << self.level.min(31));
        if let Some(p) = &self.perspective {
            // Clipped to the image: the area may reach the horizon line, beyond which the
            // inverse map runs off to infinity (ADR 0038).
            let (w, h) = (f64::from(p.size.width), f64::from(p.size.height));
            let [x0, y0, x1, y1] = p.inverse.map_rect(area);
            let m = (MAX_EXTENT + 1.0) * factor;
            return [
                (x0 - m).max(-m),
                (y0 - m).max(-m),
                (x1 + m).min(w + m),
                (y1 + m).min(h + m),
            ];
        }
        let [x0, y0, x1, y1] = self.inverse.map_rect(area);
        let (mx, my) = match self.filter {
            Filter::Nearest => (1.0, 1.0),
            Filter::Ewa { extent, .. } => ((extent[0] + 1.0) * factor, (extent[1] + 1.0) * factor),
        };
        [x0 - mx, y0 - my, x1 + mx, y1 + my]
    }

    /// Whether sampling at document point `(x, y)` reads any texel of a level of `width` ×
    /// `height` texels (if not, the sample is transparent).
    pub fn reaches(&self, (x, y): (f64, f64), width: u32, height: u32) -> bool {
        let Some(((u, v), filter)) = self.at(x, y) else {
            return false;
        };
        let [eu, ev] = match filter {
            Filter::Nearest => [0.5, 0.5],
            Filter::Ewa { extent, .. } => extent,
        };
        u + eu > 0.0 && v + ev > 0.0 && u - eu < f64::from(width) && v - ev < f64::from(height)
    }

    /// Premultiplied color at document point `(x, y)`, and the share of the filter's weight
    /// that falls inside the image. `fetch(i, j)` returns texel (i, j) of the level, `None`
    /// outside the image (transparent there).
    pub fn sample(
        &self,
        table: &[f32; TABLE_SIZE],
        (x, y): (f64, f64),
        mut fetch: impl FnMut(i64, i64) -> Option<[f64; 4]>,
    ) -> ([f64; 4], f64) {
        let Some(((u, v), filter)) = self.at(x, y) else {
            return ([0.0; 4], 0.0);
        };
        let Filter::Ewa { q, extent } = filter else {
            return match fetch(u.floor() as i64, v.floor() as i64) {
                Some(color) => (color, 1.0),
                None => ([0.0; 4], 0.0),
            };
        };
        let radius2 = RADIUS * RADIUS;
        let (i0, i1) = ((u - 0.5 - extent[0]).ceil(), (u - 0.5 + extent[0]).floor());
        let (j0, j1) = ((v - 0.5 - extent[1]).ceil(), (v - 0.5 + extent[1]).floor());
        let mut sum = [0.0; 4];
        let (mut total, mut inside) = (0.0, 0.0);
        let (mut lo, mut hi) = ([f64::INFINITY; 4], [f64::NEG_INFINITY; 4]);
        let mut j = j0;
        while j <= j1 {
            let dv = j + 0.5 - v;
            let mut i = i0;
            while i <= i1 {
                let du = i + 0.5 - u;
                let r2 = q[0] * du * du + q[1] * du * dv + q[2] * dv * dv;
                if r2 < radius2 {
                    let w = table_weight(table, r2);
                    let texel = fetch(i as i64, j as i64);
                    let color = texel.unwrap_or([0.0; 4]);
                    if texel.is_some() {
                        inside += w;
                    }
                    for c in 0..4 {
                        sum[c] += w * color[c];
                    }
                    total += w;
                    if r2 <= ANTIRING_R2 {
                        for c in 0..4 {
                            lo[c] = lo[c].min(color[c]);
                            hi[c] = hi[c].max(color[c]);
                        }
                    }
                }
                i += 1.0;
            }
            j += 1.0;
        }
        if total.is_nan() || total <= 1e-12 {
            return ([0.0; 4], 0.0);
        }
        // The nearest texel center is within r² ≤ ½: `lo` and `hi` are set.
        let color = std::array::from_fn(|c| (sum[c] / total).clamp(lo[c], hi[c]));
        (color, (inside / total).clamp(0.0, 1.0))
    }
}

/// `scale`, at least 1 (NaN: 1).
fn at_least_one(scale: f64) -> f64 {
    if scale > 1.0 { scale } else { 1.0 }
}

/// Lengths of the major and minor axes of an output pixel (a unit circle of `scale` document
/// pixels) in the image's pixels, through `inverse`.
fn axes(inverse: Affine, scale: f64) -> (f64, f64) {
    let (l1, l2, _) = eigen(footprint(inverse, scale));
    (l1.sqrt(), l2.sqrt())
}

/// `J·Jᵀ` as `[p, r, t]` (`[[p, r], [r, t]]`), where `J` maps output-pixel steps to texel steps.
fn footprint(to_texel: Affine, scale: f64) -> [f64; 3] {
    let (a, b, c, d) = (
        to_texel.a * scale,
        to_texel.b * scale,
        to_texel.c * scale,
        to_texel.d * scale,
    );
    [a * a + c * c, a * b + c * d, b * b + d * d]
}

/// Eigenvalues (largest first) of the symmetric `[[p, r], [r, t]]`, and the unit eigenvector of
/// the largest.
fn eigen([p, r, t]: [f64; 3]) -> (f64, f64, (f64, f64)) {
    let mean = (p + t) / 2.0;
    let delta = (((p - t) / 2.0).powi(2) + r * r).sqrt();
    let (l1, l2) = (mean + delta, (mean - delta).max(0.0));
    // Of the two candidate vectors, the longer is the better conditioned.
    let (v1, v2) = ((r, l1 - p), (l1 - t, r));
    let v = if v1.0.hypot(v1.1) >= v2.0.hypot(v2.1) {
        v1
    } else {
        v2
    };
    let len = v.0.hypot(v.1);
    let v = if len > 0.0 {
        (v.0 / len, v.1 / len)
    } else {
        (1.0, 0.0)
    };
    (l1, l2, v)
}

/// The EWA ellipse of an output pixel in texel space, never smaller than one texel per axis, and
/// never larger than [`MAX_EXTENT`] (reductions beyond the coarsest level: the image then covers
/// about one output pixel).
fn ewa(to_texel: Affine, scale: f64) -> Filter {
    let (l1, l2, (x, y)) = eigen(footprint(to_texel, scale));
    let cap = (MAX_EXTENT / RADIUS).powi(2);
    let (l1, l2) = (l1.clamp(1.0, cap), l2.clamp(1.0, cap));
    // M = l1·v·vᵀ + l2·w·wᵀ with w ⟂ v; Q = M⁻¹ (same vectors, inverse values).
    let m = [
        l1 * x * x + l2 * y * y,
        (l1 - l2) * x * y,
        l1 * y * y + l2 * x * x,
    ];
    let (i1, i2) = (1.0 / l1, 1.0 / l2);
    let q = [
        i1 * x * x + i2 * y * y,
        2.0 * (i1 - i2) * x * y,
        i1 * y * y + i2 * x * x,
    ];
    Filter::Ewa {
        q,
        extent: [RADIUS * m[0].sqrt(), RADIUS * m[2].sqrt()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kernel_is_one_at_the_center_and_ends_at_the_radius() {
        assert_eq!(kernel(0.0), 1.0);
        assert!(kernel(RADIUS * RADIUS * 0.9999).abs() < 1e-6);
        assert_eq!(kernel(RADIUS * RADIUS), 0.0);
        // J1's first zero is at 3.8317: Jinc's first zero at 3.8317 / π.
        assert!(jinc(JINC_FIRST_ZERO).abs() < 1e-12);
        assert!((bessel_j1(1.0) - 0.440_050_585_744_933_5).abs() < 1e-15);
        // A negative lobe beyond the first zero (sharpening), then a positive one.
        assert!(kernel(1.5 * 1.5) < 0.0);
        let table = weight_table();
        assert_eq!(table[0], 1.0);
        assert_eq!(table[TABLE_SIZE - 1], 0.0);
        assert!((table_weight(table, 0.7) - kernel(0.7)).abs() < 1e-4);
    }

    /// Samples an image of `size` texels (`None` outside) given by `value`.
    fn sample(
        r: &Resampling,
        size: i64,
        value: impl Fn(i64, i64) -> f64,
        p: (f64, f64),
    ) -> ([f64; 4], f64) {
        r.sample(weight_table(), p, |i, j| {
            ((0..size).contains(&i) && (0..size).contains(&j)).then(|| [value(i, j); 4])
        })
    }

    #[test]
    fn flat_areas_stay_flat_at_any_scale_and_angle() {
        for t in [
            Affine::scale(3.0, 3.0),
            Affine::scale(0.3, 0.7),
            Affine::rotation(0.4).then(Affine::scale(1.3, 0.9)),
        ] {
            let t = t.then(Affine::translation(50.0, 50.0));
            let r = Resampling::new(t, 1.0, 1).unwrap();
            let (color, inside) = sample(&r, 400, |_, _| 0.25, t.apply(200.3, 190.7));
            assert!((color[0] - 0.25).abs() < 1e-9, "{t:?}: {color:?}");
            assert!((inside - 1.0).abs() < 1e-9);
        }
    }

    #[test]
    fn enlarged_edges_do_not_ring() {
        // A hard vertical edge, enlarged 4×: no value beyond the two sides.
        let r = Resampling::new(Affine::scale(4.0, 4.0), 1.0, 1).unwrap();
        let mut seen_between = false;
        // Away from the image's own edges, which fade to transparent.
        for x in 16..64 {
            let (color, _) = sample(
                &r,
                20,
                |i, _| if i < 10 { 0.1 } else { 0.9 },
                (x as f64 + 0.5, 40.5),
            );
            assert!((0.1..=0.9).contains(&color[0]), "x {x}: {}", color[0]);
            seen_between |= color[0] > 0.2 && color[0] < 0.8;
        }
        assert!(seen_between, "the edge is interpolated");
    }

    #[test]
    fn edges_of_the_image_fade_to_transparent() {
        let r = Resampling::new(Affine::scale(2.0, 2.0), 1.0, 1).unwrap();
        let (_, inside_edge) = sample(&r, 10, |_, _| 1.0, (20.0, 10.0));
        assert!(inside_edge > 0.3 && inside_edge < 0.7, "{inside_edge}");
        let (color, inside) = sample(&r, 10, |_, _| 1.0, (30.0, 10.0));
        assert_eq!((color[0], inside), (0.0, 0.0));
    }

    #[test]
    fn levels_follow_the_reduction_and_bound_the_footprint() {
        let at = |t: Affine, scale: f64| Resampling::new(t, scale, 8).unwrap();
        assert_eq!(at(Affine::scale(2.0, 2.0), 1.0).level, 0);
        assert_eq!(at(Affine::scale(0.6, 0.6), 1.0).level, 0);
        assert_eq!(at(Affine::scale(0.3, 0.3), 1.0).level, 1);
        // Zoomed out 4× on a layer at scale 1: level 2.
        assert_eq!(at(Affine::IDENTITY, 4.0).level, 2);
        // Very anisotropic: coarser than the minor axis asks, so the box stays bounded.
        let squeezed = at(Affine::scale(1.0, 0.01), 1.0);
        let Filter::Ewa { extent, .. } = squeezed.filter else {
            panic!("filtered")
        };
        assert!(extent[0] <= MAX_EXTENT + 1e-9 && extent[1] <= MAX_EXTENT + 1e-9);
        // Out of levels: as coarse as it gets, and the box stays bounded.
        let tiny = Resampling::new(Affine::scale(0.001, 0.001), 1.0, 3).unwrap();
        assert_eq!(tiny.level, 2);
        let Filter::Ewa { extent, .. } = tiny.filter else {
            panic!("filtered")
        };
        assert!(extent[0] <= MAX_EXTENT + 1e-9);
    }

    #[test]
    fn exact_transforms_copy_texels() {
        let flip = Affine::scale(-1.0, 1.0).then(Affine::translation(10.0, 0.0));
        let r = Resampling::new(flip, 1.0, 1).unwrap();
        assert_eq!(r.filter, Filter::Nearest);
        // Document pixel 0 shows texel 9.
        let (color, _) = sample(&r, 10, |i, j| (i * 10 + j) as f64, (0.5, 3.5));
        assert_eq!(color[0], 93.0);
        // Zoomed out, it is filtered.
        assert_ne!(
            Resampling::new(flip, 2.0, 2).unwrap().filter,
            Filter::Nearest
        );
    }

    #[test]
    fn a_rotated_ellipse_is_the_pixel_footprint() {
        // Reduced 2× along the image's x axis, then rotated 90°: the ellipse is long along u.
        let t = Affine::scale(0.5, 1.0).then(Affine::rotation(std::f64::consts::FRAC_PI_2));
        let r = Resampling::new(t, 1.0, 1).unwrap();
        let Filter::Ewa { q, extent } = r.filter else {
            panic!("filtered")
        };
        assert!((extent[0] - RADIUS * 2.0).abs() < 1e-9, "{extent:?}");
        assert!((extent[1] - RADIUS).abs() < 1e-9);
        assert!((q[0] - 0.25).abs() < 1e-12 && q[1].abs() < 1e-12 && (q[2] - 1.0).abs() < 1e-12);
    }
}
