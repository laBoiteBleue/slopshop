//! Color lookup tables (Image > Adjustments > Color Lookup): a 3D table mapping each color to
//! another, or three 1D tables, one per channel, as `.cube` and `.3dl` files hold them. A table
//! is immutable and shared (`Arc`): the adjustment layers and stack entries using it, the history
//! and the renderer hold the same allocation. The values are what the file gives, read on
//! sRGB-encoded colors (`Adjustment::ColorLookup` says where it applies them).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use crate::color::{AlphaMode, ChannelLayout, ColorSpace, PixelFormat, SampleType};
use crate::geom::Size;
use crate::raster::TILE_SIZE;
use crate::raster::{RasterError, RasterImage};
use crate::tile::TileCoord;

/// The largest 3D table: 129 points a side (`.cube` files allow 256, but a table of 256³ is
/// 200 MB; common ones are 17, 32, 33, 64 or 65).
pub const MAX_3D_SIZE: u32 = 129;
/// The largest 1D table (the `.cube` specification's limit).
pub const MAX_1D_SIZE: u32 = 65536;
/// Values and domain bounds are finite numbers within this.
pub const MAX_VALUE: f32 = 1e6;

/// Process-unique identity of a [`ColorLut`] (the app refers to tables by it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LutId(u64);

impl LutId {
    fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

/// Whether a table maps colors (3D) or each channel on its own (1D).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LutDimension {
    One,
    Three,
}

impl LutDimension {
    /// The number of entries of a table of `size` points a side.
    pub fn entries(self, size: u32) -> usize {
        match self {
            LutDimension::One => size as usize,
            LutDimension::Three => (size as usize).pow(3),
        }
    }
}

/// Why a table cannot be made.
#[derive(Debug, Clone, PartialEq)]
pub enum LutError {
    /// The size is out of range for the dimension (2 to [`MAX_3D_SIZE`] or [`MAX_1D_SIZE`]).
    Size(u32),
    /// The values are not `3 × entries` of them.
    Length { expected: usize, actual: usize },
    /// A value or a domain bound is not finite or beyond [`MAX_VALUE`].
    Value,
    /// A domain's minimum is not below its maximum.
    Domain,
    /// The image holding the values is not one [`ColorLut::image`] makes.
    Image,
}

impl std::fmt::Display for LutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LutError::Size(size) => write!(f, "a lookup table of size {size} is not supported"),
            LutError::Length { expected, actual } => {
                write!(f, "a lookup table needs {expected} values, not {actual}")
            }
            LutError::Value => write!(f, "a lookup table value is not a usable number"),
            LutError::Domain => write!(f, "a lookup table's domain is empty"),
            LutError::Image => write!(f, "a lookup table's image does not match it"),
        }
    }
}

impl std::error::Error for LutError {}

/// A color lookup table. Inputs are mapped from the domain (`domain[0]` to `domain[1]` per
/// channel, clamped) to the table's points; 3D tables are interpolated tetrahedrally (the four
/// points around the color along the cube's diagonal: smoother than trilinear on neutral
/// colors, the usual choice of grading software), 1D ones linearly.
pub struct ColorLut {
    id: LutId,
    /// What the user knows it by: the file's name.
    name: String,
    dimension: LutDimension,
    size: u32,
    /// Per channel, the inputs mapped to the first and the last point.
    domain: [[f32; 3]; 2],
    /// RGB triplets. 3D: red changing fastest, then green, then blue (`.cube` order), entry
    /// `(b · size + g) · size + r`. 1D: entry `i` holds the three channels' outputs at point `i`.
    values: Box<[f32]>,
    /// The values as an image, for files (made once: saving it again finds its tiles known).
    image: OnceLock<Arc<RasterImage>>,
}

impl std::fmt::Debug for ColorLut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ColorLut")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("dimension", &self.dimension)
            .field("size", &self.size)
            .field("domain", &self.domain)
            .finish_non_exhaustive()
    }
}

/// Tables are equal when they map colors the same way and have the same name.
impl PartialEq for ColorLut {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            || (self.name == other.name
                && self.dimension == other.dimension
                && self.size == other.size
                && self.domain == other.domain
                && self.values == other.values)
    }
}

/// The width of [`ColorLut::image`]: one tile.
const IMAGE_WIDTH: u32 = TILE_SIZE;

impl ColorLut {
    /// The domain of most tables: `[0, 1]` in each channel.
    pub const UNIT_DOMAIN: [[f32; 3]; 2] = [[0.0; 3], [1.0; 3]];

