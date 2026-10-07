//! The coverage of a placed outline (ADR 0041): how much of each pixel it covers, anti-aliased
//! analytically by `vello_cpu`, tile by tile. Tiles no edge crosses are one shared tile, full or
//! empty (by the winding number at their center): what a shape costs grows with its edges, not
//! its area. The compositor and the GPU both read these tiles, so they agree exactly.

use std::sync::{Arc, OnceLock};

use kurbo::{BezPath, Point, Rect as KurboRect, Shape as _};
use vello_cpu::color::palette::css::WHITE;
use vello_cpu::peniko::Fill;
use vello_cpu::{Pixmap, RasterizerSettings, RenderContext, Resources};

use super::model::FillRule;
use crate::color::{AlphaMode, ChannelLayout, ColorSpace, PixelFormat, SampleType};
use crate::geom::{Rect, Size};
use crate::raster::{RasterError, RasterImage, TILE_SIZE, parallel_for_each};

/// The coverage's pixels: 8-bit gray, linear (ADR 0041: 8 bits).
pub const COVERAGE_FORMAT: PixelFormat = PixelFormat {
    layout: ChannelLayout::Gray,
    sample: SampleType::U8,
    color_space: ColorSpace::LINEAR_SRGB,
    alpha: AlphaMode::Straight,
};

const T: usize = TILE_SIZE as usize;

/// One tile all covered, one not at all: shared by every coverage.
fn uniform(full: bool) -> Arc<[u8]> {
    static FULL: OnceLock<Arc<[u8]>> = OnceLock::new();
    static EMPTY: OnceLock<Arc<[u8]>> = OnceLock::new();
    let cell = if full { &FULL } else { &EMPTY };
    Arc::clone(cell.get_or_init(|| Arc::from(vec![if full { 255 } else { 0 }; T * T])))
}

/// The coverage of `path` (document pixels) filled by `rule` over `area`: a gray image of the
/// area's size, its pixel (0, 0) the area's top left. Tile by tile on every core.
pub fn coverage(path: &BezPath, rule: FillRule, area: Rect) -> Result<RasterImage, RasterError> {
    // Curves flattened finely here: the rasterizer's own flattening is coarse enough to lose a
    // tenth of a pixel of coverage along a curved edge (polygons inscribed in the curves).
    let path = &flattened(path, FLATTENING);
    let size = Size::new(area.width, area.height);
    let (columns, rows) = (
        size.width.div_ceil(TILE_SIZE) as usize,
        size.height.div_ceil(TILE_SIZE) as usize,
    );
    let origin = (f64::from(area.x), f64::from(area.y));
    // Which tiles an edge crosses: each segment's box binned to the tiles it touches.
    let mut edges = vec![false; columns * rows];
    let t = f64::from(TILE_SIZE);
    for segment in path.segments() {
        let b = segment.bounding_box();
        let to_tile = |v: f64, o: f64, n: usize| ((v - o) / t).floor().clamp(-1.0, n as f64) as i64;
        let (c0, c1) = (
            to_tile(b.x0, origin.0, columns),
            to_tile(b.x1, origin.0, columns),
        );
        let (r0, r1) = (to_tile(b.y0, origin.1, rows), to_tile(b.y1, origin.1, rows));
        for r in r0.max(0)..=r1.min(rows as i64 - 1) {
            for c in c0.max(0)..=c1.min(columns as i64 - 1) {
                edges[r as usize * columns + c as usize] = true;
            }
        }
    }
    let inside = |p: Point| {
        let w = path.winding(p);
        match rule {
            FillRule::NonZero => w != 0,
            FillRule::EvenOdd => w % 2 != 0,
        }
    };
    let bounds = path.bounding_box();
    let mut tiles: Vec<(usize, Option<Arc<[u8]>>)> =
        (0..columns * rows).map(|i| (i, None)).collect();
    parallel_for_each(&mut tiles, |(i, out)| {
        let (col, row) = (*i % columns, *i / columns);
        let x0 = origin.0 + (col * T) as f64;
        let y0 = origin.1 + (row * T) as f64;
        let tile_box = KurboRect::new(x0, y0, x0 + t, y0 + t);
        if !edges[*i] {
            // No edge in it: all in or all out.
            let full = bounds.intersect(tile_box).area() > 0.0
                && inside(Point::new(x0 + t / 2.0, y0 + t / 2.0));
            *out = Some(uniform(full));
            return;
        }
        let (w, h) = (
            (size.width as usize - col * T).min(T),
            (size.height as usize - row * T).min(T),
        );
        *out = Some(edge_tile(path, rule, (x0, y0), (w, h)));
    });
    let tiles = tiles
        .into_iter()
        .map(|(_, tile)| tile.unwrap_or_else(|| uniform(false)))
        .collect();
    RasterImage::from_level0_tiles(size, COVERAGE_FORMAT, tiles)
}

