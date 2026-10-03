//! A raster layer's own stack (ADR 0029): its original pixels, then the paint and the effects
//! applied to it, bottom to top. The layer shows the stack's result, evaluated tile by tile.
//!
//! - **Paint** is a delta: it turns what is below it, `B`, into `P + k·B` per pixel, `P` a
//!   color and `k` a factor in `[0, 1]`, in the premultiplied values of the blend space recorded
//!   with it. It stores the tiles it touched only; elsewhere it is the identity (`P = 0`,
//!   `k = 1`). `P` is a pixel of the layer's own format (with alpha); `k` has the same sample
//!   type (floats: `f32`), so that an opaque pixel painted over stays exactly opaque.
//! - **Effects** are parameters: an adjustment applied to the layer's own color, limited by the
//!   selection it was applied with (kept by reference, with the layer's placement then).
//!
//! The result is quantized to the layer's format after every paint and every effect, as each
//! one wrote the layer's pixels in Photoshop. So evaluating from the original and adding to a
//! result already evaluated give the same pixels, and only the tiles that an entry reaches are
//! ever recomputed. Entries are not edited; deleting one merges the neighbours that become alike.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, Mutex, OnceLock};

use crate::adjust::{Adjustment, Prepared};
use crate::blend::{BlendSpace, Blender};
use crate::color::{
    ChannelLayout, IDENTITY, LinearRgba, Mat3, PixelFormat, SampleType, WORKING_SPACE, f16_to_f32,
    f32_to_f16, mat_vec,
};
use crate::geom::Size;
use crate::paint::MaskReader;
use crate::raster::{
    Codec, RasterError, RasterImage, TILE_SIZE, pad_tile, parallel_for_each, stored_format,
};
use crate::selection::Selection;
use crate::tile::TileCoord;
use crate::transform::Affine;

/// Pixels per tile.
const TILE_PIXELS: usize = (TILE_SIZE * TILE_SIZE) as usize;
/// Rows a thread computes at a time when one tile is shared between threads.
const BAND_ROWS: usize = 8;
/// The largest pixel: RGBA of `f32`.
const MAX_PIXEL_BYTES: usize = 16;

/// A raster layer's original pixels and the entries applied to them, bottom to top. Cloning
/// shares everything.
#[derive(Debug, Clone)]
pub struct LayerStack {
    original: Arc<RasterImage>,
    entries: Vec<Entry>,
}

/// One entry of a [`LayerStack`]. Entries are immutable and shared: two entries are the same
/// when they are the same allocation.
#[derive(Debug, Clone)]
pub enum Entry {
    Paint(Arc<PaintEntry>),
    Effect(Arc<EffectEntry>),
}

impl PartialEq for Entry {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Paint(a), Self::Paint(b)) => Arc::ptr_eq(a, b),
            (Self::Effect(a), Self::Effect(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

/// Paint: `P + k·B` over what is below it, on the tiles it touched.
#[derive(Debug)]
pub struct PaintEntry {
    size: Size,
    /// The format of `P`: the layer's, with an alpha channel.
    format: PixelFormat,
    space: BlendSpace,
    tiles: BTreeMap<TileCoord, PaintTile>,
    /// `P` and `k` as whole images, made once when asked (files store them so).
    images: OnceLock<(Arc<RasterImage>, Arc<RasterImage>)>,
    /// What was below it when it was laid, the tiles it reached: the next stroke continuing it
    /// reuses them while the stack below is the same (a cache, never saved).
    below: Mutex<Option<BelowTiles>>,
}

/// Tiles of the result of a stack below a paint (see [`PaintEntry`]).
#[derive(Debug, Clone)]
struct BelowTiles {
    stack: LayerStack,
    format: PixelFormat,
    tiles: HashMap<TileCoord, Arc<[u8]>>,
}

/// The paint of one tile: `TILE_SIZE²` pixels of `P` (the entry's stored format) and of `k`,
/// padded as image tiles are.
#[derive(Debug, Clone)]
pub struct PaintTile {
    color: Arc<[u8]>,
    keep: Arc<[u8]>,
    /// Some pixel has `P`'s alpha + `k` below 1: it lowers the alpha of an opaque pixel.
    lowers_alpha: bool,
}

impl PaintTile {
    /// `P`'s pixels.
    pub fn color(&self) -> &Arc<[u8]> {
        &self.color
    }

    /// `k`'s samples.
    pub fn keep(&self) -> &Arc<[u8]> {
        &self.keep
    }
}

/// A paint tile as stored: its position, `P`'s pixels and `k`'s samples.
pub type RawPaintTile = (TileCoord, Arc<[u8]>, Arc<[u8]>);

/// What a painting tool does to the paint where it reaches (by an amount in `[0, 1]`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PaintOp {
    /// Lay a color (working space, straight; its alpha is ignored) over what is there: the
    /// Brush, Fill, Stroke.
    Color(LinearRgba),
    /// Lower the alpha of the result: the Eraser, Delete.
    Erase,
    /// Bring back what is below the paint: the Restore Eraser.
    Restore,
}

/// An applied adjustment (Image > Adjustments).
#[derive(Debug, Clone, PartialEq)]
pub struct Effect {
    pub adjustment: Adjustment,
    /// Where it applies (a coverage at the document origin); everywhere when `None`.
    pub selection: Option<Selection>,
    /// The layer's pixels → document when it was applied: where the selection is read, so that
    /// the effect stays where it was applied when the layer moves.
    pub to_document: Affine,
    pub space: BlendSpace,
}

impl Effect {
    pub fn is_valid(&self) -> bool {
        self.adjustment.is_valid()
            && self.to_document.is_finite()
            && self.to_document.inverse().is_some()
    }

    /// Two Inverts in a row with nothing between them do nothing.
    fn cancels(&self, next: &Effect) -> bool {
        matches!(self.adjustment, Adjustment::Invert)
            && matches!(next.adjustment, Adjustment::Invert)
            && self.selection.is_none()
            && next.selection.is_none()
            && self.space == next.space
    }
}

/// Effects of one kind applied in a row: one entry (deleting a paint between two effects of the
/// same kind joins them).
#[derive(Debug)]
pub struct EffectEntry {
    steps: Vec<Arc<Effect>>,
}

impl EffectEntry {
    /// One entry of `steps`, in order: at least one, all valid and of the same kind.
    pub fn new(steps: Vec<Arc<Effect>>) -> Result<Self, StackError> {
        let first = steps.first().ok_or(StackError::InvalidEffect)?;
        let kind = first.adjustment.id();
        if steps
            .iter()
            .any(|s| !s.is_valid() || s.adjustment.id() != kind)
        {
            return Err(StackError::InvalidEffect);
        }
        Ok(Self { steps })
    }

    pub fn steps(&self) -> &[Arc<Effect>] {
        &self.steps
    }

    /// The adjustment's kind ([`Adjustment::id`]).
    pub fn kind(&self) -> &'static str {
        self.steps[0].adjustment.id()
    }
}

/// Why a stack, an entry or a tile was refused.
#[derive(Debug, Clone, PartialEq)]
pub enum StackError {
    /// Paint of another size than the layer.
    SizeMismatch,
    /// Paint of another format than the layer's (with alpha).
    FormatMismatch,
    /// A tile of the wrong length.
    TileLength,
    /// A tile outside the layer.
    TileOutside(TileCoord),
    /// An effect that is not valid, or an entry mixing kinds.
    InvalidEffect,
    /// No entry at this index.
    IndexOutOfRange(usize),
    Raster(RasterError),
}

