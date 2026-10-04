//! Selections (ADR 0024): what the next operation applies to, as a coverage mask.
//!
//! A selection is a gray, 16-bit, linear [`RasterImage`] the size of the canvas, at the
//! document origin: 0 is not selected, 65535 selected, values between are soft edges
//! (anti-aliasing, feather). Nothing outside it is selected. It is immutable and shared like any
//! raster. Tiles wholly selected or wholly unselected are one shared allocation each (and so are
//! their pyramid tiles), so a selection costs memory in proportion to its outline, not to the
//! canvas.
//!
//! This module builds selections: shapes (rectangles, ellipses, polygons) rasterized with exact
//! area coverage, feathered, combined with the current selection (replace, add, subtract,
//! intersect), inverted; and reads them: their bounds and their outline (the marching ants).

use std::collections::HashMap;
use std::sync::Arc;

use crate::color::{AlphaMode, ChannelLayout, ColorSpace, PixelFormat, SampleType};
use crate::geom::{Rect, Size};
use crate::raster::{RasterImage, TILE_SIZE};
use crate::tile::TileCoord;
use crate::transform::Affine;

/// The format of every selection made here.
pub const SELECTION_FORMAT: PixelFormat = PixelFormat {
    layout: ChannelLayout::Gray,
    sample: SampleType::U16,
    color_space: ColorSpace::LINEAR_SRGB,
    alpha: AlphaMode::Straight,
};

/// The largest feather radius, in pixels. Feathering reads a neighborhood about three radii
/// wide around each edge tile; this bounds its memory (about 12 MB per worker).
pub const MAX_FEATHER: f64 = 250.0;

const T: usize = TILE_SIZE as usize;
const FULL: u16 = u16::MAX;
/// Coverage at or above this is inside the outline (half covered).
const HALF: u16 = 32768;

/// A shape to select, in document pixels (fractions allowed: pixel `(x, y)` is the square from
/// `(x, y)` to `(x + 1, y + 1)`).
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    /// The rectangle from (`left`, `top`) to (`right`, `bottom`).
    Rectangle {
        left: f64,
        top: f64,
        right: f64,
        bottom: f64,
    },
    /// The ellipse inscribed in that rectangle.
    Ellipse {
        left: f64,
        top: f64,
        right: f64,
        bottom: f64,
    },
    /// A closed polygon (the lasso): the last point joins the first. Self-intersections are
    /// filled with the nonzero rule, so every loop of a figure eight is selected.
    Polygon { points: Vec<[f64; 2]> },
}

/// How a new shape combines with the current selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Combine {
    /// The shape alone.
    #[default]
    Replace,
    /// The union (the larger coverage).
    Add,
    /// The current selection minus the shape.
    Subtract,
    /// Where both are (the smaller coverage).
    Intersect,
}

/// How a shape's edges are drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EdgeOptions {
    /// Pixels on the edge get their exact coverage; otherwise they are in or out (more than
    /// half covered: in).
    pub anti_alias: bool,
    /// Gaussian softening of the edge (its standard deviation in pixels, as Photoshop's feather
    /// radius); 0 for none. At most [`MAX_FEATHER`].
    pub feather: f64,
}

impl Default for EdgeOptions {
    fn default() -> Self {
        Self {
            anti_alias: true,
            feather: 0.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SelectionError {
    /// A coordinate or the feather radius is not finite, or the feather is out of range.
    InvalidShape,
    EmptyCanvas,
}

impl std::fmt::Display for SelectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SelectionError::InvalidShape => write!(f, "invalid selection shape"),
            SelectionError::EmptyCanvas => write!(f, "a canvas must have pixels"),
        }
    }
}

impl std::error::Error for SelectionError {}

/// A document's selection: a gray coverage mask at the origin, immutable and shared, compared
/// by identity like layer images.
#[derive(Debug, Clone)]
pub struct Selection(Arc<RasterImage>);

impl Selection {
    /// `None` unless `image` is gray (without alpha).
    pub fn new(image: Arc<RasterImage>) -> Option<Self> {
        (image.format().layout == ChannelLayout::Gray).then_some(Self(image))
    }

    pub fn image(&self) -> &Arc<RasterImage> {
        &self.0
    }
}

impl PartialEq for Selection {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// Everything on a `canvas`.
pub fn select_all(canvas: Size) -> Result<RasterImage, SelectionError> {
    let mut mask = Mask::new(canvas)?;
    mask.tiles.fill(Tile::Const(FULL));
    // Full tiles on a canvas with pixels: never empty.
    mask.into_image().ok_or(SelectionError::EmptyCanvas)
}

/// `shape` combined with `current` (the document's selection, if any) on a `canvas`. `None`:
/// nothing is selected (e.g. subtracting everything), which is no selection.
pub fn select_shape(
    canvas: Size,
    current: Option<&RasterImage>,
    shape: &Shape,
    edges: EdgeOptions,
    combine: Combine,
) -> Result<Option<RasterImage>, SelectionError> {
    if !edges.feather.is_finite() || !(0.0..=MAX_FEATHER).contains(&edges.feather) {
        return Err(SelectionError::InvalidShape);
    }
    let contour = shape.contour()?;
    let mut new = rasterize(canvas, &contour, edges.anti_alias)?;
    if edges.feather > 0.0 {
        new = feather(&new, edges.feather);
    }
    finish(canvas, current, new, combine)
}

/// A new selection `new` combined with `current` (the document's selection, if any).
fn finish(
    canvas: Size,
    current: Option<&RasterImage>,
    new: Mask,
    combine: Combine,
) -> Result<Option<RasterImage>, SelectionError> {
    let result = match (combine, current) {
        (Combine::Replace, _) | (Combine::Add, None) => new,
        // Nothing selected: nothing to subtract from or intersect with.
        (Combine::Subtract | Combine::Intersect, None) => return Ok(None),
        (_, Some(current)) => combine_masks(&Mask::from_image(canvas, current)?, &new, combine),
    };
    Ok(result.into_image())
}

/// `other` (a selection of the canvas, a saved one) combined with `current` by `how`: what
/// loading a saved selection into the current one does (replace, add, subtract, intersect).
/// `None` when nothing is left selected.
pub fn combined(
    canvas: Size,
    current: Option<&RasterImage>,
    other: &RasterImage,
    how: Combine,
) -> Result<Option<RasterImage>, SelectionError> {
    finish(canvas, current, Mask::from_image(canvas, other)?, how)
}

/// Everything `current` leaves out (Select > Inverse); `None` selects everything.
pub fn invert(
    canvas: Size,
    current: Option<&RasterImage>,
) -> Result<Option<RasterImage>, SelectionError> {
    let Some(current) = current else {
        return select_all(canvas).map(Some);
    };
    let mut mask = Mask::from_image(canvas, current)?;
    for tile in &mut mask.tiles {
        let inverted = match &*tile {
            Tile::Const(v) => Tile::Const(FULL - v),
            other => Tile::Data(other.values().iter().map(|v| FULL - v).collect()),
        };
        *tile = inverted;
    }
    Ok(mask.into_image())
}

/// `current` moved by (`dx`, `dy`) whole pixels onto a canvas of `canvas` pixels (its own size,
/// or the new one after Crop or Canvas Size), what leaves the canvas dropped (the Move tool
/// moving selected pixels): values are copied exactly, and a tile that only reads uniform tiles
/// of one value stays uniform (shared). `None` when nothing stays selected.
pub fn translated(
    canvas: Size,
    current: &RasterImage,
    dx: i64,
    dy: i64,
) -> Result<Option<RasterImage>, SelectionError> {
    // Read at its own size: the canvas may have changed since (Crop, Canvas Size).
    let source = Mask::from_image(current.size(), current)?;
    let from = source.size;
    let mut moved = Mask::new(canvas)?;
    let t = T as i64;
    let mut work: Vec<(usize, Tile)> = (0..moved.tiles.len())
        .map(|index| (index, Tile::Const(0)))
        .collect();
    crate::raster::parallel_for_each(&mut work, |(index, tile)| {
        let (col, row) = (*index % moved.columns, *index / moved.columns);
        let (w, h) = moved.valid(col, row);
        // The source pixels this tile reads.
        let (x0, y0) = ((col * T) as i64 - dx, (row * T) as i64 - dy);
        let (x1, y1) = (x0 + w as i64, y0 + h as i64);
        let inside =
            x0 >= 0 && y0 >= 0 && x1 <= i64::from(from.width) && y1 <= i64::from(from.height);
        let outside =
            x1 <= 0 || y1 <= 0 || x0 >= i64::from(from.width) || y0 >= i64::from(from.height);
        if outside {
            return;
        }
        // Uniform when every source tile it reads holds one constant (0 outside the canvas).
        let mut constant = None;
        let mut uniform = true;
        'reads: for r in y0.max(0) / t..=(y1 - 1).min(i64::from(from.height) - 1) / t {
            for c in x0.max(0) / t..=(x1 - 1).min(i64::from(from.width) - 1) / t {
                let value = source.tiles[r as usize * source.columns + c as usize].constant();
                uniform = match (value, constant) {
                    (None, _) => false,
                    (Some(v), None) => {
                        constant = Some(v);
                        true
                    }
                    (Some(v), Some(previous)) => v == previous,
                };
                if !uniform {
                    break 'reads;
                }
            }
        }
        if uniform && (inside || constant == Some(0)) {
            *tile = Tile::Const(constant.unwrap_or(0));
            return;
        }
        let mut values = vec![0u16; T * T];
        for y in 0..h {
            for x in 0..w {
                values[y * T + x] = source.get(x0 + x as i64, y0 + y as i64);
            }
        }
        pad(&mut values, w, h);
        *tile = Tile::Data(values);
    });
    moved.tiles = work.into_iter().map(|(_, tile)| tile).collect();
    Ok(moved.into_image())
}

/// Select > Transform Selection: `current` mapped by `transform` (document pixels to document
/// pixels) on a canvas of `canvas` pixels, resampled as a transformed layer is (ADR 0018: from
/// the pyramid level matching the scale, exact for whole-pixel moves), so that soft edges stay
/// soft; what leaves the canvas is dropped. A tile whose samples only read uniform tiles of one
/// value stays uniform (shared). `None` when nothing stays selected; an error for a transform
/// that cannot be inverted.
pub fn transformed(
    canvas: Size,
    current: &RasterImage,
    transform: Affine,
) -> Result<Option<RasterImage>, SelectionError> {
    if let Some((dx, dy)) = transform.integer_translation() {
        return translated(canvas, current, dx, dy);
    }
    // Selections made here are in that format already; anything else is read into it first.
    let normalized;
    let current = if current.stored_format() == SELECTION_FORMAT {
        current
    } else {
        match Mask::from_image(current.size(), current)?.into_image() {
            Some(image) => {
                normalized = image;
                &normalized
            }
            None => return Ok(None),
        }
    };
    let Some(selected) = bounds(current) else {
        return Ok(None);
    };
    let resampling = crate::resample::Resampling::new(transform, 1.0, current.levels().len())
        .ok_or(SelectionError::InvalidShape)?;
    let level = current
        .levels()
        .get(resampling.level)
        .ok_or(SelectionError::InvalidShape)?;
    let texels = level.size();
    let table = crate::resample::weight_table();
    let factor = f64::from(1u32 << resampling.level.min(31));
    // Where the selection lands, with a pixel of margin for the filter.
    let [lx0, ly0, lx1, ly1] = transform.map_rect([
        f64::from(selected.x),
        f64::from(selected.y),
        f64::from(selected.x) + f64::from(selected.width),
        f64::from(selected.y) + f64::from(selected.height),
    ]);
    let landing = [lx0 - 2.0, ly0 - 2.0, lx1 + 2.0, ly1 + 2.0];
    let texel = |i: i64, j: i64| -> Option<f64> {
        if i < 0 || j < 0 || i >= i64::from(texels.width) || j >= i64::from(texels.height) {
            return None;
        }
        let (x, y) = (i as u32, j as u32);
        let coord = TileCoord {
            col: x / TILE_SIZE,
            row: y / TILE_SIZE,
        };
        let at = ((y % TILE_SIZE) * TILE_SIZE + x % TILE_SIZE) as usize * 2;
        let value = level
            .tile(coord)
            .and_then(|tile| tile.get(at..at + 2))
            .map_or(0, |b| u16::from_ne_bytes([b[0], b[1]]));
        Some(f64::from(value) / f64::from(FULL))
    };
    let mut out = Mask::new(canvas)?;
    let mut work: Vec<(usize, Tile)> = (0..out.tiles.len())
        .map(|index| (index, Tile::Const(0)))
        .collect();
    let shape = &out;
    crate::raster::parallel_for_each(&mut work, |(index, tile)| {
        let (col, row) = (*index % shape.columns, *index / shape.columns);
        let (w, h) = shape.valid(col, row);
        let area = [
            (col * T) as f64,
            (row * T) as f64,
            (col * T + w) as f64,
            (row * T + h) as f64,
        ];
        let misses = area[2] <= landing[0]
            || area[3] <= landing[1]
            || area[0] >= landing[2]
            || area[1] >= landing[3];
        if misses {
            return;
        }
        // Uniform when every texel its samples read is in uniform tiles of one value.
        let [sx0, sy0, sx1, sy1] = resampling.source_area(area).map(|v| v / factor);
        let inside = sx0 >= 0.0
            && sy0 >= 0.0
            && sx1 <= f64::from(texels.width)
            && sy1 <= f64::from(texels.height);
        if inside {
            let tiles_of = |a: f64, b: f64| {
                (a as u32 / TILE_SIZE)..=((b as u32).saturating_sub(1) / TILE_SIZE)
            };
            let mut constant = None;
            let mut uniform = true;
            'reads: for r in tiles_of(sy0, sy1) {
                for c in tiles_of(sx0, sx1) {
                    let value = level
                        .tile(TileCoord { col: c, row: r })
                        .map_or(Some(0), |t| uniform_value(t));
                    uniform = match (value, constant) {
                        (None, _) => false,
                        (Some(v), None) => {
                            constant = Some(v);
                            true
                        }
                        (Some(v), Some(previous)) => v == previous,
                    };
                    if !uniform {
                        break 'reads;
                    }
                }
            }
            if uniform && let Some(v) = constant {
                *tile = Tile::Const(v);
                return;
            }
        }
        let mut values = vec![0u16; T * T];
        for y in 0..h {
            for x in 0..w {
                let point = ((col * T + x) as f64 + 0.5, (row * T + y) as f64 + 0.5);
                let (sample, _) =
                    resampling.sample(table, point, |i, j| texel(i, j).map(|v| [v, 0.0, 0.0, 0.0]));
                values[y * T + x] = (sample[0].clamp(0.0, 1.0) * f64::from(FULL)).round() as u16;
            }
        }
        pad(&mut values, w, h);
        *tile = computed_tile(values);
    });
    out.tiles = work.into_iter().map(|(_, tile)| tile).collect();
    Ok(out.into_image())
}

/// The coverage (0–1) of `selection` at the centers of a `width × height` grid stretched over
/// `area` of the canvas: the nearest pixel of the finest pyramid level whose pixels are no
/// larger than the grid's (a level's pixel averages the ones it covers). Reads the tiles
/// directly: a grid of a few megapixels takes milliseconds whatever the canvas.
pub fn sample_grid(selection: &RasterImage, area: Rect, width: usize, height: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; width * height];
    if width == 0 || height == 0 || area.is_empty() {
        return out;
    }
    let levels = selection.levels();
    let scale = (f64::from(area.width) / width as f64).max(f64::from(area.height) / height as f64);
    let level = (scale.max(1.0).log2().floor() as usize).min(levels.len() - 1);
    let raster = &levels[level];
    let size = raster.size();
    let factor = f64::from(1u32 << level);
    let (sx, sy) = (
        f64::from(area.width) / width as f64,
        f64::from(area.height) / height as f64,
    );
    for (row, line) in out.chunks_mut(width).enumerate() {
        let y = (f64::from(area.y) + (row as f64 + 0.5) * sy) / factor;
        let y = (y as u32).min(size.height - 1);
        for (col, value) in line.iter_mut().enumerate() {
            let x = (f64::from(area.x) + (col as f64 + 0.5) * sx) / factor;
            let x = (x as u32).min(size.width - 1);
            let coord = TileCoord {
                col: x / TILE_SIZE,
                row: y / TILE_SIZE,
            };
            let Some(tile) = raster.tile(coord) else {
                continue;
            };
            let at = (((y % TILE_SIZE) * TILE_SIZE + x % TILE_SIZE) * 2) as usize;
            if let Some(bytes) = tile.get(at..at + 2) {
                *value = f32::from(u16::from_ne_bytes([bytes[0], bytes[1]])) / f32::from(FULL);
            }
        }
    }
    out
}

/// The smallest rectangle holding every selected pixel (coverage above 0), or `None`.
pub fn bounds(selection: &RasterImage) -> Option<Rect> {
    let size = selection.size();
    let level = &selection.levels()[0];
    let grid = level.grid();
    let (mut left, mut top, mut right, mut bottom) = (u32::MAX, u32::MAX, 0u32, 0u32);
    let mut uniform: HashMap<*const u8, Option<u16>> = HashMap::new();
    for row in 0..grid.rows() {
        for col in 0..grid.columns() {
            let Some(tile) = level.tile(TileCoord { col, row }) else {
                continue;
            };
            let constant = *uniform
                .entry(tile.as_ptr())
                .or_insert_with(|| uniform_value(tile));
            let (x0, y0) = (col * TILE_SIZE, row * TILE_SIZE);
            let w = (size.width - x0).min(TILE_SIZE);
            let h = (size.height - y0).min(TILE_SIZE);
            let (tl, tt, tr, tb) = match constant {
                Some(0) => continue,
                Some(_) => (0, 0, w, h),
                None => {
                    let values = decode(tile);
                    let (mut l, mut t, mut r, mut b) = (u32::MAX, u32::MAX, 0, 0);
                    for y in 0..h {
                        for x in 0..w {
                            if values[(y * TILE_SIZE + x) as usize] != 0 {
                                l = l.min(x);
                                t = t.min(y);
                                r = r.max(x + 1);
                                b = b.max(y + 1);
                            }
                        }
                    }
                    if r == 0 {
                        continue;
                    }
                    (l, t, r, b)
                }
            };
            left = left.min(x0 + tl);
            top = top.min(y0 + tt);
            right = right.max(x0 + tr);
            bottom = bottom.max(y0 + tb);
        }
    }
    (right > left).then(|| Rect::new(left, top, right - left, bottom - top))
}

/// A polyline in document pixels.
pub type Line = Vec<[u32; 2]>;

/// The outline of `selection` (where coverage crosses one half) along the pixel edges of its
/// pyramid level `level`, within `region` (document pixels), as polylines in document pixels:
/// closed loops repeat their first point, outlines cut by the region are open. `None` if it
/// would take more than `max_points` points: the caller tries a coarser level.
pub fn outline(
    selection: &RasterImage,
    level: usize,
    region: Rect,
    max_points: usize,
) -> Option<Vec<Line>> {
    match Area::read(selection, level, region) {
        Some(area) => area.trace(|v| v >= HALF, max_points),
        None => Some(Vec::new()),
    }
}

/// The values of a region of a pyramid level, one pixel wider on each side to see the edges on
/// its border.
struct Area {
    values: Vec<u16>,
    w: usize,
    h: usize,
    x0: u32,
    y0: u32,
    scale: u32,
    size: Size,
    /// The region stops short of the image on that side (left, top, right, bottom): nothing is
    /// known beyond it, so no edge is drawn there; otherwise the image edge closes the outline.
    cut: [bool; 4],
}

impl Area {
    fn read(selection: &RasterImage, level: usize, region: Rect) -> Option<Self> {
        let levels = selection.levels();
        let level = level.min(levels.len() - 1);
        let scale = 1u32 << level;
        let pixels = &levels[level];
        let size = pixels.size();
        let x0 = (region.x / scale).saturating_sub(1);
        let y0 = (region.y / scale).saturating_sub(1);
        let x1 = (u32::try_from(region.right().div_ceil(u64::from(scale))).unwrap_or(u32::MAX))
            .saturating_add(1)
            .min(size.width);
        let y1 = (u32::try_from(region.bottom().div_ceil(u64::from(scale))).unwrap_or(u32::MAX))
            .saturating_add(1)
            .min(size.height);
        if x0 >= x1 || y0 >= y1 {
            return None;
        }
        let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
        let mut values = vec![0u16; w * h];
        for row in y0 / TILE_SIZE..=(y1 - 1) / TILE_SIZE {
            for col in x0 / TILE_SIZE..=(x1 - 1) / TILE_SIZE {
                let Some(tile) = pixels.tile(TileCoord { col, row }) else {
                    continue;
                };
                let constant = uniform_value(tile);
                let decoded = if constant.is_none() {
                    decode(tile)
                } else {
                    Vec::new()
                };
                let (tx, ty) = (col * TILE_SIZE, row * TILE_SIZE);
                for y in ty.max(y0)..(ty + TILE_SIZE).min(y1) {
                    for x in tx.max(x0)..(tx + TILE_SIZE).min(x1) {
                        values[(y - y0) as usize * w + (x - x0) as usize] = constant
                            .unwrap_or_else(|| decoded[((y - ty) * TILE_SIZE + x - tx) as usize]);
                    }
                }
            }
        }
        Some(Self {
            values,
            w,
            h,
            x0,
            y0,
            scale,
            size: selection.size(),
            cut: [x0 > 0, y0 > 0, x1 < size.width, y1 < size.height],
        })
    }