/// How far a flattened curve strays from the true one, in pixels: a few hundredths, so that the
/// coverage lost along a curved edge stays below a rounding of 8 bits.
const FLATTENING: f64 = 0.02;

/// `path` with its curves as lines within `tolerance`.
fn flattened(path: &BezPath, tolerance: f64) -> BezPath {
    let mut out = BezPath::new();
    kurbo::flatten(path.iter(), tolerance, |el| out.push(el));
    out
}

/// A tile an edge crosses, its top left at `at` (document pixels), `valid` pixels of it in the
/// image (the rest padded by repeating the last row and column, as tiles are).
fn edge_tile(path: &BezPath, rule: FillRule, at: (f64, f64), valid: (usize, usize)) -> Arc<[u8]> {
    let side = TILE_SIZE as u16;
    let mut ctx = RenderContext::new(side, side);
    ctx.set_fill_rule(match rule {
        FillRule::NonZero => Fill::NonZero,
        FillRule::EvenOdd => Fill::EvenOdd,
    });
    ctx.set_transform(kurbo::Affine::translate((-at.0, -at.1)));
    ctx.set_paint(WHITE);
    ctx.fill_path(path);
    ctx.flush();
    let mut pixmap = Pixmap::new(side, side);
    ctx.render_with(
        &mut pixmap,
        &mut Resources::default(),
        RasterizerSettings::default(),
    );
    let rgba = pixmap.data_as_u8_slice();
    let mut tile = vec![0u8; T * T];
    let (w, h) = valid;
    for y in 0..T {
        let sy = y.min(h - 1);
        for x in 0..T {
            let sx = x.min(w - 1);
            // White, premultiplied: the coverage is the alpha.
            tile[y * T + x] = rgba[(sy * T + sx) * 4 + 3];
        }
    }
    Arc::from(tile)
}