impl fmt::Display for StackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StackError::SizeMismatch => write!(f, "paint of another size than its layer"),
            StackError::FormatMismatch => write!(f, "paint of another format than its layer"),
            StackError::TileLength => write!(f, "paint tile of the wrong length"),
            StackError::TileOutside(coord) => write!(f, "paint tile {coord:?} outside its layer"),
            StackError::InvalidEffect => write!(f, "invalid effect"),
            StackError::IndexOutOfRange(index) => write!(f, "no entry at index {index}"),
            StackError::Raster(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for StackError {}

impl From<RasterError> for StackError {
    fn from(e: RasterError) -> Self {
        StackError::Raster(e)
    }
}

/// The format of the paint of a layer of `format`: the same, with an alpha channel.
pub fn paint_format(format: PixelFormat) -> PixelFormat {
    let layout = if format.layout.is_gray() {
        ChannelLayout::GrayAlpha
    } else {
        ChannelLayout::Rgba
    };
    PixelFormat { layout, ..format }
}

/// The sample type of `k` beside `P`'s: the same grid for integers, so that `P`'s alpha and `k`
/// round to complementary values; `f32` for floats.
fn keep_sample(sample: SampleType) -> SampleType {
    match sample {
        SampleType::U8 => SampleType::U8,
        SampleType::U16 => SampleType::U16,
        SampleType::F16 | SampleType::F32 => SampleType::F32,
    }
}

fn read_keep(sample: SampleType, keep: &[u8], i: usize) -> f64 {
    let v = match sample {
        SampleType::U8 => f64::from(keep[i]) / 255.0,
        SampleType::U16 => f64::from(u16::from_ne_bytes([keep[2 * i], keep[2 * i + 1]])) / 65535.0,
        SampleType::F16 => f64::from(f16_to_f32(u16::from_ne_bytes([
            keep[2 * i],
            keep[2 * i + 1],
        ]))),
        SampleType::F32 => {
            let b = &keep[4 * i..4 * i + 4];
            f64::from(f32::from_ne_bytes([b[0], b[1], b[2], b[3]]))
        }
    };
    if v.is_nan() { 1.0 } else { v.clamp(0.0, 1.0) }
}

fn write_keep(sample: SampleType, v: f64, keep: &mut [u8], i: usize) {
    let v = v.clamp(0.0, 1.0);
    match sample {
        SampleType::U8 => keep[i] = (v * 255.0).round() as u8,
        SampleType::U16 => {
            keep[2 * i..2 * i + 2].copy_from_slice(&((v * 65535.0).round() as u16).to_ne_bytes())
        }
        SampleType::F16 => {
            keep[2 * i..2 * i + 2].copy_from_slice(&f32_to_f16(v as f32).to_ne_bytes())
        }
        SampleType::F32 => keep[4 * i..4 * i + 4].copy_from_slice(&(v as f32).to_ne_bytes()),
    }
}

/// Half a step of `sample`: what `P`'s alpha + `k` may miss 1 by and still keep a pixel opaque.
fn half_step(sample: SampleType) -> f64 {
    match sample {
        SampleType::U8 => 0.5 / 255.0,
        SampleType::U16 => 0.5 / 65535.0,
        SampleType::F16 | SampleType::F32 => 1e-6,
    }
}

/// Whether pixels of `format` hold the very values `space` blends: 8-bit sRGB color with
/// straight alpha in a perceptual document, the common case. They are then read and written as
/// they are, without decoding and encoding their transfer curve: the same values, much faster.
fn blends_as_stored(format: PixelFormat, space: BlendSpace) -> bool {
    format.sample == SampleType::U8
        && !format.layout.is_gray()
        && format.layout.has_alpha()
        && format.alpha == crate::color::AlphaMode::Straight
        && format.color_space == crate::color::ColorSpace::SRGB
        && space == BlendSpace::Perceptual
}

/// An 8-bit straight RGBA pixel (see [`blends_as_stored`]) as premultiplied blend values.
fn read_stored(px: &[u8]) -> [f64; 4] {
    let a = f64::from(px[3]) / 255.0;
    let v = |i: usize| f64::from(px[i]) / 255.0 * a;
    [v(0), v(1), v(2), a]
}

/// Premultiplied blend values as an 8-bit straight RGBA pixel (see [`blends_as_stored`]),
/// rounded and clamped as the codec writes them.
fn write_stored(values: [f64; 4], out: &mut [u8]) {
    let alpha = values[3].clamp(0.0, 1.0);
    if alpha <= 0.0 {
        out[..4].fill(0);
        return;
    }
    let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    for (n, sample) in out[..3].iter_mut().enumerate() {
        *sample = byte(values[n] / values[3]);
    }
    out[3] = byte(alpha);
}

/// The pixels of a layer of one format as premultiplied working-space colors, and back.
#[derive(Debug, Clone, Copy)]
struct Pixels {
    gray: bool,
    to_working: Mat3,
    from_working: Mat3,
    luma: [f64; 3],
}

impl Pixels {
    fn new(format: PixelFormat) -> Self {
        let space = format.color_space;
        let gray = format.layout.is_gray();
        Self {
            gray,
            // A gray value is its own luminance (as painting reads it, ADR 0027).
            to_working: if gray {
                IDENTITY
            } else {
                space.matrix_to(&WORKING_SPACE)
            },
            from_working: if gray {
                IDENTITY
            } else {
                WORKING_SPACE.matrix_to(&space)
            },
            luma: WORKING_SPACE.primaries.to_xyz()[1],
        }
    }

    fn read(&self, codec: &Codec, px: &[u8]) -> [f64; 4] {
        let (color, alpha) = codec.read_mapped(px, &mut |v| v);
        let color = mat_vec(&self.to_working, color.map(f64::from));
        [color[0], color[1], color[2], f64::from(alpha)]
    }

    fn write(&self, codec: &Codec, [r, g, b, a]: [f64; 4], out: &mut [u8]) {
        let rgb = if self.gray {
            [r * self.luma[0] + g * self.luma[1] + b * self.luma[2]; 3]
        } else {
            mat_vec(&self.from_working, [r, g, b])
        };
        codec.write(rgb.map(|v| v as f32), a as f32, out);
    }
}

/// The valid pixels (width, height) of tile `coord` of an image of `size`.
fn valid_area(size: Size, coord: TileCoord) -> (usize, usize) {
    (
        (size.width - coord.col * TILE_SIZE).min(TILE_SIZE) as usize,
        (size.height - coord.row * TILE_SIZE).min(TILE_SIZE) as usize,
    )
}

fn grid_coords(size: Size) -> impl Iterator<Item = TileCoord> {
    let (columns, rows) = (
        size.width.div_ceil(TILE_SIZE),
        size.height.div_ceil(TILE_SIZE),
    );
    (0..rows).flat_map(move |row| (0..columns).map(move |col| TileCoord { col, row }))
}

fn contains(size: Size, coord: TileCoord) -> bool {
    coord.col < size.width.div_ceil(TILE_SIZE) && coord.row < size.height.div_ceil(TILE_SIZE)
}

/// Encodes and decodes the paint of one entry.
struct PaintPixels {
    /// `P`'s format.
    format: PixelFormat,
    /// `P`'s pixels are the blend values as they are (see [`blends_as_stored`]).
    stored: bool,
    pixels: Pixels,
    codec: Codec,
    keep: SampleType,
    bpp: usize,
    blender: Blender,
}

impl PaintPixels {
    fn new(format: PixelFormat, space: BlendSpace) -> Self {
        let codec = Codec::new(format);
        Self {
            format,
            stored: blends_as_stored(stored_format(format), space),
            pixels: Pixels::new(format),
            bpp: codec.bytes_per_pixel,
            codec,
            keep: keep_sample(format.sample),
            blender: Blender::new(space),
        }
    }

    /// `P` (premultiplied blend-space values) and `k` of pixel `i`.
    fn read(&self, color: &[u8], keep: &[u8], i: usize) -> ([f64; 4], f64) {
        let px = &color[i * self.bpp..(i + 1) * self.bpp];
        if self.stored {
            return (read_stored(px), read_keep(self.keep, keep, i));
        }
        let p = self.pixels.read(&self.codec, px);
        (
            self.blender.encode_premultiplied(&p),
            read_keep(self.keep, keep, i),
        )
    }

    fn write(&self, p: [f64; 4], k: f64, color: &mut [u8], keep: &mut [u8], i: usize) {
        if self.stored {
            write_stored(p, &mut color[i * self.bpp..(i + 1) * self.bpp]);
            write_keep(self.keep, k, keep, i);
            return;
        }
        let p = self.blender.decode_premultiplied(&p);
        self.pixels
            .write(&self.codec, p, &mut color[i * self.bpp..(i + 1) * self.bpp]);
        write_keep(self.keep, k, keep, i);
    }

    /// The identity tiles: `P = 0`, `k = 1`.
    fn identity(&self) -> (Vec<u8>, Vec<u8>) {
        let mut keep = vec![0; TILE_PIXELS * self.keep.bytes() as usize];
        for i in 0..TILE_PIXELS {
            write_keep(self.keep, 1.0, &mut keep, i);
        }
        (vec![0; TILE_PIXELS * self.bpp], keep)
    }

    /// `tile` once its valid pixels (`from`: width, height) become the first ones of a tile
    /// whose valid pixels are `to`: the identity where it had only padding, padded again.
    fn reframed(&self, tile: &PaintTile, from: (usize, usize), to: (usize, usize)) -> PaintTile {
        if from == to {
            return tile.clone();
        }
        let (mut color, mut keep) = self.identity();
        let t = TILE_SIZE as usize;
        let kb = self.keep.bytes() as usize;
        for y in 0..from.1 {
            let (a, b) = (y * t, y * t + from.0);
            color[a * self.bpp..b * self.bpp]
                .copy_from_slice(&tile.color[a * self.bpp..b * self.bpp]);
            keep[a * kb..b * kb].copy_from_slice(&tile.keep[a * kb..b * kb]);
        }
        pad_tile(&mut color, to.0, to.1, self.bpp);
        pad_tile(&mut keep, to.0, to.1, kb);
        let mut out = self.tile(color, keep);
        out.lowers_alpha |= tile.lowers_alpha;
        out
    }

    /// A tile from its padded buffers.
    fn tile(&self, color: Vec<u8>, keep: Vec<u8>) -> PaintTile {
        let limit = 1.0 - half_step(self.keep);
        let lowers_alpha = (0..TILE_PIXELS).any(|i| {
            let alpha = f64::from(self.codec.alpha(&color[i * self.bpp..(i + 1) * self.bpp]));
            alpha + read_keep(self.keep, &keep, i) < limit
        });
        PaintTile {
            color: Arc::from(color),
            keep: Arc::from(keep),
            lowers_alpha,
        }
    }
}

impl PaintEntry {
    /// No paint yet, on a layer of `layer_format` and `size`, blending in `space`.
    pub fn empty(layer_format: PixelFormat, size: Size, space: BlendSpace) -> Self {
        Self {
            size,
            format: paint_format(layer_format),
            space,
            images: OnceLock::new(),
            below: Mutex::new(None),
            tiles: BTreeMap::new(),
        }
    }

    /// Paint read back (e.g. from a file): `P` and `k` of each tile it touched.
    pub fn from_tiles(
        layer_format: PixelFormat,
        size: Size,
        space: BlendSpace,
        tiles: Vec<RawPaintTile>,
    ) -> Result<Self, StackError> {
        let empty = Self::empty(layer_format, size, space);
        let math = empty.math();
        let mut out = BTreeMap::new();
        for (coord, color, keep) in tiles {
            out.insert(
                coord,
                empty.checked(&math, coord, color.to_vec(), keep.to_vec())?,
            );
        }
        Ok(Self {
            tiles: out,
            ..empty
        })
    }

    /// The paint of an image painted the way ADR 0027 did (a `.slop` v6 file): `P` the painted
    /// pixels and `k = 0` where they differ from the original, the identity elsewhere. The
    /// layer then shows exactly the painted pixels.
    pub fn from_painted(
        original: &RasterImage,
        painted: &RasterImage,
        space: BlendSpace,
    ) -> Result<Self, StackError> {
        Self::between(original.format(), original, painted, space)
    }

    /// The paint that turns `before` into `after` (two images of a layer of `layer_format`):
    /// `P` the pixels of `after` and `k = 0` where they differ, the identity elsewhere. What
    /// tools that read pixels write (moved pixels, ADR 0029): the values they took are baked.
    /// Only the tiles that are not shared are compared.
    pub fn between(
        layer_format: PixelFormat,
        before: &RasterImage,
        after: &RasterImage,
        space: BlendSpace,
    ) -> Result<Self, StackError> {
        let size = before.size();
        if after.size() != size {
            return Err(StackError::SizeMismatch);
        }
        let empty = Self::empty(layer_format, size, space);
        let math = empty.math();
        let (painted, original) = (after, before);
        let (before, after) = (&original.levels()[0], &painted.levels()[0]);
        let (old, new) = (
            Codec::new(original.stored_format()),
            Codec::new(painted.stored_format()),
        );
        let same_format = painted.stored_format() == empty.format;
        let (old_bpp, new_bpp) = (old.bytes_per_pixel, new.bytes_per_pixel);
        let mut work: Vec<(TileCoord, Option<PaintTile>)> = grid_coords(size)
            .filter(|&coord| match (before.tile(coord), after.tile(coord)) {
                (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
                _ => false,
            })
            .map(|coord| (coord, None))
            .collect();
        parallel_for_each(&mut work, |(coord, out)| {
            let (Some(a), Some(b)) = (before.tile(*coord), after.tile(*coord)) else {
                return;
            };
            let (mut color, mut keep) = math.identity();
            let (width, height) = valid_area(size, *coord);
            let mut changed = false;
            for y in 0..height {
                for x in 0..width {
                    let i = y * TILE_SIZE as usize + x;
                    let (pa, pb) = (
                        &a[i * old_bpp..(i + 1) * old_bpp],
                        &b[i * new_bpp..(i + 1) * new_bpp],
                    );
                    if old.read_mapped(pa, &mut |v| v) == new.read_mapped(pb, &mut |v| v) {
                        continue;
                    }
                    changed = true;
                    let p = &mut color[i * math.bpp..(i + 1) * math.bpp];
                    if same_format {
                        p.copy_from_slice(pb);
                    } else {
                        math.pixels
                            .write(&math.codec, math.pixels.read(&new, pb), p);
                    }
                    write_keep(math.keep, 0.0, &mut keep, i);
                }
            }
            if changed {
                pad_tile(&mut color, width, height, math.bpp);
                pad_tile(&mut keep, width, height, math.keep.bytes() as usize);
                *out = Some(math.tile(color, keep));
            }
        });
        let tiles = work
            .into_iter()
            .filter_map(|(coord, tile)| tile.map(|t| (coord, t)))
            .collect();
        Ok(Self { tiles, ..empty })
    }

    pub fn size(&self) -> Size {
        self.size
    }

    /// The format of `P`.
    pub fn format(&self) -> PixelFormat {
        self.format
    }

    /// The sample type of `k`.
    pub fn keep_sample(&self) -> SampleType {
        keep_sample(self.format.sample)
    }

    pub fn space(&self) -> BlendSpace {
        self.space
    }

    /// The tiles it touched; every other tile is the identity.
    pub fn tiles(&self) -> &BTreeMap<TileCoord, PaintTile> {
        &self.tiles
    }

    /// The format of `k` as an image: gray samples read as they are.
    pub fn keep_format(&self) -> PixelFormat {
        PixelFormat {
            layout: ChannelLayout::Gray,
            sample: self.keep_sample(),
            color_space: crate::color::ColorSpace::LINEAR_SRGB,
            alpha: crate::color::AlphaMode::Straight,
        }
    }

    /// `P` and `k` as whole images of the layer's size (what files store): every tile it did
    /// not touch is one shared identity tile. Made once.
    pub fn images(&self) -> Result<(Arc<RasterImage>, Arc<RasterImage>), StackError> {
        if let Some(images) = self.images.get() {
            return Ok(images.clone());
        }
        let math = self.math();
        let (color, keep) = math.identity();
        let (color, keep): (Arc<[u8]>, Arc<[u8]>) = (Arc::from(color), Arc::from(keep));
        let coords: Vec<TileCoord> = grid_coords(self.size).collect();
        let pick = |of: fn(&PaintTile) -> &Arc<[u8]>, identity: &Arc<[u8]>| -> Vec<Arc<[u8]>> {
            coords
                .iter()
                .map(|c| {
                    self.tiles
                        .get(c)
                        .map_or_else(|| Arc::clone(identity), |t| Arc::clone(of(t)))
                })
                .collect()
        };
        let images = (
            Arc::new(RasterImage::from_level0_tiles(
                self.size,
                self.format,
                pick(PaintTile::color, &color),
            )?),
            Arc::new(RasterImage::from_level0_tiles(
                self.size,
                self.keep_format(),
                pick(PaintTile::keep, &keep),
            )?),
        );
        Ok(self.images.get_or_init(|| images).clone())
    }

    /// Paint read back from its images ([`Self::images`]) on a layer of `layer_format`.
    pub fn from_images(
        layer_format: PixelFormat,
        color: &RasterImage,
        keep: &RasterImage,
        space: BlendSpace,
    ) -> Result<Self, StackError> {
        let empty = Self::empty(layer_format, color.size(), space);
        if keep.size() != color.size() {
            return Err(StackError::SizeMismatch);
        }
        if color.format() != empty.format || keep.format() != empty.keep_format() {
            return Err(StackError::FormatMismatch);
        }
        let math = empty.math();
        let (identity_color, identity_keep) = math.identity();
        let (colors, keeps) = (&color.levels()[0], &keep.levels()[0]);
        let mut tiles = Vec::new();
        for coord in grid_coords(empty.size) {
            let (Some(c), Some(k)) = (colors.tile(coord), keeps.tile(coord)) else {
                return Err(StackError::TileOutside(coord));
            };
            if **c != *identity_color || **k != *identity_keep {
                tiles.push((coord, Arc::clone(c), Arc::clone(k)));
            }
        }
        Self::from_tiles(layer_format, empty.size, space, tiles)
    }

    /// Bytes its tiles hold (tiles shared between positions counted once).
    pub fn memory_bytes(&self) -> u64 {
        let mut seen = HashSet::new();
        self.tiles
            .values()
            .flat_map(|t| [&t.color, &t.keep])
            .filter(|tile| seen.insert(tile.as_ptr() as usize))
            .map(|tile| tile.len() as u64)
            .sum()
    }

    /// Whether it changes nothing.
    pub fn is_identity(&self) -> bool {
        self.tiles.is_empty()
    }

    /// Whether it can lower the alpha of an opaque pixel (the layer then needs an alpha
    /// channel).
    pub fn lowers_alpha(&self) -> bool {
        self.tiles.values().any(|t| t.lowers_alpha)
    }

    fn math(&self) -> PaintPixels {
        PaintPixels::new(self.format, self.space)
    }

    fn checked(
        &self,
        math: &PaintPixels,
        coord: TileCoord,
        color: Vec<u8>,
        keep: Vec<u8>,
    ) -> Result<PaintTile, StackError> {
        if !contains(self.size, coord) {
            return Err(StackError::TileOutside(coord));
        }
        if color.len() != TILE_PIXELS * math.bpp
            || keep.len() != TILE_PIXELS * math.keep.bytes() as usize
        {
            return Err(StackError::TileLength);
        }
        Ok(math.tile(color, keep))
    }

    /// Tile `coord` of this paint with `op` applied at `amount(x, y)` (clamped to `[0, 1]`) to
    /// each valid pixel of it: a stroke recomputes the tiles it reaches from the paint it
    /// started from. The tile must be inside the layer.
    pub fn painted_tile(
        &self,
        coord: TileCoord,
        op: PaintOp,
        amount: impl Fn(usize, usize) -> f32,
    ) -> PaintTile {
        let math = self.math();
        let (mut color, mut keep) = match self.tiles.get(&coord) {
            Some(tile) => (tile.color.to_vec(), tile.keep.to_vec()),
            None => math.identity(),
        };
        let paint = op_color(&math, op);
        let (width, height) = valid_area(self.size, coord);
        // Erased once, a tile keeps asking for an alpha channel, so that the layer's format
        // does not depend on how small the erasing was.
        let mut erased = self.tiles.get(&coord).is_some_and(|t| t.lowers_alpha);
        for y in 0..height {
            for x in 0..width {
                let a = f64::from(amount(x, y));
                if a.is_nan() || a <= 0.0 {
                    continue;
                }
                let a = a.min(1.0);
                erased |= op == PaintOp::Erase;
                let i = y * TILE_SIZE as usize + x;
                let (p, k) = math.read(&color, &keep, i);
                let (p, k) = lay(op, paint, a, p, k);
                math.write(p, k, &mut color, &mut keep, i);
            }
        }
        pad_tile(&mut color, width, height, math.bpp);
        pad_tile(&mut keep, width, height, math.keep.bytes() as usize);
        let mut tile = math.tile(color, keep);
        tile.lowers_alpha |= erased;
        tile
    }

    /// This paint with `tiles` replaced (tiles made by [`Self::painted_tile`] of a paint of the
    /// same format, or read back).
    pub fn with_tiles(&self, tiles: Vec<(TileCoord, PaintTile)>) -> Result<Self, StackError> {
        let math = self.math();
        let mut out = self.tiles.clone();
        for (coord, tile) in tiles {
            if !contains(self.size, coord) {
                return Err(StackError::TileOutside(coord));
            }
            if tile.color.len() != TILE_PIXELS * math.bpp
                || tile.keep.len() != TILE_PIXELS * math.keep.bytes() as usize
            {
                return Err(StackError::TileLength);
            }
            out.insert(coord, tile);
        }
        Ok(Self {
            tiles: out,
            size: self.size,
            format: self.format,
            space: self.space,
            images: OnceLock::new(),
            below: Mutex::new(None),
        })
    }

    /// Whether `above` can merge with this paint (below it): same layer and blend space.
    fn merges_with(&self, above: &PaintEntry) -> bool {
        self.size == above.size && self.format == above.format && self.space == above.space
    }

    /// This paint then `above`, as one: `P₂ + k₂·P₁`, `k₂·k₁`.
    fn merged(&self, above: &PaintEntry) -> PaintEntry {
        let math = self.math();
        let coords: BTreeSet<TileCoord> = self
            .tiles
            .keys()
            .chain(above.tiles.keys())
            .copied()
            .collect();
        let mut work: Vec<(TileCoord, Option<PaintTile>)> =
            coords.into_iter().map(|coord| (coord, None)).collect();
        parallel_for_each(&mut work, |(coord, out)| {
            let tile = match (self.tiles.get(coord), above.tiles.get(coord)) {
                (Some(below), Some(top)) => {
                    let (mut color, mut keep) = math.identity();
                    let (width, height) = valid_area(self.size, *coord);
                    for y in 0..height {
                        for x in 0..width {
                            let i = y * TILE_SIZE as usize + x;
                            let (p1, k1) = math.read(&below.color, &below.keep, i);
                            let (p2, k2) = math.read(&top.color, &top.keep, i);
                            let p = std::array::from_fn(|n| p2[n] + k2 * p1[n]);
                            math.write(p, k2 * k1, &mut color, &mut keep, i);
                        }
                    }
                    pad_tile(&mut color, width, height, math.bpp);
                    pad_tile(&mut keep, width, height, math.keep.bytes() as usize);
                    let mut tile = math.tile(color, keep);
                    tile.lowers_alpha |= below.lowers_alpha || top.lowers_alpha;
                    tile
                }
                (Some(only), None) | (None, Some(only)) => only.clone(),
                (None, None) => return,
            };
            *out = Some(tile);
        });
        PaintEntry {
            size: self.size,
            format: self.format,
            space: self.space,
            images: OnceLock::new(),
            below: Mutex::new(None),
            tiles: work
                .into_iter()
                .filter_map(|(coord, tile)| tile.map(|t| (coord, t)))
                .collect(),
        }
    }
}

/// `op`'s color as premultiplied blend-space values (`None` for the erasers).
fn op_color(math: &PaintPixels, op: PaintOp) -> Option<[f64; 4]> {
    match op {
        PaintOp::Color(c) => Some(math.blender.encode_premultiplied(&[
            f64::from(c.r),
            f64::from(c.g),
            f64::from(c.b),
            1.0,
        ])),
        PaintOp::Erase | PaintOp::Restore => None,
    }
}

/// `P` and `k` once `op` (its color from [`op_color`]) is laid at `a` in `(0, 1]` over them.
fn lay(op: PaintOp, color: Option<[f64; 4]>, a: f64, p: [f64; 4], k: f64) -> ([f64; 4], f64) {
    match (op, color) {
        (PaintOp::Color(_), Some(c)) => (
            std::array::from_fn(|n| a * c[n] + (1.0 - a) * p[n]),
            (1.0 - a) * k,
        ),
        (PaintOp::Restore, _) => (p.map(|v| (1.0 - a) * v), (1.0 - a) * k + a),
        _ => (p.map(|v| (1.0 - a) * v), (1.0 - a) * k),
    }
}

/// The tiles of a layer that one paint or one effect step reaches.
#[derive(Debug, Clone, PartialEq)]
enum Footprint {
    All,
    Tiles(BTreeSet<TileCoord>),
}

impl Footprint {
    fn of_paint(paint: &PaintEntry) -> Self {
        Self::Tiles(paint.tiles.keys().copied().collect())
    }

    /// The tiles under the selection's bounds (one pixel more around them), all without one.
    fn of_effect(effect: &Effect, size: Size) -> Self {
        let Some(selection) = &effect.selection else {
            return Self::All;
        };
        let Some(bounds) = crate::selection::bounds(selection.image()) else {
            return Self::Tiles(BTreeSet::new());
        };
        let Some(to_image) = effect.to_document.inverse() else {
            return Self::Tiles(BTreeSet::new());
        };
        let [x0, y0, x1, y1] = to_image.map_rect([
            f64::from(bounds.x),
            f64::from(bounds.y),
            bounds.right() as f64,
            bounds.bottom() as f64,
        ]);
        let (w, h) = (f64::from(size.width), f64::from(size.height));
        let (x0, y0) = ((x0 - 1.0).clamp(0.0, w), (y0 - 1.0).clamp(0.0, h));
        let (x1, y1) = ((x1 + 1.0).clamp(0.0, w), (y1 + 1.0).clamp(0.0, h));
        let t = f64::from(TILE_SIZE);
        let mut tiles = BTreeSet::new();
        if x0 < x1 && y0 < y1 {
            for row in (y0 / t) as u32..(y1 / t).ceil() as u32 {
                for col in (x0 / t) as u32..(x1 / t).ceil() as u32 {
                    tiles.insert(TileCoord { col, row });
                }
            }
        }
        Self::Tiles(tiles)
    }

    fn reaches(&self, coord: TileCoord) -> bool {
        match self {
            Self::All => true,
            Self::Tiles(tiles) => tiles.contains(&coord),
        }
    }
}

/// One paint, or one step of an effect entry: what the stack evaluates in order, and what it
/// compares to know which tiles changed.
#[derive(Debug, Clone, Copy)]
enum Atom<'a> {
    Paint(&'a Arc<PaintEntry>),
    Effect(&'a Arc<Effect>),
}

impl Atom<'_> {
    fn address(self) -> usize {
        match self {
            Atom::Paint(p) => Arc::as_ptr(p) as *const () as usize,
            Atom::Effect(e) => Arc::as_ptr(e) as *const () as usize,
        }
    }

    fn footprint(self, size: Size) -> Footprint {
        match self {
            Atom::Paint(p) => Footprint::of_paint(p),
            Atom::Effect(e) => Footprint::of_effect(e, size),
        }
    }
}

fn atoms(entries: &[Entry]) -> Vec<Atom<'_>> {
    let mut out = Vec::new();
    for entry in entries {
        match entry {
            Entry::Paint(p) => out.push(Atom::Paint(p)),
            Entry::Effect(e) => out.extend(e.steps.iter().map(Atom::Effect)),
        }
    }
    out
}

/// One atom, ready to evaluate.
enum Step<'a> {
    Paint {
        paint: &'a PaintEntry,
        math: PaintPixels,
    },
    Effect {
        effect: &'a Effect,
        prepared: Prepared,
        blender: Blender,
        selection: Option<(&'a RasterImage, Codec)>,
        footprint: Footprint,
    },
}

impl Step<'_> {
    fn reaches(&self, coord: TileCoord) -> bool {
        match self {
            Step::Paint { paint, .. } => paint.tiles.contains_key(&coord),
            Step::Effect { footprint, .. } => footprint.reaches(coord),
        }
    }
}