    /// The edges between pixels `inside` and the others, chained into polylines.
    fn trace(&self, inside: impl Fn(u16) -> bool, max_points: usize) -> Option<Vec<Line>> {
        let (w, h) = (self.w, self.h);
        let [cut_left, cut_top, cut_right, cut_bottom] = self.cut;
        let at = |x: isize, y: isize| -> Option<bool> {
            if x < 0 {
                return (!cut_left).then_some(false);
            }
            if y < 0 {
                return (!cut_top).then_some(false);
            }
            if x >= w as isize {
                return (!cut_right).then_some(false);
            }
            if y >= h as isize {
                return (!cut_bottom).then_some(false);
            }
            Some(inside(self.values[y as usize * w + x as usize]))
        };

        // Directed edges with the inside on their right (y down), between vertices of the
        // pixel grid of the level, relative to (x0, y0).
        let mut edges: Vec<([u32; 2], [u32; 2])> = Vec::new();
        for y in 0..=h as isize {
            for x in 0..=w as isize {
                // Horizontal edge on line y, from x to x + 1: between (x, y - 1) and (x, y).
                if x < w as isize
                    && let (Some(above), Some(below)) = (at(x, y - 1), at(x, y))
                    && above != below
                {
                    let (a, b) = ([x as u32, y as u32], [x as u32 + 1, y as u32]);
                    edges.push(if below { (a, b) } else { (b, a) });
                }
                // Vertical edge on line x, from y to y + 1: between (x - 1, y) and (x, y).
                if y < h as isize
                    && let (Some(left), Some(right)) = (at(x - 1, y), at(x, y))
                    && left != right
                {
                    let (a, b) = ([x as u32, y as u32], [x as u32, y as u32 + 1]);
                    edges.push(if left { (a, b) } else { (b, a) });
                }
            }
        }

        let mut outgoing: HashMap<[u32; 2], Vec<usize>> = HashMap::with_capacity(edges.len());
        let mut incoming: HashMap<[u32; 2], usize> = HashMap::with_capacity(edges.len());
        for (i, (a, b)) in edges.iter().enumerate() {
            outgoing.entry(*a).or_default().push(i);
            *incoming.entry(*b).or_default() += 1;
        }
        let mut used = vec![false; edges.len()];
        let mut lines = Vec::new();
        let mut points = 0usize;
        let to_document = |p: [u32; 2]| {
            [
                ((p[0] + self.x0) * self.scale).min(self.size.width),
                ((p[1] + self.y0) * self.scale).min(self.size.height),
            ]
        };
        let trace = |start: usize, used: &mut [bool]| -> Line {
            let mut line = vec![edges[start].0];
            let mut current = start;
            loop {
                used[current] = true;
                let (_, end) = edges[current];
                line.push(end);
                let next = outgoing
                    .get(&end)
                    .and_then(|list| list.iter().copied().find(|&e| !used[e]));
                match next {
                    Some(e) => current = e,
                    None => break,
                }
            }
            simplify(&mut line);
            line.into_iter().map(to_document).collect()
        };
        // Open outlines first (cut by the region): they start where nothing comes in.
        for i in 0..edges.len() {
            if !used[i] && !incoming.contains_key(&edges[i].0) {
                let line = trace(i, &mut used);
                points += line.len();
                lines.push(line);
            }
        }
        for i in 0..edges.len() {
            if !used[i] {
                let line = trace(i, &mut used);
                points += line.len();
                lines.push(line);
            }
            if points > max_points {
                return None;
            }
        }
        (points <= max_points).then_some(lines)
    }
}

/// Drop the points in the middle of straight runs.
fn simplify(line: &mut Vec<[u32; 2]>) {
    if line.len() < 3 {
        return;
    }
    let mut out: Vec<[u32; 2]> = Vec::with_capacity(line.len());
    for &p in line.iter() {
        if out.len() >= 2 {
            let a = out[out.len() - 2];
            let b = out[out.len() - 1];
            let straight = (a[0] == b[0] && b[0] == p[0]) || (a[1] == b[1] && b[1] == p[1]);
            if straight {
                out.pop();
            }
        }
        out.push(p);
    }
    *line = out;
}

impl Shape {
    /// The shape as one or more closed polygons.
    fn contour(&self) -> Result<Vec<[f64; 2]>, SelectionError> {
        let finite = |values: &[f64]| values.iter().all(|v| v.is_finite());
        match self {
            Shape::Rectangle {
                left,
                top,
                right,
                bottom,
            } => {
                if !finite(&[*left, *top, *right, *bottom]) {
                    return Err(SelectionError::InvalidShape);
                }
                Ok(vec![
                    [*left, *top],
                    [*right, *top],
                    [*right, *bottom],
                    [*left, *bottom],
                ])
            }
            Shape::Ellipse {
                left,
                top,
                right,
                bottom,
            } => {
                if !finite(&[*left, *top, *right, *bottom]) {
                    return Err(SelectionError::InvalidShape);
                }
                let (cx, cy) = ((left + right) / 2.0, (top + bottom) / 2.0);
                let (rx, ry) = (((right - left) / 2.0).abs(), ((bottom - top) / 2.0).abs());
                // Chords within 1/100 pixel of the curve: r (1 - cos(θ/2)) ≈ r θ² / 8.
                let r = rx.max(ry).max(1.0);
                let step = (0.08 / r).sqrt();
                let n = ((std::f64::consts::TAU / step).ceil() as usize).clamp(16, 200_000);
                Ok((0..n)
                    .map(|i| {
                        let a = std::f64::consts::TAU * i as f64 / n as f64;
                        [cx + rx * a.cos(), cy + ry * a.sin()]
                    })
                    .collect())
            }
            Shape::Polygon { points } => {
                if !points.iter().all(|p| finite(p)) {
                    return Err(SelectionError::InvalidShape);
                }
                Ok(points.clone())
            }
        }
    }
}

// --- Sparse masks ------------------------------------------------------------------------------

/// A selection being built: one entry per tile of the canvas.
#[derive(Debug, Clone)]
enum Tile {
    /// Every pixel has this coverage.
    Const(u16),
    /// A tile of an existing selection, kept as it is (shared, not copied).
    Shared(Arc<[u8]>),
    /// `T²` values, row-major, padded like raster tiles (the last row and column repeated).
    Data(Vec<u16>),
}

impl Tile {
    fn values(&self) -> std::borrow::Cow<'_, [u16]> {
        match self {
            Tile::Const(v) => vec![*v; T * T].into(),
            Tile::Shared(bytes) => decode(bytes).into(),
            Tile::Data(values) => values.as_slice().into(),
        }
    }

    /// The coverage of every pixel when they are all the same.
    fn constant(&self) -> Option<u16> {
        match self {
            Tile::Const(v) => Some(*v),
            Tile::Shared(_) | Tile::Data(_) => None,
        }
    }
}

#[derive(Debug, Clone)]
struct Mask {
    size: Size,
    columns: usize,
    rows: usize,
    tiles: Vec<Tile>,
}

impl Mask {
    fn new(size: Size) -> Result<Self, SelectionError> {
        if size.is_empty() {
            return Err(SelectionError::EmptyCanvas);
        }
        let columns = size.width.div_ceil(TILE_SIZE) as usize;
        let rows = size.height.div_ceil(TILE_SIZE) as usize;
        Ok(Self {
            size,
            columns,
            rows,
            tiles: vec![Tile::Const(0); columns * rows],
        })
    }

    /// An existing selection on a canvas of `size`. Normally the same size; otherwise it is
    /// read pixel by pixel at the document origin (nothing selected outside it).
    fn from_image(size: Size, image: &RasterImage) -> Result<Self, SelectionError> {
        let mut mask = Self::new(size)?;
        let level = &image.levels()[0];
        let same = image.size() == size && image.stored_format() == SELECTION_FORMAT;
        let mut uniform: HashMap<*const u8, Option<u16>> = HashMap::new();
        for row in 0..mask.rows {
            for col in 0..mask.columns {
                let index = row * mask.columns + col;
                if same {
                    let coord = TileCoord {
                        col: col as u32,
                        row: row as u32,
                    };
                    let Some(tile) = level.tile(coord) else {
                        continue;
                    };
                    let constant = *uniform
                        .entry(tile.as_ptr())
                        .or_insert_with(|| uniform_value(tile));
                    mask.tiles[index] = match constant {
                        Some(v) => Tile::Const(v),
                        None => Tile::Shared(Arc::clone(tile)),
                    };
                } else {
                    let mut values = vec![0u16; T * T];
                    let (x0, y0) = (col * T, row * T);
                    let (w, h) = mask.valid(col, row);
                    for y in 0..h {
                        for x in 0..w {
                            let v = image.gray_at((x0 + x) as u32, (y0 + y) as u32);
                            values[y * T + x] = (v * f32::from(FULL)).round() as u16;
                        }
                    }
                    pad(&mut values, w, h);
                    mask.tiles[index] = Tile::Data(values);
                }
            }
        }
        Ok(mask)
    }

    /// The valid part of a tile: its width and height inside the canvas.
    fn valid(&self, col: usize, row: usize) -> (usize, usize) {
        (
            (self.size.width as usize - col * T).min(T),
            (self.size.height as usize - row * T).min(T),
        )
    }

    /// Coverage at (`x`, `y`), 0 outside the canvas.
    fn get(&self, x: i64, y: i64) -> u16 {
        let inside = (0..i64::from(self.size.width)).contains(&x)
            && (0..i64::from(self.size.height)).contains(&y);
        if inside {
            self.at(x as isize, y as isize)
        } else {
            0
        }
    }

    /// Coverage at (`x`, `y`), clamped to the canvas (the edge repeats).
    fn at(&self, x: isize, y: isize) -> u16 {
        let x = x.clamp(0, self.size.width as isize - 1) as usize;
        let y = y.clamp(0, self.size.height as isize - 1) as usize;
        let tile = &self.tiles[(y / T) * self.columns + x / T];
        match tile {
            Tile::Const(v) => *v,
            Tile::Data(values) => values[(y % T) * T + x % T],
            Tile::Shared(bytes) => {
                let at = ((y % T) * T + x % T) * 2;
                u16::from_ne_bytes([bytes[at], bytes[at + 1]])
            }
        }
    }

    /// The selection image, uniform tiles shared; `None` if nothing is selected.
    fn into_image(self) -> Option<RasterImage> {
        self.build(false)
    }

    /// The image, uniform tiles shared; with `keep_empty`, even when everything is 0 (a mask
    /// hiding everything), else `None` then.
    fn build(self, keep_empty: bool) -> Option<RasterImage> {
        let mut constants: HashMap<u16, Arc<[u8]>> = HashMap::new();
        let constant = |v: u16, constants: &mut HashMap<u16, Arc<[u8]>>| {
            Arc::clone(
                constants
                    .entry(v)
                    .or_insert_with(|| v.to_ne_bytes().repeat(T * T).into()),
            )
        };
        let mut any = false;
        let tiles: Vec<Arc<[u8]>> = self
            .tiles
            .into_iter()
            .map(|tile| match tile {
                Tile::Const(v) => {
                    any |= v != 0;
                    constant(v, &mut constants)
                }
                Tile::Shared(bytes) => {
                    any = true;
                    bytes
                }
                Tile::Data(values) => {
                    let first = values[0];
                    if values.iter().all(|&v| v == first) {
                        any |= first != 0;
                        constant(first, &mut constants)
                    } else {
                        any = true;
                        values
                            .iter()
                            .flat_map(|v| v.to_ne_bytes())
                            .collect::<Vec<u8>>()
                            .into()
                    }
                }
            })
            .collect();
        if !any && !keep_empty {
            return None;
        }
        // Invariant: one tile of `T²` u16 per cell of the canvas grid.
        RasterImage::from_level0_tiles(self.size, SELECTION_FORMAT, tiles).ok()
    }
}

/// Repeat the last valid row and column over the padding, as raster tiles are padded.
fn pad(values: &mut [u16], w: usize, h: usize) {
    if w == 0 || h == 0 {
        return;
    }
    for y in 0..h {
        let last = values[y * T + w - 1];
        values[y * T + w..(y + 1) * T].fill(last);
    }
    for y in h..T {
        let (done, rest) = values.split_at_mut(y * T);
        rest[..T].copy_from_slice(&done[(h - 1) * T..h * T]);
    }
}

fn decode(bytes: &[u8]) -> Vec<u16> {
    let (samples, _) = bytes.as_chunks::<2>();
    samples.iter().map(|b| u16::from_ne_bytes(*b)).collect()
}

/// The value of a tile whose samples are all the same (16-bit gray), or `None`.
fn uniform_value(bytes: &[u8]) -> Option<u16> {
    let (samples, _) = bytes.as_chunks::<2>();
    let first = samples.first()?;
    samples
        .iter()
        .all(|b| b == first)
        .then(|| u16::from_ne_bytes(*first))
}

// --- Layer masks -------------------------------------------------------------------------------

/// A layer mask of `size` pixels, everything shown (Layer > Layer Mask > Reveal All) or hidden
/// (Hide All): one shared tile.
pub fn uniform_mask(size: Size, shown: bool) -> Result<RasterImage, SelectionError> {
    let mut mask = Mask::new(size)?;
    mask.tiles.fill(Tile::Const(if shown { FULL } else { 0 }));
    // A canvas with pixels always makes an image.
    mask.build(true).ok_or(SelectionError::EmptyCanvas)
}

/// A layer mask from the selection (Layer > Layer Mask > Reveal Selection, Hide Selection): `size`
/// pixels in the layer's space, which `layer_to_document` places in the document (ADR 0014), each
/// taking the selection's coverage where it lands, or its complement with `hide`. A whole-pixel
/// move copies the values exactly; any other transform samples them bilinearly. Uniform tiles are
/// shared, and the work is spread over every core.
pub fn layer_mask(
    selection: &RasterImage,
    size: Size,
    layer_to_document: Affine,
    hide: bool,
) -> Result<RasterImage, SelectionError> {
    if !layer_to_document.is_finite() || layer_to_document.inverse().is_none() {
        return Err(SelectionError::InvalidShape);
    }
    let source = Mask::from_image(selection.size(), selection)?;
    let mut mask = Mask::new(size)?;
    let offset = layer_to_document.integer_translation();
    let sample = |x: usize, y: usize| -> u16 {
        let v = match offset {
            Some((dx, dy)) => source.get(x as i64 + dx, y as i64 + dy),
            None => {
                // The pixel's center in the document, read between the selection's centers.
                let (px, py) = layer_to_document.apply(x as f64 + 0.5, y as f64 + 0.5);
                let (fx, fy) = (px - 0.5, py - 0.5);
                let (x0, y0) = (fx.floor(), fy.floor());
                let (tx, ty) = (fx - x0, fy - y0);
                let (x0, y0) = (x0 as i64, y0 as i64);
                let at = |x: i64, y: i64| f64::from(source.get(x, y));
                let top = at(x0, y0) * (1.0 - tx) + at(x0 + 1, y0) * tx;
                let bottom = at(x0, y0 + 1) * (1.0 - tx) + at(x0 + 1, y0 + 1) * tx;
                (top * (1.0 - ty) + bottom * ty)
                    .round()
                    .clamp(0.0, f64::from(FULL)) as u16
            }
        };
        if hide { FULL - v } else { v }
    };
    let rows: Vec<usize> = (0..mask.rows).collect();
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let per_thread = rows.len().div_ceil(threads).max(1);
    let columns = mask.columns;
    let shape = &mask;
    let sample = &sample;
    let mut done: Vec<(usize, Vec<u16>)> = Vec::new();
    std::thread::scope(|scope| {
        let workers: Vec<_> = rows
            .chunks(per_thread)
            .map(|chunk| {
                scope.spawn(move || {
                    let mut out = Vec::new();
                    for &row in chunk {
                        for col in 0..columns {
                            let (w, h) = shape.valid(col, row);
                            let mut values = vec![0u16; T * T];
                            for y in 0..h {
                                for x in 0..w {
                                    values[y * T + x] = sample(col * T + x, row * T + y);
                                }
                            }
                            pad(&mut values, w, h);
                            out.push((row * columns + col, values));
                        }
                    }
                    out
                })
            })
            .collect();
        for worker in workers {
            // Invariant: sampling does not panic (reads outside the selection are 0).
            done.extend(worker.join().expect("layer mask worker panicked"));
        }
    });
    for (index, values) in done {
        mask.tiles[index] = Tile::Data(values);
    }
    mask.build(true).ok_or(SelectionError::EmptyCanvas)
}

// --- Modify ------------------------------------------------------------------------------------

/// The largest radius or width of Select > Modify, in pixels (Feather keeps [`MAX_FEATHER`]).
pub const MAX_MODIFY: f64 = 500.0;

/// Select > Modify: a change of the whole selection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Modify {
    /// Gaussian softening, its standard deviation in pixels (Photoshop's radius).
    Feather(f64),
    /// Grow the selection by this many pixels, corners rounded.
    Expand(f64),
    /// Shrink it by this many pixels.
    Contract(f64),
    /// Keep a band this wide, centered on its outline.
    Border(f64),
    /// Round its corners and drop its specks, about this radius.
    Smooth(f64),
    /// Harden its soft edges, percent (0: unchanged, 100: no soft edge left), around one
    /// half: Select and Mask's Contrast.
    Contrast(f64),
}

/// `selection` changed by `how` on a `canvas`. `None`: nothing is left selected. Expand,
/// Contract and Border follow the exact distance to the outline (where coverage crosses one
/// half), with anti-aliased edges; a soft selection gets a crisp edge at that distance. As in
/// Photoshop (without "apply effect at canvas bounds"), the canvas edge does not count as an
/// outline. Only the tiles near the outline are computed, on every core.
pub fn modify(
    canvas: Size,
    selection: &RasterImage,
    how: Modify,
) -> Result<Option<RasterImage>, SelectionError> {
    let (Modify::Feather(r)
    | Modify::Expand(r)
    | Modify::Contract(r)
    | Modify::Border(r)
    | Modify::Smooth(r)
    | Modify::Contrast(r)) = how;
    let max = match how {
        Modify::Feather(_) => MAX_FEATHER,
        Modify::Contrast(_) => 100.0,
        _ => MAX_MODIFY,
    };
    if !r.is_finite() || !(0.0..=max).contains(&r) {
        return Err(SelectionError::InvalidShape);
    }
    let mask = Mask::from_image(canvas, selection)?;
    if r == 0.0 {
        return Ok(mask.into_image());
    }
    let result = match how {
        Modify::Feather(sigma) => feather(&mask, sigma),
        Modify::Smooth(radius) => {
            // Blurred, then brought back to a one-pixel ramp around one half: corners round
            // and specks narrower than the blur fade out.
            let sigma = radius / 2.0;
            let gain = (sigma * (2.0 * std::f64::consts::PI).sqrt()).max(1.0) as f32;
            sharpened(feather(&mask, sigma), gain)
        }
        // 100 % is a step: a gain far beyond any 16-bit ramp.
        Modify::Contrast(percent) => {
            sharpened(mask, (1.0 / (1.0 - percent / 100.0).max(1e-5)) as f32)
        }
        Modify::Expand(r) => by_distance(&mask, r, move |h| r - h + 0.5),
        Modify::Contract(r) => by_distance(&mask, r, move |h| -r - h + 0.5),
        Modify::Border(width) => {
            by_distance(&mask, width / 2.0, move |h| width / 2.0 - h.abs() + 0.5)
        }
    };
    Ok(result.into_image())
}

/// `mask`'s coverage steepened by `gain` around one half (1: unchanged).
fn sharpened(mut mask: Mask, gain: f32) -> Mask {
    let sharp = |v: u16| {
        let c = (f32::from(v) / f32::from(FULL) - 0.5) * gain + 0.5;
        (c.clamp(0.0, 1.0) * f32::from(FULL)).round() as u16
    };
    for tile in &mut mask.tiles {
        *tile = match &*tile {
            Tile::Const(v) => Tile::Const(sharp(*v)),
            other => Tile::Data(other.values().iter().map(|&v| sharp(v)).collect()),
        };
    }
    mask
}

/// Select and Mask's edge settings, applied in this order: Smooth (a radius), Shift Edge
/// (pixels, outward when positive: Expand or Contract), Feather (a radius), Contrast (percent).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct EdgeSettings {
    pub smooth: f64,
    pub shift: f64,
    pub feather: f64,
    pub contrast: f64,
}