    /// A table of `size` points a side, `values` as [`ColorLut`] orders them.
    pub fn new(
        name: impl Into<String>,
        dimension: LutDimension,
        size: u32,
        domain: [[f32; 3]; 2],
        values: Vec<f32>,
    ) -> Result<Arc<Self>, LutError> {
        let max = match dimension {
            LutDimension::One => MAX_1D_SIZE,
            LutDimension::Three => MAX_3D_SIZE,
        };
        if !(2..=max).contains(&size) {
            return Err(LutError::Size(size));
        }
        let expected = dimension.entries(size) * 3;
        if values.len() != expected {
            return Err(LutError::Length {
                expected,
                actual: values.len(),
            });
        }
        let usable = |v: &f32| v.is_finite() && v.abs() <= MAX_VALUE;
        if !values.iter().all(usable) || !domain.iter().flatten().all(usable) {
            return Err(LutError::Value);
        }
        if (0..3).any(|i| domain[0][i] >= domain[1][i]) {
            return Err(LutError::Domain);
        }
        Ok(Arc::new(Self {
            id: LutId::next(),
            name: name.into(),
            dimension,
            size,
            domain,
            values: values.into_boxed_slice(),
            image: OnceLock::new(),
        }))
    }

    pub fn id(&self) -> LutId {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn dimension(&self) -> LutDimension {
        self.dimension
    }

    pub fn size(&self) -> u32 {
        self.size
    }

    pub fn domain(&self) -> [[f32; 3]; 2] {
        self.domain
    }

    /// The RGB triplets, in [`ColorLut`]'s order.
    pub fn values(&self) -> &[f32] {
        &self.values
    }

    /// The color `c` maps to.
    pub fn apply(&self, c: [f64; 3]) -> [f64; 3] {
        let n = self.size as usize;
        let last = (n - 1) as f64;
        // Each channel as a position among the points: the integer point below and how far
        // past it (the last cell ends exactly on the last point).
        let at = |i: usize| {
            let (lo, hi) = (f64::from(self.domain[0][i]), f64::from(self.domain[1][i]));
            let x = ((c[i] - lo) / (hi - lo)).clamp(0.0, 1.0) * last;
            // NaN in, the first point out (clamp keeps NaN; the cast makes it 0).
            let x = if x.is_nan() { 0.0 } else { x };
            let base = (x.floor() as usize).min(n - 2);
            (base, x - base as f64)
        };
        let value = |entry: usize, channel: usize| f64::from(self.values[3 * entry + channel]);
        match self.dimension {
            LutDimension::One => std::array::from_fn(|i| {
                let (base, t) = at(i);
                let (a, b) = (value(base, i), value(base + 1, i));
                a + (b - a) * t
            }),
            LutDimension::Three => {
                let ((r, fr), (g, fg), (b, fb)) = (at(0), at(1), at(2));
                let index = |dr: usize, dg: usize, db: usize| ((b + db) * n + g + dg) * n + r + dr;
                // The tetrahedron holding the color: from the cell's first corner, one channel
                // at a time in decreasing order of their fractions, to the opposite corner.
                let (first, second, third, steps) = tetrahedron([fr, fg, fb]);
                let corners = [
                    index(0, 0, 0),
                    index(steps[0][0], steps[0][1], steps[0][2]),
                    index(steps[1][0], steps[1][1], steps[1][2]),
                    index(1, 1, 1),
                ];
                std::array::from_fn(|i| {
                    let v = corners.map(|e| value(e, i));
                    v[0] + first * (v[1] - v[0]) + second * (v[2] - v[1]) + third * (v[3] - v[2])
                })
            }
        }
    }

    /// The format of [`Self::image`]: gray `f32` samples, never converted (only stored).
    pub fn image_format() -> PixelFormat {
        PixelFormat {
            layout: ChannelLayout::Gray,
            sample: SampleType::F32,
            color_space: ColorSpace::LINEAR_SRGB,
            alpha: AlphaMode::Straight,
        }
    }

    /// The values as an image (what files store): [`Self::image_format`], 256 pixels wide, one
    /// value a pixel in [`ColorLut`]'s order, row by row, the last row padded with zeros. Made
    /// once.
    pub fn image(&self) -> Result<Arc<RasterImage>, RasterError> {
        if let Some(image) = self.image.get() {
            return Ok(Arc::clone(image));
        }
        let size = image_size(self.values.len());
        let mut bytes = Vec::with_capacity(size.pixel_count() as usize * 4);
        bytes.extend(self.values.iter().flat_map(|v| v.to_ne_bytes()));
        bytes.resize(size.pixel_count() as usize * 4, 0);
        let image = Arc::new(RasterImage::from_pixels(
            size,
            Self::image_format(),
            &bytes,
        )?);
        Ok(Arc::clone(self.image.get_or_init(|| image)))
    }

    /// A table read back from its image ([`Self::image`]).
    pub fn from_image(
        name: impl Into<String>,
        dimension: LutDimension,
        size: u32,
        domain: [[f32; 3]; 2],
        image: &Arc<RasterImage>,
    ) -> Result<Arc<Self>, LutError> {
        let max = match dimension {
            LutDimension::One => MAX_1D_SIZE,
            LutDimension::Three => MAX_3D_SIZE,
        };
        if !(2..=max).contains(&size) {
            return Err(LutError::Size(size));
        }
        let count = dimension.entries(size) * 3;
        if image.size() != image_size(count) || image.format() != Self::image_format() {
            return Err(LutError::Image);
        }
        let level = &image.levels()[0];
        let per_tile = (TILE_SIZE * TILE_SIZE) as usize;
        let mut values = Vec::with_capacity(count);
        for row in 0..image.size().height.div_ceil(TILE_SIZE) {
            let tile = level
                .tile(TileCoord { col: 0, row })
                .ok_or(LutError::Image)?;
            let wanted = (count - values.len()).min(per_tile);
            let samples = tile.get(..wanted * 4).ok_or(LutError::Image)?;
            values.extend(
                samples
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|b| f32::from_ne_bytes(*b)),
            );
        }
        let lut = Self::new(name, dimension, size, domain, values)?;
        // Saved again, the same image: its tiles are known.
        let _ = lut.image.set(Arc::clone(image));
        Ok(lut)
    }
}

