//! Painting (ADR 0027): a stroke turns pointer samples into dabs, accumulates them as a coverage
//! in the painted image's pixel grid, and applies that coverage to the pixels the stroke started
//! from, tile by tile on every core.
//!
//! Every frame of a stroke ([`Stroke::image`]) is a real [`RasterImage`] sharing every tile the
//! stroke did not touch; the last one is the stroke's result. Pixels are always recomputed from
//! the stroke's starting pixels and its coverage, so that dabs never re-quantize what earlier
//! dabs wrote and the stroke's opacity is a true cap.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use crate::blend::{BlendMode, BlendSpace, Blender};
use crate::color::{IDENTITY, LinearRgba, Mat3, WORKING_SPACE, mat_vec};
use crate::raster::{Codec, RasterError, RasterImage, TILE_SIZE, bands, parallel_for_each};
use crate::tile::TileCoord;
use crate::transform::Affine;

/// Pixels per tile.
const TILE_PIXELS: usize = (TILE_SIZE * TILE_SIZE) as usize;
/// Largest brush diameter, in document pixels.
pub const MAX_DIAMETER: f32 = 5000.0;

/// A round brush, as Photoshop's options bar sets it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Brush {
    /// Diameter in document pixels, at full pressure: from 1 to [`MAX_DIAMETER`].
    pub diameter: f32,
    /// The share of the radius painted at full strength, in `[0, 1]`; the rest fades out.
    pub hardness: f32,
    /// Distance between dabs as a share of the diameter, from 0.01 to 10 (Photoshop's 1 % to
    /// 1000 %).
    pub spacing: f32,
    /// How much each dab adds, in `[0, 1]`.
    pub flow: f32,
    /// The most the stroke reaches, in `[0, 1]`.
    pub opacity: f32,
    /// The pen's pressure scales the diameter.
    pub pressure_size: bool,
    /// The pen's pressure scales the opacity.
    pub pressure_opacity: bool,
}

impl Default for Brush {
    fn default() -> Self {
        Self {
            diameter: 30.0,
            hardness: 1.0,
            spacing: 0.25,
            flow: 1.0,
            opacity: 1.0,
            pressure_size: true,
            pressure_opacity: false,
        }
    }
}

impl Brush {
    pub fn is_valid(&self) -> bool {
        let unit = |v: f32| (0.0..=1.0).contains(&v);
        (1.0..=MAX_DIAMETER).contains(&self.diameter)
            && (0.01..=10.0).contains(&self.spacing)
            && unit(self.hardness)
            && unit(self.flow)
            && unit(self.opacity)
    }

    /// Diameter at `pressure`, at least one pixel.
    fn diameter_at(&self, pressure: f32) -> f64 {
        let scale = if self.pressure_size { pressure } else { 1.0 };
        f64::from(self.diameter * scale).max(1.0)
    }

    /// The stroke's opacity at `pressure`.
    fn opacity_at(&self, pressure: f32) -> f32 {
        let scale = if self.pressure_opacity { pressure } else { 1.0 };
        self.opacity * scale
    }

    /// Path length between dabs at `pressure` (at least half a pixel).
    fn step_at(&self, pressure: f32) -> f64 {
        (f64::from(self.spacing) * self.diameter_at(pressure)).max(0.5)
    }
}

/// What a stroke puts down.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Paint {
    /// A color in the working space (straight; its alpha is ignored): the Brush.
    Color(LinearRgba),
    /// Lower the alpha: the Eraser.
    Erase,
}

/// A pointer position in document pixels (pixel centers at `.5`) and the pen's pressure in
/// `[0, 1]` (1 for a mouse).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointerSample {
    pub x: f64,
    pub y: f64,
    pub pressure: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PaintError {
    InvalidBrush,
    /// A color component that is not finite.
    InvalidColor,
    /// The layer's transform cannot be inverted.
    InvalidTransform,
    Raster(RasterError),
}