/// `base` with `settings` applied ([`EdgeSettings`]), each step a [`modify`]; unchanged
/// (sharing its tiles) without any. `None`: nothing is left selected.
pub fn refine_edges(
    canvas: Size,
    base: &RasterImage,
    settings: EdgeSettings,
) -> Result<Option<RasterImage>, SelectionError> {
    let steps = [
        (settings.smooth > 0.0).then_some(Modify::Smooth(settings.smooth)),
        (settings.shift > 0.0).then_some(Modify::Expand(settings.shift)),
        (settings.shift < 0.0).then_some(Modify::Contract(-settings.shift)),
        (settings.feather > 0.0).then_some(Modify::Feather(settings.feather)),
        (settings.contrast > 0.0).then_some(Modify::Contrast(settings.contrast)),
    ];
    let mut out: Option<RasterImage> = None;
    for how in steps.into_iter().flatten() {
        match modify(canvas, out.as_ref().unwrap_or(base), how)? {
            Some(next) => out = Some(next),
            None => return Ok(None),
        }
    }
    match out {
        Some(out) => Ok(Some(out)),
        None => Ok(Mask::from_image(canvas, base)?.into_image()),
    }
}

/// Where Edit > Stroke draws its band, relative to the selection's outline (Photoshop's
/// Location).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrokeLocation {
    Inside,
    Center,
    Outside,
}

/// Edit > Stroke: the band `width` pixels wide along the outline of `selection` (where its
/// coverage crosses one half), inside, centered on or outside it, anti-aliased like Select >
/// Modify > Border. `None`: nothing to stroke.
pub fn stroke_band(
    canvas: Size,
    selection: &RasterImage,
    width: f64,
    location: StrokeLocation,
) -> Result<Option<RasterImage>, SelectionError> {
    if !width.is_finite() || !(0.0..=MAX_MODIFY).contains(&width) {
        return Err(SelectionError::InvalidShape);
    }
    if width == 0.0 {
        return Ok(None);
    }
    if location == StrokeLocation::Center {
        return modify(canvas, selection, Modify::Border(width));
    }
    let mask = Mask::from_image(canvas, selection)?;
    // Coverage 1 within the band, a one-pixel anti-aliased ramp at each of its edges.
    let band = if location == StrokeLocation::Inside {
        by_distance(&mask, width, move |h| (0.5 - h).min(width + h + 0.5))
    } else {
        by_distance(&mask, width, move |h| (h + 0.5).min(width - h + 0.5))
    };
    Ok(band.into_image())
}

/// A new mask whose coverage is `cover(h)` (clamped to `[0, 1]`), `h` being the signed
/// distance from each pixel center to the outline of `mask` (negative inside), exact up to
/// `reach` pixels (and beyond it only known to be farther).
fn by_distance(mask: &Mask, reach: f64, cover: impl Fn(f64) -> f64 + Sync) -> Mask {
    let halo = reach.ceil() as usize + 2;
    let tiles_reach = halo.div_ceil(T);
    let mut out = mask.clone();
    let mut todo = Vec::new();
    for index in 0..mask.tiles.len() {
        let (col, row) = (index % mask.columns, index / mask.columns);
        let mut seen: Option<bool> = None;
        let mut mixed = false;
        'scan: for r in row.saturating_sub(tiles_reach)..=(row + tiles_reach).min(mask.rows - 1) {
            for c in col.saturating_sub(tiles_reach)..=(col + tiles_reach).min(mask.columns - 1) {
                match (mask.tiles[r * mask.columns + c].constant(), seen) {
                    (None, _) => {
                        mixed = true;
                        break 'scan;
                    }
                    (Some(v), None) => seen = Some(v >= HALF),
                    (Some(v), Some(inside)) if (v >= HALF) != inside => {
                        mixed = true;
                        break 'scan;
                    }
                    _ => {}
                }
            }
        }
        if mixed {
            todo.push(index);
        } else {
            // Far from any outline: wholly inside or outside.
            let inside = seen.unwrap_or(false);
            let h = if inside {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            };
            let v = cover(h).clamp(0.0, 1.0);
            out.tiles[index] = Tile::Const((v * f64::from(FULL)).round() as u16);
        }
    }
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let per_thread = todo.len().div_ceil(threads).max(1);
    let cover = &cover;
    let mut computed: Vec<(usize, Vec<u16>)> = Vec::new();
    std::thread::scope(|scope| {
        let workers: Vec<_> = todo
            .chunks(per_thread)
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .map(|&index| (index, distance_tile(mask, index, halo, cover)))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        for worker in workers {
            // Invariant: the distance transform does not panic (reads are clamped).
            computed.extend(worker.join().expect("selection distance worker panicked"));
        }
    });
    for (index, values) in computed {
        out.tiles[index] = Tile::Data(values);
    }
    out
}

/// One tile of [`by_distance`]: the distance transforms of a window of the tile plus `halo`
/// pixels around it (the canvas edge repeating).
fn distance_tile(
    mask: &Mask,
    index: usize,
    halo: usize,
    cover: &(impl Fn(f64) -> f64 + Sync),
) -> Vec<u16> {
    let (col, row) = (index % mask.columns, index / mask.columns);
    let side = T + 2 * halo;
    let (x0, y0) = (
        (col * T) as isize - halo as isize,
        (row * T) as isize - halo as isize,
    );
    let inside: Vec<bool> = (0..side * side)
        .map(|i| mask.at(x0 + (i % side) as isize, y0 + (i / side) as isize) >= HALF)
        .collect();
    let to_inside = squared_distances(&inside, side, true);
    let to_outside = squared_distances(&inside, side, false);
    let mut values = vec![0u16; T * T];
    for y in 0..T {
        for x in 0..T {
            let i = (y + halo) * side + x + halo;
            // Between the centers on both sides of an edge, the outline is half a pixel away.
            let h = if inside[i] {
                -(to_outside[i].sqrt() - 0.5)
            } else {
                to_inside[i].sqrt() - 0.5
            };
            values[y * T + x] = (cover(h).clamp(0.0, 1.0) * f64::from(FULL)).round() as u16;
        }
    }
    let (w, h) = mask.valid(col, row);
    pad(&mut values, w, h);
    values
}

/// The squared distance from each pixel of a `side`² window to the nearest pixel where
/// `inside` equals `feature` (a large number where there is none): exact Euclidean distance
/// transform, by rows then columns (Felzenszwalb and Huttenlocher).
fn squared_distances(inside: &[bool], side: usize, feature: bool) -> Vec<f64> {
    /// "No feature": large, yet finite so that the envelope's arithmetic stays defined.
    const FAR: f64 = 1e20;
    let mut grid: Vec<f64> = inside
        .iter()
        .map(|&v| if v == feature { 0.0 } else { FAR })
        .collect();
    let mut line = vec![0.0f64; side];
    let mut result = vec![0.0f64; side];
    let mut hull = vec![0usize; side];
    let mut bounds = vec![0.0f64; side + 1];
    for pass in 0..2 {
        for i in 0..side {
            let at = |j: usize| {
                if pass == 0 {
                    i * side + j
                } else {
                    j * side + i
                }
            };
            for (j, value) in line.iter_mut().enumerate() {
                *value = grid[at(j)];
            }
            transform_1d(&line, &mut result, &mut hull, &mut bounds);
            for (j, value) in result.iter().enumerate() {
                grid[at(j)] = *value;
            }
        }
    }
    grid
}

/// The 1D squared distance transform of `f` into `d` (lower envelope of parabolas).
fn transform_1d(f: &[f64], d: &mut [f64], hull: &mut [usize], bounds: &mut [f64]) {
    let n = f.len();
    let intersect = |q: usize, p: usize| {
        let (q2, p2) = ((q * q) as f64, (p * p) as f64);
        ((f[q] + q2) - (f[p] + p2)) / (2.0 * (q as f64 - p as f64))
    };
    let mut k = 0usize;
    hull[0] = 0;
    bounds[0] = f64::NEG_INFINITY;
    bounds[1] = f64::INFINITY;
    for q in 1..n {
        let mut s = intersect(q, hull[k]);
        while s <= bounds[k] {
            k -= 1;
            s = intersect(q, hull[k]);
        }
        k += 1;
        hull[k] = q;
        bounds[k] = s;
        bounds[k + 1] = f64::INFINITY;
    }
    k = 0;
    for (q, out) in d.iter_mut().enumerate().take(n) {
        while bounds[k + 1] < q as f64 {
            k += 1;
        }
        let dq = q as f64 - hull[k] as f64;
        *out = dq * dq + f[hull[k]];
    }
}

// --- Magic Wand --------------------------------------------------------------------------------

/// How the Magic Wand picks pixels (Photoshop's options).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WandOptions {
    /// The largest difference from the clicked pixel's color (any of red, green, blue, alpha, in
    /// 8-bit display values: 0–255) for a pixel to be selected.
    pub tolerance: f32,
    /// Only pixels connected to the clicked one (4-connected); otherwise every similar pixel.
    pub contiguous: bool,
    /// Soften the edge over about a pixel.
    pub anti_alias: bool,
}

/// How many composited tiles the contiguous fill keeps at once (64 MB).
const WAND_TILE_CACHE: usize = 256;

/// The Magic Wand: the pixels of `source` (the whole document, or a document holding only the
/// layer to sample) whose color is within the tolerance of the one at `seed`, combined with
/// `current` on its canvas. Colors are compared as displayed (8-bit sRGB, straight alpha).
/// `source` is composited tile by tile when needed: a contiguous fill only reads the tiles it
/// reaches, and memory stays bounded whatever the canvas.
pub fn magic_wand(
    source: &crate::document::Document,
    current: Option<&RasterImage>,
    seed: (u32, u32),
    options: WandOptions,
    combine: Combine,
) -> Result<Option<RasterImage>, SelectionError> {
    let canvas = source.size();
    if !options.tolerance.is_finite() || !(0.0..=255.0).contains(&options.tolerance) {
        return Err(SelectionError::InvalidShape);
    }
    let mask = Mask::new(canvas)?;
    if seed.0 >= canvas.width || seed.1 >= canvas.height {
        return finish(canvas, current, mask, combine);
    }
    let sampler = WandSampler::new(source);
    let (col, row) = (seed.0 as usize / T, seed.1 as usize / T);
    let reference = sampler.tile(col, row, &mask)[(seed.1 as usize % T) * T + seed.0 as usize % T];
    let tolerance = options.tolerance;
    let seeds = vec![(
        row * mask.columns + col,
        vec![(seed.0 as usize % T, seed.1 as usize % T)],
    )];
    let similar = move |p: [f32; 4]| within(p, reference, reference, tolerance);
    let mask = wand_mask(&sampler, mask, seeds, options, &similar);
    finish(canvas, current, mask, combine)
}

/// Whether `color` is within `tolerance` of the range from `low` to `high` on each of red,
/// green, blue and alpha (as displayed): the Magic Wand's test, around one color (`low` and
/// `high` the same) or around the range of a selection's colors (Grow, Similar).
fn within(color: [f32; 4], low: [f32; 4], high: [f32; 4], tolerance: f32) -> bool {
    let tolerance = tolerance + 1e-3;
    (0..4).all(|c| color[c] >= low[c] - tolerance && color[c] <= high[c] + tolerance)
}

/// Select > Grow (`options.contiguous`) and Select > Similar (not contiguous), as in Photoshop:
/// the Magic Wand from every selected pixel (half covered or more) at once, its tolerance taken
/// around the range of their colors (each channel's lowest and highest value), the pixels found
/// added to `current`. `source` is the composited document, or one holding only the layer to
/// sample, as for the Magic Wand.
pub fn grow(
    source: &crate::document::Document,
    current: &RasterImage,
    options: WandOptions,
) -> Result<Option<RasterImage>, SelectionError> {
    let canvas = source.size();
    if !options.tolerance.is_finite() || !(0.0..=255.0).contains(&options.tolerance) {
        return Err(SelectionError::InvalidShape);
    }
    let selected = Mask::from_image(canvas, current)?;
    let sampler = WandSampler::new(source);
    // Each tile's selected colors' range, and its selected pixels next to an unselected one
    // (the fill reaches the others from them), on every core.
    let tiles: Vec<usize> = (0..selected.tiles.len())
        .filter(|&i| !matches!(selected.tiles[i], Tile::Const(v) if v < HALF))
        .collect();
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let per_thread = tiles.len().div_ceil(threads).max(1);
    let (shape, sampler_ref) = (&selected, &sampler);
    let mut found: Vec<GrowTile> = Vec::new();
    std::thread::scope(|scope| {
        let workers: Vec<_> = tiles
            .chunks(per_thread)
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .map(|&index| grow_seeds(sampler_ref, shape, index))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        for worker in workers {
            // Invariant: reading colors does not panic.
            found.extend(
                worker
                    .join()
                    .expect("grow worker panicked")
                    .into_iter()
                    .flatten(),
            );
        }
    });
    let mut low = [f32::MAX; 4];
    let mut high = [f32::MIN; 4];
    let mut seeds = Vec::new();
    for tile in found {
        for c in 0..4 {
            low[c] = low[c].min(tile.low[c]);
            high[c] = high[c].max(tile.high[c]);
        }
        if !tile.seeds.is_empty() {
            seeds.push((tile.index, tile.seeds));
        }
    }
    let mut mask = Mask::new(canvas)?;
    if low[0] <= high[0] {
        let tolerance = options.tolerance;
        let similar = move |p: [f32; 4]| within(p, low, high, tolerance);
        mask = wand_mask(&sampler, mask, seeds, options, &similar);
    }
    finish(canvas, Some(current), mask, Combine::Add)
}

/// What [`grow`] reads of a tile of the selection.
struct GrowTile {
    index: usize,
    /// The selected pixels that have an unselected neighbor, in the tile.
    seeds: Vec<(usize, usize)>,
    /// The lowest and highest selected colors, channel by channel.
    low: [f32; 4],
    high: [f32; 4],
}

/// Tile `index` of `selected` for [`grow`]; `None` if none of its pixels is selected.
fn grow_seeds(sampler: &WandSampler<'_>, selected: &Mask, index: usize) -> Option<GrowTile> {
    let (col, row) = (index % selected.columns, index / selected.columns);
    let (w, h) = selected.valid(col, row);
    let values = selected.tiles[index].values();
    let pixels = sampler.tile(col, row, selected);
    let (x0, y0) = ((col * T) as i64, (row * T) as i64);
    let inside = |x: i64, y: i64| selected.get(x, y) >= HALF;
    let mut low = [f32::MAX; 4];
    let mut high = [f32::MIN; 4];
    let mut seeds = Vec::new();
    for y in 0..h {
        for x in 0..w {
            if values[y * T + x] < HALF {
                continue;
            }
            let color = pixels[y * T + x];
            for c in 0..4 {
                low[c] = low[c].min(color[c]);
                high[c] = high[c].max(color[c]);
            }
            let (gx, gy) = (x0 + x as i64, y0 + y as i64);
            let in_canvas = |x: i64, y: i64| {
                (0..i64::from(selected.size.width)).contains(&x)
                    && (0..i64::from(selected.size.height)).contains(&y)
            };
            let edge = [(-1, 0), (1, 0), (0, -1), (0, 1)]
                .iter()
                .any(|&(dx, dy)| in_canvas(gx + dx, gy + dy) && !inside(gx + dx, gy + dy));
            if edge {
                seeds.push((x, y));
            }
        }
    }
    (low[0] <= high[0]).then_some(GrowTile {
        index,
        seeds,
        low,
        high,
    })
}