/// Evaluates tiles of a stack's result.
struct Evaluator<'a> {
    size: Size,
    original: &'a RasterImage,
    pixels: Pixels,
    /// The original's codec, and the result's.
    source: Codec,
    target: Codec,
    /// The result's stored format.
    format: PixelFormat,
    /// The original's tiles must be converted to the result's format (a gray layer given an
    /// alpha channel).
    convert: bool,
    steps: Vec<Step<'a>>,
}

impl<'a> Evaluator<'a> {
    /// The evaluator of `atoms` over `stack`'s original, into `format` (the stack's, or the
    /// format a stroke shows).
    fn new(stack: &'a LayerStack, atoms: &[Atom<'a>], format: PixelFormat) -> Self {
        let original = stack.original.as_ref();
        let size = original.size();
        let steps = atoms
            .iter()
            .map(|&atom| match atom {
                Atom::Paint(paint) => Step::Paint {
                    paint,
                    math: paint.math(),
                },
                Atom::Effect(effect) => Step::Effect {
                    effect,
                    prepared: effect.adjustment.prepare(),
                    blender: Blender::new(effect.space),
                    selection: effect.selection.as_ref().map(|s| {
                        let image = s.image().as_ref();
                        (image, Codec::new(image.stored_format()))
                    }),
                    footprint: Footprint::of_effect(effect, size),
                },
            })
            .collect();
        Self {
            size,
            original,
            pixels: Pixels::new(format),
            source: Codec::new(original.stored_format()),
            target: Codec::new(stored_format(format)),
            format: stored_format(format),
            convert: original.stored_format() != stored_format(format),
            steps,
        }
    }

    /// Tile `coord` of the result: from `start` (the result of the steps before `from`), or
    /// from the original when `None`. Shared with it when no step reaches the tile.
    fn tile(&self, coord: TileCoord, from: usize, start: Option<&Arc<[u8]>>) -> Arc<[u8]> {
        let current = match start {
            Some(tile) => Arc::clone(tile),
            None => self.original_tile(coord),
        };
        let mut buffer: Option<Vec<u8>> = None;
        for step in &self.steps[from..] {
            if step.reaches(coord) {
                let rows = buffer.get_or_insert_with(|| current.to_vec());
                self.apply_rows(step, coord, rows, 0);
            }
        }
        self.finished(coord, buffer).unwrap_or(current)
    }

    /// [`Self::tile`] from the original, each step on every core (bands of rows): one tile
    /// as fast as many, for a stroke reaching a new tile.
    fn tile_on_every_core(&self, coord: TileCoord) -> Arc<[u8]> {
        let current = self.original_tile(coord);
        let row_bytes = TILE_SIZE as usize * self.target.bytes_per_pixel;
        let height = valid_area(self.size, coord).1;
        let mut buffer: Option<Vec<u8>> = None;
        for step in &self.steps {
            if step.reaches(coord) {
                let rows = buffer.get_or_insert_with(|| current.to_vec());
                let mut bands: Vec<(usize, &mut [u8])> = rows[..height * row_bytes]
                    .chunks_mut(BAND_ROWS * row_bytes)
                    .enumerate()
                    .map(|(n, band)| (n * BAND_ROWS, band))
                    .collect();
                parallel_for_each(&mut bands, |(first, band)| {
                    self.apply_rows(step, coord, band, *first);
                });
            }
        }
        self.finished(coord, buffer).unwrap_or(current)
    }

    /// A tile computed into `buffer`, padded.
    fn finished(&self, coord: TileCoord, buffer: Option<Vec<u8>>) -> Option<Arc<[u8]>> {
        let mut buffer = buffer?;
        let (width, height) = valid_area(self.size, coord);
        pad_tile(&mut buffer, width, height, self.target.bytes_per_pixel);
        Some(Arc::from(buffer))
    }

    fn original_tile(&self, coord: TileCoord) -> Arc<[u8]> {
        let Some(tile) = self.original.levels()[0].tile(coord) else {
            return Arc::from(vec![0; TILE_PIXELS * self.target.bytes_per_pixel]);
        };
        if !self.convert {
            return Arc::clone(tile);
        }
        let (from, to) = (self.source.bytes_per_pixel, self.target.bytes_per_pixel);
        let mut out = vec![0; TILE_PIXELS * to];
        for i in 0..TILE_PIXELS {
            let (color, alpha) = self
                .source
                .read_mapped(&tile[i * from..(i + 1) * from], &mut |v| v);
            self.target
                .write(color, alpha, &mut out[i * to..(i + 1) * to]);
        }
        Arc::from(out)
    }

    /// `step` applied in place to `rows`, whole rows of tile `coord` from row `first` on (its
    /// valid pixels; the caller pads).
    fn apply_rows(&self, step: &Step<'_>, coord: TileCoord, rows: &mut [u8], first: usize) {
        let bpp = self.target.bytes_per_pixel;
        let t = TILE_SIZE as usize;
        let (width, height) = valid_area(self.size, coord);
        let last = (first + rows.len() / (t * bpp)).min(height);
        match step {
            Step::Paint { paint, math } => {
                let Some(tile) = paint.tiles.get(&coord) else {
                    return;
                };
                let mut below = [0u8; MAX_PIXEL_BYTES];
                for y in first..last {
                    for x in 0..width {
                        let px = &mut rows[((y - first) * t + x) * bpp..][..bpp];
                        below[..bpp].copy_from_slice(px);
                        self.paint_pixel(
                            math,
                            &tile.color,
                            &tile.keep,
                            y * t + x,
                            &below[..bpp],
                            px,
                        );
                    }
                }
            }
            Step::Effect {
                effect,
                prepared,
                blender,
                selection,
                ..
            } => {
                let (x0, y0) = (
                    f64::from(coord.col * TILE_SIZE),
                    f64::from(coord.row * TILE_SIZE),
                );
                // Adjustments in linear light decode the pixels whatever they are.
                let stored = blends_as_stored(self.format, effect.space)
                    && !prepared.adjustment().is_linear();
                for y in first..last {
                    for x in 0..width {
                        let coverage = match selection {
                            Some((image, codec)) => {
                                let (dx, dy) = effect
                                    .to_document
                                    .apply(x0 + x as f64 + 0.5, y0 + y as f64 + 0.5);
                                f64::from(MaskReader { image, codec }.at(dx.floor(), dy.floor()))
                            }
                            None => 1.0,
                        };
                        if coverage <= 0.0 {
                            continue;
                        }
                        let px = &mut rows[((y - first) * t + x) * bpp..][..bpp];
                        if stored {
                            // The straight color is the blend values: adjusted, then mixed by
                            // the coverage (the same alpha on both sides).
                            if px[3] == 0 {
                                continue;
                            }
                            let below = [px[0], px[1], px[2]].map(|v| f64::from(v) / 255.0);
                            let adjusted = prepared.apply(below);
                            let coverage = coverage.min(1.0);
                            for (n, sample) in px[..3].iter_mut().enumerate() {
                                let v = below[n] + (adjusted[n] - below[n]) * coverage;
                                *sample = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                            }
                            continue;
                        }
                        let b = self.pixels.read(&self.target, px);
                        let r = blender.adjust(prepared, &b, coverage);
                        self.pixels.write(&self.target, r, px);
                    }
                }
            }
        }
    }

    /// Pixel `i` of the paint (`color`, `keep`: `P` and `k` tiles of `math`'s format) over
    /// `below` (a pixel of the result's format), into `out`.
    fn paint_pixel(
        &self,
        math: &PaintPixels,
        color: &[u8],
        keep: &[u8],
        i: usize,
        below: &[u8],
        out: &mut [u8],
    ) {
        let k = read_keep(math.keep, keep, i);
        let p_bytes = &color[i * math.bpp..(i + 1) * math.bpp];
        if k >= 1.0 && math.codec.alpha(p_bytes) <= 0.0 {
            out.copy_from_slice(below);
            return;
        }
        // `P` copies as is where it replaces everything (`k = 0`), in the same format.
        if k <= 0.0 && stored_format(math.format) == self.format {
            out.copy_from_slice(p_bytes);
            return;
        }
        let (p, k) = math.read(color, keep, i);
        let stored = blends_as_stored(self.format, math.blender.space());
        let b = if stored {
            read_stored(below)
        } else {
            math.blender
                .encode_premultiplied(&self.pixels.read(&self.target, below))
        };
        let r = std::array::from_fn(|n| p[n] + k * b[n]);
        if stored {
            write_stored(r, out);
            return;
        }
        let r = math.blender.decode_premultiplied(&r);
        self.pixels.write(&self.target, r, out);
    }

    /// The tiles `coords` of the result, evaluated from scratch, on every core.
    fn tiles(&self, coords: Vec<TileCoord>) -> Vec<(TileCoord, Arc<[u8]>)> {
        let mut work: Vec<(TileCoord, Option<Arc<[u8]>>)> =
            coords.into_iter().map(|c| (c, None)).collect();
        parallel_for_each(&mut work, |(coord, out)| {
            *out = Some(self.tile(*coord, 0, None));
        });
        work.into_iter()
            .filter_map(|(coord, tile)| tile.map(|t| (coord, t)))
            .collect()
    }
}