impl fmt::Display for PaintError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PaintError::InvalidBrush => write!(f, "brush parameters out of range"),
            PaintError::InvalidColor => write!(f, "paint color components must be finite"),
            PaintError::InvalidTransform => write!(f, "the layer's transform is not invertible"),
            PaintError::Raster(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for PaintError {}

impl From<RasterError> for PaintError {
    fn from(e: RasterError) -> Self {
        PaintError::Raster(e)
    }
}

/// One stamp of the brush: its center in document pixels, radius, and the stroke's opacity
/// there.
#[derive(Debug, Clone, Copy)]
struct Dab {
    x: f64,
    y: f64,
    radius: f64,
    opacity: f32,
}

/// The coverage of one tile: what the dabs added (`flow`), and the highest stroke opacity of
/// the dabs that reached each pixel (`cap`). The paint is `min(flow, cap)`.
#[derive(Debug, Clone)]
struct Coverage {
    flow: Vec<f32>,
    cap: Vec<f32>,
}

impl Coverage {
    fn new() -> Self {
        Self {
            flow: vec![0.0; TILE_PIXELS],
            cap: vec![0.0; TILE_PIXELS],
        }
    }
}

/// A brush stroke in progress on one image.
#[derive(Debug)]
pub struct Stroke {
    brush: Brush,
    paint: Paint,
    /// The pixels the stroke starts from (with an alpha channel added for the Eraser).
    base: Arc<RasterImage>,
    /// The last frame.
    current: Arc<RasterImage>,
    /// Image pixels → document, and back.
    to_document: Affine,
    to_image: Affine,
    /// Size of an image pixel in document pixels (its anti-aliasing width).
    pixel: f64,
    /// The selection, and the codec of its pixels.
    selection: Option<(Arc<RasterImage>, Codec)>,
    blender: Blender,
    /// The codec of the image's pixels (built once: 16-bit ones hold a table).
    codec: Codec,
    /// Image linear RGB → working space, and back (identity for gray images).
    to_working: Mat3,
    from_working: Mat3,
    coverage: BTreeMap<TileCoord, Coverage>,
    /// Tiles whose coverage changed since the last frame, with the box of their changed pixels
    /// (`[x0, y0, x1, y1)` within the tile).
    dirty: BTreeMap<TileCoord, [usize; 4]>,
    last: Option<PointerSample>,
    /// Path length still to go before the next dab.
    until_next: f64,
}

impl Stroke {
    /// A stroke painting `paint` with `brush` on `base`, an image placed in the document by
    /// `to_document` (its whole transform, groups included), limited by `selection` (a coverage
    /// mask at the document origin), blending in `blend_space` (ADR 0012).
    pub fn new(
        base: Arc<RasterImage>,
        to_document: Affine,
        selection: Option<Arc<RasterImage>>,
        blend_space: BlendSpace,
        brush: Brush,
        paint: Paint,
    ) -> Result<Self, PaintError> {
        if !brush.is_valid() {
            return Err(PaintError::InvalidBrush);
        }
        if let Paint::Color(c) = paint
            && ![c.r, c.g, c.b].iter().all(|v| v.is_finite())
        {
            return Err(PaintError::InvalidColor);
        }
        let to_image = to_document.inverse().ok_or(PaintError::InvalidTransform)?;
        let base = match (paint, base.with_alpha()) {
            (Paint::Erase, Some(with_alpha)) => Arc::new(with_alpha?),
            _ => base,
        };
        let gray = base.format().layout.is_gray();
        let space = base.format().color_space;
        let (to_working, from_working) = if gray {
            (IDENTITY, IDENTITY)
        } else {
            (
                space.matrix_to(&WORKING_SPACE),
                WORKING_SPACE.matrix_to(&space),
            )
        };
        let codec = Codec::new(base.stored_format());
        let selection = selection.map(|image| {
            let codec = Codec::new(image.stored_format());
            (image, codec)
        });
        Ok(Self {
            brush,
            paint,
            codec,
            current: Arc::clone(&base),
            base,
            to_document,
            to_image,
            pixel: to_document.determinant().abs().sqrt(),
            selection,
            blender: Blender::new(blend_space),
            to_working,
            from_working,
            coverage: BTreeMap::new(),
            dirty: BTreeMap::new(),
            last: None,
            until_next: 0.0,
        })
    }

    /// Continue the stroke through `samples`: dabs every spacing along the path (the first
    /// sample of the stroke gets one), added to the coverage. Samples that are not finite are
    /// skipped; pressures are clamped to `[0, 1]`.
    pub fn add(&mut self, samples: &[PointerSample]) {
        let mut dabs = Vec::new();
        for sample in samples {
            if !(sample.x.is_finite() && sample.y.is_finite()) {
                continue;
            }
            let pressure = if sample.pressure.is_nan() {
                1.0
            } else {
                sample.pressure.clamp(0.0, 1.0)
            };
            let sample = PointerSample {
                pressure,
                ..*sample
            };
            match self.last {
                None => {
                    dabs.push(self.dab(sample));
                    self.until_next = self.brush.step_at(pressure);
                }
                Some(previous) => {
                    let length = (sample.x - previous.x).hypot(sample.y - previous.y);
                    let mut travelled = 0.0;
                    while self.until_next <= length - travelled {
                        travelled += self.until_next;
                        let t = travelled / length;
                        let at = PointerSample {
                            x: previous.x + (sample.x - previous.x) * t,
                            y: previous.y + (sample.y - previous.y) * t,
                            pressure: previous.pressure
                                + (sample.pressure - previous.pressure) * t as f32,
                        };
                        dabs.push(self.dab(at));
                        self.until_next = self.brush.step_at(at.pressure);
                    }
                    self.until_next -= length - travelled;
                }
            }
            self.last = Some(sample);
        }
        self.stamp(&dabs);
    }

    fn dab(&self, at: PointerSample) -> Dab {
        Dab {
            x: at.x,
            y: at.y,
            radius: self.brush.diameter_at(at.pressure) / 2.0,
            opacity: self.brush.opacity_at(at.pressure),
        }
    }

    /// Add `dabs` to the coverage of the tiles they reach, every tile on its own thread (a
    /// tile's dabs in order).
    fn stamp(&mut self, dabs: &[Dab]) {
        let size = self.base.size();
        let mut reached: BTreeMap<TileCoord, Vec<usize>> = BTreeMap::new();
        // Each dab's box in the image's pixels.
        let mut boxes = vec![[0.0; 4]; dabs.len()];
        for (index, dab) in dabs.iter().enumerate() {
            // The dab's box in the document, then in the image (the box of its corners).
            let reach = dab.radius + self.pixel;
            let corners = [
                (-reach, -reach),
                (reach, -reach),
                (-reach, reach),
                (reach, reach),
            ]
            .map(|(dx, dy)| self.to_image.apply(dab.x + dx, dab.y + dy));
            let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
            for (x, y) in corners {
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            }
            if x1 < 0.0 || y1 < 0.0 || x0 >= f64::from(size.width) || y0 >= f64::from(size.height) {
                continue;
            }
            boxes[index] = [x0, y0, x1, y1];
            let tile = f64::from(TILE_SIZE);
            let last_col = (size.width - 1) / TILE_SIZE;
            let last_row = (size.height - 1) / TILE_SIZE;
            let (c0, c1) = (
                (x0.max(0.0) / tile) as u32,
                ((x1 / tile) as u32).min(last_col),
            );
            let (r0, r1) = (
                (y0.max(0.0) / tile) as u32,
                ((y1 / tile) as u32).min(last_row),
            );
            for row in r0..=r1 {
                for col in c0..=c1 {
                    reached
                        .entry(TileCoord { col, row })
                        .or_default()
                        .push(index);
                }
            }
        }
        let mut work: Vec<(TileCoord, Coverage, Vec<usize>)> = reached
            .into_iter()
            .map(|(coord, indices)| {
                let coverage = self.coverage.remove(&coord).unwrap_or_else(Coverage::new);
                (coord, coverage, indices)
            })
            .collect();
        let (brush, to_document, pixel) = (self.brush, self.to_document, self.pixel);
        let changed = parallel_for_each(&mut work, |(coord, coverage, indices)| {
            let mut changed: Option<[usize; 4]> = None;
            for &index in indices.iter() {
                let dab = &dabs[index];
                let area = stamp_tile(
                    coverage,
                    *coord,
                    dab,
                    boxes[index],
                    &brush,
                    to_document,
                    pixel,
                );
                changed = union(changed, area);
            }
            changed
        });
        for ((coord, coverage, _), changed) in work.into_iter().zip(changed) {
            if changed.is_some() {
                let previous = self.dirty.get(&coord).copied();
                if let Some(area) = union(previous, changed) {
                    self.dirty.insert(coord, area);
                }
            }
            self.coverage.insert(coord, coverage);
        }
    }

    /// Whether the stroke has painted anything yet.
    pub fn has_paint(&self) -> bool {
        !self.coverage.is_empty()
    }

    /// The image as painted so far: the tiles whose coverage changed since the last call are
    /// recomputed; every other tile is shared with the previous frame.
    pub fn image(&mut self) -> Result<Arc<RasterImage>, PaintError> {
        if self.dirty.is_empty() {
            return Ok(Arc::clone(&self.current));
        }
        let dirty: Vec<(TileCoord, [usize; 4])> =
            std::mem::take(&mut self.dirty).into_iter().collect();
        let tiles = self.paint_tiles(&dirty);
        let replaced = dirty
            .into_iter()
            .zip(tiles)
            .map(|((coord, area), tile)| (coord, tile, area.map(|v| v as u32)))
            .collect();
        self.current = Arc::new(self.current.with_changed_tiles(replaced)?);
        Ok(Arc::clone(&self.current))
    }

    /// The stroke's result: `None` if it painted nothing (e.g. outside the image).
    pub fn finish(mut self) -> Result<Option<Arc<RasterImage>>, PaintError> {
        if !self.has_paint() {
            return Ok(None);
        }
        self.image().map(Some)
    }

    /// The tiles of `dirty` from the last frame, with the pixels of each box (`[x0, y0, x1,
    /// y1)` within the tile) recomputed from the base image and the stroke's coverage, padded:
    /// in bands of rows, on every core.
    fn paint_tiles(&self, dirty: &[(TileCoord, [usize; 4])]) -> Vec<Arc<[u8]>> {
        let level = &self.base.levels()[0];
        let current = &self.current.levels()[0];
        let size = level.size();
        let bpp = self.codec.bytes_per_pixel;
        let row_bytes = TILE_SIZE as usize * bpp;
        // The valid pixels of each tile (the rest is padding).
        let valid: Vec<(usize, usize)> = dirty
            .iter()
            .map(|(coord, _)| {
                let width = (size.width - coord.col * TILE_SIZE).min(TILE_SIZE) as usize;
                let height = (size.height - coord.row * TILE_SIZE).min(TILE_SIZE) as usize;
                (width, height)
            })
            .collect();
        let mut tiles: Vec<Vec<u8>> = dirty
            .iter()
            .map(|(coord, _)| current.tile(*coord).map_or_else(Vec::new, |t| t.to_vec()))
            .collect();
        let spans: Vec<(usize, usize)> = dirty
            .iter()
            .zip(&valid)
            .map(|((_, area), &(_, height))| (area[1].min(height), area[3].min(height)))
            .collect();
        let mut work = bands(&mut tiles, &spans, row_bytes);
        parallel_for_each(&mut work, |band| {
            let (coord, area) = dirty[band.tile];
            let (Some(base), Some(coverage)) = (level.tile(coord), self.coverage.get(&coord))
            else {
                return;
            };
            let first = band.first_row;
            let last = first + band.rows.len() / row_bytes;
            let columns = area[0]..area[2].min(valid[band.tile].0);
            for y in first..last {
                let row = &mut band.rows[(y - first) * row_bytes..][..row_bytes];
                for x in columns.clone() {
                    let i = y * TILE_SIZE as usize + x;
                    let px = &mut row[x * bpp..(x + 1) * bpp];
                    // Coverage only grows: a pixel it does not reach yet is the base's.
                    px.copy_from_slice(&base[i * bpp..(i + 1) * bpp]);
                    let amount = coverage.flow[i].min(coverage.cap[i]);
                    if amount > 0.0 {
                        self.paint_pixel(coord, x, y, amount, px);
                    }
                }
            }
        });
        tiles
            .into_iter()
            .zip(valid)
            .map(|(mut tile, (width, height))| {
                if !tile.is_empty() {
                    pad(&mut tile, width, height, bpp);
                }
                Arc::from(tile)
            })
            .collect()
    }

    /// Apply the paint at `amount` (before the selection) to pixel (`x`, `y`) of tile `coord`,
    /// `px` holding the base pixel.
    fn paint_pixel(&self, coord: TileCoord, x: usize, y: usize, amount: f32, px: &mut [u8]) {
        let mut amount = amount;
        if let Some((image, codec)) = &self.selection {
            let (dx, dy) = self.to_document.apply(
                f64::from(coord.col * TILE_SIZE) + x as f64 + 0.5,
                f64::from(coord.row * TILE_SIZE) + y as f64 + 0.5,
            );
            amount *= MaskReader { image, codec }.at(dx.floor(), dy.floor());
            if amount <= 0.0 {
                return;
            }
        }
        let codec = &self.codec;
        let (color, alpha) = codec.read_mapped(px, &mut |v| v);
        let below = mat_vec(&self.to_working, color.map(f64::from));
        let mut dst = [below[0], below[1], below[2], f64::from(alpha)];
        let amount = f64::from(amount);
        match self.paint {
            Paint::Color(c) => {
                let src = [
                    f64::from(c.r) * amount,
                    f64::from(c.g) * amount,
                    f64::from(c.b) * amount,
                    amount,
                ];
                self.blender.blend(BlendMode::Normal, &src, &mut dst);
            }
            Paint::Erase => {
                for v in &mut dst {
                    *v *= 1.0 - amount;
                }
            }
        }
        let rgb = [dst[0], dst[1], dst[2]];
        let rgb = if self.base.format().layout.is_gray() {
            let luma = luma_of_working();
            [rgb[0] * luma[0] + rgb[1] * luma[1] + rgb[2] * luma[2]; 3]
        } else {
            mat_vec(&self.from_working, rgb)
        };
        codec.write(rgb.map(|v| v as f32), dst[3] as f32, px);
    }
}

/// Add `dab`, whose box in the image's pixels is `[x0, y0, x1, y1]`, to the coverage of tile
/// `coord`; the box of the pixels that changed (`[x0, y0, x1, y1)` in the tile), if any.
fn stamp_tile(
    coverage: &mut Coverage,
    coord: TileCoord,
    dab: &Dab,
    [bx0, by0, bx1, by1]: [f64; 4],
    brush: &Brush,
    to_document: Affine,
    pixel: f64,
) -> Option<[usize; 4]> {
    let t = TILE_SIZE as usize;
    let (x0, y0) = (
        f64::from(coord.col * TILE_SIZE),
        f64::from(coord.row * TILE_SIZE),
    );
    // The pixels of the tile within the dab's box.
    let span = |lo: f64, hi: f64, origin: f64| {
        let first = (lo - origin).floor().clamp(0.0, t as f64) as usize;
        let last = ((hi - origin).ceil() + 1.0).clamp(0.0, t as f64) as usize;
        first..last
    };
    let (xs, ys) = (span(bx0, bx1, x0), span(by0, by1, y0));
    let radius = dab.radius;
    let inner = f64::from(brush.hardness) * radius;
    let flow = f64::from(brush.flow);
    let reach2 = (radius + pixel).powi(2);
    let mut changed: Option<[usize; 4]> = None;
    for y in ys {
        for x in xs.clone() {
            let (dx, dy) = to_document.apply(x0 + x as f64 + 0.5, y0 + y as f64 + 0.5);
            let d2 = (dx - dab.x).powi(2) + (dy - dab.y).powi(2);
            if d2 >= reach2 {
                continue;
            }
            let strength = profile(d2.sqrt(), radius, inner, pixel) * flow;
            if strength <= 0.0 {
                continue;
            }
            let i = y * t + x;
            let c = f64::from(coverage.flow[i]);
            coverage.flow[i] = (c + strength * (1.0 - c)) as f32;
            coverage.cap[i] = coverage.cap[i].max(dab.opacity);
            changed = union(changed, Some([x, y, x + 1, y + 1]));
        }
    }
    changed
}

/// The box holding both boxes (`[x0, y0, x1, y1)`), if any.
fn union(a: Option<[usize; 4]>, b: Option<[usize; 4]>) -> Option<[usize; 4]> {
    match (a, b) {
        (Some(a), Some(b)) => Some([
            a[0].min(b[0]),
            a[1].min(b[1]),
            a[2].max(b[2]),
            a[3].max(b[3]),
        ]),
        (a, None) => a,
        (None, b) => b,
    }
}

/// The brush tip at distance `d` from its center: 1 within `inner`, fading smoothly to 0 at
/// `radius`, its edge anti-aliased over one image pixel (`pixel`, in document pixels).
fn profile(d: f64, radius: f64, inner: f64, pixel: f64) -> f64 {
    let edge = ((radius - d) / pixel + 0.5).clamp(0.0, 1.0);
    if d <= inner || radius - inner < 1e-9 {
        return edge;
    }
    let t = ((d - inner) / (radius - inner)).clamp(0.0, 1.0);
    // 1 − smoothstep: flat at both ends, so soft tips have no visible rim.
    (1.0 - t * t * (3.0 - 2.0 * t)) * edge
}

/// Fill the padding of a tile whose `width` × `height` first pixels are valid: each row repeats
/// its last valid pixel, then the rows below repeat the last valid row (as tiles are padded).
fn pad(tile: &mut [u8], width: usize, height: usize, bpp: usize) {
    let t = TILE_SIZE as usize;
    let row_bytes = t * bpp;
    if width < t {
        for y in 0..height {
            let row = &mut tile[y * row_bytes..(y + 1) * row_bytes];
            let (valid, padding) = row.split_at_mut(width * bpp);
            let last = &valid[(width - 1) * bpp..];
            for px in padding.chunks_exact_mut(bpp) {
                px.copy_from_slice(last);
            }
        }
    }
    if height < t {
        let (valid, padding) = tile.split_at_mut(height * row_bytes);
        let last = &valid[(height - 1) * row_bytes..];
        for row in padding.chunks_exact_mut(row_bytes) {
            row.copy_from_slice(last);
        }
    }
}

/// Luminance weights of the working space's linear RGB (the Y row of its XYZ matrix).
fn luma_of_working() -> [f64; 3] {
    WORKING_SPACE.primaries.to_xyz()[1]
}

/// Reads a gray mask's coverage at document pixels.
struct MaskReader<'a> {
    image: &'a RasterImage,
    codec: &'a Codec,
}