/// The Magic Wand's selection of the pixels whose color `similar` accepts: those connected to
/// `seeds` (a tile's index and positions in it) when `options.contiguous`, else every one,
/// anti-aliased on request. `mask` is empty, the size of the canvas.
fn wand_mask(
    sampler: &WandSampler<'_>,
    mut mask: Mask,
    seeds: Vec<(usize, Vec<(usize, usize)>)>,
    options: WandOptions,
    similar: &(impl Fn([f32; 4]) -> bool + Sync),
) -> Mask {
    if options.contiguous {
        flood(sampler, &mut mask, seeds, similar);
    } else {
        let tiles: Vec<usize> = (0..mask.tiles.len()).collect();
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
        let per_thread = tiles.len().div_ceil(threads).max(1);
        let shape = &mask;
        let mut done: Vec<(usize, Tile)> = Vec::new();
        std::thread::scope(|scope| {
            let workers: Vec<_> = tiles
                .chunks(per_thread)
                .map(|chunk| {
                    scope.spawn(move || {
                        chunk
                            .iter()
                            .map(|&index| {
                                let (col, row) = (index % shape.columns, index / shape.columns);
                                let pixels = sampler.tile(col, row, shape);
                                let (w, h) = shape.valid(col, row);
                                let mut values = vec![0u16; T * T];
                                for y in 0..h {
                                    for x in 0..w {
                                        if similar(pixels[y * T + x]) {
                                            values[y * T + x] = FULL;
                                        }
                                    }
                                }
                                pad(&mut values, w, h);
                                (index, computed_tile(values))
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            for worker in workers {
                // Invariant: comparing colors does not panic.
                done.extend(worker.join().expect("magic wand worker panicked"));
            }
        });
        for (index, tile) in done {
            mask.tiles[index] = tile;
        }
    }
    if options.anti_alias {
        mask = soften(&mask);
    }
    mask
}

/// The composited colors of a document, tile by tile, as displayed: whole 8-bit sRGB values
/// (0–255), straight alpha (transparent pixels read as transparent black).
struct WandSampler<'a> {
    document: &'a crate::document::Document,
    to_srgb: crate::color::Mat3,
}

impl<'a> WandSampler<'a> {
    fn new(document: &'a crate::document::Document) -> Self {
        Self {
            document,
            to_srgb: document.working_space().matrix_to(&ColorSpace::LINEAR_SRGB),
        }
    }

    /// Tile (`col`, `row`): `T²` colors, row-major (its valid part; the rest transparent).
    fn tile(&self, col: usize, row: usize, mask: &Mask) -> Vec<[f32; 4]> {
        let (w, h) = mask.valid(col, row);
        let region = Rect::new((col * T) as u32, (row * T) as u32, w as u32, h as u32);
        let mut rgba = vec![0f32; w * h * 4];
        let mut out = vec![[0f32; 4]; T * T];
        // Callers composite tiles on every core already: one thread per tile.
        if crate::composite::composite_region_serial(self.document, region, &mut rgba).is_err() {
            return out;
        }
        for y in 0..h {
            for x in 0..w {
                let p = &rgba[(y * w + x) * 4..(y * w + x) * 4 + 4];
                let a = p[3].clamp(0.0, 1.0);
                let color = if a > 0.0 {
                    let linear = crate::color::mat_vec(
                        &self.to_srgb,
                        [
                            f64::from(p[0] / a),
                            f64::from(p[1] / a),
                            f64::from(p[2] / a),
                        ],
                    );
                    linear.map(|v| f32::from(srgb_byte(v as f32)))
                } else {
                    [0.0; 3]
                };
                out[y * T + x] = [color[0], color[1], color[2], (a * 255.0).round()];
            }
        }
        out
    }
}

/// `round(srgb_encode(v) × 255)` for `v` clamped to `[0, 1]`, without a power per call: the
/// smallest value of each code, found once by bisection on that very formula (so the result is
/// the same bit for bit), then a binary search.
fn srgb_byte(v: f32) -> u8 {
    static THRESHOLDS: std::sync::OnceLock<[f32; 255]> = std::sync::OnceLock::new();
    let exact = |v: f32| (crate::color::srgb_encode(v.clamp(0.0, 1.0)) * 255.0).round();
    let thresholds = THRESHOLDS.get_or_init(|| {
        std::array::from_fn(|k| {
            let code = (k + 1) as f32;
            // Non-negative floats are ordered as their bits: bisect on them.
            let (mut low, mut high) = (0u32, 1f32.to_bits());
            while low < high {
                let mid = low + (high - low) / 2;
                if exact(f32::from_bits(mid)) >= code {
                    high = mid;
                } else {
                    low = mid + 1;
                }
            }
            f32::from_bits(low)
        })
    });
    thresholds.partition_point(|&t| t <= v) as u8
}

/// The contiguous fill from `seeds` (a tile's index and positions in it), tile by tile: each
/// tile is filled from the pixels where the fill entered it (a scanline fill), and passes on the
/// pixels where it leaves it. The tiles a wave of the fill reaches are composited together, on
/// every core, and kept in a bounded cache (8-bit colors: 256 KB a tile).
fn flood(
    sampler: &WandSampler<'_>,
    mask: &mut Mask,
    seeds: Vec<(usize, Vec<(usize, usize)>)>,
    similar: &(impl Fn([f32; 4]) -> bool + Sync),
) {
    let (columns, rows) = (mask.columns, mask.rows);
    let mut selected: HashMap<usize, Vec<u16>> = HashMap::new();
    let mut cache: HashMap<usize, (u64, Vec<[u8; 4]>)> = HashMap::new();
    let mut clock = 0u64;
    let mut pending = seeds;
    while !pending.is_empty() {
        clock += 1;
        // This wave: at most half the cache's tiles, so that they all stay cached meanwhile.
        let mut wave: Vec<(usize, Vec<(usize, usize)>)> = Vec::new();
        let mut tiles: Vec<usize> = Vec::new();
        let mut rest = Vec::new();
        for (index, seeds) in pending.drain(..) {
            if tiles.contains(&index) || tiles.len() < WAND_TILE_CACHE / 2 {
                if !tiles.contains(&index) {
                    tiles.push(index);
                }
                wave.push((index, seeds));
            } else {
                rest.push((index, seeds));
            }
        }
        pending = rest;
        let missing: Vec<usize> = tiles
            .iter()
            .copied()
            .filter(|i| !cache.contains_key(i))
            .collect();
        while cache.len() + missing.len() > WAND_TILE_CACHE {
            let oldest = cache
                .iter()
                .filter(|(i, _)| !tiles.contains(i))
                .min_by_key(|(_, (t, _))| *t)
                .map(|(i, _)| *i);
            match oldest {
                Some(i) => {
                    cache.remove(&i);
                }
                None => break,
            }
        }
        let shape = &*mask;
        let composited: Vec<(usize, Vec<[u8; 4]>)> = std::thread::scope(|scope| {
            let workers: Vec<_> = missing
                .iter()
                .map(|&index| {
                    scope.spawn(move || {
                        let tile = sampler.tile(index % columns, index / columns, shape);
                        (index, tile.iter().map(|p| p.map(|v| v as u8)).collect())
                    })
                })
                .collect();
            workers
                .into_iter()
                // Invariant: compositing a tile does not panic.
                .map(|w| w.join().expect("magic wand compositing panicked"))
                .collect()
        });
        for (index, pixels) in composited {
            cache.insert(index, (clock, pixels));
        }
        for (index, seeds) in wave {
            let (col, row) = (index % columns, index / columns);
            let (w, h) = mask.valid(col, row);
            let Some(entry) = cache.get_mut(&index) else {
                continue;
            };
            entry.0 = clock;
            let pixels = &entry.1;
            let at = |x: usize, y: usize| pixels[y * T + x].map(f32::from);
            let values = selected.entry(index).or_insert_with(|| vec![0u16; T * T]);
            let mut stack: Vec<(usize, usize)> = seeds
                .into_iter()
                .filter(|&(x, y)| x < w && y < h && values[y * T + x] == 0 && similar(at(x, y)))
                .collect();
            // Pixels leaving the tile, by neighbor: above, below, left, right.
            let mut out: [Vec<(usize, usize)>; 4] = Default::default();
            while let Some((x, y)) = stack.pop() {
                if values[y * T + x] != 0 {
                    continue;
                }
                // The run of similar, unselected pixels through (x, y).
                let mut x0 = x;
                while x0 > 0 && values[y * T + x0 - 1] == 0 && similar(at(x0 - 1, y)) {
                    x0 -= 1;
                }
                let mut x1 = x;
                while x1 + 1 < w && values[y * T + x1 + 1] == 0 && similar(at(x1 + 1, y)) {
                    x1 += 1;
                }
                for xi in x0..=x1 {
                    values[y * T + xi] = FULL;
                    if y > 0 {
                        if values[(y - 1) * T + xi] == 0 && similar(at(xi, y - 1)) {
                            stack.push((xi, y - 1));
                        }
                    } else if row > 0 {
                        out[0].push((xi, T - 1));
                    }
                    if y + 1 < h {
                        if values[(y + 1) * T + xi] == 0 && similar(at(xi, y + 1)) {
                            stack.push((xi, y + 1));
                        }
                    } else if h == T && row + 1 < rows {
                        out[1].push((xi, 0));
                    }
                }
                if x0 == 0 && col > 0 {
                    out[2].push((T - 1, y));
                }
                if x1 + 1 == w && w == T && col + 1 < columns {
                    out[3].push((0, y));
                }
            }
            let neighbors = [
                row.checked_sub(1).map(|r| r * columns + col),
                (row + 1 < rows).then(|| (row + 1) * columns + col),
                col.checked_sub(1).map(|c| row * columns + c),
                (col + 1 < columns).then(|| row * columns + col + 1),
            ];
            for (seeds, neighbor) in out.into_iter().zip(neighbors) {
                if let Some(neighbor) = neighbor
                    && !seeds.is_empty()
                {
                    pending.push((neighbor, seeds));
                }
            }
        }
    }
    for (index, mut values) in selected {
        let (w, h) = mask.valid(index % columns, index / columns);
        pad(&mut values, w, h);
        mask.tiles[index] = Tile::Data(values);
    }
}

/// A 3×3 average, about a pixel of anti-aliasing: on the tiles that are not uniform, or whose
/// neighbors differ from them. Tiles are averaged on every core.
fn soften(mask: &Mask) -> Mask {
    let (columns, rows) = (mask.columns as isize, mask.rows as isize);
    let uniform = |index: usize| {
        let value = mask.tiles[index].constant()?;
        let (col, row) = (
            (index % mask.columns) as isize,
            (index / mask.columns) as isize,
        );
        let same = (-1..=1).all(|dy| {
            (-1..=1).all(|dx| {
                let (c, r) = (
                    (col + dx).clamp(0, columns - 1),
                    (row + dy).clamp(0, rows - 1),
                );
                mask.tiles[(r * columns + c) as usize].constant() == Some(value)
            })
        });
        same.then_some(value)
    };
    let edges: Vec<usize> = (0..mask.tiles.len())
        .filter(|&i| uniform(i).is_none())
        .collect();
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let per_thread = edges.len().div_ceil(threads).max(1);
    let mut out = mask.clone();
    std::thread::scope(|scope| {
        let workers: Vec<_> = edges
            .chunks(per_thread)
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .map(|&index| (index, soften_tile(mask, index)))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        for worker in workers {
            // Invariant: averaging does not panic.
            for (index, tile) in worker.join().expect("soften worker panicked") {
                out.tiles[index] = tile;
            }
        }
    });
    out
}

/// Tile `index` of `mask` averaged over 3×3 pixels (the canvas edge repeating).
fn soften_tile(mask: &Mask, index: usize) -> Tile {
    let (col, row) = (index % mask.columns, index / mask.columns);
    let (w, h) = mask.valid(col, row);
    // The tile and a ring of one pixel around it, read from its neighbors.
    let stride = w + 2;
    let mut around = vec![0u32; stride * (h + 2)];
    let inside = mask.tiles[index].values();
    let (x0, y0) = ((col * T) as isize, (row * T) as isize);
    for y in 0..h + 2 {
        for x in 0..w + 2 {
            around[y * stride + x] = if (1..=w).contains(&x) && (1..=h).contains(&y) {
                u32::from(inside[(y - 1) * T + x - 1])
            } else {
                u32::from(mask.at(x0 + x as isize - 1, y0 + y as isize - 1))
            };
        }
    }
    // Sums of three along rows, then along columns.
    let mut sums = vec![0u32; stride * (h + 2)];
    for y in 0..h + 2 {
        let line = &around[y * stride..(y + 1) * stride];
        for x in 1..=w {
            sums[y * stride + x] = line[x - 1] + line[x] + line[x + 1];
        }
    }
    let mut values = vec![0u16; T * T];
    for y in 0..h {
        for x in 0..w {
            let at = |dy: usize| sums[(y + dy) * stride + x + 1];
            values[y * T + x] = ((at(0) + at(1) + at(2)) / 9) as u16;
        }
    }
    pad(&mut values, w, h);
    Tile::Data(values)
}

/// A tile of computed values: a constant one when they are all the same, so that later steps
/// (anti-aliasing, combining, storing) skip it.
fn computed_tile(values: Vec<u16>) -> Tile {
    let first = values[0];
    if values.iter().all(|&v| v == first) {
        Tile::Const(first)
    } else {
        Tile::Data(values)
    }
}

// --- Color Range -------------------------------------------------------------------------------

/// The largest fuzziness of Color Range (Photoshop's).
pub const MAX_FUZZINESS: f32 = 200.0;

/// What Select > Color Range selects: the colors of the sampled pixels and those within
/// `fuzziness` of them, partly selected as they get farther; minus the colors of the
/// `excluded` samples (Photoshop's subtracting eyedropper); inverted on request. Localized,
/// each included color is only selected near the point where it was sampled.
#[derive(Debug, Clone, PartialEq)]
pub struct ColorRange {
    /// Colors as displayed (whole 8-bit sRGB values, straight alpha): see [`sample_colors`].
    pub included: Vec<[f32; 4]>,
    pub excluded: Vec<[f32; 4]>,
    /// 0–200: how far from a sampled color a color is still selected, in 8-bit steps.
    pub fuzziness: f32,
    pub invert: bool,
    pub localized: Option<Localized>,
}

/// Color Range's Localized option (Photoshop's Localized Color Clusters): where each included
/// color was sampled, and how far from there it is still selected.
#[derive(Debug, Clone, PartialEq)]
pub struct Localized {
    /// Document points, one per included color, in the same order.
    pub points: Vec<(f64, f64)>,
    /// Document pixels; the selection fades linearly to nothing at that distance.
    pub radius: f64,
}

impl ColorRange {
    /// How much `color` (as displayed) is selected, in `[0, 1]`: 1 on a sampled color, falling
    /// linearly to 0 at `fuzziness` (the largest difference over red, green, blue and alpha).
    /// Localized, each included color is also weighed by its distance from document point `at`
    /// (when given).
    pub fn coverage(&self, color: [f32; 4], at: Option<(f64, f64)>) -> f32 {
        let similar = |s: &[f32; 4]| {
            let d = (0..4)
                .map(|c| (color[c] - s[c]).abs())
                .fold(0.0f32, f32::max);
            if self.fuzziness <= 0.0 {
                if d < 0.5 { 1.0 } else { 0.0 }
            } else {
                (1.0 - d / self.fuzziness).clamp(0.0, 1.0)
            }
        };
        let near = |i: usize| match (&self.localized, at) {
            (Some(localized), Some((x, y))) => localized.points.get(i).map_or(0.0, |&(px, py)| {
                let d = ((x - px).powi(2) + (y - py).powi(2)).sqrt();
                (1.0 - d / localized.radius).clamp(0.0, 1.0) as f32
            }),
            _ => 1.0,
        };
        let included = (0..self.included.len())
            .map(|i| similar(&self.included[i]) * near(i))
            .fold(0.0f32, f32::max);
        let excluded = self.excluded.iter().map(similar).fold(0.0f32, f32::max);
        let value = included * (1.0 - excluded);
        if self.invert { 1.0 - value } else { value }
    }

    /// Whether every point of document area `[x0, y0, x1, y1]` is out of reach of the
    /// localized samples (its coverage is then 0, or 1 inverted).
    fn out_of_reach(&self, [x0, y0, x1, y1]: [f64; 4]) -> bool {
        let Some(localized) = &self.localized else {
            return false;
        };
        localized.points.iter().all(|&(px, py)| {
            let dx = (x0 - px).max(px - x1).max(0.0);
            let dy = (y0 - py).max(py - y1).max(0.0);
            dx * dx + dy * dy >= localized.radius * localized.radius
        })
    }

    /// Whether the settings can be used: fuzziness in range, and Localized with a positive
    /// radius and one finite point per included color.
    fn is_valid(&self) -> bool {
        let fuzziness =
            self.fuzziness.is_finite() && (0.0..=MAX_FUZZINESS).contains(&self.fuzziness);
        let localized = self.localized.as_ref().is_none_or(|l| {
            l.radius.is_finite()
                && l.radius > 0.0
                && l.points.len() == self.included.len()
                && l.points.iter().all(|p| p.0.is_finite() && p.1.is_finite())
        });
        fuzziness && localized
    }
}

/// The colors of `source` at `points` (document pixels), as displayed: whole 8-bit sRGB values,
/// straight alpha. Points outside the canvas are skipped.
pub fn sample_colors(source: &crate::document::Document, points: &[(u32, u32)]) -> Vec<[f32; 4]> {
    let size = source.size();
    let Ok(mask) = Mask::new(size) else {
        return Vec::new();
    };
    let sampler = WandSampler::new(source);
    let mut tiles: HashMap<(usize, usize), Vec<[f32; 4]>> = HashMap::new();
    points
        .iter()
        .filter(|&&(x, y)| x < size.width && y < size.height)
        .map(|&(x, y)| {
            let (col, row) = (x as usize / T, y as usize / T);
            let tile = tiles
                .entry((col, row))
                .or_insert_with(|| sampler.tile(col, row, &mask));
            tile[(y as usize % T) * T + x as usize % T]
        })
        .collect()
}

/// Select > Color Range on `source` (the composited document, or one holding only the layer to
/// sample): every pixel, by its color, as [`ColorRange::coverage`] says. As in Photoshop, a
/// current selection limits it: the result is within it. Tiles are composited and compared on
/// every core.
pub fn color_range(
    source: &crate::document::Document,
    current: Option<&RasterImage>,
    range: &ColorRange,
) -> Result<Option<RasterImage>, SelectionError> {
    if !range.is_valid() {
        return Err(SelectionError::InvalidShape);
    }
    let canvas = source.size();
    let mut mask = Mask::new(canvas)?;
    let sampler = WandSampler::new(source);
    let tiles: Vec<usize> = (0..mask.tiles.len()).collect();
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let per_thread = tiles.len().div_ceil(threads).max(1);
    let (shape, sampler) = (&mask, &sampler);
    let mut done: Vec<(usize, Tile)> = Vec::new();
    std::thread::scope(|scope| {
        let workers: Vec<_> = tiles
            .chunks(per_thread)
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .map(|&index| {
                            let (col, row) = (index % shape.columns, index / shape.columns);
                            let (w, h) = shape.valid(col, row);
                            let (x0, y0) = ((col * T) as f64, (row * T) as f64);
                            // Localized, tiles out of reach are not even composited.
                            if range.out_of_reach([x0, y0, x0 + w as f64, y0 + h as f64]) {
                                let value = if range.invert { FULL } else { 0 };
                                return (index, Tile::Const(value));
                            }
                            let pixels = sampler.tile(col, row, shape);
                            let mut values = vec![0u16; T * T];
                            for y in 0..h {
                                for x in 0..w {
                                    let at = (x0 + x as f64 + 0.5, y0 + y as f64 + 0.5);
                                    let c = range.coverage(pixels[y * T + x], Some(at));
                                    values[y * T + x] = (c * f32::from(FULL)).round() as u16;
                                }
                            }
                            pad(&mut values, w, h);
                            (index, computed_tile(values))
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        for worker in workers {
            // Invariant: comparing colors does not panic.
            done.extend(worker.join().expect("color range worker panicked"));
        }
    });
    for (index, tile) in done {
        mask.tiles[index] = tile;
    }
    match current {
        Some(current) => finish(canvas, Some(current), mask, Combine::Intersect),
        None => Ok(mask.into_image()),
    }
}

// --- AI masks ----------------------------------------------------------------------------------

/// A model's mask (ADR 0025): `side`² logits (positive: inside) stretched over `area` of the
/// canvas (the whole image, or the region the model saw), read bilinearly, inside where
/// positive, its edge softened over about a pixel; nothing outside `area`. Specks and pinholes
/// smaller than [`MIN_LOGIT_REGION`] of the mask are dropped first, as SAM's own
/// post-processing does (an uncertain area otherwise turns into noise and countless ants).
/// Combined with `current` by `combine`. The coarse selection that Refine Edge improves.
pub fn select_logits(
    canvas: Size,
    current: Option<&RasterImage>,
    logits: &[f32],
    side: usize,
    area: Rect,
    combine: Combine,
) -> Result<Option<RasterImage>, SelectionError> {
    if side == 0 || logits.len() != side * side || logits.iter().any(|v| v.is_nan()) {
        return Err(SelectionError::InvalidShape);
    }
    let cleaned = without_specks(logits, side);
    select_scores(canvas, current, &cleaned, side, side, area, combine)
}

/// A mask of `width × height` scores (positive: inside) stretched over `area` of the canvas,
/// read bilinearly, inside where positive, its edge softened over about a pixel; nothing outside
/// `area`. Combined with `current` by `combine`.
pub fn select_scores(
    canvas: Size,
    current: Option<&RasterImage>,
    scores: &[f32],
    width: usize,
    height: usize,
    area: Rect,
    combine: Combine,
) -> Result<Option<RasterImage>, SelectionError> {
    let mask = stretch(canvas, scores, width, height, area, |v| {
        if v > 0.0 { FULL } else { 0 }
    })?;
    finish(canvas, current, soften(&mask), combine)
}

/// A model's mask of `side`² logits over `area`, as [`select_logits`], but kept soft: each
/// pixel's coverage is the model's probability (the logistic of its logit read bilinearly),
/// almost sure values rounded to sure. Thin details a model sees with little confidence
/// (strands of hair) stay partly selected instead of being cut at one half.
pub fn select_logits_soft(
    canvas: Size,
    current: Option<&RasterImage>,
    logits: &[f32],
    side: usize,
    area: Rect,
    combine: Combine,
) -> Result<Option<RasterImage>, SelectionError> {
    let mask = stretch(canvas, logits, side, side, area, |v| {
        let p = 1.0 / (1.0 + (-v).exp());
        if p <= SOFT_FLOOR {
            0
        } else if p >= 1.0 - SOFT_FLOOR {
            FULL
        } else {
            (p * f32::from(FULL)).round() as u16
        }
    })?;
    finish(canvas, current, mask, combine)
}

/// Below this probability a soft mask selects nothing (and above one minus it, everything):
/// the faint noise of a model's output would otherwise cover the canvas.
const SOFT_FLOOR: f32 = 0.01;

/// `width × height` scores stretched over `area` of the canvas, read bilinearly, each pixel's
/// coverage given by `coverage`; nothing outside `area`.
fn stretch(
    canvas: Size,
    scores: &[f32],
    width: usize,
    height: usize,
    area: Rect,
    coverage: impl Fn(f32) -> u16 + Sync,
) -> Result<Mask, SelectionError> {
    if width == 0
        || height == 0
        || scores.len() != width * height
        || scores.iter().any(|v| v.is_nan())
    {
        return Err(SelectionError::InvalidShape);
    }
    let area = area
        .intersection(Rect::new(0, 0, canvas.width, canvas.height))
        .ok_or(SelectionError::InvalidShape)?;
    let mut mask = Mask::new(canvas)?;
    let (sx, sy) = (
        width as f64 / f64::from(area.width),
        height as f64 / f64::from(area.height),
    );
    let (left, top) = (area.x as usize, area.y as usize);
    let (right, bottom) = (left + area.width as usize, top + area.height as usize);
    let at = |x: usize, y: usize| scores[y.min(height - 1) * width + x.min(width - 1)];
    let value = |x: usize, y: usize| -> u16 {
        if x < left || y < top || x >= right || y >= bottom {
            return 0;
        }
        let fx = (((x - left) as f64 + 0.5) * sx - 0.5).max(0.0);
        let fy = (((y - top) as f64 + 0.5) * sy - 0.5).max(0.0);
        let (x0, y0) = (fx as usize, fy as usize);
        let (tx, ty) = ((fx - x0 as f64) as f32, (fy - y0 as f64) as f32);
        let upper = at(x0, y0) * (1.0 - tx) + at(x0 + 1, y0) * tx;
        let lower = at(x0, y0 + 1) * (1.0 - tx) + at(x0 + 1, y0 + 1) * tx;
        coverage(upper * (1.0 - ty) + lower * ty)
    };
    // A tile whose scores (the cells its pixels read) all give the same coverage is that
    // constant, without reading each pixel: most tiles of a large canvas.
    let uniform = |col: usize, row: usize, w: usize, h: usize| -> Option<u16> {
        let (x0, y0) = (col * T, row * T);
        let (x1, y1) = (x0 + w, y0 + h);
        let inside = x0 >= left && y0 >= top && x1 <= right && y1 <= bottom;
        let (cx0, cx1) = (
            (((x0.max(left) - left) as f64 + 0.5) * sx - 0.5).max(0.0) as usize,
            (((x1.min(right) - left) as f64 - 0.5) * sx - 0.5).max(0.0) as usize + 1,
        );
        let (cy0, cy1) = (
            (((y0.max(top) - top) as f64 + 0.5) * sy - 0.5).max(0.0) as usize,
            (((y1.min(bottom) - top) as f64 - 0.5) * sy - 0.5).max(0.0) as usize + 1,
        );
        let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
        for cy in cy0..=cy1.min(height - 1) {
            for cx in cx0..=cx1.min(width - 1) {
                let v = at(cx, cy);
                lo = lo.min(v);
                hi = hi.max(v);
            }
        }
        let v = coverage(lo);
        // Outside the area nothing is selected: a partly covered tile is only uniform at 0.
        (v == coverage(hi) && (inside || v == 0)).then_some(v)
    };
    // The tiles over the area; the others stay empty.
    let tiles: Vec<usize> = (top / T..bottom.div_ceil(T))
        .flat_map(|row| (left / T..right.div_ceil(T)).map(move |col| (col, row)))
        .map(|(col, row)| row * mask.columns + col)
        .collect();
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let per_thread = tiles.len().div_ceil(threads).max(1);
    let (shape, value, uniform) = (&mask, &value, &uniform);
    let mut done: Vec<(usize, Tile)> = Vec::new();
    std::thread::scope(|scope| {
        let workers: Vec<_> = tiles
            .chunks(per_thread)
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .map(|&index| {
                            let (col, row) = (index % shape.columns, index / shape.columns);
                            let (w, h) = shape.valid(col, row);
                            if let Some(v) = uniform(col, row, w, h) {
                                return (index, Tile::Const(v));
                            }
                            let mut values = vec![0u16; T * T];
                            for y in 0..h {
                                for x in 0..w {
                                    values[y * T + x] = value(col * T + x, row * T + y);
                                }
                            }
                            pad(&mut values, w, h);
                            (index, computed_tile(values))
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        for worker in workers {
            // Invariant: reading scores does not panic (indices are clamped).
            done.extend(worker.join().expect("scores worker panicked"));
        }
    });
    for (index, tile) in done {
        mask.tiles[index] = tile;
    }
    Ok(mask)
}

/// A model's mask (`side`² logits, positive inside) as inside or not per cell, without specks
/// (see [`select_logits`]): what a hover shows before anything is selected.
pub fn logits_mask(logits: &[f32], side: usize) -> Vec<bool> {
    without_specks(logits, side)
        .iter()
        .map(|&v| v > 0.0)
        .collect()
}

/// The smallest region of a model's mask kept, as a fraction of the mask.
pub const MIN_LOGIT_REGION: f64 = 1.0 / 1024.0;

/// `logits` (a `side`² mask, positive inside) with every connected region (4-neighbors) of
/// either sign smaller than [`MIN_LOGIT_REGION`] of the mask flipped to its surroundings.
fn without_specks(logits: &[f32], side: usize) -> Vec<f32> {
    let min = ((side * side) as f64 * MIN_LOGIT_REGION).ceil() as usize;
    let mut out = logits.to_vec();
    let mut seen = vec![false; side * side];
    let mut region = Vec::new();
    let mut stack = Vec::new();
    for start in 0..side * side {
        if seen[start] {
            continue;
        }
        let inside = logits[start] > 0.0;
        region.clear();
        stack.push(start);
        seen[start] = true;
        while let Some(i) = stack.pop() {
            region.push(i);
            let (x, y) = (i % side, i / side);
            let neighbors = [
                (x > 0).then(|| i - 1),
                (x + 1 < side).then(|| i + 1),
                (y > 0).then(|| i - side),
                (y + 1 < side).then(|| i + side),
            ];
            for n in neighbors.into_iter().flatten() {
                if !seen[n] && (logits[n] > 0.0) == inside {
                    seen[n] = true;
                    stack.push(n);
                }
            }
        }
        if region.len() < min {
            // Just past zero on the other side: the region takes its surroundings' sign
            // and the edge around it stays smooth once upsampled.
            let flipped = if inside { -1e-3 } else { 1e-3 };
            for &i in &region {
                out[i] = flipped;
            }
        }
    }
    out
}

// --- Refine Edge -------------------------------------------------------------------------------

/// A window of the canvas to matte (Refine Edge, ADR 0025): `rect` (document pixels, within the
/// canvas) is seen at one pixel per `scale`² document pixels; its `inner` part tiles the canvas
/// with the other windows' and the rest, `margin` wide, is shared with its neighbors, where
/// their mattes are blended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EdgeWindow {
    pub rect: Rect,
    pub inner: Rect,
    pub scale: u32,
    pub margin: u32,
}

impl EdgeWindow {
    /// The size of the image (and trimap) the matting model gets for this window.
    pub fn input_size(&self) -> Size {
        Size::new(
            self.rect.width.div_ceil(self.scale),
            self.rect.height.div_ceil(self.scale),
        )
    }
}

/// Refining a selection's edge: the windows along its outline, the band of each to decide, and
/// the selection as the mattes come back.
#[derive(Debug, Clone)]
pub struct EdgeRefinement {
    mask: Mask,
    /// Document pixels around the outline that the model decides.
    band: RefineBand,
    /// Where the model decides as well, whatever the outline (Select and Mask's refine-edge
    /// brush): pixels covered at least half.
    unknown: Option<Mask>,
    windows: Vec<EdgeWindow>,
    /// The mattes of the undecided pixels, by tile: the sum of each window's alpha times its
    /// weight there, and the sum of the weights.
    blend: HashMap<usize, (Vec<f32>, Vec<f32>)>,
}

/// How far from the outline Refine Edge lets the model decide, document pixels: `inward` into
/// the selection, `outward` away from it. Hair and fur reach far outside a coarse mask, rarely
/// far inside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefineBand {
    pub inward: u32,
    pub outward: u32,
}

impl RefineBand {
    /// The same distance on both sides.
    pub fn both(radius: u32) -> Self {
        Self {
            inward: radius,
            outward: radius,
        }
    }
}

/// Coverage at or above which a pixel is surely selected for Refine Edge, and at or below
/// which surely not; the model decides between (a soft mask's thin details).
const SURE_IN: u16 = 64_224; // 98 %
const SURE_OUT: u16 = 1_311; // 2 %

/// A tile's coverage, seen from the outline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    In,
    Out,
    Both,
}

/// Plans Refine Edge on `selection` (`canvas`-sized): windows of at most `side`² model pixels
/// along the outline (where coverage is neither sure in nor sure out, or changes between
/// them), each deciding `band` around it. Long outlines are seen coarser (two, four… document
/// pixels per model pixel) so that there are at most `max_windows`.
pub fn plan_refinement(
    canvas: Size,
    selection: &RasterImage,
    band: RefineBand,
    side: u32,
    max_windows: usize,
) -> Result<EdgeRefinement, SelectionError> {
    plan_refinement_with(canvas, selection, band, None, side, max_windows)
}

/// [`plan_refinement`], the model also deciding wherever `unknown` covers at least half
/// (Select and Mask's refine-edge brush), near the outline or not.
pub fn plan_refinement_with(
    canvas: Size,
    selection: &RasterImage,
    band: RefineBand,
    unknown: Option<&RasterImage>,
    side: u32,
    max_windows: usize,
) -> Result<EdgeRefinement, SelectionError> {
    if side < 64 || band.inward == 0 || band.outward == 0 {
        return Err(SelectionError::InvalidShape);
    }
    let mask = Mask::from_image(canvas, selection)?;
    let unknown = unknown.map(|u| Mask::from_image(canvas, u)).transpose()?;
    // The tiles where the brush asks the model to decide.
    let forced: Vec<bool> = match &unknown {
        Some(u) => u
            .tiles
            .iter()
            .map(|tile| match tile {
                Tile::Const(v) => *v >= HALF,
                other => other.values().iter().any(|&v| v >= HALF),
            })
            .collect(),
        None => vec![false; mask.tiles.len()],
    };
    let forced_in = |inner: Rect| {
        let t = TILE_SIZE;
        (inner.y / t..=(inner.y + inner.height - 1) / t).any(|row| {
            (inner.x / t..=(inner.x + inner.width - 1) / t)
                .any(|col| forced[row as usize * mask.columns + col as usize])
        })
    };
    let side_of = |v: u16| {
        if v >= SURE_IN {
            Side::In
        } else if v <= SURE_OUT {
            Side::Out
        } else {
            Side::Both
        }
    };
    let sides: Vec<Side> = mask
        .tiles
        .iter()
        .map(|tile| match tile {
            Tile::Const(v) => side_of(*v),
            other => {
                let values = other.values();
                let first = side_of(values[0]);
                if values.iter().all(|&v| side_of(v) == first) {
                    first
                } else {
                    Side::Both
                }
            }
        })
        .collect();
    let reach = band.inward.max(band.outward);
    let mut scale = 1u32;
    loop {
        // Windows of `side` model pixels; their inner parts tile the canvas, the rest (an
        // eighth on each side) is context.
        let margin = side / 8 * scale;
        let cell = (side * scale).saturating_sub(2 * margin).max(1);
        let mut windows = Vec::new();
        for y in (0..canvas.height).step_by(cell as usize) {
            for x in (0..canvas.width).step_by(cell as usize) {
                let inner = Rect::new(
                    x,
                    y,
                    cell.min(canvas.width - x),
                    cell.min(canvas.height - y),
                );
                if crosses(&mask, &sides, inner, reach) || forced_in(inner) {
                    let x0 = x.saturating_sub(margin);
                    let y0 = y.saturating_sub(margin);
                    let rect = Rect::new(
                        x0,
                        y0,
                        (x + inner.width + margin).min(canvas.width) - x0,
                        (y + inner.height + margin).min(canvas.height) - y0,
                    );
                    windows.push(EdgeWindow {
                        rect,
                        inner,
                        scale,
                        margin,
                    });
                }
            }
        }
        if windows.len() <= max_windows || cell >= canvas.width.max(canvas.height) {
            return Ok(EdgeRefinement {
                mask,
                band,
                unknown,
                windows,
                blend: HashMap::new(),
            });
        }
        scale *= 2;
    }
}

/// Whether the contour passes within `band` of `inner`: the tiles there are not all on one side.
fn crosses(mask: &Mask, sides: &[Side], inner: Rect, band: u32) -> bool {
    let t = TILE_SIZE;
    let col0 = inner.x.saturating_sub(band) / t;
    let row0 = inner.y.saturating_sub(band) / t;
    let col1 = ((inner.x + inner.width + band).min(mask.size.width) - 1) / t;
    let row1 = ((inner.y + inner.height + band).min(mask.size.height) - 1) / t;
    let mut seen = None;
    for row in row0..=row1 {
        for col in col0..=col1 {
            match (sides[row as usize * mask.columns + col as usize], seen) {
                (Side::Both, _) => return true,
                (s, Some(previous)) if s != previous => return true,
                (s, _) => seen = Some(s),
            }
        }
    }
    false
}

/// The trimap value the model decides.
pub const TRIMAP_UNKNOWN: u8 = 128;

impl EdgeRefinement {
    pub fn windows(&self) -> &[EdgeWindow] {
        &self.windows
    }

    /// The trimap of `window`, [`EdgeWindow::input_size`] pixels: 255 surely inside, 0 surely
    /// outside, [`TRIMAP_UNKNOWN`] where the model decides: pixels neither sure in nor sure out,
    /// sure ones within the band's inward distance of a pixel not sure in, and sure outside ones
    /// within its outward distance of a pixel not sure out.
    pub fn trimap(&self, window: &EdgeWindow) -> Vec<u8> {
        let size = window.input_size();
        let (w, h) = (size.width as usize, size.height as usize);
        let s = i64::from(window.scale);
        let coverage: Vec<u16> = (0..w * h)
            .map(|i| {
                let x = i64::from(window.rect.x) + (i % w) as i64 * s + s / 2;
                let y = i64::from(window.rect.y) + (i / w) as i64 * s + s / 2;
                self.mask.get(x, y)
            })
            .collect();
        // Summed counts of the pixels not sure in, and not sure out.
        let summed = |count: &dyn Fn(u16) -> bool| {
            let mut sat = vec![0u32; (w + 1) * (h + 1)];
            for y in 0..h {
                let mut row = 0;
                for x in 0..w {
                    row += u32::from(count(coverage[y * w + x]));
                    sat[(y + 1) * (w + 1) + x + 1] = sat[y * (w + 1) + x + 1] + row;
                }
            }
            sat
        };
        let not_in = summed(&|v| v < SURE_IN);
        let not_out = summed(&|v| v > SURE_OUT);
        let near = |sat: &[u32], x: usize, y: usize, distance: u32| {
            let r = distance.div_ceil(window.scale).max(1) as usize;
            let (xa, xb) = (x.saturating_sub(r), (x + r + 1).min(w));
            let (ya, yb) = (y.saturating_sub(r), (y + r + 1).min(h));
            sat[yb * (w + 1) + xb] + sat[ya * (w + 1) + xa]
                - sat[ya * (w + 1) + xb]
                - sat[yb * (w + 1) + xa]
                > 0
        };
        let forced = |x: usize, y: usize| {
            self.unknown.as_ref().is_some_and(|u| {
                let dx = i64::from(window.rect.x) + x as i64 * s + s / 2;
                let dy = i64::from(window.rect.y) + y as i64 * s + s / 2;
                u.get(dx, dy) >= HALF
            })
        };
        (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                let v = coverage[i];
                if forced(x, y) {
                    TRIMAP_UNKNOWN
                } else if v >= SURE_IN {
                    if near(&not_in, x, y, self.band.inward) {
                        TRIMAP_UNKNOWN
                    } else {
                        255
                    }
                } else if v <= SURE_OUT {
                    if near(&not_out, x, y, self.band.outward) {
                        TRIMAP_UNKNOWN
                    } else {
                        0
                    }
                } else {
                    TRIMAP_UNKNOWN
                }
            })
            .collect()
    }

    /// Takes the model's matte of `window` (`alpha`, 16-bit, [`EdgeWindow::input_size`]
    /// pixels, read bilinearly when the window is seen coarser). Where `trimap` left a pixel
    /// undecided, the matte is blended with the other windows' that cover it, each weighing less
    /// toward its edges (but those on the canvas's edge), so that neighbors meet without a seam;
    /// elsewhere in its inner part, the trimap's side, sharp.
    pub fn apply(&mut self, window: &EdgeWindow, trimap: &[u8], alpha: &[u16]) {
        let size = window.input_size();
        let (w, h) = (size.width as usize, size.height as usize);
        if trimap.len() != w * h || alpha.len() != w * h {
            return;
        }
        let s = f64::from(window.scale);
        let sample = |values: &dyn Fn(usize) -> f64, fx: f64, fy: f64| -> f64 {
            let fx = fx.clamp(0.0, (w - 1) as f64);
            let fy = fy.clamp(0.0, (h - 1) as f64);
            let (x0, y0) = (fx as usize, fy as usize);
            let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
            let (tx, ty) = (fx - x0 as f64, fy - y0 as f64);
            let top = values(y0 * w + x0) * (1.0 - tx) + values(y0 * w + x1) * tx;
            let bottom = values(y1 * w + x0) * (1.0 - tx) + values(y1 * w + x1) * tx;
            top * (1.0 - ty) + bottom * ty
        };
        let alpha_at = |i: usize| f64::from(alpha[i]);
        let unknown_at = |i: usize| f64::from(u8::from(trimap[i] == TRIMAP_UNKNOWN));
        let inside_at = |i: usize| f64::from(u8::from(trimap[i] == 255));
        let (rect, inner, canvas) = (window.rect, window.inner, self.mask.size);
        // Rising from the window's edges over its margin, except along the canvas's edges
        // (no neighbor shares them).
        let ramp = f64::from(window.margin.max(1));
        let weight = |dx: u32, dy: u32| -> f64 {
            let mut d = f64::INFINITY;
            if rect.x > 0 {
                d = d.min(f64::from(dx - rect.x) + 0.5);
            }
            if rect.y > 0 {
                d = d.min(f64::from(dy - rect.y) + 0.5);
            }
            if rect.x + rect.width < canvas.width {
                d = d.min(f64::from(rect.x + rect.width - dx) - 0.5);
            }
            if rect.y + rect.height < canvas.height {
                d = d.min(f64::from(rect.y + rect.height - dy) - 0.5);
            }
            (d / ramp).clamp(1e-3, 1.0)
        };
        for row in rect.y / TILE_SIZE..(rect.y + rect.height).div_ceil(TILE_SIZE) {
            for col in rect.x / TILE_SIZE..(rect.x + rect.width).div_ceil(TILE_SIZE) {
                let index = row as usize * self.mask.columns + col as usize;
                let (vw, vh) = self.mask.valid(col as usize, row as usize);
                let (tx0, ty0) = (col * TILE_SIZE, row * TILE_SIZE);
                let mut sharp: Option<Vec<u16>> = None;
                for y in 0..vh {
                    let dy = ty0 + y as u32;
                    if dy < rect.y || dy >= rect.y + rect.height {
                        continue;
                    }
                    for x in 0..vw {
                        let dx = tx0 + x as u32;
                        if dx < rect.x || dx >= rect.x + rect.width {
                            continue;
                        }
                        // The window pixel whose center is this document pixel's center.
                        let fx = (f64::from(dx - rect.x) + 0.5) / s - 0.5;
                        let fy = (f64::from(dy - rect.y) + 0.5) / s - 0.5;
                        if sample(&unknown_at, fx, fy) > 0.0 {
                            let wt = weight(dx, dy);
                            let (sums, weights) = self
                                .blend
                                .entry(index)
                                .or_insert_with(|| (vec![0.0; T * T], vec![0.0; T * T]));
                            sums[y * T + x] += (sample(&alpha_at, fx, fy) * wt) as f32;
                            weights[y * T + x] += wt as f32;
                        } else if dx >= inner.x
                            && dy >= inner.y
                            && dx < inner.x + inner.width
                            && dy < inner.y + inner.height
                        {
                            let v = if sample(&inside_at, fx, fy) >= 0.5 {
                                FULL
                            } else {
                                0
                            };
                            let tile = &self.mask.tiles[index];
                            sharp.get_or_insert_with(|| tile.values().into_owned())[y * T + x] = v;
                        }
                    }
                }
                if let Some(mut values) = sharp {
                    pad(&mut values, vw, vh);
                    self.mask.tiles[index] = Tile::Data(values);
                }
            }
        }
    }

    /// The refined selection (the blended mattes written), combined with `current` by
    /// `combine`.
    pub fn finish(
        mut self,
        current: Option<&RasterImage>,
        combine: Combine,
    ) -> Result<Option<RasterImage>, SelectionError> {
        for (index, (sums, weights)) in std::mem::take(&mut self.blend) {
            let (vw, vh) = self
                .mask
                .valid(index % self.mask.columns, index / self.mask.columns);
            let mut values = self.mask.tiles[index].values().into_owned();
            for y in 0..vh {
                for x in 0..vw {
                    let wt = weights[y * T + x];
                    if wt > 0.0 {
                        let v = sums[y * T + x] / wt;
                        values[y * T + x] = v.round().clamp(0.0, f32::from(FULL)) as u16;
                    }
                }
            }
            pad(&mut values, vw, vh);
            self.mask.tiles[index] = computed_tile(values);
        }
        finish(self.mask.size, current, self.mask, combine)
    }
}

// --- Rasterization -----------------------------------------------------------------------------

/// The coverage of a closed polygon on a canvas: each pixel gets the exact fraction of its area
/// inside (nonzero rule), or 0 or 1 by that fraction without anti-aliasing. Computed one band of
/// tile rows at a time, only over the polygon's bounds, on every core.
fn rasterize(canvas: Size, polygon: &[[f64; 2]], anti_alias: bool) -> Result<Mask, SelectionError> {
    let mut mask = Mask::new(canvas)?;
    if polygon.len() < 3 {
        return Ok(mask);
    }
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for p in polygon {
        min_x = min_x.min(p[0]);
        min_y = min_y.min(p[1]);
        max_x = max_x.max(p[0]);
        max_y = max_y.max(p[1]);
    }
    let (w, h) = (f64::from(canvas.width), f64::from(canvas.height));
    let cx0 = min_x.floor().clamp(0.0, w) as usize;
    let cx1 = max_x.ceil().clamp(0.0, w) as usize;
    let cy0 = min_y.floor().clamp(0.0, h) as usize;
    let cy1 = max_y.ceil().clamp(0.0, h) as usize;
    if cx0 >= cx1 || cy0 >= cy1 {
        return Ok(mask);
    }
    let segments: Vec<([f64; 2], [f64; 2])> = (0..polygon.len())
        .map(|i| (polygon[i], polygon[(i + 1) % polygon.len()]))
        .filter(|(a, b)| a[1] != b[1])
        .collect();
    let bands: Vec<usize> = (cy0 / T..cy1.div_ceil(T)).collect();
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let per_thread = bands.len().div_ceil(threads).max(1);
    let columns = mask.columns;
    let mut done: Vec<(usize, Vec<Tile>)> = Vec::new();
    let mask_ref = &mask;
    std::thread::scope(|scope| {
        let workers: Vec<_> = bands
            .chunks(per_thread)
            .map(|chunk| {
                let segments = &segments;
                scope.spawn(move || {
                    let mut out = Vec::new();
                    let mut acc = Vec::new();
                    for &row in chunk {
                        let tiles = rasterize_band(
                            mask_ref, row, cx0, cx1, cy0, cy1, segments, anti_alias, &mut acc,
                        );
                        out.push((row, tiles));
                    }
                    out
                })
            })
            .collect();
        for worker in workers {
            // Invariant: the band rasterizer does not panic (indices are clamped).
            done.extend(worker.join().expect("selection rasterizer panicked"));
        }
    });
    for (row, tiles) in done {
        for (col, tile) in tiles.into_iter().enumerate() {
            mask.tiles[row * columns + col] = tile;
        }
    }
    Ok(mask)
}

/// One tile row of [`rasterize`]: its tiles, left to right.
#[allow(clippy::too_many_arguments)]
fn rasterize_band(
    mask: &Mask,
    row: usize,
    cx0: usize,
    cx1: usize,
    cy0: usize,
    cy1: usize,
    segments: &[([f64; 2], [f64; 2])],
    anti_alias: bool,
    acc: &mut Vec<f32>,
) -> Vec<Tile> {
    let band_y0 = (row * T).max(cy0);
    let band_y1 = ((row + 1) * T).min(cy1);
    let bw = cx1 - cx0;
    let bh = band_y1 - band_y0;
    let stride = bw + 2;
    acc.clear();
    acc.resize(stride * bh, 0.0);
    let (ox, oy) = (cx0 as f64, band_y0 as f64);
    for &(a, b) in segments {
        let a = [a[0] - ox, a[1] - oy];
        let b = [b[0] - ox, b[1] - oy];
        accumulate_clipped(acc, stride, bw as f64, bh as f64, a, b);
    }
    let first_col = cx0 / T;
    let last_col = (cx1 - 1) / T;
    let mut tiles: Vec<Tile> = (0..mask.columns)
        .map(|col| {
            if (first_col..=last_col).contains(&col) {
                Tile::Data(vec![0u16; T * T])
            } else {
                Tile::Const(0)
            }
        })
        .collect();
    // Prefix sums row by row, written into the tiles.
    for y in 0..bh {
        let line = &acc[y * stride..(y + 1) * stride];
        let mut sum = 0.0f32;
        let ty = band_y0 + y - row * T;
        for (x, value) in line.iter().take(bw).enumerate() {
            sum += value;
            let coverage = sum.abs().min(1.0);
            let coverage = if anti_alias {
                coverage
            } else if coverage >= 0.5 {
                1.0
            } else {
                0.0
            };
            let gx = cx0 + x;
            if let Tile::Data(values) = &mut tiles[gx / T] {
                values[ty * T + gx % T] = (coverage * f32::from(FULL)).round() as u16;
            }
        }
    }
    for (col, tile) in tiles
        .iter_mut()
        .enumerate()
        .take(last_col + 1)
        .skip(first_col)
    {
        if let Tile::Data(values) = tile {
            let (w, h) = mask.valid(col, row);
            pad(values, w, h);
        }
    }
    tiles
}

/// Accumulate segment `a`–`b` (band coordinates) after clipping it to the band: to rows
/// `[0, h]` by cutting, to columns `[0, w]` by moving what is left of 0 onto 0 (it still covers
/// everything to its right) and what is right of `w` onto `w` (it covers nothing visible).
fn accumulate_clipped(acc: &mut [f32], stride: usize, w: f64, h: f64, a: [f64; 2], b: [f64; 2]) {
    let (mut a, mut b) = (a, b);
    // Rows.
    if (a[1] <= 0.0 && b[1] <= 0.0) || (a[1] >= h && b[1] >= h) {
        return;
    }
    let cut_y = |p: [f64; 2], q: [f64; 2], y: f64| {
        let t = (y - p[1]) / (q[1] - p[1]);
        [p[0] + (q[0] - p[0]) * t, y]
    };
    if a[1] < 0.0 {
        a = cut_y(a, b, 0.0);
    } else if a[1] > h {
        a = cut_y(a, b, h);
    }
    if b[1] < 0.0 {
        b = cut_y(a, b, 0.0);
    } else if b[1] > h {
        b = cut_y(a, b, h);
    }
    // Columns: split where the segment crosses x = 0 and x = w.
    let mut cuts = vec![0.0f64, 1.0];
    for edge in [0.0, w] {
        let (da, db) = (a[0] - edge, b[0] - edge);
        if (da < 0.0 && db > 0.0) || (da > 0.0 && db < 0.0) {
            cuts.push(da / (da - db));
        }
    }
    cuts.sort_by(f64::total_cmp);
    let lerp = |t: f64| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
    for pair in cuts.windows(2) {
        let (p, q) = (lerp(pair[0]), lerp(pair[1]));
        let p = [p[0].clamp(0.0, w), p[1]];
        let q = [q[0].clamp(0.0, w), q[1]];
        accumulate(acc, stride, h, p, q);
    }
}

/// Signed area accumulation of one segment inside the band (the method of font-rs and
/// stb_truetype): after a prefix sum along each row, every pixel holds the signed fraction of
/// its area covered by the polygon.
fn accumulate(acc: &mut [f32], stride: usize, h: f64, p0: [f64; 2], p1: [f64; 2]) {
    if p0[1] == p1[1] {
        return;
    }
    let (dir, a, b) = if p0[1] < p1[1] {
        (1.0, p0, p1)
    } else {
        (-1.0, p1, p0)
    };
    let dxdy = (b[0] - a[0]) / (b[1] - a[1]);
    let mut x = a[0];
    let y_start = a[1].floor().max(0.0) as usize;
    let y_end = (b[1].ceil().min(h)) as usize;
    for y in y_start..y_end {
        let dy = (y as f64 + 1.0).min(b[1]) - (y as f64).max(a[1]);
        if dy <= 0.0 {
            continue;
        }
        let x_next = x + dxdy * dy;
        let d = (dy * dir) as f32;
        let (xa, xb) = if x < x_next { (x, x_next) } else { (x_next, x) };
        let xa_floor = xa.floor();
        let xa_i = xa_floor as usize;
        let xb_ceil = xb.ceil();
        let xb_i = (xb_ceil as usize).max(xa_i + 1);
        let row = &mut acc[y * stride..(y + 1) * stride];
        if xb_i <= xa_i + 1 {
            // Within one pixel column: split by the mean x.
            let xm = (0.5 * (x + x_next) - xa_floor) as f32;
            row[xa_i] += d * (1.0 - xm);
            row[xa_i + 1] += d * xm;
        } else {
            let s = (1.0 / (xb - xa)) as f32;
            let x0f = (xa - xa_floor) as f32;
            let a0 = 0.5 * s * (1.0 - x0f) * (1.0 - x0f);
            let x1f = (xb - xb_ceil + 1.0) as f32;
            let am = 0.5 * s * x1f * x1f;
            row[xa_i] += d * a0;
            if xb_i == xa_i + 2 {
                row[xa_i + 1] += d * (1.0 - a0 - am);
            } else {
                let a1 = s * (1.5 - x0f);
                row[xa_i + 1] += d * (a1 - a0);
                for value in &mut row[xa_i + 2..xb_i - 1] {
                    *value += d * s;
                }
                let a2 = a1 + (xb_i - xa_i - 3) as f32 * s;
                row[xb_i - 1] += d * (1.0 - a2 - am);
            }
            row[xb_i] += d * am;
        }
        x = x_next;
    }
}

// --- Feather -----------------------------------------------------------------------------------

/// Gaussian softening (standard deviation `sigma`, approximated by three box blurs) of the
/// tiles near an edge; tiles whose whole neighborhood is uniform stay as they are. The canvas
/// edge repeats, so a selection reaching it is not faded there.
fn feather(mask: &Mask, sigma: f64) -> Mask {
    let radii = box_radii(sigma);
    let halo: usize = radii.iter().sum();
    let reach = halo.div_ceil(T);
    let mut out = mask.clone();
    let todo: Vec<usize> = (0..mask.tiles.len())
        .filter(|&index| {
            let (col, row) = (index % mask.columns, index / mask.columns);
            let mut seen: Option<u16> = None;
            for r in row.saturating_sub(reach)..=(row + reach).min(mask.rows - 1) {
                for c in col.saturating_sub(reach)..=(col + reach).min(mask.columns - 1) {
                    match (mask.tiles[r * mask.columns + c].constant(), seen) {
                        (None, _) => return true,
                        (Some(v), None) => seen = Some(v),
                        (Some(v), Some(s)) if v != s => return true,
                        _ => {}
                    }
                }
            }
            false
        })
        .collect();
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let per_thread = todo.len().div_ceil(threads).max(1);
    let mut blurred: Vec<(usize, Vec<u16>)> = Vec::new();
    std::thread::scope(|scope| {
        let workers: Vec<_> = todo
            .chunks(per_thread)
            .map(|chunk| {
                let radii = &radii;
                scope.spawn(move || {
                    chunk
                        .iter()
                        .map(|&index| (index, blur_tile(mask, index, radii, halo)))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        for worker in workers {
            // Invariant: the blur does not panic (reads are clamped to the canvas).
            blurred.extend(worker.join().expect("feather worker panicked"));
        }
    });
    for (index, values) in blurred {
        out.tiles[index] = Tile::Data(values);
    }
    out
}

/// The radii of three box blurs whose succession approximates a Gaussian of `sigma`.
fn box_radii(sigma: f64) -> [usize; 3] {
    let n = 3.0;
    let ideal = (12.0 * sigma * sigma / n + 1.0).sqrt();
    let mut lower = ideal.floor() as i64;
    if lower % 2 == 0 {
        lower -= 1;
    }
    let lower = lower.max(1);
    let upper = lower + 2;
    let l = lower as f64;
    let m = ((12.0 * sigma * sigma - n * l * l - 4.0 * n * l - 3.0 * n) / (-4.0 * l - 4.0)).round();
    let mut radii = [0usize; 3];
    for (i, radius) in radii.iter_mut().enumerate() {
        let size = if (i as f64) < m { lower } else { upper };
        *radius = ((size - 1) / 2) as usize;
    }
    radii
}

fn blur_tile(mask: &Mask, index: usize, radii: &[usize; 3], halo: usize) -> Vec<u16> {
    let (col, row) = (index % mask.columns, index / mask.columns);
    let side = T + 2 * halo;
    let (x0, y0) = (
        (col * T) as isize - halo as isize,
        (row * T) as isize - halo as isize,
    );
    let mut a: Vec<f32> = (0..side * side)
        .map(|i| f32::from(mask.at(x0 + (i % side) as isize, y0 + (i / side) as isize)))
        .collect();
    let mut b = vec![0.0f32; side * side];
    for &r in radii {
        box_pass(&a, &mut b, side, r, true);
        box_pass(&b, &mut a, side, r, false);
    }
    let mut values = vec![0u16; T * T];
    for y in 0..T {
        for x in 0..T {
            let v = a[(y + halo) * side + x + halo];
            values[y * T + x] = v.round().clamp(0.0, f32::from(FULL)) as u16;
        }
    }
    let (w, h) = mask.valid(col, row);
    pad(&mut values, w, h);
    values
}

/// One box blur of radius `r` along rows (`horizontal`) or columns, with running sums; the
/// window's own edge repeats (only its center is kept).
fn box_pass(src: &[f32], dst: &mut [f32], side: usize, r: usize, horizontal: bool) {
    if r == 0 {
        dst.copy_from_slice(src);
        return;
    }
    let width = (2 * r + 1) as f32;
    let at = |line: usize, i: usize| {
        if horizontal {
            line * side + i
        } else {
            i * side + line
        }
    };
    for line in 0..side {
        let read = |i: isize| src[at(line, i.clamp(0, side as isize - 1) as usize)];
        let mut sum: f32 = (-(r as isize)..=r as isize).map(read).sum();
        for i in 0..side {
            dst[at(line, i)] = sum / width;
            sum += read(i as isize + r as isize + 1) - read(i as isize - r as isize);
        }
    }
}

// --- Combining ---------------------------------------------------------------------------------

/// `current` combined with `new`, tile by tile; tiles that do not change stay shared.
fn combine_masks(current: &Mask, new: &Mask, combine: Combine) -> Mask {
    let mut out = current.clone();
    for (index, tile) in out.tiles.iter_mut().enumerate() {
        let shape = &new.tiles[index];
        *tile = match (combine, shape.constant()) {
            (Combine::Replace, _) => shape.clone(),
            (Combine::Add | Combine::Subtract, Some(0)) | (Combine::Intersect, Some(FULL)) => {
                continue;
            }
            (Combine::Add, Some(FULL)) => Tile::Const(FULL),
            (Combine::Subtract, Some(FULL)) | (Combine::Intersect, Some(0)) => Tile::Const(0),
            _ => {
                let (a, b) = (tile.values(), shape.values());
                let op: fn(u16, u16) -> u16 = match combine {
                    Combine::Add => u16::max,
                    Combine::Subtract => |a, b| a.min(FULL - b),
                    Combine::Intersect => u16::min,
                    Combine::Replace => |_, b| b,
                };
                Tile::Data(a.iter().zip(b.iter()).map(|(&a, &b)| op(a, b)).collect())
            }
        };
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The selected area in pixels (the sum of coverage), read tile by tile.
    fn coverage(image: &RasterImage) -> f64 {
        let size = image.size();
        let level = &image.levels()[0];
        let mut sum = 0.0;
        for row in 0..level.grid().rows() {
            for col in 0..level.grid().columns() {
                let values = decode(level.tile(TileCoord { col, row }).unwrap());
                let w = (size.width - col * TILE_SIZE).min(TILE_SIZE);
                let h = (size.height - row * TILE_SIZE).min(TILE_SIZE);
                for y in 0..h {
                    for x in 0..w {
                        sum += f64::from(values[(y * TILE_SIZE + x) as usize]);
                    }
                }
            }
        }
        sum / f64::from(FULL)
    }

    fn rect(left: f64, top: f64, right: f64, bottom: f64) -> Shape {
        Shape::Rectangle {
            left,
            top,
            right,
            bottom,
        }
    }

    fn select(canvas: Size, shape: &Shape) -> RasterImage {
        select_shape(
            canvas,
            None,
            shape,
            EdgeOptions::default(),
            Combine::Replace,
        )
        .unwrap()
        .unwrap()
    }

    #[test]
    fn a_whole_pixel_rectangle_selects_exactly_its_pixels() {
        let canvas = Size::new(600, 300);
        let image = select(canvas, &rect(10.0, 20.0, 310.0, 280.0));
        assert_eq!(image.format(), SELECTION_FORMAT);
        assert_eq!(image.gray_at(10, 20), 1.0);
        assert_eq!(image.gray_at(309, 279), 1.0);
        assert_eq!(image.gray_at(9, 20), 0.0);
        assert_eq!(image.gray_at(310, 20), 0.0);
        assert_eq!(image.gray_at(10, 280), 0.0);
        assert_eq!(bounds(&image), Some(Rect::new(10, 20, 300, 260)));
        assert!((coverage(&image) - 300.0 * 260.0).abs() < 0.5);
    }

    #[test]
    fn fractional_edges_get_their_exact_coverage() {
        let image = select(Size::new(10, 10), &rect(1.25, 2.0, 3.5, 4.0));
        assert!((image.gray_at(1, 2) - 0.75).abs() < 1e-3);
        assert_eq!(image.gray_at(2, 3), 1.0);
        assert!((image.gray_at(3, 2) - 0.5).abs() < 1e-3);
        assert!((coverage(&image) - 2.25 * 2.0).abs() < 1e-2);
    }

    #[test]
    fn without_anti_aliasing_pixels_are_in_or_out() {
        let edges = EdgeOptions {
            anti_alias: false,
            feather: 0.0,
        };
        let shape = Shape::Ellipse {
            left: 3.3,
            top: 2.7,
            right: 90.1,
            bottom: 60.6,
        };
        let image = select_shape(Size::new(100, 70), None, &shape, edges, Combine::Replace)
            .unwrap()
            .unwrap();
        for y in 0..70 {
            for x in 0..100 {
                let v = image.gray_at(x, y);
                assert!(v == 0.0 || v == 1.0, "({x}, {y}): {v}");
            }
        }
    }

    #[test]
    fn an_ellipse_covers_pi_a_b() {
        let shape = Shape::Ellipse {
            left: 100.5,
            top: 40.0,
            right: 700.5,
            bottom: 440.0,
        };
        let image = select(Size::new(800, 500), &shape);
        let expected = std::f64::consts::PI * 300.0 * 200.0;
        assert!((coverage(&image) - expected).abs() / expected < 1e-4);
        assert_eq!(image.gray_at(400, 240), 1.0);
        assert_eq!(image.gray_at(101, 41), 0.0);
    }

    #[test]
    fn a_polygon_covers_its_area_both_ways_round_and_across_tiles() {
        // A triangle over several tiles, its area 0.5 · 900 · 600.
        let points = vec![[10.0, 10.0], [910.0, 10.0], [10.0, 610.0]];
        let clockwise = select(
            Size::new(1000, 700),
            &Shape::Polygon {
                points: points.clone(),
            },
        );
        let mut reversed = points;
        reversed.reverse();
        let counter = select(Size::new(1000, 700), &Shape::Polygon { points: reversed });
        let expected = 0.5 * 900.0 * 600.0;
        assert!((coverage(&clockwise) - expected).abs() < 1.0);
        assert!((coverage(&counter) - expected).abs() < 1.0);
    }

    #[test]
    fn both_loops_of_a_figure_eight_are_selected() {
        let points = vec![[0.0, 0.0], [10.0, 10.0], [10.0, 0.0], [0.0, 10.0]];
        let image = select(Size::new(20, 20), &Shape::Polygon { points });
        assert_eq!(image.gray_at(1, 5), 1.0);
        assert_eq!(image.gray_at(8, 5), 1.0);
        assert!((coverage(&image) - 50.0).abs() < 1e-2);
    }

    #[test]
    fn shapes_past_the_canvas_are_cut_at_its_edges() {
        let canvas = Size::new(300, 200);
        let image = select(canvas, &rect(-50.0, -20.5, 120.0, 500.0));
        assert_eq!(bounds(&image), Some(Rect::new(0, 0, 120, 200)));
        assert!((coverage(&image) - 120.0 * 200.0).abs() < 0.5);
        assert!(
            select_shape(
                canvas,
                None,
                &rect(400.0, 0.0, 500.0, 10.0),
                EdgeOptions::default(),
                Combine::Replace
            )
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn whole_tiles_share_one_allocation() {
        let canvas = Size::new(2048, 1024);
        let image = select(canvas, &rect(0.0, 0.0, 1536.0, 1024.0));
        let tiles = image.levels()[0].tiles();
        assert!(Arc::ptr_eq(&tiles[0], &tiles[1]));
        assert!(Arc::ptr_eq(&tiles[0], &tiles[9]));
        assert!(Arc::ptr_eq(&tiles[6], &tiles[7]));
        assert!(!Arc::ptr_eq(&tiles[0], &tiles[7]));
    }

    #[test]
    fn combine_modes() {
        let canvas = Size::new(100, 100);
        let edges = EdgeOptions::default();
        let left = select(canvas, &rect(0.0, 0.0, 60.0, 100.0));
        let right = rect(40.0, 0.0, 100.0, 100.0);
        let with = |combine| {
            select_shape(canvas, Some(&left), &right, edges, combine)
                .unwrap()
                .map_or(0.0, |image| coverage(&image))
        };
        assert!((with(Combine::Replace) - 6000.0).abs() < 0.5);
        assert!((with(Combine::Add) - 10000.0).abs() < 0.5);
        assert!((with(Combine::Subtract) - 4000.0).abs() < 0.5);
        assert!((with(Combine::Intersect) - 2000.0).abs() < 0.5);
        let everything = rect(0.0, 0.0, 100.0, 100.0);
        assert!(
            select_shape(canvas, Some(&left), &everything, edges, Combine::Subtract)
                .unwrap()
                .is_none()
        );
        assert!(
            select_shape(canvas, None, &everything, edges, Combine::Intersect)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn unchanged_tiles_of_the_current_selection_stay_shared() {
        let canvas = Size::new(1024, 256);
        let current = select(canvas, &rect(10.0, 10.0, 200.0, 200.0));
        let added = select_shape(
            canvas,
            Some(&current),
            &rect(800.0, 10.0, 900.0, 100.0),
            EdgeOptions::default(),
            Combine::Add,
        )
        .unwrap()
        .unwrap();
        assert!(Arc::ptr_eq(
            &current.levels()[0].tiles()[0],
            &added.levels()[0].tiles()[0]
        ));
    }

    #[test]
    fn invert_and_select_all() {
        let canvas = Size::new(300, 300);
        let all = select_all(canvas).unwrap();
        assert!((coverage(&all) - 90000.0).abs() < 0.5);
        assert!(invert(canvas, Some(&all)).unwrap().is_none());
        let part = select(canvas, &rect(0.0, 0.0, 100.0, 300.0));
        let inverse = invert(canvas, Some(&part)).unwrap().unwrap();
        assert!((coverage(&inverse) - 60000.0).abs() < 0.5);
        assert_eq!(bounds(&inverse), Some(Rect::new(100, 0, 200, 300)));
        assert!(invert(canvas, None).unwrap().is_some());
    }

    #[test]
    fn feather_softens_the_edge_and_keeps_the_area() {
        let canvas = Size::new(600, 600);
        let shape = rect(150.0, 150.0, 450.0, 450.0);
        let edges = EdgeOptions {
            anti_alias: true,
            feather: 10.0,
        };
        let image = select_shape(canvas, None, &shape, edges, Combine::Replace)
            .unwrap()
            .unwrap();
        // About half at the edge, nothing far outside, everything deep inside.
        assert!((image.gray_at(150, 300) - 0.5).abs() < 0.05);
        assert!(image.gray_at(100, 300) < 0.01);
        assert_eq!(image.gray_at(300, 300), 1.0);
        assert!((coverage(&image) - 90000.0).abs() / 90000.0 < 0.01);
        // A canvas filled up to its edges stays filled there.
        let all = select_shape(
            canvas,
            None,
            &rect(0.0, 0.0, 600.0, 600.0),
            edges,
            Combine::Replace,
        )
        .unwrap()
        .unwrap();
        assert_eq!(all.gray_at(0, 0), 1.0);
    }

    #[test]
    fn invalid_shapes_are_errors() {
        let canvas = Size::new(10, 10);
        let bad = rect(f64::NAN, 0.0, 1.0, 1.0);
        assert_eq!(
            select_shape(canvas, None, &bad, EdgeOptions::default(), Combine::Replace).err(),
            Some(SelectionError::InvalidShape)
        );
        let edges = EdgeOptions {
            anti_alias: true,
            feather: MAX_FEATHER + 1.0,
        };
        assert!(
            select_shape(
                canvas,
                None,
                &rect(0.0, 0.0, 5.0, 5.0),
                edges,
                Combine::Replace
            )
            .is_err()
        );
        assert_eq!(
            select_all(Size::new(0, 5)).err(),
            Some(SelectionError::EmptyCanvas)
        );
    }

    #[test]
    fn selecting_is_an_undoable_edit() {
        use crate::document::Document;
        use crate::edit::Edit;
        let canvas = Size::new(64, 64);
        let mut doc = Document::new(canvas);
        let image = Arc::new(select(canvas, &rect(0.0, 0.0, 10.0, 10.0)));
        let selection = Selection::new(Arc::clone(&image)).unwrap();
        let inverse = Edit::SetSelection {
            selection: Some(selection.clone()),
        }
        .apply(&mut doc)
        .unwrap();
        assert_eq!(doc.selection(), Some(&selection));
        assert_eq!(inverse, Edit::SetSelection { selection: None });
        inverse.apply(&mut doc).unwrap();
        assert!(doc.selection().is_none());
        // Selections are gray.
        let color = RasterImage::from_pixels(Size::new(1, 1), PixelFormat::RGBA8_SRGB, &[0; 4]);
        assert!(Selection::new(Arc::new(color.unwrap())).is_none());
    }

    #[test]
    fn reframing_the_image_deselects_and_undo_brings_the_selection_back() {
        use crate::document::Document;
        use crate::edit::Edit;
        let canvas = Size::new(64, 64);
        let mut doc = Document::new(canvas);
        let selection =
            Selection::new(Arc::new(select(canvas, &rect(0.0, 0.0, 9.0, 9.0)))).unwrap();
        Edit::SetSelection {
            selection: Some(selection.clone()),
        }
        .apply(&mut doc)
        .unwrap();
        let crop = Edit::crop(&doc, [0, 0, 32, 32]).unwrap();
        let undo = crop.apply(&mut doc).unwrap();
        assert!(doc.selection().is_none());
        undo.apply(&mut doc).unwrap();
        assert_eq!(doc.selection(), Some(&selection));
    }

    #[test]
    fn the_outline_of_a_rectangle_is_one_closed_loop() {
        let canvas = Size::new(600, 400);
        let image = select(canvas, &rect(10.0, 20.0, 310.0, 280.0));
        let lines = outline(&image, 0, canvas.bounds(), 10_000).unwrap();
        assert_eq!(lines.len(), 1);
        let line = &lines[0];
        assert_eq!(line.first(), line.last());
        let mut corners: Vec<[u32; 2]> = line[..line.len() - 1].to_vec();
        corners.sort();
        assert_eq!(corners, vec![[10, 20], [10, 280], [310, 20], [310, 280]]);
    }

    #[test]
    fn an_outline_cut_by_the_region_is_open_and_a_budget_is_kept() {
        let canvas = Size::new(600, 400);
        let image = select(canvas, &rect(10.0, 20.0, 310.0, 280.0));
        let lines = outline(&image, 0, Rect::new(0, 0, 100, 100), 10_000).unwrap();
        assert!(!lines.is_empty());
        assert!(lines.iter().all(|line| line.first() != line.last()));
        assert!(outline(&image, 0, canvas.bounds(), 2).is_none());
        // A selection to the canvas edge closes along it.
        let all = select_all(canvas).unwrap();
        let lines = outline(&all, 0, canvas.bounds(), 100).unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].first(), lines[0].last());
    }

    #[test]
    fn layer_masks_follow_the_layer_and_can_hide_the_selection() {
        let canvas = Size::new(600, 300);
        let selection = select(canvas, &rect(100.0, 50.0, 200.0, 150.0));
        // A layer moved 40 pixels right: its pixel (60, 50) is document pixel (100, 50).
        let moved = Affine::translation(40.0, 0.0);
        let reveal = layer_mask(&selection, Size::new(300, 200), moved, false).unwrap();
        assert_eq!(reveal.size(), Size::new(300, 200));
        assert_eq!(reveal.gray_at(60, 50), 1.0);
        assert_eq!(reveal.gray_at(59, 50), 0.0);
        assert_eq!(reveal.gray_at(159, 149), 1.0);
        assert_eq!(reveal.gray_at(160, 149), 0.0);
        let hide = layer_mask(&selection, Size::new(300, 200), moved, true).unwrap();
        assert_eq!(hide.gray_at(60, 50), 0.0);
        assert_eq!(hide.gray_at(10, 10), 1.0);
        // A layer scaled twice: its pixel (60, 40) lands on document (120.5, 80.5) or so.
        let scaled = Affine::scale(2.0, 2.0);
        let resampled = layer_mask(&selection, Size::new(300, 150), scaled, false).unwrap();
        assert_eq!(resampled.gray_at(60, 40), 1.0);
        assert_eq!(resampled.gray_at(20, 20), 0.0);
        // Hiding everything is still a mask.
        let none = layer_mask(
            &selection,
            Size::new(10, 10),
            Affine::translation(500.0, 0.0),
            false,
        )
        .unwrap();
        assert_eq!(none.gray_at(5, 5), 0.0);
        assert_eq!(
            uniform_mask(Size::new(10, 10), true).unwrap().gray_at(9, 9),
            1.0
        );
        assert_eq!(
            uniform_mask(Size::new(10, 10), false)
                .unwrap()
                .gray_at(0, 0),
            0.0
        );
    }

    #[test]
    fn expand_contract_and_border_follow_the_distance_to_the_outline() {
        let canvas = Size::new(400, 300);
        let square = select(canvas, &rect(100.0, 100.0, 200.0, 200.0));
        let expanded = modify(canvas, &square, Modify::Expand(10.0))
            .unwrap()
            .unwrap();
        assert_eq!(expanded.gray_at(90, 150), 1.0);
        assert_eq!(expanded.gray_at(89, 150), 0.0);
        assert_eq!(expanded.gray_at(209, 150), 1.0);
        // Rounded corners: the corner of the bounds is farther than 10 pixels.
        assert_eq!(expanded.gray_at(95, 95), 1.0);
        assert_eq!(expanded.gray_at(91, 91), 0.0);

        let contracted = modify(canvas, &square, Modify::Contract(10.0))
            .unwrap()
            .unwrap();
        assert_eq!(contracted.gray_at(110, 150), 1.0);
        assert_eq!(contracted.gray_at(109, 150), 0.0);
        assert_eq!(contracted.gray_at(189, 189), 1.0);
        assert_eq!(contracted.gray_at(190, 150), 0.0);
        assert!(
            modify(canvas, &square, Modify::Contract(60.0))
                .unwrap()
                .is_none()
        );

        let border = modify(canvas, &square, Modify::Border(10.0))
            .unwrap()
            .unwrap();
        assert_eq!(border.gray_at(95, 150), 1.0);
        assert_eq!(border.gray_at(104, 150), 1.0);
        assert_eq!(border.gray_at(94, 150), 0.0);
        assert_eq!(border.gray_at(105, 150), 0.0);
        assert_eq!(border.gray_at(150, 150), 0.0);

        // The canvas edge is not an outline: everything contracted stays everything.
        let all = select_all(canvas).unwrap();
        let still = modify(canvas, &all, Modify::Contract(10.0))
            .unwrap()
            .unwrap();
        assert_eq!(still.gray_at(0, 0), 1.0);
        assert!(
            modify(canvas, &all, Modify::Border(10.0))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn strokes_lie_inside_centered_on_or_outside_the_outline() {
        let canvas = Size::new(400, 300);
        let square = select(canvas, &rect(100.0, 100.0, 200.0, 200.0));
        let band = |location| {
            stroke_band(canvas, &square, 4.0, location)
                .unwrap()
                .unwrap()
        };
        let inside = band(StrokeLocation::Inside);
        assert_eq!(inside.gray_at(100, 150), 1.0);
        assert_eq!(inside.gray_at(103, 150), 1.0);
        assert_eq!(inside.gray_at(104, 150), 0.0);
        assert_eq!(inside.gray_at(99, 150), 0.0);
        assert_eq!(inside.gray_at(150, 150), 0.0);
        let outside = band(StrokeLocation::Outside);
        assert_eq!(outside.gray_at(96, 150), 1.0);
        assert_eq!(outside.gray_at(99, 150), 1.0);
        assert_eq!(outside.gray_at(95, 150), 0.0);
        assert_eq!(outside.gray_at(100, 150), 0.0);
        assert_eq!(outside.gray_at(10, 10), 0.0);
        let center = band(StrokeLocation::Center);
        assert_eq!(center.gray_at(98, 150), 1.0);
        assert_eq!(center.gray_at(101, 150), 1.0);
        assert_eq!(center.gray_at(97, 150), 0.0);
        assert_eq!(center.gray_at(102, 150), 0.0);
        assert!(
            stroke_band(canvas, &square, 0.0, StrokeLocation::Inside)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn smooth_drops_specks_and_feather_softens() {
        let canvas = Size::new(300, 300);
        let square = select(canvas, &rect(50.0, 50.0, 150.0, 150.0));
        let speck = select_shape(
            canvas,
            Some(&square),
            &rect(250.0, 250.0, 252.0, 252.0),
            EdgeOptions::default(),
            Combine::Add,
        )
        .unwrap()
        .unwrap();
        let smooth = modify(canvas, &speck, Modify::Smooth(10.0))
            .unwrap()
            .unwrap();
        assert_eq!(smooth.gray_at(251, 251), 0.0, "the speck is gone");
        assert_eq!(smooth.gray_at(100, 100), 1.0);
        // A straight edge stays where it was, anti-aliased on its pixel.
        assert!(smooth.gray_at(50, 100) > 0.5);
        assert_eq!(smooth.gray_at(52, 100), 1.0);
        assert_eq!(smooth.gray_at(48, 100), 0.0);
        assert_eq!(smooth.gray_at(50, 50), 0.0, "a corner rounds");

        let soft = modify(canvas, &square, Modify::Feather(5.0))
            .unwrap()
            .unwrap();
        assert!((soft.gray_at(50, 100) - 0.5).abs() < 0.06);
        assert_eq!(
            modify(canvas, &square, Modify::Expand(0.0))
                .unwrap()
                .unwrap()
                .gray_at(50, 50),
            1.0
        );
        assert!(modify(canvas, &square, Modify::Expand(MAX_MODIFY + 1.0)).is_err());
        assert!(modify(canvas, &square, Modify::Feather(MAX_FEATHER + 1.0)).is_err());
    }

    /// A 600×300 document: red on the left half, blue on the right with a red square in it
    /// (not touching the left half), and a slightly different red strip at the top left.
    fn wand_document() -> crate::document::Document {
        use crate::document::{Document, Layer, LayerContent};
        use crate::edit::Edit;
        let (w, h) = (600usize, 300usize);
        let mut pixels = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let red = x < 300 || ((450..500).contains(&x) && (100..150).contains(&y));
                let rgb = if x < 300 && y < 20 {
                    [245, 8, 0]
                } else if red {
                    [255, 0, 0]
                } else {
                    [0, 0, 255]
                };
                pixels[(y * w + x) * 4..(y * w + x) * 4 + 4]
                    .copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
            }
        }
        let image = RasterImage::from_pixels(Size::new(600, 300), PixelFormat::RGBA8_SRGB, &pixels)
            .unwrap();
        let mut doc = Document::new(Size::new(600, 300));
        let id = doc.allocate_layer_id();
        Edit::InsertLayer {
            parent: None,
            index: 0,
            layer: Layer {
                style: None,
                id,
                name: "colors".into(),
                visible: true,
                opacity: 1.0,
                blend_mode: crate::blend::BlendMode::Normal,
                content: LayerContent::Raster {
                    stack: None,
                    image: crate::stack::Pixels::ready(Arc::new(image)),
                },
                mask: None,
                clipped: false,
                transform: Affine::IDENTITY,
            },
        }
        .apply(&mut doc)
        .unwrap();
        doc
    }

    #[test]
    fn the_magic_wand_selects_similar_colors_contiguous_or_not() {
        let doc = wand_document();
        let wand = |contiguous: bool, tolerance: f32| {
            let options = WandOptions {
                tolerance,
                contiguous,
                anti_alias: false,
            };
            magic_wand(&doc, None, (100, 150), options, Combine::Replace)
                .unwrap()
                .unwrap()
        };
        // Contiguous: the left half (across tiles), not the red square inside the blue.
        let left = wand(true, 32.0);
        assert_eq!(left.gray_at(0, 299), 1.0);
        assert_eq!(left.gray_at(299, 0), 1.0);
        assert_eq!(left.gray_at(300, 150), 0.0);
        assert_eq!(left.gray_at(460, 120), 0.0);
        assert_eq!(bounds(&left), Some(Rect::new(0, 0, 300, 300)));
        // A low tolerance leaves out the slightly different strip.
        let strict = wand(true, 4.0);
        assert_eq!(strict.gray_at(100, 10), 0.0);
        assert_eq!(strict.gray_at(100, 20), 1.0);
        // Not contiguous: every red pixel, the square included.
        let all = wand(false, 32.0);
        assert_eq!(all.gray_at(460, 120), 1.0);
        assert_eq!(all.gray_at(449, 120), 0.0);
        // Anti-aliased: a soft pixel on the edge.
        let options = WandOptions {
            tolerance: 32.0,
            contiguous: true,
            anti_alias: true,
        };
        let soft = magic_wand(&doc, None, (100, 150), options, Combine::Replace)
            .unwrap()
            .unwrap();
        let edge = soft.gray_at(299, 150);
        assert!(edge > 0.5 && edge < 1.0, "{edge}");
        // Added to a selection.
        let square = select(Size::new(600, 300), &rect(500.0, 200.0, 550.0, 250.0));
        let added = magic_wand(&doc, Some(&square), (100, 150), options, Combine::Add)
            .unwrap()
            .unwrap();
        assert_eq!(added.gray_at(520, 220), 1.0);
        assert_eq!(added.gray_at(100, 150), 1.0);
        // Outside the canvas: nothing.
        assert!(
            magic_wand(&doc, None, (900, 10), options, Combine::Replace)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn grow_and_similar_extend_the_selection_by_its_colors() {
        let doc = wand_document();
        let canvas = Size::new(600, 300);
        let options = |tolerance: f32, contiguous: bool| WandOptions {
            tolerance,
            contiguous,
            anti_alias: false,
        };
        let inside_red = select(canvas, &rect(100.0, 100.0, 120.0, 120.0));
        // Grow: the connected red, not the slightly different strip nor the square in the blue.
        let grown = grow(&doc, &inside_red, options(4.0, true))
            .unwrap()
            .unwrap();
        assert_eq!(grown.gray_at(0, 299), 1.0);
        assert_eq!(grown.gray_at(299, 20), 1.0);
        assert_eq!(grown.gray_at(100, 10), 0.0);
        assert_eq!(grown.gray_at(460, 120), 0.0);
        assert_eq!(grown.gray_at(300, 150), 0.0);
        // A wider tolerance takes the strip too.
        let wider = grow(&doc, &inside_red, options(32.0, true))
            .unwrap()
            .unwrap();
        assert_eq!(wider.gray_at(100, 10), 1.0);
        // Similar: the red square inside the blue as well.
        let similar = grow(&doc, &inside_red, options(4.0, false))
            .unwrap()
            .unwrap();
        assert_eq!(similar.gray_at(460, 120), 1.0);
        assert_eq!(similar.gray_at(449, 120), 0.0);
        assert_eq!(similar.gray_at(100, 10), 0.0);
        // A selection over red and blue takes the range between them: both colors.
        let across = select(canvas, &rect(290.0, 200.0, 310.0, 210.0));
        let both = grow(&doc, &across, options(4.0, true)).unwrap().unwrap();
        assert_eq!(both.gray_at(10, 290), 1.0);
        assert_eq!(both.gray_at(590, 290), 1.0);
        assert_eq!(both.gray_at(100, 10), 0.0);
        // Added to the selection: what the grown area does not reach is kept.
        let blue = select(canvas, &rect(500.0, 10.0, 520.0, 30.0));
        let two = finish(
            canvas,
            Some(&inside_red),
            Mask::from_image(canvas, &blue).unwrap(),
            Combine::Add,
        )
        .unwrap()
        .unwrap();
        let kept = grow(&doc, &two, options(4.0, true)).unwrap().unwrap();
        assert_eq!(kept.gray_at(510, 20), 1.0);
        assert_eq!(kept.gray_at(0, 299), 1.0);
        // Nothing half selected: nothing grows, the soft selection stays.
        let dot = select(canvas, &rect(100.0, 100.0, 101.0, 101.0));
        let faint = modify(canvas, &dot, Modify::Feather(8.0)).unwrap().unwrap();
        assert!(faint.gray_at(100, 100) < 0.5);
        let same = grow(&doc, &faint, options(4.0, true)).unwrap().unwrap();
        assert_eq!(same.gray_at(0, 299), 0.0);
        assert_eq!(same.gray_at(100, 100), faint.gray_at(100, 100));
        assert!(grow(&doc, &inside_red, options(300.0, true)).is_err());
    }

    #[test]
    fn transforming_a_selection_resamples_its_coverage() {
        let canvas = Size::new(1000, 700);
        let square = select(canvas, &rect(100.0, 100.0, 200.0, 200.0));
        // Twice as large, resampled as a layer is: the outline (half covered) doubles, the
        // edge a little soft.
        let doubled = transformed(canvas, &square, Affine::scale(2.0, 2.0))
            .unwrap()
            .unwrap();
        assert_eq!(doubled.gray_at(250, 250), 1.0);
        for (x, inside) in [(199, false), (200, true), (399, true), (400, false)] {
            assert_eq!(doubled.gray_at(x, 300) >= 0.5, inside, "at {x}");
        }
        assert_eq!(doubled.gray_at(150, 150), 0.0);
        assert_eq!(doubled.gray_at(450, 300), 0.0);
        // Turned by 45° about its center: inside stays selected, the edges are soft.
        let center = Affine::translation(-150.0, -150.0)
            .then(Affine::rotation(std::f64::consts::FRAC_PI_4))
            .then(Affine::translation(150.0, 150.0));
        let turned = transformed(canvas, &square, center).unwrap().unwrap();
        assert_eq!(turned.gray_at(150, 150), 1.0);
        // A corner now points up, about 70.7 pixels above the center.
        assert_eq!(turned.gray_at(150, 82), 1.0);
        assert_eq!(turned.gray_at(105, 105), 0.0);
        let soft = (60..90)
            .map(|y| turned.gray_at(150, y))
            .filter(|v| *v > 0.0 && *v < 1.0);
        assert!(soft.count() >= 1);
        // A whole-pixel move is exact.
        let moved = transformed(canvas, &square, Affine::translation(30.0, -20.0))
            .unwrap()
            .unwrap();
        assert_eq!(bounds(&moved), Some(Rect::new(130, 80, 100, 100)));
        // Off the canvas: nothing selected; flattened: refused.
        assert!(
            transformed(canvas, &square, Affine::translation(5000.5, 0.0))
                .unwrap()
                .is_none()
        );
        assert!(transformed(canvas, &square, Affine::scale(0.0, 1.0)).is_err());
    }

    #[test]
    fn a_saved_selection_combines_with_the_current_one() {
        let canvas = Size::new(300, 200);
        let left = select(canvas, &rect(0.0, 0.0, 150.0, 200.0));
        let top = select(canvas, &rect(0.0, 0.0, 300.0, 100.0));
        let at =
            |image: &Option<RasterImage>, x, y| image.as_ref().map_or(0.0, |i| i.gray_at(x, y));
        let replaced = combined(canvas, Some(&left), &top, Combine::Replace).unwrap();
        assert_eq!((at(&replaced, 10, 150), at(&replaced, 250, 10)), (0.0, 1.0));
        let added = combined(canvas, Some(&left), &top, Combine::Add).unwrap();
        assert_eq!((at(&added, 10, 150), at(&added, 250, 10)), (1.0, 1.0));
        let subtracted = combined(canvas, Some(&left), &top, Combine::Subtract).unwrap();
        assert_eq!(
            (at(&subtracted, 10, 150), at(&subtracted, 10, 10)),
            (1.0, 0.0)
        );
        let intersected = combined(canvas, Some(&left), &top, Combine::Intersect).unwrap();
        assert_eq!(
            (at(&intersected, 10, 10), at(&intersected, 250, 10)),
            (1.0, 0.0)
        );
        // Without a selection: adding selects it, subtracting or intersecting leaves nothing.
        assert!(
            combined(canvas, None, &top, Combine::Add)
                .unwrap()
                .is_some()
        );
        assert!(
            combined(canvas, None, &top, Combine::Intersect)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn select_and_mask_settings_chain_the_modify_steps() {
        let canvas = Size::new(400, 300);
        let square = select(canvas, &rect(100.0, 100.0, 200.0, 200.0));
        // Nothing to do: the same coverage.
        let same = refine_edges(canvas, &square, EdgeSettings::default())
            .unwrap()
            .unwrap();
        assert_eq!(same.gray_at(150, 150), 1.0);
        assert_eq!(same.gray_at(99, 150), 0.0);
        // Shifted out by 10 then feathered: the half-coverage edge moved, soft.
        let settings = EdgeSettings {
            shift: 10.0,
            feather: 4.0,
            ..EdgeSettings::default()
        };
        let soft = refine_edges(canvas, &square, settings).unwrap().unwrap();
        let edge = soft.gray_at(90, 150);
        assert!(edge > 0.4 && edge < 0.6, "{edge}");
        assert_eq!(soft.gray_at(150, 150), 1.0);
        // Contrast hardens what Feather softened; 100 % leaves no soft pixel.
        let hard = refine_edges(
            canvas,
            &square,
            EdgeSettings {
                contrast: 100.0,
                ..settings
            },
        )
        .unwrap()
        .unwrap();
        assert!((80..100).all(|x| [0.0, 1.0].contains(&hard.gray_at(x, 150))));
        // Shifted in past its size: nothing left.
        let gone = EdgeSettings {
            shift: -60.0,
            ..EdgeSettings::default()
        };
        assert!(refine_edges(canvas, &square, gone).unwrap().is_none());
        assert!(modify(canvas, &square, Modify::Contrast(101.0)).is_err());
    }

    #[test]
    fn the_refine_brush_lets_the_model_decide_where_it_painted() {
        let canvas = Size::new(1200, 600);
        let square = select(canvas, &rect(100.0, 100.0, 300.0, 300.0));
        // Painted far from the outline: windows there too, undecided where painted.
        let painted = select(canvas, &rect(800.0, 300.0, 900.0, 400.0));
        let band = RefineBand::both(8);
        let plain = plan_refinement(canvas, &square, band, 256, 100).unwrap();
        let with = plan_refinement_with(canvas, &square, band, Some(&painted), 256, 100).unwrap();
        let reaches = |plan: &EdgeRefinement| {
            plan.windows().iter().any(|w| {
                w.inner.x <= 850
                    && 850 < w.inner.x + w.inner.width
                    && w.inner.y <= 350
                    && 350 < w.inner.y + w.inner.height
            })
        };
        assert!(!reaches(&plain));
        assert!(reaches(&with));
        let window = with
            .windows()
            .iter()
            .find(|w| {
                w.rect.x <= 850
                    && 850 < w.rect.x + w.rect.width
                    && w.rect.y <= 350
                    && 350 < w.rect.y + w.rect.height
            })
            .unwrap();
        let trimap = with.trimap(window);
        let at = |x: u32, y: u32| {
            let size = window.input_size();
            let (i, j) = (
                (x - window.rect.x) / window.scale,
                (y - window.rect.y) / window.scale,
            );
            trimap[(j * size.width + i) as usize]
        };
        assert_eq!(at(850, 350), TRIMAP_UNKNOWN);
        assert_eq!(at(850, 250), 0);
    }

    #[test]
    fn srgb_bytes_match_the_transfer_function() {
        let exact = |v: f32| (crate::color::srgb_encode(v.clamp(0.0, 1.0)) * 255.0).round() as u8;
        for i in -1000..=201_000 {
            let v = i as f32 / 200_000.0;
            assert_eq!(srgb_byte(v), exact(v), "at {v}");
        }
        for v in [
            f32::NAN,
            f32::NEG_INFINITY,
            f32::INFINITY,
            0.0,
            1.0,
            1e-9,
            0.003_130_8,
        ] {
            assert_eq!(srgb_byte(v), exact(v), "at {v}");
        }
    }

    #[test]
    fn softening_reaches_uniform_tiles_along_an_edge() {
        // A vertical edge on a tile boundary: both sides soften, as if the tiles were one.
        let size = Size::new(2 * TILE_SIZE, 4);
        let mut mask = Mask::new(size).unwrap();
        mask.tiles[0] = Tile::Const(FULL);
        let soft = soften(&mask);
        let left = soft.at(TILE_SIZE as isize - 1, 1);
        let right = soft.at(TILE_SIZE as isize, 1);
        assert_eq!(left, (u32::from(FULL) * 6 / 9) as u16);
        assert_eq!(right, (u32::from(FULL) * 3 / 9) as u16);
        assert_eq!(soft.at(0, 1), FULL);
    }

    #[test]
    fn color_range_selects_sampled_colors_with_fuzziness() {
        let doc = wand_document();
        let red = sample_colors(&doc, &[(100, 150)]);
        assert_eq!(red, vec![[255.0, 0.0, 0.0, 255.0]]);
        let range = ColorRange {
            included: red.clone(),
            excluded: Vec::new(),
            fuzziness: 40.0,
            invert: false,
            localized: None,
        };
        // Every red pixel, the square inside the blue too; the nearby red strip partly.
        let image = color_range(&doc, None, &range).unwrap().unwrap();
        assert_eq!(image.gray_at(100, 150), 1.0);
        assert_eq!(image.gray_at(460, 120), 1.0);
        assert_eq!(image.gray_at(400, 250), 0.0);
        let strip = image.gray_at(100, 10);
        assert!(strip > 0.7 && strip < 0.8, "{strip}");
        // Excluding the strip's color, and inverting.
        let strip_color = sample_colors(&doc, &[(100, 10)]);
        let narrower = ColorRange {
            excluded: strip_color,
            fuzziness: 4.0,
            ..range.clone()
        };
        let image = color_range(&doc, None, &narrower).unwrap().unwrap();
        assert_eq!(image.gray_at(100, 10), 0.0);
        let inverted = ColorRange {
            invert: true,
            ..range.clone()
        };
        let image = color_range(&doc, None, &inverted).unwrap().unwrap();
        assert_eq!(image.gray_at(400, 250), 1.0);
        assert_eq!(image.gray_at(100, 150), 0.0);
        // Within the current selection only.
        let left = select(Size::new(600, 300), &rect(0.0, 0.0, 200.0, 300.0));
        let image = color_range(&doc, Some(&left), &range).unwrap().unwrap();
        assert_eq!(image.gray_at(100, 150), 1.0);
        assert_eq!(image.gray_at(250, 150), 0.0);
        assert_eq!(image.gray_at(460, 120), 0.0);
        let bad = ColorRange {
            fuzziness: 300.0,
            ..range.clone()
        };
        assert!(color_range(&doc, None, &bad).is_err());
        // Localized around the sample at (100, 150): the red nearby only, fading with the
        // distance; the red square inside the blue is far.
        let localized = ColorRange {
            localized: Some(Localized {
                points: vec![(100.5, 150.5)],
                radius: 100.0,
            }),
            ..range.clone()
        };
        let image = color_range(&doc, None, &localized).unwrap().unwrap();
        assert!(image.gray_at(100, 150) > 0.99);
        let halfway = image.gray_at(150, 150);
        assert!(halfway > 0.45 && halfway < 0.55, "{halfway}");
        assert_eq!(image.gray_at(250, 150), 0.0);
        assert_eq!(image.gray_at(460, 120), 0.0);
        // Inverted, what is out of reach is selected.
        let inverted = ColorRange {
            invert: true,
            ..localized.clone()
        };
        let image = color_range(&doc, None, &inverted).unwrap().unwrap();
        assert_eq!(image.gray_at(460, 120), 1.0);
        // One point per included color, and a radius.
        for (points, radius) in [(Vec::new(), 100.0), (vec![(1.0, 1.0)], 0.0)] {
            let wrong = ColorRange {
                localized: Some(Localized { points, radius }),
                ..range.clone()
            };
            assert!(color_range(&doc, None, &wrong).is_err());
        }
    }

    #[test]
    fn model_logits_become_a_selection_over_the_canvas() {
        // 4×4 logits, positive in the middle 2×2: the middle half of the canvas.
        let mut logits = vec![-5.0f32; 16];
        for (x, y) in [(1, 1), (2, 1), (1, 2), (2, 2)] {
            logits[y * 4 + x] = 5.0;
        }
        let canvas = Size::new(800, 400);
        let whole = Rect::new(0, 0, 800, 400);
        let image = select_logits(canvas, None, &logits, 4, whole, Combine::Replace)
            .unwrap()
            .unwrap();
        assert_eq!(image.gray_at(400, 200), 1.0);
        assert_eq!(image.gray_at(20, 20), 0.0);
        assert_eq!(image.gray_at(780, 380), 0.0);
        let b = bounds(&image).unwrap();
        // The zero crossing lies halfway between the logits' centers: at 1/4 and 3/4.
        assert!(
            (b.x as i64 - 200).abs() <= 2 && (b.y as i64 - 100).abs() <= 2,
            "{b:?}"
        );
        assert!(select_logits(canvas, None, &logits, 3, whole, Combine::Replace).is_err());
        let negative = vec![-1.0f32; 16];
        assert!(
            select_logits(canvas, None, &negative, 4, whole, Combine::Replace)
                .unwrap()
                .is_none()
        );
        // A one-logit speck and a one-logit hole are dropped (64² mask: below 4 logits).
        let mut specks = vec![-5.0f32; 64 * 64];
        for y in 16..48 {
            for x in 16..48 {
                specks[y * 64 + x] = 5.0;
            }
        }
        specks[32 * 64 + 32] = -5.0;
        specks[4 * 64 + 4] = 5.0;
        let cleaned = without_specks(&specks, 64);
        assert!(cleaned[32 * 64 + 32] > 0.0 && cleaned[4 * 64 + 4] < 0.0);
        assert!(cleaned[20 * 64 + 20] > 0.0 && cleaned[60 * 64 + 60] < 0.0);
        // Over a region: the region's middle half, nothing outside it.
        let region = Rect::new(400, 0, 400, 400);
        let image = select_logits(canvas, None, &logits, 4, region, Combine::Replace)
            .unwrap()
            .unwrap();
        let b = bounds(&image).unwrap();
        assert!(
            (b.x as i64 - 500).abs() <= 2 && (b.width as i64 - 200).abs() <= 4,
            "{b:?}"
        );
        assert_eq!(image.gray_at(200, 200), 0.0);
        assert!(
            select_logits(
                canvas,
                None,
                &logits,
                4,
                Rect::new(900, 0, 10, 10),
                Combine::Replace
            )
            .is_err()
        );
    }

    #[test]
    fn refine_edge_mattes_the_band_along_the_outline_only() {
        // The left half of a 2048 × 1024 canvas.
        let canvas = Size::new(2048, 1024);
        let half = Shape::Rectangle {
            left: 0.0,
            top: 0.0,
            right: 1024.0,
            bottom: 1024.0,
        };
        let selection = select_shape(
            canvas,
            None,
            &half,
            EdgeOptions::default(),
            Combine::Replace,
        )
        .unwrap()
        .unwrap();
        let mut plan = plan_refinement(canvas, &selection, RefineBand::both(16), 512, 64).unwrap();
        let windows = plan.windows().to_vec();
        assert!(!windows.is_empty());
        // Only windows along x = 1024, at full resolution.
        for w in &windows {
            assert_eq!(w.scale, 1);
            assert!(
                w.inner.x <= 1024 + 16 && w.inner.x + w.inner.width + 16 >= 1024,
                "{w:?}"
            );
        }
        // A model answering one half everywhere.
        for w in &windows {
            let trimap = plan.trimap(w);
            let size = w.input_size();
            assert_eq!(trimap.len(), (size.width * size.height) as usize);
            assert!(trimap.contains(&TRIMAP_UNKNOWN));
            let alpha = vec![HALF; trimap.len()];
            plan.apply(w, &trimap, &alpha);
        }
        let refined = plan.finish(None, Combine::Replace).unwrap().unwrap();
        // In the band: the model's answer.
        assert!((refined.gray_at(1020, 500) - 0.5).abs() < 1e-3);
        assert!((refined.gray_at(1030, 500) - 0.5).abs() < 1e-3);
        assert_eq!(refined.gray_at(900, 500), 1.0, "outside the band: kept");
        assert_eq!(refined.gray_at(1200, 500), 0.0);
        // At most one window: the outline is seen coarser.
        let coarse = plan_refinement(canvas, &selection, RefineBand::both(16), 512, 1).unwrap();
        assert!(coarse.windows().len() <= 1 || coarse.windows()[0].scale > 1);
        assert!(coarse.windows().iter().all(|w| w.input_size().width <= 512));
    }

    #[test]
    fn refine_edge_decides_farther_outside_than_inside_and_on_soft_coverage() {
        // The left half of a 2048 × 1024 canvas, and faint strands far to its right.
        let canvas = Size::new(2048, 1024);
        let mut logits = vec![-20.0f32; 64 * 64];
        for y in 0..64 {
            for x in 0..64 {
                if x < 32 {
                    logits[y * 64 + x] = 20.0;
                } else if (48..50).contains(&x) && y < 16 {
                    logits[y * 64 + x] = -1.0; // About 27 %.
                }
            }
        }
        let area = canvas.bounds();
        let soft = select_logits_soft(canvas, None, &logits, 64, area, Combine::Replace)
            .unwrap()
            .unwrap();
        let strand = soft.gray_at(1568, 100);
        assert!(strand > 0.1 && strand < 0.5, "{strand}");
        assert_eq!(
            soft.gray_at(1300, 600),
            0.0,
            "noise below the floor: nothing"
        );
        let band = RefineBand {
            inward: 8,
            outward: 64,
        };
        let plan = plan_refinement(canvas, &soft, band, 512, 64).unwrap();
        let unknown_at = |x: u32, y: u32| {
            plan.windows().iter().any(|w| {
                let size = w.input_size();
                let inside = x >= w.rect.x
                    && y >= w.rect.y
                    && x < w.rect.x + size.width * w.scale
                    && y < w.rect.y + size.height * w.scale;
                inside && {
                    let trimap = plan.trimap(w);
                    let (tx, ty) = ((x - w.rect.x) / w.scale, (y - w.rect.y) / w.scale);
                    trimap[(ty * size.width + tx) as usize] == TRIMAP_UNKNOWN
                }
            })
        };
        // The soft edge spans x ≈ 1021–1028 (logits interpolated across one model cell).
        assert!(unknown_at(1080, 600), "outward, within 64 of the soft edge");
        assert!(!unknown_at(1300, 600), "outward, beyond the band");
        assert!(!unknown_at(900, 600), "inward, beyond 8 pixels");
        assert!(unknown_at(1568, 100), "a faint strand far from the edge");
    }

    #[test]
    fn scores_stretched_over_a_large_area_match_pixel_by_pixel() {
        // A 4 × 3 grid of scores over an offset area of a 2000 × 1500 canvas: most tiles are
        // uniform (taken without reading their pixels); each pixel must still be the bilinear
        // reading, before softening.
        let canvas = Size::new(2000, 1500);
        let area = Rect::new(300, 200, 1600, 1200);
        let scores = [
            -1.0, -1.0, -1.0, -1.0, -1.0, 2.0, -0.5, -1.0, -1.0, -1.0, -1.0, -1.0,
        ];
        let mask = stretch(
            canvas,
            &scores,
            4,
            3,
            area,
            |v| if v > 0.0 { FULL } else { 0 },
        )
        .unwrap();
        let (sx, sy) = (4.0 / 1600.0, 3.0 / 1200.0);
        let at = |x: usize, y: usize| scores[y.min(2) * 4 + x.min(3)];
        for y in (0..1500).step_by(7) {
            for x in (0..2000).step_by(7) {
                let inside_area = (300..1900).contains(&x) && (200..1400).contains(&y);
                let expected = inside_area && {
                    let fx = (((x - 300) as f64 + 0.5) * sx - 0.5).max(0.0);
                    let fy = (((y - 200) as f64 + 0.5) * sy - 0.5).max(0.0);
                    let (x0, y0) = (fx as usize, fy as usize);
                    let (tx, ty) = ((fx - x0 as f64) as f32, (fy - y0 as f64) as f32);
                    let upper = at(x0, y0) * (1.0 - tx) + at(x0 + 1, y0) * tx;
                    let lower = at(x0, y0 + 1) * (1.0 - tx) + at(x0 + 1, y0 + 1) * tx;
                    upper * (1.0 - ty) + lower * ty > 0.0
                };
                let got = mask.at(x as isize, y as isize) == FULL;
                assert_eq!(got, expected, "at {x}, {y}");
            }
        }
        assert!(mask.tiles.iter().filter(|t| t.constant().is_some()).count() > 30);
    }

    #[test]
    fn a_selection_is_sampled_on_a_grid_as_pixel_reads_do() {
        let canvas = Size::new(3000, 2000);
        let image = select(canvas, &rect(500.0, 300.0, 2100.0, 1500.0));
        // At full resolution: the same as reading each pixel.
        let area = Rect::new(400, 200, 300, 200);
        let fine = sample_grid(&image, area, 300, 200);
        for (i, v) in fine.iter().enumerate() {
            let (x, y) = (400 + (i % 300) as u32, 200 + (i / 300) as u32);
            assert_eq!(*v, image.gray_at(x, y), "at {x}, {y}");
        }
        // Coarser (a pyramid level): inside and outside where far from the edge.
        let coarse = sample_grid(&image, canvas.bounds(), 300, 200);
        assert_eq!(coarse[100 * 300 + 130], 1.0);
        assert_eq!(coarse[10 * 300 + 10], 0.0);
        assert_eq!(coarse[190 * 300 + 290], 0.0);
    }

    #[test]
    fn neighboring_windows_mattes_are_blended_without_a_seam() {
        // A tall selection edge crossed by windows stacked vertically, mattes 0 and full in
        // turn: along the edge, coverage changes gradually where windows overlap.
        let canvas = Size::new(1024, 2048);
        let half = select(canvas, &rect(0.0, 0.0, 512.0, 2048.0));
        let mut plan = plan_refinement(canvas, &half, RefineBand::both(16), 256, 1000).unwrap();
        let windows = plan.windows().to_vec();
        assert!(windows.len() > 2);
        for (i, w) in windows.iter().enumerate() {
            let trimap = plan.trimap(w);
            let value = if (w.inner.y / w.inner.height.max(1)) % 2 == 0 {
                0
            } else {
                FULL
            };
            let _ = i;
            plan.apply(w, &trimap, &vec![value; trimap.len()]);
        }
        let refined = plan.finish(None, Combine::Replace).unwrap().unwrap();
        let mut previous = refined.gray_at(512, 0);
        let mut largest = 0.0f32;
        for y in 1..2048 {
            let v = refined.gray_at(512, y);
            largest = largest.max((v - previous).abs());
            previous = v;
        }
        assert!(
            largest < 0.1,
            "a jump of {largest} between neighboring pixels"
        );
    }

    #[test]
    fn the_outline_of_a_coarser_level_is_in_document_pixels() {
        let canvas = Size::new(2000, 1000);
        let image = select(canvas, &rect(400.0, 200.0, 1200.0, 800.0));
        let lines = outline(&image, 2, canvas.bounds(), 10_000).unwrap();
        assert_eq!(lines.len(), 1);
        let mut corners: Vec<[u32; 2]> = lines[0][..lines[0].len() - 1].to_vec();
        corners.sort();
        assert_eq!(
            corners,
            vec![[400, 200], [400, 800], [1200, 200], [1200, 800]]
        );
    }

    #[test]
    fn a_translated_selection_moves_exactly_and_shares_uniform_tiles() {
        let canvas = Size::new(1000, 700);
        // A soft edge at column 100.5, a hard one elsewhere; a whole tile inside.
        let image = select(canvas, &rect(100.5, 0.0, 700.0, 600.0));
        let moved = translated(canvas, &image, 37, -20).unwrap().unwrap();
        assert_eq!(moved.gray_at(137, 0), image.gray_at(100, 20));
        assert_eq!(moved.gray_at(138, 5), 1.0);
        assert_eq!(moved.gray_at(136, 5), 0.0);
        assert_eq!(moved.gray_at(736, 579), 1.0);
        assert_eq!(moved.gray_at(737, 579), 0.0);
        assert_eq!(moved.gray_at(400, 580), 0.0);
        assert_eq!(bounds(&moved), Some(Rect::new(137, 0, 600, 580)));
        // Wholly selected tiles are one shared allocation.
        let tile = |col, row| Arc::clone(moved.levels()[0].tile(TileCoord { col, row }).unwrap());
        assert!(Arc::ptr_eq(&tile(1, 1), &tile(1, 0)));
    }

    #[test]
    fn a_selection_moved_off_the_canvas_selects_nothing() {
        let canvas = Size::new(300, 300);
        let image = select(canvas, &rect(10.0, 10.0, 50.0, 50.0));
        assert!(translated(canvas, &image, 300, 0).unwrap().is_none());
        assert!(translated(canvas, &image, -50, 0).unwrap().is_none());
        let back = translated(canvas, &image, 0, 0).unwrap().unwrap();
        assert_eq!(bounds(&back), bounds(&image));
    }
}
