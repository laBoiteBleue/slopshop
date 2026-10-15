//! Vector masks (ADR 0041, point 4): a path masking a layer besides its pixel mask, as
//! Photoshop's. Its coverage is drawn in the layer's space (where the pixel mask lies), at the
//! pixels the layer's mask would have, multiplied with the pixel mask into the one mask the
//! compositor and the GPU apply: they read the same image, so they agree by construction.

use std::sync::{Arc, OnceLock};

use super::model::{FillRule, Geometry};
use crate::document::LayerMask;
use crate::geom::Size;
use crate::raster::RasterImage;

/// The largest feather, in pixels (a selection's: its blur reads about three times as far).
pub const MAX_FEATHER: f64 = crate::selection::MAX_FEATHER;

/// A layer's vector mask: a path in the layer's space (any geometry: a live shape stays live,
/// as Photoshop keeps a shape's when it becomes a mask) showing what it encloses, or hiding it
/// when `inverted` (Reveal All). An empty path hides everything (Hide All), or shows everything
/// inverted (Reveal All). Immutable but for `enabled`: a changed path or feather is a new mask
/// ([`VectorMask::with_geometry`], [`VectorMask::with_feather`]), so that what it covers, kept
/// with it, never goes stale.
#[derive(Debug, Clone)]
pub struct VectorMask {
    /// Shared by the clones (undo, duplicates): a path can be long, and a layer stays small.
    shared: Arc<Shared>,
    /// Applied; a disabled vector mask is kept but ignored.
    pub enabled: bool,
}

#[derive(Debug)]
struct Shared {
    outline: Arc<Outline>,
    /// What it covers, drawn when first needed (never saved).
    covered: Covered,
}

/// What a vector mask covers, but for being applied.
#[derive(Debug, PartialEq)]
struct Outline {
    geometry: Geometry,
    inverted: bool,
    feather: f64,
}

impl PartialEq for VectorMask {
    fn eq(&self, other: &Self) -> bool {
        self.shared.outline == other.shared.outline && self.enabled == other.enabled
    }
}

impl VectorMask {
    /// An enabled vector mask of `geometry`, sharp; `None` for an invalid geometry.
    pub fn new(geometry: Geometry, inverted: bool) -> Option<Self> {
        geometry
            .is_valid()
            .then(|| Self::of(geometry, inverted, 0.0, true))
    }

    fn of(geometry: Geometry, inverted: bool, feather: f64, enabled: bool) -> Self {
        Self {
            shared: Arc::new(Shared {
                outline: Arc::new(Outline {
                    geometry,
                    inverted,
                    feather,
                }),
                covered: Covered::default(),
            }),
            enabled,
        }
    }

    /// Reveal All: an empty path, inverted (everything shows until a path hides some).
    pub fn reveal_all() -> Self {
        Self::empty(true)
    }

    /// Hide All: an empty path (nothing shows until a path reveals some).
    pub fn hide_all() -> Self {
        Self::empty(false)
    }

    fn empty(inverted: bool) -> Self {
        let none = Geometry::Path {
            subpaths: Vec::new(),
            rule: FillRule::NonZero,
        };
        Self::of(none, inverted, 0.0, true)
    }

    pub fn geometry(&self) -> &Geometry {
        &self.shared.outline.geometry
    }

    /// The path hides what it encloses (Reveal All) rather than showing it.
    pub fn inverted(&self) -> bool {
        self.shared.outline.inverted
    }

    /// How far its edge is softened: a Gaussian's standard deviation, in the layer's pixels.
    pub fn feather(&self) -> f64 {
        self.shared.outline.feather
    }

    /// The same mask along another path; `None` for an invalid geometry.
    pub fn with_geometry(&self, geometry: Geometry) -> Option<Self> {
        geometry
            .is_valid()
            .then(|| Self::of(geometry, self.inverted(), self.feather(), self.enabled))
    }

    /// The same mask softened by `feather` pixels; `None` out of `[0, MAX_FEATHER]`.
    pub fn with_feather(&self, feather: f64) -> Option<Self> {
        (feather.is_finite() && (0.0..=MAX_FEATHER).contains(&feather)).then(|| {
            Self::of(
                self.geometry().clone(),
                self.inverted(),
                feather,
                self.enabled,
            )
        })
    }

    /// Whether it may be a layer's: a valid geometry, a feather in range.
    pub fn is_valid(&self) -> bool {
        let feather = self.feather();
        self.geometry().is_valid() && feather.is_finite() && (0.0..=MAX_FEATHER).contains(&feather)
    }