/// The size of the image of `count` values.
fn image_size(count: usize) -> Size {
    Size::new(IMAGE_WIDTH, (count as u32).div_ceil(IMAGE_WIDTH).max(1))
}

/// The tetrahedron of a cell holding the point at fractions `f` (red, green, blue): the three
/// fractions in decreasing order and the corners after the first and the second step (offsets
/// in red, green, blue). On ties either order gives the same color (the tetrahedra share that
/// face).
/// `color_lookup` in composite.wgsl makes the same choices.
fn tetrahedron(f: [f64; 3]) -> (f64, f64, f64, [[usize; 3]; 2]) {
    let [fr, fg, fb] = f;
    if fr > fg {
        if fg > fb {
            (fr, fg, fb, [[1, 0, 0], [1, 1, 0]])
        } else if fr > fb {
            (fr, fb, fg, [[1, 0, 0], [1, 0, 1]])
        } else {
            (fb, fr, fg, [[0, 0, 1], [1, 0, 1]])
        }
    } else if fb > fg {
        (fb, fg, fr, [[0, 0, 1], [0, 1, 1]])
    } else if fb > fr {
        (fg, fb, fr, [[0, 1, 0], [0, 1, 1]])
    } else {
        (fg, fr, fb, [[0, 1, 0], [1, 1, 0]])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 3D table of `f` at `size` points a side.
    fn table(size: u32, f: impl Fn([f32; 3]) -> [f32; 3]) -> Arc<ColorLut> {
        let last = (size - 1) as f32;
        let mut values = Vec::new();
        for b in 0..size {
            for g in 0..size {
                for r in 0..size {
                    values.extend(f([r as f32 / last, g as f32 / last, b as f32 / last]));
                }
            }
        }
        ColorLut::new(
            "test.cube",
            LutDimension::Three,
            size,
            ColorLut::UNIT_DOMAIN,
            values,
        )
        .unwrap()
    }

    fn close(a: [f64; 3], b: [f64; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6)
    }

    #[test]
    fn an_identity_table_changes_nothing_within_its_domain() {
        let identity = table(17, |c| c);
        for c in [
            [0.0, 0.0, 0.0],
            [1.0, 1.0, 1.0],
            [0.3, 0.7, 0.1],
            [0.91, 0.05, 0.5],
            [0.5, 0.5, 0.5],
            [0.2, 0.2, 0.8],
        ] {
            assert!(close(identity.apply(c), c), "{c:?}");
        }
        // Outside, clamped to it.
        assert!(close(identity.apply([-0.5, 1.5, 0.5]), [0.0, 1.0, 0.5]));
        assert!(close(identity.apply([f64::NAN, 0.5, 0.5]), [0.0, 0.5, 0.5]));
    }

    #[test]
    fn tetrahedral_interpolation_is_exact_for_linear_maps_and_hits_the_points() {
        // Any affine map is reproduced exactly between the points (each tetrahedron is
        // linear), whatever the order of the fractions.
        let affine = |c: [f32; 3]| {
            [
                0.2 + 0.5 * c[0] - 0.1 * c[1] + 0.3 * c[2],
                0.9 * c[1] + 0.05,
                1.0 - c[2] + 0.2 * c[0],
            ]
        };
        let lut = table(5, affine);
        for c in [
            [0.1, 0.2, 0.3],
            [0.3, 0.2, 0.1],
            [0.2, 0.3, 0.1],
            [0.1, 0.3, 0.2],
            [0.3, 0.1, 0.2],
            [0.2, 0.1, 0.3],
            [0.6, 0.6, 0.6],
            [0.99, 0.01, 0.5],
        ] {
            let expected = affine(c.map(|v| v as f32)).map(f64::from);
            assert!(close(lut.apply(c), expected), "{c:?}: {:?}", lut.apply(c));
        }
        // A nonlinear table gives its own values at its points.
        let squares = table(3, |c| c.map(|v| v * v));
        assert!(close(squares.apply([0.5, 1.0, 0.0]), [0.25, 1.0, 0.0]));
        // A neutral color stays neutral on a table neutral on its diagonal (tetrahedral
        // interpolation only reads the diagonal's points there).
        let c = squares.apply([0.3, 0.3, 0.3]);
        assert!((c[0] - c[1]).abs() < 1e-12 && (c[1] - c[2]).abs() < 1e-12);
    }

    #[test]
    fn domains_and_1d_tables_map_each_channel() {
        // Red doubled, green inverted, blue squared on 3 points, over [0, 2] in red.
        let lut = ColorLut::new(
            "curves.cube",
            LutDimension::One,
            3,
            [[0.0; 3], [2.0, 1.0, 1.0]],
            vec![0.0, 1.0, 0.0, 1.0, 0.5, 0.25, 2.0, 0.0, 1.0],
        )
        .unwrap();
        // Red 1 is the middle point of [0, 2]: 1; green 0.25 → 0.75; blue 0.75 between 0.25
        // and 1.
        assert!(close(lut.apply([1.0, 0.25, 0.75]), [1.0, 0.75, 0.625]));
        assert!(close(lut.apply([4.0, -1.0, 2.0]), [2.0, 1.0, 1.0]));
    }

    #[test]
    fn tables_out_of_their_limits_are_refused() {
        let new = |dimension, size, domain, values| {
            ColorLut::new("", dimension, size, domain, values).map(|_| ())
        };
        let unit = ColorLut::UNIT_DOMAIN;
        assert_eq!(
            new(LutDimension::Three, 1, unit, vec![0.0; 3]),
            Err(LutError::Size(1))
        );
        assert_eq!(
            new(LutDimension::Three, MAX_3D_SIZE + 1, unit, vec![]),
            Err(LutError::Size(MAX_3D_SIZE + 1))
        );
        assert_eq!(
            new(LutDimension::One, 2, unit, vec![0.0; 5]),
            Err(LutError::Length {
                expected: 6,
                actual: 5
            })
        );
        assert_eq!(
            new(
                LutDimension::One,
                2,
                unit,
                vec![0.0, 0.0, f32::NAN, 0.0, 0.0, 0.0]
            ),
            Err(LutError::Value)
        );
        assert_eq!(
            new(
                LutDimension::One,
                2,
                [[0.0; 3], [1.0, 0.0, 1.0]],
                vec![0.0; 6]
            ),
            Err(LutError::Domain)
        );
        assert_eq!(
            new(
                LutDimension::One,
                2,
                [[0.0; 3], [1.0, f32::INFINITY, 1.0]],
                vec![0.0; 6]
            ),
            Err(LutError::Value)
        );
    }

    #[test]
    fn a_table_comes_back_from_its_image() {
        for lut in [
            table(33, |c| [c[1], c[2] * 0.5, 1.0 - c[0]]),
            table(2, |c| c),
            ColorLut::new(
                "1d.cube",
                LutDimension::One,
                1000,
                [[-0.1; 3], [1.2; 3]],
                (0..3000).map(|i| i as f32 / 3000.0).collect(),
            )
            .unwrap(),
        ] {
            let image = lut.image().unwrap();
            assert!(Arc::ptr_eq(&image, &lut.image().unwrap()), "made once");
            let back = ColorLut::from_image(
                lut.name(),
                lut.dimension(),
                lut.size(),
                lut.domain(),
                &image,
            )
            .unwrap();
            assert_eq!(*back, *lut);
            assert_ne!(back.id(), lut.id());
            assert!(Arc::ptr_eq(&back.image().unwrap(), &image));
        }
        // An image of another table's size is refused.
        let image = table(3, |c| c).image().unwrap();
        assert_eq!(
            ColorLut::from_image("", LutDimension::Three, 40, ColorLut::UNIT_DOMAIN, &image)
                .map(|_| ()),
            Err(LutError::Image)
        );
    }
}
