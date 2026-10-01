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
    let result = match (combine, current) {
        (Combine::Replace, _) | (Combine::Add, None) => new,
        // Nothing selected: nothing to subtract from or intersect with.
        (Combine::Subtract | Combine::Intersect, None) => return Ok(None),
        (_, Some(current)) => combine_masks(&Mask::from_image(canvas, current)?, &new, combine),
    };
    Ok(result.into_image())
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
}