    /// What it covers over `size` pixels of the layer's space, multiplied by `pixel` (the
    /// layer's enabled pixel mask, of that size): drawn now if it is not, then kept with the
    /// mask. `None` if it cannot be drawn (an empty size).
    pub(crate) fn applied(
        &self,
        size: Size,
        pixel: Option<&Arc<RasterImage>>,
    ) -> Option<&LayerMask> {
        self.shared.covered.get(self, size, pixel)
    }

    /// Whether it lets point `p` of the layer's space show (its feather left out: the edge is
    /// where it is half covered).
    pub fn shows(&self, p: [f64; 2]) -> bool {
        let fill = super::model::Shape {
            geometry: self.geometry().clone(),
            fill: Some(super::model::Paint::Solid(crate::color::LinearRgba::new(
                1.0, 1.0, 1.0, 1.0,
            ))),
            stroke: None,
        };
        super::path::shape_covers(&fill, p) != self.inverted()
    }

    /// What it covers alone over `size` pixels of the layer's space (its thumbnail, a selection
    /// from it).
    pub fn coverage(&self, size: Size) -> Option<RasterImage> {
        let o = &self.shared.outline;
        crate::selection::vector_mask(&o.geometry, size, o.inverted, o.feather, None).ok()
    }

    /// Forget what it covers: the layer it masks changed, so that the drawings it keeps do not
    /// pile up.
    pub(crate) fn forget(&mut self) {
        self.shared = Arc::new(Shared {
            outline: Arc::clone(&self.shared.outline),
            covered: Covered::default(),
        });
    }
}

/// `geometry` of a layer placed in the document by `from`, in the space of a layer placed by
/// `to` (Layer > Vector Mask > Current Path): as it is when both are placed alike, else mapped
/// (a live shape the map would not keep, a rotated rectangle say, as a regular path). `None`
/// under a projective map (a Bézier is no longer one there, ADR 0038), or one that collapses.
pub fn placed_path(
    geometry: &Geometry,
    from: crate::transform::Projective,
    to: crate::transform::Projective,
) -> Option<Geometry> {
    let map = from.then(to.inverse()?).as_affine()?;
    if map.is_identity() {
        return Some(geometry.clone());
    }
    geometry
        .transformed(map)
        .or_else(|| super::path::as_path(geometry).transformed(map))
}

/// What a vector mask covers, for each size and pixel mask it was drawn with, in a chain (a
/// drawing handed out is never replaced). The layer's edits start a new one.
#[derive(Debug, Default)]
struct Covered {
    ready: OnceLock<(Size, Option<Arc<RasterImage>>, Option<LayerMask>)>,
    next: OnceLock<Box<Covered>>,
}