impl PartialEq for LayerStack {
    fn eq(&self, other: &Self) -> bool {
        // Immutable and shared: the same allocations, the same stack.
        Arc::ptr_eq(&self.original, &other.original) && self.entries == other.entries
    }
}

impl LayerStack {
    /// A layer's original pixels, nothing applied yet.
    pub fn new(original: Arc<RasterImage>) -> Self {
        Self {
            original,
            entries: Vec::new(),
        }
    }

    /// A stack read back (e.g. from a file): every paint fits the layer, every effect is valid.
    pub fn with_entries(
        original: Arc<RasterImage>,
        entries: Vec<Entry>,
    ) -> Result<Self, StackError> {
        let stack = Self {
            original,
            entries: Vec::new(),
        };
        for entry in &entries {
            stack.check(entry)?;
        }
        Ok(Self { entries, ..stack })
    }

    fn check(&self, entry: &Entry) -> Result<(), StackError> {
        match entry {
            Entry::Paint(paint) => {
                if paint.size != self.original.size() {
                    return Err(StackError::SizeMismatch);
                }
                if paint.format != paint_format(self.original.format()) {
                    return Err(StackError::FormatMismatch);
                }
            }
            Entry::Effect(effect) => {
                if effect.steps.is_empty() || effect.steps.iter().any(|s| !s.is_valid()) {
                    return Err(StackError::InvalidEffect);
                }
            }
        }
        Ok(())
    }

    pub fn original(&self) -> &Arc<RasterImage> {
        &self.original
    }

    /// Bottom to top.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The top entry when it is paint: what a new stroke continues.
    pub fn top_paint(&self) -> Option<&Arc<PaintEntry>> {
        match self.entries.last() {
            Some(Entry::Paint(paint)) => Some(paint),
            _ => None,
        }
    }

    /// The stack with `paint` as its top paint: in place of the top entry when it is paint (a
    /// stroke continues it), on top otherwise. Paint that changes nothing is dropped.
    pub fn with_top_paint(&self, paint: Arc<PaintEntry>) -> Result<Self, StackError> {
        let entry = Entry::Paint(paint);
        self.check(&entry)?;
        let mut entries = self.entries.clone();
        if self.top_paint().is_some() {
            entries.pop();
        }
        if matches!(&entry, Entry::Paint(p) if !p.is_identity()) {
            entries.push(entry);
        }
        Ok(Self {
            original: Arc::clone(&self.original),
            entries,
        })
    }