impl MaskReader<'_> {
    /// Coverage of pixel (`x`, `y`) in `[0, 1]`; 0 outside the mask.
    fn at(&self, x: f64, y: f64) -> f32 {
        let size = self.image.size();
        if x < 0.0 || y < 0.0 || x >= f64::from(size.width) || y >= f64::from(size.height) {
            return 0.0;
        }
        let (x, y) = (x as u32, y as u32);
        let coord = TileCoord {
            col: x / TILE_SIZE,
            row: y / TILE_SIZE,
        };
        let Some(tile) = self.image.levels()[0].tile(coord) else {
            return 0.0;
        };
        let bpp = self.codec.bytes_per_pixel;
        let at = ((y % TILE_SIZE) * TILE_SIZE + x % TILE_SIZE) as usize * bpp;
        tile.get(at..at + bpp).map_or(0.0, |px| {
            self.codec.read_mapped(px, &mut |v| v).0[0].clamp(0.0, 1.0)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::{AlphaMode, ChannelLayout, ColorSpace, PixelFormat, SampleType};
    use crate::geom::Size;

    fn rgba8() -> PixelFormat {
        PixelFormat::RGBA8_SRGB
    }

    /// A `size` image of `format` filled with one pixel's bytes.
    fn filled(size: Size, format: PixelFormat, pixel: &[u8]) -> Arc<RasterImage> {
        let pixels = pixel.repeat(size.pixel_count() as usize);
        Arc::new(RasterImage::from_pixels(size, format, &pixels).unwrap())
    }

    fn transparent(size: Size) -> Arc<RasterImage> {
        filled(size, rgba8(), &[0, 0, 0, 0])
    }

    fn red() -> Paint {
        Paint::Color(LinearRgba::from_srgb_encoded_to_working(1.0, 0.0, 0.0, 1.0))
    }

    fn black() -> Paint {
        Paint::Color(LinearRgba::new(0.0, 0.0, 0.0, 1.0))
    }

    fn sample(x: f64, y: f64) -> PointerSample {
        PointerSample {
            x,
            y,
            pressure: 1.0,
        }
    }

    /// Stored bytes of pixel (`x`, `y`) of level 0.
    fn pixel(image: &RasterImage, x: u32, y: u32) -> Vec<u8> {
        let bpp = image.stored_format().bytes_per_pixel() as usize;
        let tile = image.levels()[0]
            .tile(TileCoord {
                col: x / TILE_SIZE,
                row: y / TILE_SIZE,
            })
            .unwrap();
        let at = ((y % TILE_SIZE) * TILE_SIZE + x % TILE_SIZE) as usize * bpp;
        tile[at..at + bpp].to_vec()
    }

    fn stroke(base: Arc<RasterImage>, brush: Brush, paint: Paint) -> Stroke {
        Stroke::new(
            base,
            Affine::IDENTITY,
            None,
            BlendSpace::Perceptual,
            brush,
            paint,
        )
        .unwrap()
    }

    #[test]
    fn a_hard_dab_paints_a_disc_with_an_antialiased_edge() {
        let brush = Brush {
            diameter: 20.0,
            ..Brush::default()
        };
        let mut s = stroke(transparent(Size::new(64, 64)), brush, red());
        // Centered on pixel (32, 32): pixel 42 is centered on the edge, half inside.
        s.add(&[sample(32.5, 32.5)]);
        let image = s.finish().unwrap().unwrap();
        assert_eq!(pixel(&image, 32, 32), [255, 0, 0, 255]);
        assert_eq!(pixel(&image, 41, 32), [255, 0, 0, 255]);
        let edge = pixel(&image, 42, 32)[3];
        assert!((126..=129).contains(&edge), "{edge}");
        assert_eq!(pixel(&image, 43, 32), [0, 0, 0, 0]);
    }

    #[test]
    fn opacity_caps_the_stroke_and_flow_builds_up() {
        let brush = Brush {
            diameter: 20.0,
            flow: 0.2,
            opacity: 0.5,
            spacing: 0.05,
            ..Brush::default()
        };
        let base = filled(Size::new(64, 64), rgba8(), &[255, 255, 255, 255]);
        let mut s = stroke(base, brush, black());
        // One dab: 20 % of black.
        s.add(&[sample(32.0, 32.0)]);
        let after_one = pixel(&s.image().unwrap(), 32, 32)[0];
        // Back and forth over the same place: builds up, but not past 50 %.
        for _ in 0..20 {
            s.add(&[sample(40.0, 32.0), sample(24.0, 32.0)]);
        }
        let after_many = pixel(&s.finish().unwrap().unwrap(), 32, 32)[0];
        assert!(after_many < after_one, "{after_one} → {after_many}");
        // Half of white in sRGB values (a perceptual document): 128 (± rounding).
        assert!((127..=128).contains(&after_many), "{after_many}");
    }

    #[test]
    fn dabs_follow_the_path_at_the_spacing() {
        let brush = Brush {
            diameter: 10.0,
            ..Brush::default()
        };
        let mut s = stroke(transparent(Size::new(300, 40)), brush, red());
        s.add(&[sample(10.0, 20.0), sample(150.0, 20.0)]);
        s.add(&[sample(290.0, 20.0)]);
        let image = s.finish().unwrap().unwrap();
        // A continuous line, whatever the batches.
        for x in 10..290 {
            assert_eq!(pixel(&image, x, 20)[3], 255, "gap at {x}");
        }
        assert_eq!(pixel(&image, 150, 30)[3], 0);
    }

    #[test]
    fn pressure_scales_the_diameter() {
        let brush = Brush {
            diameter: 40.0,
            ..Brush::default()
        };
        let mut s = stroke(transparent(Size::new(100, 100)), brush, red());
        s.add(&[PointerSample {
            x: 50.0,
            y: 50.0,
            pressure: 0.25,
        }]);
        let image = s.finish().unwrap().unwrap();
        assert_eq!(pixel(&image, 53, 50)[3], 255);
        assert_eq!(pixel(&image, 57, 50)[3], 0);
    }

    #[test]
    fn frames_share_untouched_tiles_and_the_result_is_the_last_frame() {
        let base = transparent(Size::new(1000, 600));
        let brush = Brush {
            diameter: 30.0,
            hardness: 0.0,
            ..Brush::default()
        };
        let mut s = stroke(Arc::clone(&base), brush, red());
        s.add(&[sample(100.0, 100.0)]);
        let first = s.image().unwrap();
        s.add(&[sample(120.0, 110.0)]);
        let second = s.image().unwrap();
        let level = |image: &RasterImage| image.levels()[0].tiles().to_vec();
        let (b, f, g) = (level(&base), level(&first), level(&second));
        // Tile 0 changed; the far ones are the base's.
        assert!(!Arc::ptr_eq(&b[0], &f[0]));
        assert!(Arc::ptr_eq(&b[3], &g[3]) && Arc::ptr_eq(&b[11], &g[11]));
        let result = s.finish().unwrap().unwrap();
        assert!(Arc::ptr_eq(&result, &second));
        // Nothing painted: no result.
        let mut empty = stroke(base, brush, red());
        empty.add(&[sample(-500.0, -500.0)]);
        assert!(empty.finish().unwrap().is_none());
    }

    #[test]
    fn soft_strokes_on_8_bit_layers_are_quantized_once() {
        // Recomputed from the base at each frame, the result is the composite of the final
        // coverage, whatever the frames: not dab after dab rounded.
        let brush = Brush {
            diameter: 60.0,
            hardness: 0.0,
            flow: 0.1,
            spacing: 0.02,
            ..Brush::default()
        };
        let base = filled(Size::new(128, 128), rgba8(), &[200, 200, 200, 255]);
        let mut s = stroke(Arc::clone(&base), brush, black());
        for i in 0..40 {
            s.add(&[sample(30.0 + f64::from(i), 64.0)]);
            s.image().unwrap();
        }
        let painted = s.finish().unwrap().unwrap();
        let mut once = stroke(base, brush, black());
        let path: Vec<_> = (0..40).map(|i| sample(30.0 + f64::from(i), 64.0)).collect();
        once.add(&path);
        let reference = once.finish().unwrap().unwrap();
        for x in 0..128 {
            assert_eq!(pixel(&painted, x, 64), pixel(&reference, x, 64), "x {x}");
        }
    }

    #[test]
    fn the_eraser_lowers_alpha_and_adds_it_when_missing() {
        let rgb = PixelFormat {
            layout: ChannelLayout::Rgb,
            ..rgba8()
        };
        let base = filled(Size::new(64, 64), rgb, &[10, 20, 30]);
        let brush = Brush {
            diameter: 10.0,
            opacity: 0.5,
            ..Brush::default()
        };
        let mut s = stroke(base, brush, Paint::Erase);
        s.add(&[sample(32.0, 32.0)]);
        let image = s.finish().unwrap().unwrap();
        assert_eq!(image.format().layout, ChannelLayout::Rgba);
        // Straight color kept, alpha halved.
        assert_eq!(pixel(&image, 32, 32), [10, 20, 30, 128]);
        assert_eq!(pixel(&image, 2, 2), [10, 20, 30, 255]);
    }

    #[test]
    fn the_selection_limits_the_paint() {
        let size = Size::new(64, 64);
        // The left half selected.
        let bytes: Vec<u8> = (0..64 * 64)
            .flat_map(|i| if i % 64 < 32 { 65535u16 } else { 0 }.to_ne_bytes())
            .collect();
        let format = PixelFormat {
            layout: ChannelLayout::Gray,
            sample: SampleType::U16,
            color_space: ColorSpace::LINEAR_SRGB,
            alpha: AlphaMode::Straight,
        };
        let selection = Arc::new(RasterImage::from_pixels(size, format, &bytes).unwrap());
        let brush = Brush {
            diameter: 30.0,
            ..Brush::default()
        };
        let mut s = Stroke::new(
            transparent(size),
            Affine::IDENTITY,
            Some(selection),
            BlendSpace::Perceptual,
            brush,
            red(),
        )
        .unwrap();
        s.add(&[sample(32.0, 32.0)]);
        let image = s.finish().unwrap().unwrap();
        assert_eq!(pixel(&image, 31, 32)[3], 255);
        assert_eq!(pixel(&image, 32, 32)[3], 0);
    }

    #[test]
    fn transformed_layers_get_round_dabs_on_the_canvas() {
        // A layer shown twice as large and moved: a 20 px dab covers 10 of its pixels.
        let to_document = Affine::scale(2.0, 2.0).then(Affine::translation(100.0, 0.0));
        let brush = Brush {
            diameter: 20.0,
            ..Brush::default()
        };
        let base = transparent(Size::new(64, 64));
        let mut s = Stroke::new(
            base,
            to_document,
            None,
            BlendSpace::Perceptual,
            brush,
            red(),
        )
        .unwrap();
        // Document (164, 64) is layer point (32, 32).
        s.add(&[sample(164.0, 64.0)]);
        let image = s.finish().unwrap().unwrap();
        assert_eq!(pixel(&image, 32, 32)[3], 255);
        assert_eq!(pixel(&image, 35, 32)[3], 255);
        assert_eq!(pixel(&image, 38, 32)[3], 0);
        assert_eq!(pixel(&image, 32, 38)[3], 0);
    }

    #[test]
    fn gray_layers_get_the_luminance_of_the_color() {
        let gray = PixelFormat {
            layout: ChannelLayout::Gray,
            sample: SampleType::U8,
            color_space: ColorSpace::SRGB,
            alpha: AlphaMode::Straight,
        };
        let base = filled(Size::new(32, 32), gray, &[0]);
        let green = Paint::Color(LinearRgba::from_srgb_encoded_to_working(0.0, 1.0, 0.0, 1.0));
        let mut s = stroke(base, Brush::default(), green);
        s.add(&[sample(16.0, 16.0)]);
        let image = s.finish().unwrap().unwrap();
        // sRGB green's luminance (0.7152) encoded: 220.
        let v = pixel(&image, 16, 16)[0];
        assert!((219..=221).contains(&v), "{v}");
    }

    #[test]
    fn invalid_brushes_and_transforms_are_refused() {
        let base = transparent(Size::new(8, 8));
        let new = |brush: Brush, t: Affine| {
            Stroke::new(Arc::clone(&base), t, None, BlendSpace::Linear, brush, red()).err()
        };
        let bad = Brush {
            diameter: 0.5,
            ..Brush::default()
        };
        assert_eq!(new(bad, Affine::IDENTITY), Some(PaintError::InvalidBrush));
        assert_eq!(
            new(Brush::default(), Affine::scale(0.0, 1.0)),
            Some(PaintError::InvalidTransform)
        );
    }

    /// Timing of a long soft stroke on a large layer (run with `--ignored --nocapture`).
    #[test]
    #[ignore = "benchmark"]
    fn bench_soft_stroke() {
        let base = transparent(Size::new(8000, 8000));
        let brush = Brush {
            diameter: 400.0,
            hardness: 0.0,
            ..Brush::default()
        };
        let mut s = stroke(base, brush, red());
        let started = std::time::Instant::now();
        let frames = 120;
        for i in 0..frames {
            // 40 px per frame: a fast hand at 60 Hz.
            s.add(&[sample(400.0 + 40.0 * f64::from(i), 4000.0)]);
            s.image().unwrap();
        }
        let per_frame = started.elapsed().as_secs_f64() * 1000.0 / f64::from(frames);
        println!("{frames} frames of a 400 px soft brush: {per_frame:.2} ms per frame");
    }
}