impl Covered {
    fn get(
        &self,
        mask: &VectorMask,
        size: Size,
        pixel: Option<&Arc<RasterImage>>,
    ) -> Option<&LayerMask> {
        let (at, with, drawn) = self.ready.get_or_init(|| {
            let o = &mask.shared.outline;
            let image = crate::selection::vector_mask(
                &o.geometry,
                size,
                o.inverted,
                o.feather,
                pixel.map(|p| p.as_ref()),
            )
            .ok();
            let drawn = image.map(|image| LayerMask {
                image: Arc::new(image),
                enabled: true,
                replaces_alpha: false,
                original: None,
            });
            (size, pixel.cloned(), drawn)
        });
        let same_pixel = match (with, pixel) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if *at == size && same_pixel {
            return drawn.as_ref();
        }
        self.next.get_or_init(Box::default).get(mask, size, pixel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shape::model::{Segment, Subpath};

    fn square(l: f64, t: f64, r: f64, b: f64) -> Geometry {
        Geometry::Path {
            subpaths: vec![Subpath {
                start: [l, t],
                segments: vec![
                    Segment::Line([r, t]),
                    Segment::Line([r, b]),
                    Segment::Line([l, b]),
                ],
                closed: true,
            }],
            rule: FillRule::NonZero,
        }
    }

    #[test]
    fn a_path_shows_what_it_encloses_and_inverted_hides_it() {
        let size = Size::new(20, 10);
        let mask = VectorMask::new(square(5.0, 0.0, 15.0, 10.0), false).unwrap();
        let shown = mask.coverage(size).unwrap();
        assert_eq!(shown.gray_at(2, 5), 0.0);
        assert_eq!(shown.gray_at(10, 5), 1.0);
        let inverted = VectorMask::new(square(5.0, 0.0, 15.0, 10.0), true).unwrap();
        let hidden = inverted.coverage(size).unwrap();
        assert_eq!(hidden.gray_at(2, 5), 1.0);
        assert_eq!(hidden.gray_at(10, 5), 0.0);
        // A half-covered column is anti-aliased.
        let half = VectorMask::new(square(5.5, 0.0, 15.0, 10.0), false).unwrap();
        let v = half.coverage(size).unwrap().gray_at(5, 5);
        assert!((v - 0.5).abs() < 0.01, "{v}");
    }

    #[test]
    fn a_path_is_placed_from_one_layer_into_another() {
        use crate::transform::{Affine, Projective};
        let ellipse = Geometry::Ellipse {
            center: [10.0, 20.0],
            radii: [4.0, 2.0],
        };
        let at = |x, y| Projective::from(Affine::translation(x, y));
        // Placed alike: the same live shape.
        assert_eq!(
            placed_path(&ellipse, at(5.0, 5.0), at(5.0, 5.0)),
            Some(ellipse.clone())
        );
        // Moved: the ellipse moves, still live.
        assert_eq!(
            placed_path(&ellipse, at(5.0, 0.0), at(0.0, 0.0)),
            Some(Geometry::Ellipse {
                center: [15.0, 20.0],
                radii: [4.0, 2.0],
            })
        );
        // Turned: a path of the same outline.
        let turned = Projective::from(Affine::rotation(0.3));
        let path = placed_path(&ellipse, turned, Projective::IDENTITY).unwrap();
        assert!(matches!(path, Geometry::Path { .. }));
        // In perspective: refused.
        let keystone = Projective::from_rect_to_quad(
            [0.0, 0.0, 10.0, 10.0],
            [(0.0, 0.0), (10.0, 1.0), (9.0, 9.0), (1.0, 10.0)],
        )
        .unwrap();
        assert!(placed_path(&ellipse, keystone, Projective::IDENTITY).is_none());
    }

    #[test]
    fn reveal_all_shows_everything_and_hide_all_nothing() {
        let size = Size::new(300, 300);
        let all = VectorMask::reveal_all().coverage(size).unwrap();
        let none = VectorMask::hide_all().coverage(size).unwrap();
        for (x, y) in [(0, 0), (299, 299), (150, 7)] {
            assert_eq!(all.gray_at(x, y), 1.0);
            assert_eq!(none.gray_at(x, y), 0.0);
        }
    }

    #[test]
    fn feather_softens_the_edge() {
        let size = Size::new(60, 20);
        let mask = VectorMask::new(square(30.0, -10.0, 100.0, 30.0), false)
            .unwrap()
            .with_feather(4.0)
            .unwrap();
        let image = mask.coverage(size).unwrap();
        let edge = image.gray_at(30, 10);
        assert!((edge - 0.5).abs() < 0.1, "{edge}");
        assert!(image.gray_at(26, 10) > 0.05);
        assert!(image.gray_at(34, 10) < 0.95);
        assert!(image.gray_at(10, 10) < 0.01);
        assert!(mask.with_feather(MAX_FEATHER + 1.0).is_none());
        assert!(mask.with_feather(f64::NAN).is_none());
    }

    #[test]
    fn the_pixel_mask_multiplies_the_coverage_and_drawings_are_kept() {
        let size = Size::new(4, 1);
        let pixel = Arc::new(
            RasterImage::from_pixels(size, crate::shape::COVERAGE_FORMAT, &[255, 128, 255, 0])
                .unwrap(),
        );
        let mask = VectorMask::new(square(0.0, 0.0, 2.0, 1.0), false).unwrap();
        let applied = mask.applied(size, Some(&pixel)).unwrap();
        let v: Vec<f32> = (0..4).map(|x| applied.image.gray_at(x, 0)).collect();
        assert_eq!(v[0], 1.0);
        assert!((v[1] - 128.0 / 255.0).abs() < 1e-3, "{v:?}");
        assert_eq!(&v[2..], &[0.0, 0.0]);
        // Asked again the same way: the same drawing.
        let again = mask.applied(size, Some(&pixel)).unwrap();
        assert!(Arc::ptr_eq(&applied.image, &again.image));
        // Without the pixel mask: another one.
        let alone = mask.applied(size, None).unwrap();
        assert_eq!(alone.image.gray_at(1, 0), 1.0);
    }
}