    /// The stack with the paint that turns `before` into `after` on top (both of this layer's
    /// size; see [`PaintEntry::between`]), continuing the top paint when it can: what moving
    /// selected pixels leaves.
    pub fn with_painted(
        &self,
        before: &RasterImage,
        after: &RasterImage,
        space: BlendSpace,
    ) -> Result<Self, StackError> {
        let delta = PaintEntry::between(self.original.format(), before, after, space)?;
        let paint = match self.top_paint() {
            Some(top) if top.merges_with(&delta) => top.merged(&delta),
            _ => {
                let mut entries = self.entries.clone();
                let entry = Entry::Paint(Arc::new(delta));
                self.check(&entry)?;
                entries.push(entry);
                return Ok(Self {
                    original: Arc::clone(&self.original),
                    entries,
                });
            }
        };
        self.with_top_paint(Arc::new(paint))
    }

    /// The stack of the layer grown by `offset` whole tiles (columns, rows) before its pixels
    /// to `size` (painting beyond its bounds, ADR 0027): the original given an alpha channel
    /// and grown, transparent around; the paint moved with it; the effects' selections still
    /// read where they were applied.
    pub fn grown(&self, offset: (u32, u32), size: Size) -> Result<Self, StackError> {
        let with_alpha = match self.original.with_alpha() {
            Some(converted) => Arc::new(converted?),
            None => Arc::clone(&self.original),
        };
        let original = Arc::new(
            with_alpha
                .grown(offset, size)
                .ok_or(StackError::SizeMismatch)??,
        );
        let (left, top) = offset;
        let t = f64::from(TILE_SIZE);
        let shift = Affine::translation(-f64::from(left) * t, -f64::from(top) * t);
        let format = paint_format(original.format());
        let old_size = self.original.size();
        let entries = self
            .entries
            .iter()
            .map(|entry| match entry {
                Entry::Paint(paint) => Entry::Paint(Arc::new(PaintEntry {
                    size,
                    format,
                    space: paint.space,
                    images: OnceLock::new(),
                    below: Mutex::new(None),
                    tiles: paint
                        .tiles
                        .iter()
                        .map(|(coord, tile)| {
                            let moved = TileCoord {
                                col: coord.col + left,
                                row: coord.row + top,
                            };
                            let math = paint.math();
                            (
                                moved,
                                math.reframed(
                                    tile,
                                    valid_area(old_size, *coord),
                                    valid_area(size, moved),
                                ),
                            )
                        })
                        .collect(),
                })),
                Entry::Effect(effect) => Entry::Effect(Arc::new(EffectEntry {
                    steps: effect
                        .steps
                        .iter()
                        .map(|step| {
                            Arc::new(Effect {
                                to_document: shift.then(step.to_document),
                                ..(**step).clone()
                            })
                        })
                        .collect(),
                })),
            })
            .collect();
        Self::with_entries(original, entries)
    }

    /// The stack with `effect` applied on top: an entry of its own, or joining the top entry
    /// when it holds effects of the same kind (contiguous entries never are alike); an Invert
    /// on an Invert cancels it.
    pub fn with_effect(&self, effect: Effect) -> Result<Self, StackError> {
        let added = EffectEntry::new(vec![Arc::new(effect)])?;
        let mut entries = self.entries.clone();
        match entries.last() {
            Some(Entry::Effect(top)) if top.kind() == added.kind() => {
                let steps = joined(top, &added);
                entries.pop();
                if !steps.is_empty() {
                    entries.push(Entry::Effect(Arc::new(EffectEntry::new(steps)?)));
                }
            }
            _ => entries.push(Entry::Effect(Arc::new(added))),
        }
        Ok(Self {
            original: Arc::clone(&self.original),
            entries,
        })
    }

    /// The stack without entry `index`; the neighbours that become alike merge: two paints
    /// exactly, two effects of the same kind as one entry, two Inverts cancel.
    pub fn without(&self, index: usize) -> Result<Self, StackError> {
        if index >= self.entries.len() {
            return Err(StackError::IndexOutOfRange(index));
        }
        let mut entries = self.entries.clone();
        entries.remove(index);
        let mut seam = index;
        while seam > 0 && seam < entries.len() {
            match (&entries[seam - 1], &entries[seam]) {
                (Entry::Paint(below), Entry::Paint(above)) if below.merges_with(above) => {
                    let merged = Entry::Paint(Arc::new(below.merged(above)));
                    entries.splice(seam - 1..=seam, [merged]);
                    break;
                }
                (Entry::Effect(below), Entry::Effect(above)) if below.kind() == above.kind() => {
                    let steps = joined(below, above);
                    if steps.is_empty() {
                        entries.drain(seam - 1..=seam);
                        seam -= 1;
                    } else {
                        let merged = Entry::Effect(Arc::new(EffectEntry::new(steps)?));
                        entries.splice(seam - 1..=seam, [merged]);
                        break;
                    }
                }
                _ => break,
            }
        }
        Ok(Self {
            original: Arc::clone(&self.original),
            entries,
        })
    }

    /// The format of the result: the original's, with an alpha channel when paint can lower
    /// alpha (shared tiles for RGB, which is stored with one).
    pub fn format(&self) -> PixelFormat {
        let format = self.original.format();
        let lowers_alpha = self.entries.iter().any(|e| match e {
            Entry::Paint(p) => p.lowers_alpha(),
            Entry::Effect(_) => false,
        });
        if format.layout.has_alpha() || !lowers_alpha {
            return format;
        }
        paint_format(format)
    }

    /// The result, evaluated from the original: every tile no entry reaches is the original's.
    pub fn evaluate(&self) -> Result<Arc<RasterImage>, StackError> {
        let format = self.format();
        if self.entries.is_empty() && format == self.original.format() {
            return Ok(Arc::clone(&self.original));
        }
        let atoms = atoms(&self.entries);
        let evaluator = Evaluator::new(self, &atoms, format);
        let size = self.original.size();
        let tiles = evaluator
            .tiles(grid_coords(size).collect())
            .into_iter()
            .map(|(_, tile)| tile)
            .collect();
        Ok(Arc::new(RasterImage::from_level0_tiles(
            size, format, tiles,
        )?))
    }

    /// The result, from `shown`, the result of `before` (an earlier state of this stack):
    /// entries added on top apply to `shown`'s tiles; otherwise only the tiles that the entries
    /// added or removed reach are evaluated again. The same pixels as [`Self::evaluate`].
    pub fn reevaluate(
        &self,
        before: &LayerStack,
        shown: &Arc<RasterImage>,
    ) -> Result<Arc<RasterImage>, StackError> {
        let format = self.format();
        if !Arc::ptr_eq(&self.original, &before.original)
            || shown.format() != format
            || shown.size() != self.original.size()
        {
            return self.evaluate();
        }
        if self.entries.is_empty() && format == self.original.format() {
            return Ok(Arc::clone(&self.original));
        }
        let size = self.original.size();
        let (old, new) = (atoms(&before.entries), atoms(&self.entries));
        let common = old
            .iter()
            .zip(&new)
            .take_while(|(a, b)| a.address() == b.address())
            .count();
        let evaluator = Evaluator::new(self, &new, format);
        let replaced: Vec<(TileCoord, Arc<[u8]>)> = if common == old.len() {
            // Added on top: from what is shown.
            let mut reached = Footprint::Tiles(BTreeSet::new());
            for atom in &new[common..] {
                reached = union(reached, atom.footprint(size));
            }
            let coords = coords_of(&reached, size);
            let level = &shown.levels()[0];
            let mut work: Vec<(TileCoord, Option<Arc<[u8]>>)> =
                coords.into_iter().map(|c| (c, None)).collect();
            parallel_for_each(&mut work, |(coord, out)| {
                *out = level
                    .tile(*coord)
                    .map(|tile| evaluator.tile(*coord, common, Some(tile)));
            });
            work.into_iter()
                .filter_map(|(coord, tile)| tile.map(|t| (coord, t)))
                .collect()
        } else {
            let kept: BTreeSet<usize> = new.iter().map(|a| a.address()).collect();
            let was: BTreeSet<usize> = old.iter().map(|a| a.address()).collect();
            let mut reached = Footprint::Tiles(BTreeSet::new());
            for atom in old.iter().filter(|a| !kept.contains(&a.address())) {
                reached = union(reached, atom.footprint(size));
            }
            for atom in new.iter().filter(|a| !was.contains(&a.address())) {
                reached = union(reached, atom.footprint(size));
            }
            evaluator.tiles(coords_of(&reached, size))
        };
        if replaced.is_empty() {
            return Ok(Arc::clone(shown));
        }
        Ok(Arc::new(shown.with_tiles(replaced)?))
    }
}

/// Paint being laid on top of a stack, tile by tile: a stroke's frames, Fill, Delete. It
/// continues the top paint (or starts one) and keeps the paint laid so far; what is below that
/// paint is evaluated once per tile and cached for the whole stroke, so that a frame computes
/// only the pixels that changed, as painting an image does (ADR 0027).
#[derive(Debug)]
pub struct TopPaint {
    /// The stack without the paint being laid.
    below: LayerStack,
    /// The paint it continues (empty if none).
    start: PaintEntry,
    /// The paint laid so far: the tiles changed since the start.
    painted: BTreeMap<TileCoord, PaintTile>,
    /// What the layer shows meanwhile.
    format: PixelFormat,
    /// What the layer showed when the paint started: what is below it where the paint it
    /// continues did not reach.
    shown: Option<Arc<RasterImage>>,
    /// Tiles of what is below the paint, evaluated once each for the whole stroke.
    cache: HashMap<TileCoord, Arc<[u8]>>,
}

impl TopPaint {
    /// Paint on `stack` blending in `space`, whose result the layer shows as `shown`; `erase`:
    /// the paint may lower alpha (the layer shows an alpha channel from the start).
    pub fn new(
        stack: &LayerStack,
        shown: &Arc<RasterImage>,
        space: BlendSpace,
        erase: bool,
    ) -> Self {
        let (below, start) = match stack.top_paint() {
            Some(top) if top.space == space => (
                LayerStack {
                    original: Arc::clone(&stack.original),
                    entries: stack.entries[..stack.entries.len() - 1].to_vec(),
                },
                PaintEntry {
                    size: top.size,
                    format: top.format,
                    space,
                    images: OnceLock::new(),
                    below: Mutex::new(None),
                    tiles: top.tiles.clone(),
                },
            ),
            _ => (
                stack.clone(),
                PaintEntry::empty(stack.original.format(), stack.original.size(), space),
            ),
        };
        let format = stack.format();
        let format = if erase && !format.layout.has_alpha() {
            paint_format(format)
        } else {
            format
        };
        // What is below the paint: what the layer shows where the paint it continues (if any)
        // did not reach, else what that paint kept, while it still applies.
        let continues = below.entries.len() < stack.entries.len();
        let shown = (shown.format() == format && shown.size() == stack.original.size())
            .then(|| Arc::clone(shown));
        let cache = match stack.top_paint() {
            Some(top) if continues => top
                .below
                .lock()
                .ok()
                .and_then(|kept| kept.clone())
                .filter(|kept| kept.stack == below && kept.format == format)
                .map(|kept| kept.tiles)
                .unwrap_or_default(),
            _ => HashMap::new(),
        };
        Self {
            below,
            start,
            painted: BTreeMap::new(),
            format,
            shown,
            cache,
        }
    }

    /// The format of the tiles shown while painting.
    pub fn format(&self) -> PixelFormat {
        self.format
    }

    /// Whether some paint was laid.
    pub fn has_paint(&self) -> bool {
        !self.painted.is_empty()
    }