/// `stroke`'s coverage kept where `shape` covers (`inside`) or where it does not: an inside or
/// outside stroke from one twice as wide (ADR 0041). Both cover the same area.
pub fn cut(
    stroke: &RasterImage,
    shape: &RasterImage,
    inside: bool,
) -> Result<RasterImage, RasterError> {
    let size = stroke.size();
    let (s, f) = (&stroke.levels()[0], &shape.levels()[0]);
    let tiles = s
        .tiles()
        .iter()
        .zip(f.tiles())
        .map(|(a, b)| {
            // Nothing of the stroke, or the shape all in or all out: the answer is known.
            if Arc::ptr_eq(a, &uniform(false)) {
                return Arc::clone(a);
            }
            if Arc::ptr_eq(b, &uniform(inside)) {
                return Arc::clone(a);
            }
            if Arc::ptr_eq(b, &uniform(!inside)) {
                return uniform(false);
            }
            let tile: Vec<u8> = a
                .iter()
                .zip(b.iter())
                .map(|(&s, &f)| {
                    let f = if inside {
                        u32::from(f)
                    } else {
                        255 - u32::from(f)
                    };
                    ((u32::from(s) * f + 127) / 255) as u8
                })
                .collect();
            Arc::from(tile)
        })
        .collect();
    RasterImage::from_level0_tiles(size, COVERAGE_FORMAT, tiles)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shape::model::Geometry;
    use crate::shape::path::{TOLERANCE, outline};

    /// The sum of an image's coverage, in pixels.
    fn covered(image: &RasterImage) -> f64 {
        let size = image.size();
        let level = &image.levels()[0];
        let mut sum = 0u64;
        for y in 0..size.height {
            for x in 0..size.width {
                let tile = level
                    .tile(crate::tile::TileCoord {
                        col: x / TILE_SIZE,
                        row: y / TILE_SIZE,
                    })
                    .unwrap();
                sum += u64::from(tile[((y % TILE_SIZE) * TILE_SIZE + x % TILE_SIZE) as usize]);
            }
        }
        sum as f64 / 255.0
    }

    fn at(image: &RasterImage, x: u32, y: u32) -> u8 {
        let tile = image.levels()[0]
            .tile(crate::tile::TileCoord {
                col: x / TILE_SIZE,
                row: y / TILE_SIZE,
            })
            .unwrap();
        tile[((y % TILE_SIZE) * TILE_SIZE + x % TILE_SIZE) as usize]
    }

    #[test]
    fn a_rectangle_covers_its_area_and_shares_its_inside_and_outside() {
        // 600 × 500, over several tiles; a fractional edge on the left.
        let rect = outline(
            &Geometry::Rectangle {
                rect: [100.5, 40.0, 700.5, 540.0],
                radii: [0.0; 4],
            },
            TOLERANCE,
        );
        let image = coverage(&rect, FillRule::NonZero, Rect::new(0, 0, 1024, 768)).unwrap();
        assert!(
            (covered(&image) - 600.0 * 500.0).abs() < 2.0,
            "{}",
            covered(&image)
        );
        assert_eq!(at(&image, 100, 100), 128, "half a pixel");
        assert_eq!(at(&image, 101, 100), 255);
        assert_eq!(at(&image, 99, 100), 0);
        // The tile (1, 1), from 256 to 512 both ways, is all inside: the shared full tile.
        let level = &image.levels()[0];
        let tile = |col, row| level.tile(crate::tile::TileCoord { col, row }).unwrap();
        assert!(Arc::ptr_eq(tile(1, 1), &uniform(true)));
        assert!(Arc::ptr_eq(tile(3, 2), &uniform(false)));
    }

    #[test]
    fn an_ellipse_covers_its_area_by_either_rule() {
        let ellipse = outline(
            &Geometry::Ellipse {
                center: [300.0, 200.0],
                radii: [150.0, 90.0],
            },
            0.01,
        );
        let area = std::f64::consts::PI * 150.0 * 90.0;
        for rule in [FillRule::NonZero, FillRule::EvenOdd] {
            let image = coverage(&ellipse, rule, Rect::new(0, 0, 600, 400)).unwrap();
            assert!(
                (covered(&image) - area).abs() / area < 1e-3,
                "{rule:?} {} {area}",
                covered(&image)
            );
        }
    }

    #[test]
    fn crossings_cover_by_their_rule() {
        // Two squares, the second inside the first and drawn the same way round: nonzero fills
        // both, even-odd leaves a hole.
        let mut path = BezPath::new();
        for (l, t, r, b) in [(10.0, 10.0, 110.0, 110.0), (40.0, 40.0, 80.0, 80.0)] {
            path.move_to((l, t));
            path.line_to((r, t));
            path.line_to((r, b));
            path.line_to((l, b));
            path.close_path();
        }
        let area = Rect::new(0, 0, 128, 128);
        let nonzero = coverage(&path, FillRule::NonZero, area).unwrap();
        let evenodd = coverage(&path, FillRule::EvenOdd, area).unwrap();
        assert_eq!((at(&nonzero, 60, 60), at(&evenodd, 60, 60)), (255, 0));
        assert!((covered(&evenodd) - (10000.0 - 1600.0)).abs() < 1.0);
    }

    #[test]
    fn an_area_away_from_the_origin_and_an_inside_stroke() {
        let square = outline(
            &Geometry::Rectangle {
                rect: [1000.0, 1000.0, 1100.0, 1100.0],
                radii: [0.0; 4],
            },
            TOLERANCE,
        );
        let area = Rect::new(768, 768, 512, 512);
        let fill = coverage(&square, FillRule::NonZero, area).unwrap();
        assert_eq!(at(&fill, 1050 - 768, 1050 - 768), 255);
        assert_eq!(at(&fill, 990 - 768, 1050 - 768), 0);
        // A stroke 10 wide drawn 20 wide, kept inside: a band of 10 along the inner side.
        let stroke = crate::shape::model::ShapeStroke {
            paint: crate::shape::model::Paint::Solid(crate::color::LinearRgba::new(
                0.0, 0.0, 0.0, 1.0,
            )),
            width: 10.0,
            align: crate::shape::model::StrokeAlign::Inside,
            cap: crate::shape::model::StrokeCap::Butt,
            join: crate::shape::model::StrokeJoin::Miter,
            miter_limit: 4.0,
            dashes: Vec::new(),
            dash_offset: 0.0,
        };
        let wide = crate::shape::path::stroke_outline(&square, &stroke, 2.0, TOLERANCE);
        let band = coverage(&wide, FillRule::NonZero, area).unwrap();
        let inside = cut(&band, &fill, true).unwrap();
        let outside = cut(&band, &fill, false).unwrap();
        assert!((covered(&inside) - (100.0 * 100.0 - 80.0 * 80.0)).abs() < 2.0);
        assert!((covered(&outside) - (120.0 * 120.0 - 100.0 * 100.0)).abs() < 2.0);
        assert_eq!(at(&inside, 1003 - 768, 1050 - 768), 255);
        assert_eq!(at(&inside, 997 - 768, 1050 - 768), 0);
        assert_eq!(at(&outside, 997 - 768, 1050 - 768), 255);
    }

    #[test]
    fn nothing_covers_nothing() {
        let image = coverage(
            &BezPath::new(),
            FillRule::NonZero,
            Rect::new(0, 0, 300, 300),
        )
        .unwrap();
        assert_eq!(covered(&image), 0.0);
    }
}