    /// Lay `op` at `amount(coord, x, y)` on the pixels `area` (`[x0, y0, x1, y1)` within the
    /// tile) of each tile of `dirty`, from the paint it started from: the tiles the layer then
    /// shows, from those of `shown` (what it showed so far, of [`Self::format`]), on every core
    /// (bands of rows). Amounts only grow during a stroke, so a pixel not reached yet keeps its
    /// start.
    pub fn lay(
        &mut self,
        dirty: &[(TileCoord, [usize; 4])],
        op: PaintOp,
        amount: impl Fn(TileCoord, usize, usize) -> f32 + Sync,
        shown: &RasterImage,
    ) -> Vec<(TileCoord, Arc<[u8]>)> {
        let atoms = atoms(&self.below.entries);
        let evaluator = Evaluator::new(&self.below, &atoms, self.format);
        let math = self.start.math();
        let color = op_color(&math, op);
        let level = &shown.levels()[0];
        let size = self.start.size;
        // Each tile with its area and what it shows so far.
        type Dirty<'a> = (TileCoord, [usize; 4], &'a Arc<[u8]>);
        let dirty: Vec<Dirty<'_>> = dirty
            .iter()
            .filter(|(c, _)| contains(size, *c))
            .filter_map(|&(c, area)| Some((c, area, level.tile(c)?)))
            .collect();
        // What is below the paint: evaluated once per tile for the whole stroke.
        let shown_below = self.shown.as_ref().map(|image| &image.levels()[0]);
        let below: Vec<Arc<[u8]>> = dirty
            .iter()
            .map(|(coord, _, _)| {
                let untouched = !self.start.tiles.contains_key(coord);
                Arc::clone(self.cache.entry(*coord).or_insert_with(|| {
                    match shown_below
                        .filter(|_| untouched)
                        .and_then(|level| level.tile(*coord))
                    {
                        Some(tile) => Arc::clone(tile),
                        None => evaluator.tile_on_every_core(*coord),
                    }
                }))
            })
            .collect();
        let identity = math.identity();
        let starts: Vec<(&[u8], &[u8], bool)> = dirty
            .iter()
            .map(|(coord, _, _)| match self.start.tiles.get(coord) {
                Some(t) => (&t.color[..], &t.keep[..], t.lowers_alpha),
                None => (&identity.0[..], &identity.1[..], false),
            })
            .collect();
        // Each tile's `P`, `k`, what it shows, and whether it was erased.
        type Buffers = (Vec<u8>, Vec<u8>, Vec<u8>, bool);
        let mut buffers: Vec<Buffers> = dirty
            .iter()
            .zip(&starts)
            .map(
                |((coord, _, current), start)| match self.painted.get(coord) {
                    Some(t) => (
                        t.color.to_vec(),
                        t.keep.to_vec(),
                        current.to_vec(),
                        t.lowers_alpha,
                    ),
                    None => (
                        start.0.to_vec(),
                        start.1.to_vec(),
                        current.to_vec(),
                        start.2,
                    ),
                },
            )
            .collect();
        let t = TILE_SIZE as usize;
        let (bpp, kb, sb) = (
            math.bpp,
            math.keep.bytes() as usize,
            evaluator.target.bytes_per_pixel,
        );
        // Bands of the rows each area reaches, every tile's together.
        type Band<'a> = (usize, usize, &'a mut [u8], &'a mut [u8], &'a mut [u8]);
        let mut work: Vec<Band<'_>> = Vec::new();
        for (index, ((p, k, px, _), (coord, area, _))) in buffers.iter_mut().zip(&dirty).enumerate()
        {
            let height = valid_area(size, *coord).1;
            let (y0, y1) = (area[1].min(height), area[3].min(height));
            if y0 >= y1 {
                continue;
            }
            let p_rows = p[y0 * t * bpp..y1 * t * bpp].chunks_mut(BAND_ROWS * t * bpp);
            let k_rows = k[y0 * t * kb..y1 * t * kb].chunks_mut(BAND_ROWS * t * kb);
            let px_rows = px[y0 * t * sb..y1 * t * sb].chunks_mut(BAND_ROWS * t * sb);
            for (n, ((pr, kr), xr)) in p_rows.zip(k_rows).zip(px_rows).enumerate() {
                work.push((index, y0 + n * BAND_ROWS, pr, kr, xr));
            }
        }
        let erased = parallel_for_each(&mut work, |(index, first, pr, kr, xr)| {
            let (coord, area, _) = dirty[*index];
            let (p0, k0, _) = starts[*index];
            let below = &below[*index];
            let width = valid_area(size, coord).0;
            let rows = pr.len() / (t * bpp);
            let mut erased = false;
            for r in 0..rows {
                let y = *first + r;
                for x in area[0]..area[2].min(width) {
                    let (i, local) = (y * t + x, r * t + x);
                    // From the start: an amount never re-applies over what the last frame laid.
                    pr[local * bpp..(local + 1) * bpp].copy_from_slice(&p0[i * bpp..(i + 1) * bpp]);
                    kr[local * kb..(local + 1) * kb].copy_from_slice(&k0[i * kb..(i + 1) * kb]);
                    let a = f64::from(amount(coord, x, y));
                    if a > 0.0 {
                        let (pv, kv) = math.read(pr, kr, local);
                        let (pv, kv) = lay(op, color, a.min(1.0), pv, kv);
                        math.write(pv, kv, pr, kr, local);
                        erased |= op == PaintOp::Erase;
                    }
                    evaluator.paint_pixel(
                        &math,
                        pr,
                        kr,
                        local,
                        &below[i * sb..(i + 1) * sb],
                        &mut xr[local * sb..(local + 1) * sb],
                    );
                }
            }
            (*index, erased)
        });
        drop(work);
        for (index, erased) in erased {
            buffers[index].3 |= erased;
        }
        let mut shown_tiles = Vec::with_capacity(dirty.len());
        for ((p, k, mut px, erased), (coord, _, _)) in buffers.into_iter().zip(&dirty) {
            let (width, height) = valid_area(size, *coord);
            let (mut p, mut k) = (p, k);
            pad_tile(&mut p, width, height, bpp);
            pad_tile(&mut k, width, height, kb);
            pad_tile(&mut px, width, height, sb);
            let mut tile = math.tile(p, k);
            tile.lowers_alpha |= erased;
            self.painted.insert(*coord, tile);
            shown_tiles.push((*coord, Arc::from(px)));
        }
        shown_tiles
    }

    /// The paint laid so far, continuing the start.
    pub fn paint(&self) -> Result<PaintEntry, StackError> {
        self.start
            .with_tiles(self.painted.iter().map(|(c, t)| (*c, t.clone())).collect())
    }

    /// The stack with the paint laid so far on top; without it when it changes nothing. The
    /// paint keeps what was below the tiles it reached, for the next stroke.
    pub fn stack(&self) -> Result<LayerStack, StackError> {
        let mut paint = self.paint()?;
        let reached = self
            .cache
            .iter()
            .filter(|(coord, _)| paint.tiles.contains_key(coord))
            .map(|(coord, tile)| (*coord, Arc::clone(tile)))
            .collect();
        paint.below = Mutex::new(Some(BelowTiles {
            stack: self.below.clone(),
            format: self.format,
            tiles: reached,
        }));
        let entry = Entry::Paint(Arc::new(paint));
        self.below.check(&entry)?;
        let mut entries = self.below.entries.clone();
        if matches!(&entry, Entry::Paint(p) if !p.is_identity()) {
            entries.push(entry);
        }
        Ok(LayerStack {
            original: Arc::clone(&self.below.original),
            entries,
        })
    }
}

/// The steps of two effect entries of one kind as one entry's, `below` first; Inverts that
/// meet cancel.
fn joined(below: &EffectEntry, above: &EffectEntry) -> Vec<Arc<Effect>> {
    let mut steps: Vec<Arc<Effect>> = Vec::new();
    for step in below.steps.iter().chain(&above.steps) {
        match steps.last() {
            Some(last) if last.cancels(step) => {
                steps.pop();
            }
            _ => steps.push(Arc::clone(step)),
        }
    }
    steps
}

/// The Restore Eraser on a stack (ADR 0029): every paint entry brought back towards the
/// identity where it rubs (`P ← (1−r)·P`, `k ← (1−r)·k + r`), the effects staying applied, and
/// the tiles it reaches evaluated again through the stack, each on every core.
#[derive(Debug)]
pub struct RestorePaint {
    stack: LayerStack,
    /// By paint entry (its index in the stack), its tiles restored so far.
    restored: BTreeMap<usize, BTreeMap<TileCoord, PaintTile>>,
}

impl RestorePaint {
    pub fn new(stack: &LayerStack) -> Self {
        Self {
            stack: stack.clone(),
            restored: BTreeMap::new(),
        }
    }

    /// The format of the tiles shown while restoring: the stack's.
    pub fn format(&self) -> PixelFormat {
        self.stack.format()
    }

    /// Whether some paint was restored.
    pub fn has_paint(&self) -> bool {
        !self.restored.is_empty()
    }

    /// Restore at `amount(coord, x, y)` on the tiles of `dirty` (amounts only grow during a
    /// stroke: every paint tile is restored from the stroke's start); the tiles the layer then
    /// shows.
    pub fn lay(
        &mut self,
        dirty: &[(TileCoord, [usize; 4])],
        amount: impl Fn(TileCoord, usize, usize) -> f32 + Sync,
    ) -> Vec<(TileCoord, Arc<[u8]>)> {
        let size = self.stack.original.size();
        let mut work: Vec<(usize, TileCoord, Option<PaintTile>)> = Vec::new();
        for (index, entry) in self.stack.entries.iter().enumerate() {
            if let Entry::Paint(paint) = entry {
                for (coord, _) in dirty {
                    if contains(size, *coord) && paint.tiles.contains_key(coord) {
                        work.push((index, *coord, None));
                    }
                }
            }
        }
        if work.is_empty() {
            return Vec::new();
        }
        let entries = &self.stack.entries;
        parallel_for_each(&mut work, |(index, coord, out)| {
            if let Entry::Paint(paint) = &entries[*index] {
                let coord = *coord;
                *out =
                    Some(paint.painted_tile(coord, PaintOp::Restore, |x, y| amount(coord, x, y)));
            }
        });
        for (index, coord, tile) in work {
            if let Some(tile) = tile {
                self.restored.entry(index).or_default().insert(coord, tile);
            }
        }
        let stack = self.current();
        let atoms = atoms(&stack.entries);
        let evaluator = Evaluator::new(&stack, &atoms, self.format());
        dirty
            .iter()
            .filter(|(coord, _)| contains(size, *coord))
            .map(|(coord, _)| (*coord, evaluator.tile_on_every_core(*coord)))
            .collect()
    }

    /// The stack with the paint restored so far (the same format as the original stack's).
    fn current(&self) -> LayerStack {
        let entries = self
            .stack
            .entries
            .iter()
            .enumerate()
            .map(|(index, entry)| match (entry, self.restored.get(&index)) {
                (Entry::Paint(paint), Some(tiles)) => {
                    let mut all = paint.tiles.clone();
                    all.extend(tiles.iter().map(|(c, t)| (*c, t.clone())));
                    Entry::Paint(Arc::new(PaintEntry {
                        size: paint.size,
                        format: paint.format,
                        space: paint.space,
                        tiles: all,
                        images: OnceLock::new(),
                        below: Mutex::new(None),
                    }))
                }
                _ => entry.clone(),
            })
            .collect();
        LayerStack {
            original: Arc::clone(&self.stack.original),
            entries,
        }
    }

    /// The stack with the paint restored: tiles back to the identity are dropped, and paint
    /// entries left with none are deleted (their neighbours merging as when deleted).
    pub fn stack(&self) -> Result<LayerStack, StackError> {
        let mut stack = self.current();
        let mut emptied = Vec::new();
        for (index, entry) in stack.entries.iter_mut().enumerate() {
            let Entry::Paint(paint) = entry else {
                continue;
            };
            if !self.restored.contains_key(&index) {
                continue;
            }
            let math = paint.math();
            let (color, keep) = math.identity();
            let tiles: BTreeMap<TileCoord, PaintTile> = paint
                .tiles
                .iter()
                .filter(|(_, t)| *t.color != *color || *t.keep != *keep)
                .map(|(c, t)| (*c, t.clone()))
                .collect();
            if tiles.is_empty() {
                emptied.push(index);
            }
            *entry = Entry::Paint(Arc::new(PaintEntry {
                size: paint.size,
                format: paint.format,
                space: paint.space,
                tiles,
                images: OnceLock::new(),
                below: Mutex::new(None),
            }));
        }
        // From the top, so that the indices below stay right.
        for index in emptied.into_iter().rev() {
            stack = stack.without(index)?;
        }
        Ok(stack)
    }
}

fn union(a: Footprint, b: Footprint) -> Footprint {
    match (a, b) {
        (Footprint::Tiles(mut a), Footprint::Tiles(b)) => {
            a.extend(b);
            Footprint::Tiles(a)
        }
        _ => Footprint::All,
    }
}

fn coords_of(footprint: &Footprint, size: Size) -> Vec<TileCoord> {
    match footprint {
        Footprint::All => grid_coords(size).collect(),
        Footprint::Tiles(tiles) => tiles
            .iter()
            .copied()
            .filter(|&c| contains(size, c))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection::SELECTION_FORMAT;

    /// Two tiles by two, the last ones partial.
    const W: u32 = 300;
    const H: u32 = 260;
    const T: u32 = TILE_SIZE;

    fn size() -> Size {
        Size::new(W, H)
    }

    fn image(format: PixelFormat, pixel: impl Fn(u32, u32) -> Vec<u8>) -> Arc<RasterImage> {
        let mut bytes = Vec::new();
        for y in 0..H {
            for x in 0..W {
                bytes.extend(pixel(x, y));
            }
        }
        Arc::new(RasterImage::from_pixels(size(), format, &bytes).unwrap())
    }

    /// An opaque 8-bit sRGB gradient, with an alpha channel or not.
    fn gradient(alpha: bool) -> Arc<RasterImage> {
        let format = if alpha {
            PixelFormat::RGBA8_SRGB
        } else {
            PixelFormat {
                layout: ChannelLayout::Rgb,
                ..PixelFormat::RGBA8_SRGB
            }
        };
        image(format, move |x, y| {
            let mut px = vec![(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8];
            if alpha {
                px.push(255);
            }
            px
        })
    }

    /// Pixel (`x`, `y`) of level 0 as stored.
    fn pixel(image: &RasterImage, x: u32, y: u32) -> Vec<u8> {
        let coord = TileCoord {
            col: x / T,
            row: y / T,
        };
        let tile = image.levels()[0].tile(coord).unwrap();
        let bpp = image.stored_format().bytes_per_pixel() as usize;
        let i = ((y % T) * T + x % T) as usize;
        tile[i * bpp..(i + 1) * bpp].to_vec()
    }

    /// The largest difference between two images' samples (same format).
    fn difference(a: &RasterImage, b: &RasterImage) -> u8 {
        assert_eq!(a.format(), b.format());
        let mut most = 0;
        for y in 0..H {
            for x in 0..W {
                for (s, t) in pixel(a, x, y).iter().zip(pixel(b, x, y)) {
                    most = most.max(s.abs_diff(t));
                }
            }
        }
        most
    }

    fn tile_of(image: &RasterImage, col: u32, row: u32) -> &Arc<[u8]> {
        image.levels()[0].tile(TileCoord { col, row }).unwrap()
    }

    /// `paint` with `op` at `amount(x, y)` (layer pixels), on the tiles it reaches.
    fn painted(
        paint: &PaintEntry,
        op: PaintOp,
        amount: impl Fn(u32, u32) -> f32 + Copy,
    ) -> Arc<PaintEntry> {
        let tiles = grid_coords(paint.size())
            .filter(|&coord| {
                (0..T).any(|y| {
                    (0..T).any(|x| {
                        let (px, py) = (coord.col * T + x, coord.row * T + y);
                        px < W && py < H && amount(px, py) > 0.0
                    })
                })
            })
            .map(|coord| {
                let tile = paint.painted_tile(coord, op, |x, y| {
                    amount(coord.col * T + x as u32, coord.row * T + y as u32)
                });
                (coord, tile)
            })
            .collect();
        Arc::new(paint.with_tiles(tiles).unwrap())
    }

    fn empty(original: &RasterImage) -> PaintEntry {
        PaintEntry::empty(original.format(), original.size(), BlendSpace::Perceptual)
    }

    fn gray(v: f32) -> PaintOp {
        PaintOp::Color(LinearRgba::new(v, v, v, 1.0))
    }

    fn effect(adjustment: Adjustment, selection: Option<Selection>) -> Effect {
        Effect {
            adjustment,
            selection,
            to_document: Affine::IDENTITY,
            space: BlendSpace::Perceptual,
        }
    }

    /// A hard selection of the document pixels where `inside(x, y)`.
    fn selection(inside: impl Fn(u32, u32) -> bool) -> Selection {
        let image = image(SELECTION_FORMAT, |x, y| {
            let v: u16 = if inside(x, y) { u16::MAX } else { 0 };
            v.to_ne_bytes().to_vec()
        });
        Selection::new(image).unwrap()
    }

    #[test]
    fn an_empty_stack_shows_its_original() {
        let original = gradient(true);
        let stack = LayerStack::new(Arc::clone(&original));
        assert!(Arc::ptr_eq(&stack.evaluate().unwrap(), &original));
    }

    #[test]
    fn paint_replaces_where_it_covers_and_shares_the_tiles_it_does_not_reach() {
        let original = gradient(true);
        let paint = painted(&empty(&original), gray(1.0), |x, y| {
            if x < 100 && y < 100 { 1.0 } else { 0.0 }
        });
        assert_eq!(paint.tiles().len(), 1);
        let stack = LayerStack::new(Arc::clone(&original))
            .with_top_paint(paint)
            .unwrap();
        let shown = stack.evaluate().unwrap();
        assert_eq!(pixel(&shown, 10, 10), [255, 255, 255, 255]);
        assert_eq!(pixel(&shown, 150, 10), pixel(&original, 150, 10));
        assert!(Arc::ptr_eq(tile_of(&shown, 1, 1), tile_of(&original, 1, 1)));
    }

    #[test]
    fn soft_paint_keeps_opaque_pixels_opaque() {
        for alpha in [true, false] {
            let original = gradient(alpha);
            let paint = painted(&empty(&original), gray(0.3), |x, y| {
                ((x * 7 + y * 13) % 256) as f32 / 255.0
            });
            assert!(!paint.lowers_alpha());
            let stack = LayerStack::new(Arc::clone(&original))
                .with_top_paint(paint)
                .unwrap();
            // No alpha channel needed: RGB stays RGB.
            assert_eq!(stack.format(), original.format());
            let shown = stack.evaluate().unwrap();
            for y in 0..H {
                for x in 0..W {
                    assert_eq!(pixel(&shown, x, y)[3], 255, "({x}, {y})");
                }
            }
        }
    }

    #[test]
    fn the_eraser_gives_rgb_layers_an_alpha_channel() {
        let original = gradient(false);
        let paint = painted(&empty(&original), PaintOp::Erase, |x, _| {
            if x < 50 { 0.5 } else { 0.0 }
        });
        assert!(paint.lowers_alpha());
        let stack = LayerStack::new(Arc::clone(&original))
            .with_top_paint(paint)
            .unwrap();
        assert_eq!(stack.format().layout, ChannelLayout::Rgba);
        let shown = stack.evaluate().unwrap();
        assert!(pixel(&shown, 10, 10)[3].abs_diff(128) <= 1);
        assert_eq!(pixel(&shown, 10, 10)[..3], pixel(&original, 10, 10)[..3]);
        assert_eq!(pixel(&shown, 100, 10), pixel(&original, 100, 10));
        // RGB is stored with alpha: the tiles the eraser did not reach are shared.
        assert!(Arc::ptr_eq(tile_of(&shown, 1, 0), tile_of(&original, 1, 0)));
    }

    #[test]
    fn gray_layers_are_painted_in_gray() {
        let original = image(
            PixelFormat {
                layout: ChannelLayout::Gray,
                ..PixelFormat::RGBA8_SRGB
            },
            |x, _| vec![(x % 256) as u8],
        );
        let white = painted(
            &empty(&original),
            gray(1.0),
            |x, _| {
                if x < 10 { 1.0 } else { 0.0 }
            },
        );
        let stack = LayerStack::new(Arc::clone(&original))
            .with_top_paint(Arc::clone(&white))
            .unwrap();
        assert_eq!(stack.format(), original.format());
        assert_eq!(pixel(&stack.evaluate().unwrap(), 5, 5), [255]);
        let erased = painted(
            &white,
            PaintOp::Erase,
            |x, _| {
                if x < 10 { 1.0 } else { 0.0 }
            },
        );
        let stack = stack.with_top_paint(erased).unwrap();
        assert_eq!(stack.format().layout, ChannelLayout::GrayAlpha);
        let shown = stack.evaluate().unwrap();
        assert_eq!(pixel(&shown, 5, 5)[1], 0);
        assert_eq!(pixel(&shown, 50, 5), [50, 255]);
    }

    #[test]
    fn paint_follows_when_an_effect_below_it_is_deleted() {
        let original = gradient(true);
        let paint = painted(&empty(&original), gray(1.0), |x, _| {
            if x < 120 { 0.5 } else { 0.0 }
        });
        let stack = LayerStack::new(Arc::clone(&original))
            .with_effect(effect(Adjustment::Invert, None))
            .unwrap()
            .with_top_paint(Arc::clone(&paint))
            .unwrap();
        let inverted = stack.evaluate().unwrap();
        assert_eq!(pixel(&inverted, 200, 10)[0], 255 - 200);
        let deleted = stack.without(0).unwrap();
        assert_eq!(deleted.entries(), &[Entry::Paint(Arc::clone(&paint))]);
        let direct = LayerStack::new(Arc::clone(&original))
            .with_top_paint(paint)
            .unwrap()
            .evaluate()
            .unwrap();
        assert_eq!(difference(&deleted.evaluate().unwrap(), &direct), 0);
    }

    #[test]
    fn neighbouring_paints_merge_when_what_separates_them_is_deleted() {
        let original = gradient(true);
        let below = painted(&empty(&original), gray(0.5), |x, _| {
            if x < 200 { 0.6 } else { 0.0 }
        });
        let above = painted(&empty(&original), gray(1.0), |_, y| {
            if y < 150 { 0.3 } else { 0.0 }
        });
        let stack = LayerStack::with_entries(
            Arc::clone(&original),
            vec![
                Entry::Paint(Arc::clone(&below)),
                Entry::Effect(Arc::new(
                    EffectEntry::new(vec![Arc::new(effect(Adjustment::Invert, None))]).unwrap(),
                )),
                Entry::Paint(Arc::clone(&above)),
            ],
        )
        .unwrap();
        let merged = stack.without(1).unwrap();
        assert_eq!(merged.entries().len(), 1);
        assert!(matches!(merged.entries()[0], Entry::Paint(_)));
        let apart = LayerStack::with_entries(
            Arc::clone(&original),
            vec![Entry::Paint(below), Entry::Paint(above)],
        )
        .unwrap();
        // Exact but for one rounding.
        assert!(difference(&merged.evaluate().unwrap(), &apart.evaluate().unwrap()) <= 1);
    }

    #[test]
    fn effects_of_a_kind_merge_and_two_inverts_cancel() {
        let original = gradient(true);
        let paint = painted(
            &empty(&original),
            gray(1.0),
            |x, _| {
                if x < 10 { 1.0 } else { 0.0 }
            },
        );
        let brighter = |amount| {
            effect(
                Adjustment::BrightnessContrast {
                    brightness: amount,
                    contrast: 0.0,
                },
                None,
            )
        };
        let base = LayerStack::new(Arc::clone(&original));
        let stack = base
            .with_effect(brighter(20.0))
            .unwrap()
            .with_top_paint(Arc::clone(&paint))
            .unwrap()
            .with_effect(brighter(-40.0))
            .unwrap();
        let merged = stack.without(1).unwrap();
        let [Entry::Effect(entry)] = merged.entries() else {
            panic!("one effect entry expected");
        };
        assert_eq!(entry.steps().len(), 2);
        let apart = base
            .with_effect(brighter(20.0))
            .unwrap()
            .with_effect(brighter(-40.0))
            .unwrap();
        // The same steps, each rounded as before: the same pixels.
        assert_eq!(
            difference(&merged.evaluate().unwrap(), &apart.evaluate().unwrap()),
            0
        );

        let inverts = base
            .with_effect(effect(Adjustment::Invert, None))
            .unwrap()
            .with_top_paint(paint)
            .unwrap()
            .with_effect(effect(Adjustment::Invert, None))
            .unwrap();
        let cancelled = inverts.without(1).unwrap();
        assert!(cancelled.is_empty());
        assert!(Arc::ptr_eq(&cancelled.evaluate().unwrap(), &original));
    }

    #[test]
    fn contiguous_effects_of_a_kind_are_one_entry_and_inverts_cancel() {
        let original = gradient(true);
        let brighter = |amount| {
            effect(
                Adjustment::BrightnessContrast {
                    brightness: amount,
                    contrast: 0.0,
                },
                None,
            )
        };
        let base = LayerStack::new(Arc::clone(&original));
        let once = base.with_effect(brighter(20.0)).unwrap();
        let twice = once.with_effect(brighter(-40.0)).unwrap();
        let [Entry::Effect(entry)] = twice.entries() else {
            panic!("one effect entry expected");
        };
        assert_eq!(entry.steps().len(), 2);
        // Applied on top of what is shown: the first step is not evaluated again.
        let shown = once.reevaluate(&base, &original).unwrap();
        let shown = twice.reevaluate(&once, &shown).unwrap();
        assert_eq!(difference(&shown, &twice.evaluate().unwrap()), 0);
        // Another kind starts an entry; an Invert on an Invert cancels.
        let other = twice.with_effect(effect(Adjustment::Invert, None)).unwrap();
        assert_eq!(other.entries().len(), 2);
        let back = other.with_effect(effect(Adjustment::Invert, None)).unwrap();
        assert_eq!(back.entries(), twice.entries());
    }

    #[test]
    fn an_effect_stays_within_its_selection() {
        let original = gradient(true);
        let stack = LayerStack::new(Arc::clone(&original))
            .with_effect(effect(Adjustment::Invert, Some(selection(|x, _| x < 100))))
            .unwrap();
        let shown = stack.evaluate().unwrap();
        assert_eq!(pixel(&shown, 10, 10)[..3], [245, 245, 235]);
        assert_eq!(pixel(&shown, 150, 10), pixel(&original, 150, 10));
        assert!(Arc::ptr_eq(tile_of(&shown, 1, 0), tile_of(&original, 1, 0)));

        // Applied to a layer placed 50 pixels to the right: the selection is read where the
        // layer's pixels were then.
        let moved = LayerStack::new(Arc::clone(&original))
            .with_effect(Effect {
                to_document: Affine::translation(50.0, 0.0),
                ..effect(Adjustment::Invert, Some(selection(|x, _| x < 100)))
            })
            .unwrap()
            .evaluate()
            .unwrap();
        assert_eq!(pixel(&moved, 40, 10)[0], 255 - 40);
        assert_eq!(pixel(&moved, 60, 10), pixel(&original, 60, 10));
    }

    #[test]
    fn reevaluating_gives_what_evaluating_gives() {
        let original = gradient(false);
        let first = painted(&empty(&original), gray(0.2), |x, y| {
            if x < 140 && y > 30 { 0.7 } else { 0.0 }
        });
        let erased = painted(&empty(&original), PaintOp::Erase, |x, _| {
            if (100..280).contains(&x) { 0.4 } else { 0.0 }
        });
        let continued = painted(&erased, gray(0.9), |_, y| if y > 200 { 0.8 } else { 0.0 });
        let s0 = LayerStack::new(Arc::clone(&original));
        let s1 = s0.with_top_paint(first).unwrap();
        let s2 = s1
            .with_effect(effect(
                Adjustment::Posterize { levels: 4.0 },
                Some(selection(|x, y| x > 20 && y < 120)),
            ))
            .unwrap();
        let s3 = s2.with_top_paint(erased).unwrap();
        let s4 = s3.with_top_paint(continued).unwrap();
        let s5 = s4.without(1).unwrap();
        let s6 = s5.without(0).unwrap();
        let states = [s0, s1, s2, s3, s4, s5, s6];
        let mut shown = states[0].evaluate().unwrap();
        // Forward, then back as undo goes.
        let order: Vec<usize> = (1..states.len())
            .chain((0..states.len() - 1).rev())
            .collect();
        let mut previous = 0;
        for index in order {
            let stack = &states[index];
            shown = stack.reevaluate(&states[previous], &shown).unwrap();
            let fresh = stack.evaluate().unwrap();
            assert_eq!(shown.format(), fresh.format(), "state {index}");
            assert_eq!(difference(&shown, &fresh), 0, "state {index}");
            previous = index;
        }
        assert!(Arc::ptr_eq(&shown, &original));
    }

    #[test]
    fn images_painted_the_old_way_convert_exactly() {
        let original = gradient(true);
        let painted_image = image(PixelFormat::RGBA8_SRGB, |x, y| {
            if x < 40 && y < 40 {
                vec![200, 30, 90, ((x * 6 + y) % 256) as u8]
            } else {
                vec![(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8, 255]
            }
        });
        let paint =
            PaintEntry::from_painted(&original, &painted_image, BlendSpace::Perceptual).unwrap();
        assert_eq!(paint.tiles().len(), 1);
        let stack = LayerStack::new(Arc::clone(&original))
            .with_top_paint(Arc::new(paint))
            .unwrap();
        assert_eq!(difference(&stack.evaluate().unwrap(), &painted_image), 0);
    }

    #[test]
    fn the_restore_eraser_brings_back_the_original() {
        let original = gradient(true);
        let paint = painted(&empty(&original), gray(0.8), |x, _| {
            if x < 100 { 0.9 } else { 0.0 }
        });
        let restored = painted(
            &paint,
            PaintOp::Restore,
            |x, _| {
                if x < 100 { 1.0 } else { 0.0 }
            },
        );
        let stack = LayerStack::new(Arc::clone(&original))
            .with_top_paint(restored)
            .unwrap();
        assert_eq!(difference(&stack.evaluate().unwrap(), &original), 0);
    }

    #[test]
    fn a_stroke_shows_what_its_stack_evaluates_to() {
        let original = gradient(false);
        let first = painted(&empty(&original), gray(0.2), |x, _| {
            if x < 140 { 0.7 } else { 0.0 }
        });
        let stack = LayerStack::new(Arc::clone(&original))
            .with_effect(effect(Adjustment::Invert, Some(selection(|_, y| y < 100))))
            .unwrap()
            .with_top_paint(first)
            .unwrap();
        for (op, erase) in [(gray(0.9), false), (PaintOp::Erase, true)] {
            let mut top = TopPaint::new(
                &stack,
                &stack.evaluate().unwrap(),
                BlendSpace::Perceptual,
                erase,
            );
            let mut shown = stack.evaluate().unwrap();
            if erase {
                // RGB is stored with alpha: the same tiles.
                shown = Arc::new(shown.with_alpha().unwrap().unwrap());
            }
            assert_eq!(shown.format(), top.format());
            let t = T as usize;
            // Two frames, the second reaching further with more.
            for (reach, more) in [(100, 0.0), (200, 0.3)] {
                let dirty = [
                    (TileCoord { col: 0, row: 0 }, [0, 0, t, t]),
                    (TileCoord { col: 1, row: 1 }, [0, 0, 20, 4]),
                ];
                let amount = move |coord: TileCoord, x: usize, _| {
                    if coord.col == 0 && x > reach {
                        0.0
                    } else {
                        0.25 + more
                    }
                };
                let tiles = top.lay(&dirty, op, amount, &shown);
                shown = Arc::new(shown.with_tiles(tiles).unwrap());
            }
            let result = top.stack().unwrap();
            assert_eq!(result.entries().len(), 2, "the top paint continues");
            assert_eq!(result.format(), top.format());
            assert_eq!(difference(&shown, &result.evaluate().unwrap()), 0);
        }
    }

    #[test]
    fn a_grown_stack_shows_the_same_pixels_moved() {
        let original = gradient(true);
        let paint = painted(&empty(&original), gray(1.0), |x, _| {
            if x > 280 { 0.6 } else { 0.0 }
        });
        let stack = LayerStack::new(Arc::clone(&original))
            .with_top_paint(paint)
            .unwrap()
            .with_effect(effect(Adjustment::Invert, Some(selection(|x, _| x > 150))))
            .unwrap();
        let shown = stack.evaluate().unwrap();
        let grown = stack.grown((1, 0), Size::new(W + 2 * T, H)).unwrap();
        let moved = grown.evaluate().unwrap();
        for y in [0, 100, H - 1] {
            for x in [0, 149, 151, 290, W - 1] {
                assert_eq!(pixel(&moved, x + T, y), pixel(&shown, x, y), "({x}, {y})");
            }
            // Transparent around, the paint's padding included.
            assert_eq!(pixel(&moved, 10, y)[3], 0);
            assert_eq!(pixel(&moved, W + T + 5, y)[3], 0);
        }
    }

    #[test]
    fn moved_pixels_are_baked_into_the_top_paint() {
        let original = gradient(true);
        let paint = painted(
            &empty(&original),
            gray(0.5),
            |x, _| {
                if x < 50 { 0.5 } else { 0.0 }
            },
        );
        let stack = LayerStack::new(Arc::clone(&original))
            .with_top_paint(paint)
            .unwrap();
        let shown = stack.evaluate().unwrap();
        let after = image(PixelFormat::RGBA8_SRGB, |x, y| {
            let mut px = pixel(&shown, x, y);
            if (100..120).contains(&x) {
                px = vec![1, 2, 3, 128];
            }
            px
        });
        let moved = stack
            .with_painted(&shown, &after, BlendSpace::Perceptual)
            .unwrap();
        assert_eq!(moved.entries().len(), 1, "it continues the top paint");
        assert!(difference(&moved.evaluate().unwrap(), &after) <= 1);
    }

    #[test]
    fn the_restore_eraser_reaches_every_paint_and_keeps_the_effects() {
        let original = gradient(true);
        let below = painted(&empty(&original), gray(0.9), |x, _| {
            if x < 200 { 1.0 } else { 0.0 }
        });
        let above = painted(&empty(&original), gray(0.1), |_, y| {
            if y < 100 { 0.5 } else { 0.0 }
        });
        let stack = LayerStack::new(Arc::clone(&original))
            .with_top_paint(below)
            .unwrap()
            .with_effect(effect(Adjustment::Invert, None))
            .unwrap()
            .with_top_paint(above)
            .unwrap();
        let mut restore = RestorePaint::new(&stack);
        let t = T as usize;
        let dirty: Vec<(TileCoord, [usize; 4])> =
            grid_coords(size()).map(|c| (c, [0, 0, t, t])).collect();
        // Everywhere at once: the original, inverted.
        let shown = restore.lay(&dirty, |_, _, _| 1.0);
        let result = restore.stack().unwrap();
        let [Entry::Effect(_)] = result.entries() else {
            panic!("only the effect is left");
        };
        let evaluated = result.evaluate().unwrap();
        for (coord, tile) in &shown {
            assert_eq!(**tile, **tile_of(&evaluated, coord.col, coord.row));
        }
        assert_eq!(pixel(&evaluated, 10, 10)[0], 255 - 10);
    }

    #[test]
    fn paint_of_another_layer_is_refused() {
        let original = gradient(true);
        let other = PaintEntry::empty(original.format(), Size::new(10, 10), BlendSpace::Perceptual);
        assert_eq!(
            LayerStack::new(original)
                .with_top_paint(Arc::new(other))
                .unwrap_err(),
            StackError::SizeMismatch
        );
    }
}
