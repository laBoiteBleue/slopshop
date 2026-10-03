//! Raster (pixel) images, stored as immutable tiles.
//!
//! Storage (ADR 0005): the whole image lives in RAM as fixed-size square tiles in the source
//! pixel format — sample type (8/16-bit integer, 16/32-bit float), gray or color, alpha mode and
//! color space are kept as they are. Tiles are shared through `Arc` so that document snapshots
//! and undo never copy pixels. A mip pyramid (display-only, derived data, in the same format)
//! serves zoomed-out views. Out-of-core storage is future work.
//!
//! The only transformation at storage time is lossless: RGB without alpha is stored as RGBA
//! with an opaque alpha channel (GPUs have no 3-channel texture formats).

use std::num::NonZeroU32;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use crate::color::{
    AlphaMode, ChannelLayout, ColorSpace, IDENTITY, LinearRgba, PixelFormat, SampleType,
    TransferFunction, f16_to_f32, f32_to_f16,
};
use crate::geom::{Rect, Size};
use crate::tile::{TileCoord, TileGrid};

/// Tile edge in pixels. 256 × 256 RGBA8 = 256 KiB, a common GPU-friendly size.
pub const TILE_SIZE: u32 = 256;

/// Magnitude that every float sample is clamped to when read for averaging (pyramid, average
/// color) and display: the largest half float. This applies to finite values too (a sample of
/// 1e5 is read as 65504), so that sums and matrices stay finite and an opaque layer still hides
/// what is below it; NaN is read as 0. The stored samples are not modified, and export reads
/// them without this clamp (see [`crate::composite`]).
pub const MAX_FINITE_SAMPLE: f32 = 65504.0;

/// Process-unique identity of a [`RasterImage`], used as a cache key (e.g. GPU tile cache).
/// Images are immutable, so the id is valid for the image's whole lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ImageId(u64);

impl ImageId {
    fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

/// One level of the pyramid: level 0 is the source image, level `n` is `2^n` times smaller.
#[derive(Debug)]
pub struct RasterLevel {
    size: Size,
    grid: TileGrid,
    /// Row-major. Every tile holds `TILE_SIZE²` pixels in the image's stored format; edge tiles
    /// are padded by repeating the last row/column, so all tiles have the same size.
    tiles: Vec<Arc<[u8]>>,
}

/// Images are immutable: the same image is the same allocation, told by its id.
impl PartialEq for RasterImage {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl RasterLevel {
    /// The same level, its tiles shared.
    fn shared(&self) -> Self {
        Self {
            size: self.size,
            grid: self.grid,
            tiles: self.tiles.clone(),
        }
    }

    pub fn size(&self) -> Size {
        self.size
    }

    pub fn grid(&self) -> TileGrid {
        self.grid
    }

    /// Every tile, row-major (see [`Self::tile`]).
    pub fn tiles(&self) -> &[Arc<[u8]>] {
        &self.tiles
    }

    /// Pixels of a tile (`TILE_SIZE²` pixels, row-major, stored format), or `None` outside
    /// the grid.
    pub fn tile(&self, coord: TileCoord) -> Option<&Arc<[u8]>> {
        if coord.col >= self.grid.columns() || coord.row >= self.grid.rows() {
            return None;
        }
        let index = coord.row as usize * self.grid.columns() as usize + coord.col as usize;
        self.tiles.get(index)
    }
}

/// An immutable tiled image with its display pyramid.
#[derive(Debug)]
pub struct RasterImage {
    id: ImageId,
    /// Format of the source pixels.
    format: PixelFormat,
    levels: Vec<RasterLevel>,
    /// Average of the coarsest level: source linear RGB, straight alpha.
    average: LinearRgba,
    /// The bounds of the pixels that are not transparent, computed once when first asked.
    content_bounds: OnceLock<Option<Rect>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RasterError {
    EmptyImage,
    /// `pixels.len()` does not match `width × height × bytes per pixel`.
    SizeMismatch {
        expected: u64,
        actual: u64,
    },
    /// The color space's primaries do not define a usable RGB space.
    InvalidColorSpace(ColorSpace),
    /// [`RasterImage::from_tiles`]: not one list of tiles per pyramid level.
    LevelCountMismatch {
        expected: usize,
        actual: usize,
    },
    /// [`RasterImage::from_tiles`]: a level does not have one tile per grid cell.
    TileCountMismatch {
        level: usize,
        expected: usize,
        actual: usize,
    },
    /// [`RasterImage::from_tiles`]: a tile does not hold `TILE_SIZE²` stored pixels.
    TileLengthMismatch {
        level: usize,
        index: usize,
        expected: usize,
        actual: usize,
    },
    /// [`RasterImage::from_placed`]: the rectangle does not fit in the image.
    RectOutside {
        rect: Rect,
        size: Size,
    },
    /// [`RasterImage::with_tiles`]: a tile outside the grid.
    TileOutside(TileCoord),
}

impl std::fmt::Display for RasterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RasterError::EmptyImage => write!(f, "image has no pixels"),
            RasterError::SizeMismatch { expected, actual } => {
                write!(f, "pixel buffer has {actual} bytes, expected {expected}")
            }
            RasterError::InvalidColorSpace(space) => {
                write!(f, "invalid color space {space:?}")
            }
            RasterError::LevelCountMismatch { expected, actual } => {
                write!(f, "{actual} pyramid levels, expected {expected}")
            }
            RasterError::TileCountMismatch {
                level,
                expected,
                actual,
            } => write!(f, "level {level} has {actual} tiles, expected {expected}"),
            RasterError::TileLengthMismatch {
                level,
                index,
                expected,
                actual,
            } => write!(
                f,
                "tile {index} of level {level} has {actual} bytes, expected {expected}"
            ),
            RasterError::RectOutside { rect, size } => write!(
                f,
                "rectangle {rect:?} does not fit in {}×{}",
                size.width, size.height
            ),
            RasterError::TileOutside(coord) => {
                write!(f, "tile {}, {} is outside the image", coord.col, coord.row)
            }
        }
    }
}

impl std::error::Error for RasterError {}

impl RasterImage {
    /// Build a tiled image (and its pyramid) from tightly packed rows of `format` pixels.
    /// Every layout and sample type is supported; nothing is converted except RGB → RGBA.
    pub fn from_pixels(
        size: Size,
        format: PixelFormat,
        pixels: &[u8],
    ) -> Result<Self, RasterError> {
        check_format(size, format)?;
        let expected = size.pixel_count() * u64::from(format.bytes_per_pixel());
        if pixels.len() as u64 != expected {
            return Err(RasterError::SizeMismatch {
                expected,
                actual: pixels.len() as u64,
            });
        }

        let source = Codec::new(format);
        let stored = Codec::new(stored_format(format));
        let mut levels = vec![tile_level(size, pixels, &source, &stored)];
        // Pyramid down to a single tile. Each level is built from the previous one (derived,
        // display-only data; the source level is never modified).
        let mut current: Option<Vec<u8>> = None;
        let mut current_size = size;
        while current_size.width > TILE_SIZE || current_size.height > TILE_SIZE {
            let (input, codec) = match &current {
                Some(level) => (level.as_slice(), &stored),
                None => (pixels, &source),
            };
            let (next, next_size) = downsample(input, current_size, codec, &stored);
            levels.push(tile_level(next_size, &next, &stored, &stored));
            current = Some(next);
            current_size = next_size;
        }

        let average = average_of(&levels[levels.len() - 1], &stored);
        Ok(Self {
            id: ImageId::next(),
            format,
            levels,
            average,
            content_bounds: OnceLock::new(),
        })
    }

    /// Rebuild an image from the tiles of every pyramid level, finest first (e.g. read from a
    /// document file): row-major, `TILE_SIZE²` pixels each in the stored format of `format`
    /// (RGB stored as RGBA), edge tiles padded by repeating the last row and column. Nothing is
    /// copied: the tiles are shared. The levels must be the ones [`Self::level_sizes`] gives.
    pub fn from_tiles(
        size: Size,
        format: PixelFormat,
        levels: Vec<Vec<Arc<[u8]>>>,
    ) -> Result<Self, RasterError> {
        check_format(size, format)?;
        let sizes = Self::level_sizes(size);
        if levels.len() != sizes.len() {
            return Err(RasterError::LevelCountMismatch {
                expected: sizes.len(),
                actual: levels.len(),
            });
        }
        let stored = Codec::new(stored_format(format));
        let levels = sizes
            .into_iter()
            .zip(levels)
            .enumerate()
            .map(|(level, (size, tiles))| checked_level(level, size, tiles, &stored))
            .collect::<Result<Vec<_>, _>>()?;
        let average = average_of(&levels[levels.len() - 1], &stored);
        Ok(Self {
            id: ImageId::next(),
            format,
            levels,
            average,
            content_bounds: OnceLock::new(),
        })
    }

    /// [`Self::from_tiles`] from level 0 only: the rest of the pyramid is rebuilt tile by tile
    /// from the tiles below, exactly as [`Self::from_pixels`] builds it.
    pub fn from_level0_tiles(
        size: Size,
        format: PixelFormat,
        tiles: Vec<Arc<[u8]>>,
    ) -> Result<Self, RasterError> {
        check_format(size, format)?;
        let stored = Codec::new(stored_format(format));
        let mut levels = vec![checked_level(0, size, tiles, &stored)?];
        while let Some(finer) = levels.last()
            && (finer.size.width > TILE_SIZE || finer.size.height > TILE_SIZE)
        {
            let coarser = downsample_level(finer, &stored);
            levels.push(coarser);
        }
        let average = average_of(&levels[levels.len() - 1], &stored);
        Ok(Self {
            id: ImageId::next(),
            format,
            levels,
            average,
            content_bounds: OnceLock::new(),
        })
    }

    /// An image of `size` holding `pixels` (packed rows of `format`, `rect.size()` of them) in
    /// `rect` and the pixel `background` (one pixel of `format`) everywhere else: a layer
    /// smaller than its canvas (e.g. a Photoshop layer). The tiles that hold only background
    /// are one shared allocation, and so are their pyramid tiles: the background costs almost
    /// no memory. `rect` may be empty (background only).
    pub fn from_placed(
        size: Size,
        format: PixelFormat,
        rect: Rect,
        pixels: &[u8],
        background: &[u8],
    ) -> Result<Self, RasterError> {
        check_format(size, format)?;
        let bpp = format.bytes_per_pixel() as usize;
        if background.len() != bpp {
            return Err(RasterError::SizeMismatch {
                expected: bpp as u64,
                actual: background.len() as u64,
            });
        }
        if rect.right() > u64::from(size.width) || rect.bottom() > u64::from(size.height) {
            return Err(RasterError::RectOutside { rect, size });
        }
        let expected = rect.size().pixel_count() * bpp as u64;
        if pixels.len() as u64 != expected {
            return Err(RasterError::SizeMismatch {
                expected,
                actual: pixels.len() as u64,
            });
        }
        let source = Codec::new(format);
        let stored = Codec::new(stored_format(format));
        let tiles = placed_tiles(size, rect, pixels, background, &source, &stored);
        Self::from_level0_tiles(size, format, tiles)
    }

    /// Sizes of the pyramid levels of an image of `size`, finest first: each level half the
    /// previous one (rounded up), down to the first that fits in one tile.
    pub fn level_sizes(size: Size) -> Vec<Size> {
        let mut sizes = vec![size];
        let mut level = size;
        while level.width > TILE_SIZE || level.height > TILE_SIZE {
            level = Size::new(level.width.div_ceil(2), level.height.div_ceil(2));
            sizes.push(level);
        }
        sizes
    }

    /// Bytes of one tile of an image of `format` (its stored format).
    pub fn tile_bytes(format: PixelFormat) -> usize {
        TILE_SIZE as usize * TILE_SIZE as usize * stored_format(format).bytes_per_pixel() as usize
    }

    /// Upper bound of the RAM that [`Self::from_pixels`] needs for an image of this size and
    /// format: padded tiles of every pyramid level, plus the transient downsampling buffers.
    /// `None` if it does not even fit in a `u64`.
    pub fn estimated_memory_bytes(size: Size, format: PixelFormat) -> Option<u64> {
        let bpp = u64::from(stored_format(format).bytes_per_pixel());
        let tile_bytes = u64::from(TILE_SIZE) * u64::from(TILE_SIZE) * bpp;
        let mut total = 0u64;
        let mut level = size;
        loop {
            let tiles = u64::from(level.width.div_ceil(TILE_SIZE))
                * u64::from(level.height.div_ceil(TILE_SIZE));
            total = total.checked_add(tiles.checked_mul(tile_bytes)?)?;
            if level.width <= TILE_SIZE && level.height <= TILE_SIZE {
                break;
            }
            level = Size::new(level.width.div_ceil(2), level.height.div_ceil(2));
        }
        // Unpadded levels 1.. are built one after the other: at most 1/4 + 1/16 + … < 1/3
        // (plus rounding up at odd sizes, covered by one tile).
        let transient = size.pixel_count().checked_mul(bpp)? / 3 + tile_bytes;
        total.checked_add(transient)
    }

    pub fn id(&self) -> ImageId {
        self.id
    }

    pub fn size(&self) -> Size {
        self.levels[0].size
    }

    /// Format of the source pixels.
    pub fn format(&self) -> PixelFormat {
        self.format
    }

    /// Format of the tiles: the source format, with RGB stored as RGBA.
    pub fn stored_format(&self) -> PixelFormat {
        stored_format(self.format)
    }

    /// The alpha channel as a mask image (ADR 0014): gray, same size and sample type, samples
    /// copied unchanged tile by tile, with a linear transfer so that a sample reads as its
    /// coverage. `None` for an image without alpha.
    pub fn alpha_mask(&self) -> Option<RasterImage> {
        // RGB is stored as RGBA with an opaque alpha: that is no transparency to take.
        if !self.format.layout.has_alpha() {
            return None;
        }
        let stored = self.stored_format();
        let sample = stored.sample.bytes() as usize;
        let pixel = stored.bytes_per_pixel() as usize;
        let format = PixelFormat {
            layout: ChannelLayout::Gray,
            sample: stored.sample,
            color_space: ColorSpace::LINEAR_SRGB,
            alpha: AlphaMode::Straight,
        };
        let tiles = self.levels[0]
            .tiles()
            .iter()
            .map(|tile| {
                let alpha: Vec<u8> = tile
                    .chunks_exact(pixel)
                    .flat_map(|px| px[pixel - sample..].iter().copied())
                    .collect();
                Arc::<[u8]>::from(alpha)
            })
            .collect();
        // Invariant: tiles of the right count and length for this size and format.
        RasterImage::from_level0_tiles(self.size(), format, tiles).ok()
    }

    /// Alpha (coverage, clamped to `[0, 1]`) of pixel (`x`, `y`) of level 0: 1 for an image
    /// without alpha, 0 outside the image.
    pub fn alpha_at(&self, x: u32, y: u32) -> f32 {
        let size = self.size();
        if x >= size.width || y >= size.height {
            return 0.0;
        }
        if !self.format.layout.has_alpha() {
            return 1.0;
        }
        let stored = Codec::new(self.stored_format());
        let coord = TileCoord {
            col: x / TILE_SIZE,
            row: y / TILE_SIZE,
        };
        let Some(tile) = self.levels[0].tile(coord) else {
            return 0.0;
        };
        let at = ((y % TILE_SIZE) * TILE_SIZE + x % TILE_SIZE) as usize * stored.bytes_per_pixel;
        tile.get(at..at + stored.bytes_per_pixel)
            .map_or(0.0, |px| stored.read(px).1)
    }

    /// Linear value of the first channel of pixel (`x`, `y`) of level 0, clamped to `[0, 1]`: a
    /// mask's coverage there (ADR 0014); 0 outside the image.
    pub fn gray_at(&self, x: u32, y: u32) -> f32 {
        let size = self.size();
        if x >= size.width || y >= size.height {
            return 0.0;
        }
        let stored = Codec::new(self.stored_format());
        let coord = TileCoord {
            col: x / TILE_SIZE,
            row: y / TILE_SIZE,
        };
        let Some(tile) = self.levels[0].tile(coord) else {
            return 0.0;
        };
        let at = ((y % TILE_SIZE) * TILE_SIZE + x % TILE_SIZE) as usize * stored.bytes_per_pixel;
        tile.get(at..at + stored.bytes_per_pixel)
            .map_or(0.0, |px| stored.read(px).0[0].clamp(0.0, 1.0))
    }

    /// The smallest rectangle holding every pixel that is not fully transparent (the whole image
    /// without alpha; `None` when all are transparent). Computed once, tile by tile in parallel;
    /// a tile shared by several places (the background of [`Self::from_placed`]) is read once.
    pub fn content_bounds(&self) -> Option<Rect> {
        *self.content_bounds.get_or_init(|| {
            if !self.format.layout.has_alpha() {
                return Some(self.size().bounds());
            }
            self.scan_content_bounds()
        })
    }

    fn scan_content_bounds(&self) -> Option<Rect> {
        let level = &self.levels[0];
        let stored = Codec::new(self.stored_format());
        let t = TILE_SIZE as usize;
        let (width, height) = (level.size.width as usize, level.size.height as usize);
        let columns = level.grid.columns() as usize;
        // Local bounds of the opaque pixels of each distinct tile, by allocation.
        let mut distinct: Vec<&Arc<[u8]>> = Vec::new();
        let mut index_of = std::collections::HashMap::new();
        for tile in &level.tiles {
            index_of.entry(tile.as_ptr()).or_insert_with(|| {
                distinct.push(tile);
                distinct.len() - 1
            });
        }
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
        let per_thread = distinct.len().div_ceil(threads).max(1);
        let mut local: Vec<Option<[usize; 4]>> = vec![None; distinct.len()];
        std::thread::scope(|scope| {
            for (chunk, out) in distinct
                .chunks(per_thread)
                .zip(local.chunks_mut(per_thread))
            {
                let stored = &stored;
                scope.spawn(move || {
                    for (tile, bounds) in chunk.iter().zip(out) {
                        let mut b: Option<[usize; 4]> = None;
                        for (i, px) in tile.chunks_exact(stored.bytes_per_pixel).enumerate() {
                            if stored.read(px).1 > 0.0 {
                                let (x, y) = (i % t, i / t);
                                b = Some(match b {
                                    None => [x, y, x, y],
                                    Some([x0, y0, x1, y1]) => {
                                        [x0.min(x), y0.min(y), x1.max(x), y1.max(y)]
                                    }
                                });
                            }
                        }
                        *bounds = b;
                    }
                });
            }
        });
        // Place each tile's bounds, clipped to the image (padding repeats edge pixels).
        let mut total: Option<[usize; 4]> = None;
        for (index, tile) in level.tiles.iter().enumerate() {
            let Some([x0, y0, x1, y1]) = local[index_of[&tile.as_ptr()]] else {
                continue;
            };
            let (ox, oy) = ((index % columns) * t, (index / columns) * t);
            let (x0, y0) = (ox + x0, oy + y0);
            let (x1, y1) = ((ox + x1).min(width - 1), (oy + y1).min(height - 1));
            if x0 > x1 || y0 > y1 {
                continue;
            }
            total = Some(match total {
                None => [x0, y0, x1, y1],
                Some([a, b, c, d]) => [a.min(x0), b.min(y0), c.max(x1), d.max(y1)],
            });
        }
        // Within the image: the sizes fit in u32.
        total.map(|[x0, y0, x1, y1]| {
            Rect::new(
                x0 as u32,
                y0 as u32,
                (x1 - x0 + 1) as u32,
                (y1 - y0 + 1) as u32,
            )
        })
    }

    /// This image with some level-0 tiles replaced (`TILE_SIZE²` pixels each in the stored
    /// format, edge tiles padded by repeating their last row and column): every other tile is
    /// shared, and only the pyramid tiles above the replaced ones are recomputed, exactly as
    /// [`Self::from_level0_tiles`] computes them. The frames of a painting stroke (ADR 0027).
    pub fn with_tiles(&self, replaced: Vec<(TileCoord, Arc<[u8]>)>) -> Result<Self, RasterError> {
        let t = TILE_SIZE;
        self.with_changed_tiles(
            replaced
                .into_iter()
                .map(|(coord, tile)| (coord, tile, [0, 0, t, t]))
                .collect(),
        )
    }

    /// [`Self::with_tiles`] where only the pixels `[x0, y0, x1, y1)` of each replaced tile
    /// differ from this image's (padding aside): only the pyramid pixels above them are
    /// recomputed. The result is the same.
    pub fn with_changed_tiles(
        &self,
        replaced: Vec<(TileCoord, Arc<[u8]>, [u32; 4])>,
    ) -> Result<Self, RasterError> {
        let stored = Codec::new(self.stored_format());
        let tile_len = Self::tile_bytes(self.format);
        let mut levels: Vec<RasterLevel> = self.levels.iter().map(RasterLevel::shared).collect();
        let (columns, rows) = (levels[0].grid.columns(), levels[0].grid.rows());
        // The changed areas, in the pixels of the level being rebuilt.
        let mut areas: Vec<[u32; 4]> = Vec::with_capacity(replaced.len());
        for (coord, tile, [ax0, ay0, ax1, ay1]) in replaced {
            if coord.col >= columns || coord.row >= rows {
                return Err(RasterError::TileOutside(coord));
            }
            if tile.len() != tile_len {
                return Err(RasterError::TileLengthMismatch {
                    level: 0,
                    index: (coord.row * columns + coord.col) as usize,
                    expected: tile_len,
                    actual: tile.len(),
                });
            }
            levels[0].tiles[(coord.row * columns + coord.col) as usize] = tile;
            let (x, y) = (coord.col * TILE_SIZE, coord.row * TILE_SIZE);
            let size = levels[0].size;
            let area = [
                x + ax0.min(TILE_SIZE),
                y + ay0.min(TILE_SIZE),
                (x + ax1.min(TILE_SIZE)).min(size.width),
                (y + ay1.min(TILE_SIZE)).min(size.height),
            ];
            if area[0] < area[2] && area[1] < area[3] {
                areas.push(area);
            }
        }
        for level in 1..levels.len() {
            let (finer, coarser) = levels.split_at_mut(level);
            let (finer, coarser) = (&finer[level - 1], &mut coarser[0]);
            let size = coarser.size;
            // Each coarse pixel averages a 2×2 block: halve, rounding outwards.
            for area in &mut areas {
                *area = [
                    area[0] / 2,
                    area[1] / 2,
                    area[2].div_ceil(2).min(size.width),
                    area[3].div_ceil(2).min(size.height),
                ];
            }
            // By coarse tile, the box of its pixels to recompute (padding included when it
            // reaches the level's last column or row).
            let mut boxes: std::collections::BTreeMap<(u32, u32), [usize; 4]> =
                std::collections::BTreeMap::new();
            for &[x0, y0, x1, y1] in &areas {
                for row in y0 / TILE_SIZE..y1.div_ceil(TILE_SIZE) {
                    for col in x0 / TILE_SIZE..x1.div_ceil(TILE_SIZE) {
                        let (tx, ty) = (col * TILE_SIZE, row * TILE_SIZE);
                        let local = |lo: u32, hi: u32, origin: u32, end: u32| {
                            let first = lo.max(origin) - origin;
                            let last = if hi >= end {
                                TILE_SIZE
                            } else {
                                hi.min(origin + TILE_SIZE) - origin
                            };
                            (first as usize, last as usize)
                        };
                        let (bx0, bx1) = local(x0, x1, tx, size.width);
                        let (by0, by1) = local(y0, y1, ty, size.height);
                        boxes
                            .entry((row, col))
                            .and_modify(|b| {
                                *b = [b[0].min(bx0), b[1].min(by0), b[2].max(bx1), b[3].max(by1)]
                            })
                            .or_insert([bx0, by0, bx1, by1]);
                    }
                }
            }
            let columns = coarser.grid.columns();
            let boxes: Vec<((u32, u32), [usize; 4])> = boxes.into_iter().collect();
            let mut tiles: Vec<Vec<u8>> = boxes
                .iter()
                .map(|&((row, col), _)| coarser.tiles[(row * columns + col) as usize].to_vec())
                .collect();
            let spans: Vec<(usize, usize)> = boxes.iter().map(|(_, a)| (a[1], a[3])).collect();
            let row_bytes = TILE_SIZE as usize * stored.bytes_per_pixel;
            let mut work = bands(&mut tiles, &spans, row_bytes);
            parallel_for_each(&mut work, |band| {
                let ((row, col), area) = boxes[band.tile];
                let last = band.first_row + band.rows.len() / row_bytes;
                coarse_pixels(
                    finer,
                    size,
                    col as usize,
                    row as usize,
                    &stored,
                    band.rows,
                    band.first_row,
                    [area[0], band.first_row, area[2], last],
                );
            });
            for (((row, col), _), tile) in boxes.into_iter().zip(tiles) {
                coarser.tiles[(row * columns + col) as usize] = Arc::from(tile);
            }
        }
        let average = average_of(&levels[levels.len() - 1], &stored);
        Ok(Self {
            id: ImageId::next(),
            format: self.format,
            levels,
            average,
            content_bounds: OnceLock::new(),
        })
    }

    /// This image with an alpha channel, every pixel opaque, or `None` when it has one already.
    /// RGB images already store one, so their tiles are shared; gray images are converted to
    /// gray and alpha (a shared tile once). Lossless: what the Eraser needs (ADR 0027).
    pub fn with_alpha(&self) -> Option<Result<Self, RasterError>> {
        let layout = match self.format.layout {
            ChannelLayout::Rgb => ChannelLayout::Rgba,
            ChannelLayout::Gray => ChannelLayout::GrayAlpha,
            _ => return None,
        };
        let format = PixelFormat {
            layout,
            ..self.format
        };
        if self.format.layout == ChannelLayout::Rgb {
            return Some(Ok(Self {
                id: ImageId::next(),
                format,
                levels: self.levels.iter().map(RasterLevel::shared).collect(),
                average: self.average,
                content_bounds: OnceLock::new(),
            }));
        }
        let (from, to) = (
            Codec::new(self.stored_format()),
            Codec::new(stored_format(format)),
        );
        let (fb, tb) = (from.bytes_per_pixel, to.bytes_per_pixel);
        let opaque = to.opaque();
        let mut converted: std::collections::HashMap<*const u8, Arc<[u8]>> =
            std::collections::HashMap::new();
        let tiles = self.levels[0]
            .tiles
            .iter()
            .map(|tile| {
                converted
                    .entry(tile.as_ptr())
                    .or_insert_with(|| {
                        let mut out = vec![0u8; tile.len() / fb * tb];
                        for (src, dst) in tile.chunks_exact(fb).zip(out.chunks_exact_mut(tb)) {
                            dst[..fb].copy_from_slice(src);
                            dst[fb..].copy_from_slice(&opaque);
                        }
                        Arc::from(out)
                    })
                    .clone()
            })
            .collect();
        Some(Self::from_level0_tiles(self.size(), format, tiles))
    }

    /// This image placed `offset` whole tiles (columns, rows) from the top-left corner of a
    /// larger image of `size`, which is filled around it with zeros (transparent, or a gray mask
    /// hiding): what a layer grows to when painted beyond its bounds (ADR 0027). Its tiles are
    /// shared, except the edge tiles that were padded; `None` when it does not fit in `size`.
    pub fn grown(&self, offset: (u32, u32), size: Size) -> Option<Result<Self, RasterError>> {
        let t = TILE_SIZE;
        let old = &self.levels[0];
        let (ox, oy) = (
            u64::from(offset.0) * u64::from(t),
            u64::from(offset.1) * u64::from(t),
        );
        if ox + u64::from(old.size.width) > u64::from(size.width)
            || oy + u64::from(old.size.height) > u64::from(size.height)
        {
            return None;
        }
        let stored = Codec::new(self.stored_format());
        let bpp = stored.bytes_per_pixel;
        let mut zero = vec![0u8; bpp];
        stored.write([0.0; 3], 0.0, &mut zero);
        let empty: Arc<[u8]> = Arc::from(zero.repeat((t * t) as usize));
        let grid = tile_grid(size);
        let mut tiles = Vec::with_capacity(grid.tile_count() as usize);
        for row in 0..grid.rows() {
            for col in 0..grid.columns() {
                let inside = col >= offset.0 && row >= offset.1;
                let coord = TileCoord {
                    col: col.wrapping_sub(offset.0),
                    row: row.wrapping_sub(offset.1),
                };
                let Some(tile) = old.tile(coord).filter(|_| inside) else {
                    tiles.push(Arc::clone(&empty));
                    continue;
                };
                let valid = |extent: u32, at: u32| (extent - at * t).min(t) as usize;
                let (width, height) = (
                    valid(old.size.width, coord.col),
                    valid(old.size.height, coord.row),
                );
                if width == t as usize && height == t as usize {
                    tiles.push(Arc::clone(tile));
                    continue;
                }
                // An edge tile: its padding becomes zeros, then the new edge is padded.
                let mut copy = tile.to_vec();
                for (i, px) in copy.chunks_exact_mut(bpp).enumerate() {
                    if i % t as usize >= width || i / t as usize >= height {
                        px.copy_from_slice(&zero);
                    }
                }
                let new_width = valid(size.width, col);
                let new_height = valid(size.height, row);
                pad_tile(&mut copy, new_width, new_height, bpp);
                tiles.push(Arc::from(copy));
            }
        }
        Some(Self::from_level0_tiles(size, self.format, tiles))
    }

    /// Pyramid levels, finest first. Never empty.
    pub fn levels(&self) -> &[RasterLevel] {
        &self.levels
    }

    /// Average color (linear light, straight alpha) expressed in `target`, computed once from
    /// the coarsest pyramid level. Meant for swatches and placeholders.
    pub fn average_color(&self, target: &ColorSpace) -> LinearRgba {
        self.average.transform(&self.matrix_to(target))
    }

    /// RAM used by the tiles of all levels, in bytes.
    pub fn memory_bytes(&self) -> u64 {
        let tile_bytes = u64::from(TILE_SIZE)
            * u64::from(TILE_SIZE)
            * u64::from(self.stored_format().bytes_per_pixel());
        self.levels
            .iter()
            .map(|l| l.tiles.len() as u64 * tile_bytes)
            .sum()
    }

    /// Matrix from this image's linear RGB to `target` (identity for gray images).
    pub fn matrix_to(&self, target: &ColorSpace) -> crate::color::Mat3 {
        if self.format.layout.is_gray() {
            IDENTITY
        } else {
            self.format.color_space.matrix_to(target)
        }
    }
}

/// What every constructor checks: pixels exist, and color images have a usable RGB space.
fn check_format(size: Size, format: PixelFormat) -> Result<(), RasterError> {
    if size.is_empty() {
        return Err(RasterError::EmptyImage);
    }
    if !format.layout.is_gray() && !format.color_space.primaries.is_valid() {
        return Err(RasterError::InvalidColorSpace(format.color_space));
    }
    Ok(())
}

/// A level of `size` from its tiles, after checking their count and length.
fn checked_level(
    level: usize,
    size: Size,
    tiles: Vec<Arc<[u8]>>,
    stored: &Codec,
) -> Result<RasterLevel, RasterError> {
    let grid = tile_grid(size);
    let expected = grid.tile_count() as usize;
    if tiles.len() != expected {
        return Err(RasterError::TileCountMismatch {
            level,
            expected,
            actual: tiles.len(),
        });
    }
    let tile_bytes = TILE_SIZE as usize * TILE_SIZE as usize * stored.bytes_per_pixel;
    if let Some((index, tile)) = tiles
        .iter()
        .enumerate()
        .find(|(_, t)| t.len() != tile_bytes)
    {
        return Err(RasterError::TileLengthMismatch {
            level,
            index,
            expected: tile_bytes,
            actual: tile.len(),
        });
    }
    Ok(RasterLevel { size, grid, tiles })
}

fn stored_format(format: PixelFormat) -> PixelFormat {
    let layout = match format.layout {
        ChannelLayout::Rgb => ChannelLayout::Rgba,
        other => other,
    };
    PixelFormat { layout, ..format }
}

/// Average (linear, straight alpha) of a level that fits in one tile.
fn average_of(level: &RasterLevel, codec: &Codec) -> LinearRgba {
    let t = TILE_SIZE as usize;
    let (w, h) = (
        (level.size.width as usize).min(t),
        (level.size.height as usize).min(t),
    );
    let mut sum = [0.0f64; 4];
    if let Some(tile) = level.tiles.first() {
        for y in 0..h {
            for x in 0..w {
                let ([r, g, b], a) = codec.read(&tile[(y * t + x) * codec.bytes_per_pixel..]);
                sum[0] += f64::from(r);
                sum[1] += f64::from(g);
                sum[2] += f64::from(b);
                sum[3] += f64::from(a);
            }
        }
    }
    let n = (w * h).max(1) as f64;
    let unpremultiply = |v: f64| {
        if sum[3] > 0.0 {
            (v / sum[3]) as f32
        } else {
            0.0
        }
    };
    LinearRgba::new(
        unpremultiply(sum[0]),
        unpremultiply(sum[1]),
        unpremultiply(sum[2]),
        (sum[3] / n) as f32,
    )
}

fn tile_grid(size: Size) -> TileGrid {
    // Invariant: TILE_SIZE is a non-zero constant.
    TileGrid::new(size, NonZeroU32::new(TILE_SIZE).expect("TILE_SIZE > 0"))
}

/// Linear value of every code of an integer sample type (index = code), exactly as pixels are
/// decoded on import; empty for float samples. Export quantizes against the same values, which
/// is what makes 8/16-bit round trips exact (see [`crate::convert`]).
pub(crate) fn decode_levels(transfer: TransferFunction, sample: SampleType) -> Vec<f32> {
    match sample {
        SampleType::U8 => (0..=255u32)
            .map(|v| transfer.decode(v as f32 / 255.0))
            .collect(),
        SampleType::U16 => (0..=65535u32)
            .map(|v| transfer.decode(v as f32 / 65535.0))
            .collect(),
        SampleType::F16 | SampleType::F32 => Vec::new(),
    }
}

/// Reads and writes pixels of one format, converting to and from linear premultiplied light.
#[derive(Debug)]
pub(crate) struct Codec {
    sample: SampleType,
    channels: usize,
    color_channels: usize,
    has_alpha: bool,
    premultiplied: bool,
    transfer: TransferFunction,
    pub(crate) bytes_per_pixel: usize,
    /// Raw integer sample → linear, for 8/16-bit data.
    decode_lut: Vec<f32>,
}

impl Codec {
    pub(crate) fn new(format: PixelFormat) -> Self {
        let sample = format.sample;
        let transfer = format.color_space.transfer;
        let decode_lut = decode_levels(transfer, sample);
        let channels = format.layout.channels() as usize;
        Self {
            sample,
            channels,
            color_channels: if format.layout.is_gray() { 1 } else { 3 },
            has_alpha: format.layout.has_alpha(),
            premultiplied: format.alpha == AlphaMode::Premultiplied,
            transfer,
            bytes_per_pixel: channels * sample.bytes() as usize,
            decode_lut,
        }
    }

    /// One sample; float samples go through `map` (see [`Self::read_mapped`]).
    fn raw(&self, px: &[u8], channel: usize, map: &mut impl FnMut(f32) -> f32) -> RawSample {
        let b = self.sample.bytes() as usize;
        let s = &px[channel * b..][..b];
        match self.sample {
            SampleType::U8 => RawSample::Int(u32::from(s[0])),
            SampleType::U16 => RawSample::Int(u32::from(u16::from_ne_bytes([s[0], s[1]]))),
            SampleType::F16 => RawSample::Float(map(f16_to_f32(u16::from_ne_bytes([s[0], s[1]])))),
            SampleType::F32 => RawSample::Float(map(f32::from_ne_bytes([s[0], s[1], s[2], s[3]]))),
        }
    }

    fn unit(&self, raw: RawSample) -> f32 {
        match raw {
            RawSample::Int(v) => match self.sample {
                SampleType::U8 => v as f32 / 255.0,
                _ => v as f32 / 65535.0,
            },
            RawSample::Float(v) => v,
        }
    }

    fn linear(&self, raw: RawSample, map: &mut impl FnMut(f32) -> f32) -> f32 {
        match raw {
            RawSample::Int(v) => self.decode_lut[v as usize],
            RawSample::Float(v) if self.transfer.is_linear() => v,
            // A finite sample can still decode out of range (HLG grows exponentially).
            RawSample::Float(v) => map(self.transfer.decode(v)),
        }
    }

    /// Linear premultiplied color (gray replicated) and alpha of one pixel, as read for
    /// averaging and display (float values clamped, see [`MAX_FINITE_SAMPLE`]).
    fn read(&self, px: &[u8]) -> ([f32; 3], f32) {
        self.read_mapped(px, &mut finite)
    }

    /// [`Self::read`] with a custom rule for float values: `map` receives every float sample
    /// and every decoded float value, and returns what to use instead (e.g. finite values
    /// unchanged, non-finite ones replaced and counted).
    pub(crate) fn read_mapped(
        &self,
        px: &[u8],
        map: &mut impl FnMut(f32) -> f32,
    ) -> ([f32; 3], f32) {
        let alpha = if self.has_alpha {
            self.unit(self.raw(px, self.channels - 1, map))
                .clamp(0.0, 1.0)
        } else {
            1.0
        };
        // Premultiplied with a non-linear transfer: the file multiplied *encoded* values by
        // alpha, so decoding needs the straight encoded value first.
        let unpremultiply_first = self.premultiplied && !self.transfer.is_linear();
        let mut color = [0.0; 3];
        for (c, value) in color.iter_mut().enumerate().take(self.color_channels) {
            let raw = self.raw(px, c, map);
            *value = if unpremultiply_first {
                if alpha > 0.0 {
                    map(self.transfer.decode(self.unit(raw) / alpha)) * alpha
                } else {
                    0.0
                }
            } else if self.premultiplied {
                self.linear(raw, map)
            } else {
                self.linear(raw, map) * alpha
            };
        }
        if self.color_channels == 1 {
            color = [color[0]; 3];
        }
        (color, alpha)
    }

    /// Write a linear premultiplied pixel (gray uses the first component), in the same alpha
    /// convention as the source (see [`Self::read`]).
    pub(crate) fn write(&self, color: [f32; 3], alpha: f32, out: &mut [u8]) {
        for (c, &value) in color.iter().enumerate().take(self.color_channels) {
            let encoded = if self.premultiplied && self.transfer.is_linear() {
                value
            } else {
                let straight = if alpha > 0.0 { value / alpha } else { 0.0 };
                let encoded = self.transfer.encode(straight);
                if self.premultiplied {
                    encoded * alpha
                } else {
                    encoded
                }
            };
            self.put(encoded, c, out);
        }
        if self.has_alpha {
            self.put(alpha, self.channels - 1, out);
        }
    }

    /// Store a unit-range (or, for floats, unbounded) value into a channel.
    fn put(&self, value: f32, channel: usize, out: &mut [u8]) {
        let b = self.sample.bytes() as usize;
        let s = &mut out[channel * b..][..b];
        match self.sample {
            SampleType::U8 => s[0] = (value.clamp(0.0, 1.0) * 255.0).round() as u8,
            SampleType::U16 => {
                s.copy_from_slice(&((value.clamp(0.0, 1.0) * 65535.0).round() as u16).to_ne_bytes())
            }
            SampleType::F16 => s.copy_from_slice(&f32_to_f16(value).to_ne_bytes()),
            SampleType::F32 => s.copy_from_slice(&value.to_ne_bytes()),
        }
    }

    /// The alpha of pixel `px` in `[0, 1]`, 1 without an alpha sample (as [`Self::read`]).
    pub(crate) fn alpha(&self, px: &[u8]) -> f32 {
        if !self.has_alpha {
            return 1.0;
        }
        self.unit(self.raw(px, self.channels - 1, &mut |v| v))
            .clamp(0.0, 1.0)
    }

    /// Multiply the alpha of pixel `px` by `factor` (in `[0, 1]`), its color samples untouched;
    /// `false` (nothing changed) where colors depend on alpha: premultiplied samples, or no
    /// alpha sample.
    pub(crate) fn scale_alpha(&self, px: &mut [u8], factor: f32) -> bool {
        if !self.has_alpha || self.premultiplied {
            return false;
        }
        let channel = self.channels - 1;
        let alpha = self.unit(self.raw(px, channel, &mut |v| v));
        self.put(alpha * factor.clamp(0.0, 1.0), channel, px);
        true
    }

    /// Bytes of a fully opaque alpha sample.
    fn opaque(&self) -> Vec<u8> {
        match self.sample {
            SampleType::U8 => vec![255],
            SampleType::U16 => 65535u16.to_ne_bytes().to_vec(),
            SampleType::F16 => f32_to_f16(1.0).to_ne_bytes().to_vec(),
            SampleType::F32 => 1.0f32.to_ne_bytes().to_vec(),
        }
    }
}

/// Fill the padding of a tile whose `width` × `height` first pixels are valid: each row repeats
/// its last valid pixel, then the rows below repeat the last valid row (as tiles are padded).
pub(crate) fn pad_tile(tile: &mut [u8], width: usize, height: usize, bpp: usize) {
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

/// `f` on every item, on every core (each thread taking a contiguous share); its results in
/// order.
pub(crate) fn parallel_for_each<T: Send, R: Send>(
    items: &mut [T],
    f: impl Fn(&mut T) -> R + Sync,
) -> Vec<R> {
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    if items.len() <= 1 || threads == 1 {
        return items.iter_mut().map(f).collect();
    }
    let per_thread = items.len().div_ceil(threads);
    let f = &f;
    std::thread::scope(|scope| {
        let workers: Vec<_> = items
            .chunks_mut(per_thread)
            .map(|chunk| scope.spawn(move || chunk.iter_mut().map(f).collect::<Vec<R>>()))
            .collect();
        workers
            .into_iter()
            // Invariant: `f` does not panic; if it does, that bug is raised again here.
            .flat_map(|worker| worker.join().expect("worker panicked"))
            .collect()
    })
}

/// Rows of one tile's buffer, from `first_row` on, for one thread to compute.
pub(crate) struct Band<'a> {
    /// Index of the tile in the buffers given to [`bands`].
    pub tile: usize,
    pub first_row: usize,
    pub rows: &'a mut [u8],
}

/// Rows per band: enough work per thread, enough bands to keep every core busy.
const BAND_ROWS: usize = 16;

/// The rows `[y0, y1)` of each tile buffer (`spans[i]` for `tiles[i]`), cut into bands that
/// can be computed in parallel.
pub(crate) fn bands<'a>(
    tiles: &'a mut [Vec<u8>],
    spans: &[(usize, usize)],
    row_bytes: usize,
) -> Vec<Band<'a>> {
    let mut out = Vec::new();
    for (tile, (buffer, &(y0, y1))) in tiles.iter_mut().zip(spans).enumerate() {
        let mut rest = &mut buffer[y0 * row_bytes..y1 * row_bytes];
        let mut first_row = y0;
        while !rest.is_empty() {
            let n = BAND_ROWS.min(rest.len() / row_bytes);
            let (rows, tail) = std::mem::take(&mut rest).split_at_mut(n * row_bytes);
            out.push(Band {
                tile,
                first_row,
                rows,
            });
            rest = tail;
            first_row += n;
        }
    }
    out
}

/// Float values as read for averaging and display: NaN → 0, everything else clamped to
/// ±[`MAX_FINITE_SAMPLE`] (finite values included).
fn finite(v: f32) -> f32 {
    if v.is_nan() {
        0.0
    } else {
        v.clamp(-MAX_FINITE_SAMPLE, MAX_FINITE_SAMPLE)
    }
}

#[derive(Clone, Copy)]
enum RawSample {
    Int(u32),
    Float(f32),
}

/// Cut packed rows (`source` format) into padded tiles in the `stored` format. Only RGB →
/// RGBA differs between the two, by adding an opaque alpha sample.
fn tile_level(size: Size, pixels: &[u8], source: &Codec, stored: &Codec) -> RasterLevel {
    let grid = tile_grid(size);
    let t = TILE_SIZE as usize;
    let width = size.width as usize;
    let (src_bpp, dst_bpp) = (source.bytes_per_pixel, stored.bytes_per_pixel);
    let expand = src_bpp != dst_bpp;
    let opaque = stored.opaque();
    let mut tiles = Vec::with_capacity(grid.tile_count() as usize);
    for row in 0..grid.rows() {
        for col in 0..grid.columns() {
            let mut tile = vec![0u8; t * t * dst_bpp];
            let x0 = col as usize * t;
            let y0 = row as usize * t;
            let w = (width - x0).min(t);
            let h = (size.height as usize - y0).min(t);
            for ty in 0..t {
                // Pad by repeating the last row/column of the image.
                let sy = y0 + ty.min(h - 1);
                let src = &pixels[(sy * width + x0) * src_bpp..][..w * src_bpp];
                let dst = &mut tile[ty * t * dst_bpp..][..t * dst_bpp];
                if expand {
                    for (s, d) in src.chunks_exact(src_bpp).zip(dst.chunks_exact_mut(dst_bpp)) {
                        d[..src_bpp].copy_from_slice(s);
                        d[src_bpp..].copy_from_slice(&opaque);
                    }
                } else {
                    dst[..w * dst_bpp].copy_from_slice(src);
                }
                let last = dst[(w - 1) * dst_bpp..w * dst_bpp].to_vec();
                for px in dst[w * dst_bpp..].chunks_exact_mut(dst_bpp) {
                    px.copy_from_slice(&last);
                }
            }
            tiles.push(Arc::from(tile));
        }
    }
    RasterLevel { size, grid, tiles }
}

/// Level-0 tiles of [`RasterImage::from_placed`]: `pixels` in `rect`, `background` elsewhere,
/// padded like [`tile_level`] pads them. Tiles outside `rect` (padding included) are all one
/// shared background tile.
fn placed_tiles(
    size: Size,
    rect: Rect,
    pixels: &[u8],
    background: &[u8],
    source: &Codec,
    stored: &Codec,
) -> Vec<Arc<[u8]>> {
    let grid = tile_grid(size);
    let t = TILE_SIZE as usize;
    let (src_bpp, dst_bpp) = (source.bytes_per_pixel, stored.bytes_per_pixel);
    let opaque = stored.opaque();
    // One source pixel into one stored pixel (RGB gains an opaque alpha).
    let put = |dst: &mut [u8], src: &[u8]| {
        dst[..src_bpp].copy_from_slice(src);
        if src_bpp != dst_bpp {
            dst[src_bpp..].copy_from_slice(&opaque);
        }
    };
    let mut stored_background = vec![0u8; dst_bpp];
    put(&mut stored_background, background);
    let shared: Arc<[u8]> = stored_background.repeat(t * t).into();

    let (rx, ry) = (rect.x as usize, rect.y as usize);
    let (rw, rh) = (rect.width as usize, rect.height as usize);
    let mut tiles = Vec::with_capacity(grid.tile_count() as usize);
    for row in 0..grid.rows() as usize {
        for col in 0..grid.columns() as usize {
            let (x0, y0) = (col * t, row * t);
            // Valid part of the tile; the rest repeats its last row and column.
            let w = (size.width as usize - x0).min(t);
            let h = (size.height as usize - y0).min(t);
            let overlaps =
                !rect.is_empty() && x0 < rx + rw && rx < x0 + w && y0 < ry + rh && ry < y0 + h;
            if !overlaps {
                tiles.push(Arc::clone(&shared));
                continue;
            }
            let mut tile = vec![0u8; t * t * dst_bpp];
            // Columns of the tile inside `rect`: [start, end).
            let start = rx.saturating_sub(x0).min(w);
            let end = (rx + rw).saturating_sub(x0).min(w);
            for ty in 0..h {
                let y = y0 + ty;
                let dst = &mut tile[ty * t * dst_bpp..][..t * dst_bpp];
                let inside = if (ry..ry + rh).contains(&y) {
                    start..end
                } else {
                    0..0
                };
                for (tx, px) in dst.chunks_exact_mut(dst_bpp).take(w).enumerate() {
                    if !inside.contains(&tx) {
                        px.copy_from_slice(&stored_background);
                    }
                }
                if !inside.is_empty() {
                    let from = ((y - ry) * rw + x0 + inside.start - rx) * src_bpp;
                    let src = &pixels[from..][..inside.len() * src_bpp];
                    let dst = &mut dst[inside.start * dst_bpp..inside.end * dst_bpp];
                    if src_bpp == dst_bpp {
                        dst.copy_from_slice(src);
                    } else {
                        for (s, d) in src.chunks_exact(src_bpp).zip(dst.chunks_exact_mut(dst_bpp)) {
                            put(d, s);
                        }
                    }
                }
                let last = dst[(w - 1) * dst_bpp..w * dst_bpp].to_vec();
                for px in dst[w * dst_bpp..].chunks_exact_mut(dst_bpp) {
                    px.copy_from_slice(&last);
                }
            }
            let (valid, padding) = tile.split_at_mut(h * t * dst_bpp);
            let last = &valid[(h - 1) * t * dst_bpp..];
            for padded_row in padding.chunks_exact_mut(t * dst_bpp) {
                padded_row.copy_from_slice(last);
            }
            tiles.push(Arc::from(tile));
        }
    }
    tiles
}

/// The next pyramid level of `finer`, computed tile by tile: each output pixel averages the
/// same 2×2 block as [`downsample`] does on packed rows, and output tiles are padded like
/// [`tile_level`] pads them. The result is identical: padding repeats the last row and column,
/// so reading a padded position gives the edge pixel that [`downsample`] clamps to.
fn downsample_level(finer: &RasterLevel, stored: &Codec) -> RasterLevel {
    let size = Size::new(finer.size.width.div_ceil(2), finer.size.height.div_ceil(2));
    let grid = tile_grid(size);
    let columns = grid.columns() as usize;
    let mut tiles: Vec<Arc<[u8]>> = Vec::with_capacity(grid.tile_count() as usize);
    let count = grid.tile_count() as usize;
    let shared = shared_coarse_tiles(finer, columns, count, stored);
    let shared = &shared;
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let per_thread = count.div_ceil(threads).max(1);
    let mut computed: Vec<Vec<Arc<[u8]>>> = Vec::new();
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..count)
            .step_by(per_thread)
            .map(|first| {
                scope.spawn(move || {
                    (first..(first + per_thread).min(count))
                        .map(|index| {
                            if let Some(tile) = &shared[index] {
                                return Arc::clone(tile);
                            }
                            coarse_tile(finer, size, index % columns, index / columns, stored)
                        })
                        .collect::<Vec<Arc<[u8]>>>()
                })
            })
            .collect();
        for worker in workers {
            // Invariant: the closure above does not panic on valid levels.
            computed.push(worker.join().expect("pyramid worker panicked"));
        }
    });
    for part in computed {
        tiles.extend(part);
    }
    RasterLevel { size, grid, tiles }
}

/// Tile (`col`, `row`) of the level of `size` above `finer`: each pixel averages its 2×2 block
/// of `finer` in linear light (premultiplied), padded like [`tile_level`] pads.
fn coarse_tile(
    finer: &RasterLevel,
    size: Size,
    col: usize,
    row: usize,
    stored: &Codec,
) -> Arc<[u8]> {
    let t = TILE_SIZE as usize;
    let mut tile = vec![0u8; t * t * stored.bytes_per_pixel];
    coarse_pixels(finer, size, col, row, stored, &mut tile, 0, [0, 0, t, t]);
    Arc::from(tile)
}

/// The pixels `[x0, y0, x1, y1)` of [`coarse_tile`], written into `rows`: the tile's rows from
/// `first_row` on.
#[allow(clippy::too_many_arguments)]
fn coarse_pixels(
    finer: &RasterLevel,
    size: Size,
    col: usize,
    row: usize,
    stored: &Codec,
    rows: &mut [u8],
    first_row: usize,
    [x0, y0, x1, y1]: [usize; 4],
) {
    let t = TILE_SIZE as usize;
    let bpp = stored.bytes_per_pixel;
    let (fw, fh) = (finer.size.width as usize, finer.size.height as usize);
    let (w, h) = (size.width as usize, size.height as usize);
    let finer_columns = finer.grid.columns() as usize;
    for ty in y0..y1 {
        for tx in x0..x1 {
            let dst = &mut rows[((ty - first_row) * t + tx) * bpp..][..bpp];
            // Padding: the last valid pixel of the level.
            let x = (col * t + tx).min(w - 1);
            let y = (row * t + ty).min(h - 1);
            let mut color = [0.0f32; 3];
            let mut alpha = 0.0f32;
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let fx = (x * 2 + dx).min(fw - 1);
                let fy = (y * 2 + dy).min(fh - 1);
                let source = &finer.tiles[(fy / t) * finer_columns + fx / t];
                let at = ((fy % t) * t + fx % t) * bpp;
                let (c, a) = stored.read(&source[at..]);
                for k in 0..3 {
                    color[k] += c[k];
                }
                alpha += a;
            }
            stored.write(color.map(|c| c / 4.0), alpha / 4.0, dst);
        }
    }
}

/// Coarse tiles of the next level that need no computing, by index: a coarse tile whose finer
/// tiles are all one shared, uniform tile (e.g. the background of [`RasterImage::from_placed`])
/// is uniform too, and is itself shared. Its pixel is averaged exactly as the general path
/// averages it, so the result is identical.
fn shared_coarse_tiles(
    finer: &RasterLevel,
    columns: usize,
    count: usize,
    stored: &Codec,
) -> Vec<Option<Arc<[u8]>>> {
    let t = TILE_SIZE as usize;
    let bpp = stored.bytes_per_pixel;
    let finer_columns = finer.grid.columns() as usize;
    let finer_rows = finer.grid.rows() as usize;
    // By address of the finer tile: its coarse tile, or `None` if it is not uniform.
    let mut known: std::collections::HashMap<*const u8, Option<Arc<[u8]>>> =
        std::collections::HashMap::new();
    (0..count)
        .map(|index| {
            let (col, row) = (index % columns, index / columns);
            let first = &finer.tiles
                [(2 * row).min(finer_rows - 1) * finer_columns + (2 * col).min(finer_columns - 1)];
            let same = (0..2).all(|dy| {
                (0..2).all(|dx| {
                    let (c, r) = (
                        (2 * col + dx).min(finer_columns - 1),
                        (2 * row + dy).min(finer_rows - 1),
                    );
                    Arc::ptr_eq(first, &finer.tiles[r * finer_columns + c])
                })
            });
            if !same {
                return None;
            }
            known
                .entry(first.as_ptr())
                .or_insert_with(|| {
                    let pixel = &first[..bpp];
                    if !first.chunks_exact(bpp).all(|px| px == pixel) {
                        return None;
                    }
                    let mut color = [0.0f32; 3];
                    let mut alpha = 0.0f32;
                    for _ in 0..4 {
                        let (c, a) = stored.read(pixel);
                        for k in 0..3 {
                            color[k] += c[k];
                        }
                        alpha += a;
                    }
                    let mut coarse = vec![0u8; bpp];
                    stored.write(color.map(|c| c / 4.0), alpha / 4.0, &mut coarse);
                    Some(coarse.repeat(t * t).into())
                })
                .clone()
        })
        .collect()
}

/// Halve an image (rounding up), averaging 2×2 blocks in linear light with premultiplied
/// alpha, as correct downsampling requires. Reads `source`, writes `stored`.
fn downsample(pixels: &[u8], size: Size, source: &Codec, stored: &Codec) -> (Vec<u8>, Size) {
    let next = Size::new(size.width.div_ceil(2), size.height.div_ceil(2));
    let (w, h) = (size.width as usize, size.height as usize);
    let (nw, nh) = (next.width as usize, next.height as usize);
    let (src_bpp, dst_bpp) = (source.bytes_per_pixel, stored.bytes_per_pixel);
    let mut out = vec![0u8; nw * nh * dst_bpp];

    // Rows are independent: split them across threads (a 100 MP image is ~25 M output pixels).
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let rows_per_chunk = nh.div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        for (chunk_index, chunk) in out.chunks_mut(rows_per_chunk * nw * dst_bpp).enumerate() {
            scope.spawn(move || {
                for (i, dst) in chunk.chunks_exact_mut(dst_bpp).enumerate() {
                    let ny = chunk_index * rows_per_chunk + i / nw;
                    let nx = i % nw;
                    let mut color = [0.0f32; 3];
                    let mut alpha = 0.0f32;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        // Clamp at odd edges: the last row/column counts twice.
                        let x = (nx * 2 + dx).min(w - 1);
                        let y = (ny * 2 + dy).min(h - 1);
                        let (c, a) = source.read(&pixels[(y * w + x) * src_bpp..]);
                        for k in 0..3 {
                            color[k] += c[k];
                        }
                        alpha += a;
                    }
                    stored.write(color.map(|c| c / 4.0), alpha / 4.0, dst);
                }
            });
        }
    });
    (out, next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::WORKING_SPACE;

    fn format(layout: ChannelLayout, sample: SampleType, space: ColorSpace) -> PixelFormat {
        PixelFormat {
            layout,
            sample,
            color_space: space,
            alpha: AlphaMode::Straight,
        }
    }

    fn solid(size: Size, rgba: [u8; 4]) -> Vec<u8> {
        rgba.repeat(size.pixel_count() as usize)
    }

    fn tile_pixel(img: &RasterImage, level: usize, x: u32, y: u32) -> Vec<u8> {
        let bpp = img.stored_format().bytes_per_pixel() as usize;
        let tile = img.levels()[level]
            .tile(TileCoord {
                col: x / TILE_SIZE,
                row: y / TILE_SIZE,
            })
            .unwrap();
        let i = ((y % TILE_SIZE) * TILE_SIZE + x % TILE_SIZE) as usize * bpp;
        tile[i..i + bpp].to_vec()
    }

    #[test]
    fn rejects_bad_input() {
        let px = solid(Size::new(2, 2), [0, 0, 0, 255]);
        assert_eq!(
            RasterImage::from_pixels(Size::new(0, 2), PixelFormat::RGBA8_SRGB, &[]).unwrap_err(),
            RasterError::EmptyImage
        );
        assert!(matches!(
            RasterImage::from_pixels(Size::new(3, 2), PixelFormat::RGBA8_SRGB, &px),
            Err(RasterError::SizeMismatch { .. })
        ));
        let mut broken = PixelFormat::RGBA8_SRGB;
        broken.color_space.primaries.red = [0.3, 0.6];
        broken.color_space.primaries.green = [0.3, 0.6];
        assert!(matches!(
            RasterImage::from_pixels(Size::new(2, 2), broken, &px),
            Err(RasterError::InvalidColorSpace(_))
        ));
    }

    #[test]
    fn tiles_keep_source_pixels_intact() {
        // 300 × 5: two tile columns, one row; distinct pixel values everywhere.
        let size = Size::new(300, 5);
        let px: Vec<u8> = (0..size.pixel_count() as usize * 4)
            .map(|i| (i % 251) as u8)
            .collect();
        let img = RasterImage::from_pixels(size, PixelFormat::RGBA8_SRGB, &px).unwrap();
        assert_eq!(img.levels()[0].grid().columns(), 2);
        for y in 0..5u32 {
            for x in 0..300u32 {
                let i = ((y * 300 + x) * 4) as usize;
                assert_eq!(tile_pixel(&img, 0, x, y), &px[i..i + 4], "pixel ({x}, {y})");
            }
        }
        // Padding repeats the edge pixel.
        let t = TILE_SIZE as usize;
        let edge = img.levels()[0].tile(TileCoord { col: 1, row: 0 }).unwrap();
        let last_x = 300 - 256 - 1;
        assert_eq!(
            &edge[(4 * t + 200) * 4..][..4],
            &edge[(4 * t + last_x) * 4..][..4]
        );
        assert_eq!(
            &edge[(200 * t + 3) * 4..][..4],
            &edge[(4 * t + 3) * 4..][..4]
        );
    }

    #[test]
    fn sixteen_bit_and_float_samples_are_stored_exactly() {
        let size = Size::new(3, 2);
        // 16-bit gray: every value must survive (no 8-bit truncation).
        let gray16: Vec<u8> = [0u16, 1, 257, 32768, 65534, 65535]
            .iter()
            .flat_map(|v| v.to_ne_bytes())
            .collect();
        let fmt = format(ChannelLayout::Gray, SampleType::U16, ColorSpace::SRGB);
        let img = RasterImage::from_pixels(size, fmt, &gray16).unwrap();
        assert_eq!(tile_pixel(&img, 0, 1, 0), 1u16.to_ne_bytes());
        assert_eq!(tile_pixel(&img, 0, 2, 1), 65535u16.to_ne_bytes());

        // f32 RGB with HDR and negative values: stored as RGBA with alpha 1.0.
        let values = [
            [2.5f32, -0.25, 1e-6],
            [0.0, 1.0, 100.0],
            [0.5, 0.5, 0.5],
            [3.0, 0.0, 0.0],
            [0.0, 0.0, 0.0],
            [1.0, 1.0, 1.0],
        ];
        let rgb32: Vec<u8> = values
            .iter()
            .flatten()
            .flat_map(|v| v.to_ne_bytes())
            .collect();
        let fmt = format(
            ChannelLayout::Rgb,
            SampleType::F32,
            ColorSpace::LINEAR_REC2020,
        );
        let img = RasterImage::from_pixels(size, fmt, &rgb32).unwrap();
        assert_eq!(img.stored_format().layout, ChannelLayout::Rgba);
        let px = tile_pixel(&img, 0, 0, 0);
        let floats: Vec<f32> = px
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_ne_bytes(*b))
            .collect();
        assert_eq!(floats, [2.5, -0.25, 1e-6, 1.0]);
    }

    /// Every layout and sample type, with values that exercise rounding (not only 0 and max).
    fn formats() -> Vec<PixelFormat> {
        let mut formats = Vec::new();
        for layout in [
            ChannelLayout::Gray,
            ChannelLayout::GrayAlpha,
            ChannelLayout::Rgb,
            ChannelLayout::Rgba,
        ] {
            for sample in [
                SampleType::U8,
                SampleType::U16,
                SampleType::F16,
                SampleType::F32,
            ] {
                for alpha in [AlphaMode::Straight, AlphaMode::Premultiplied] {
                    formats.push(PixelFormat {
                        layout,
                        sample,
                        color_space: ColorSpace::SRGB,
                        alpha,
                    });
                }
            }
        }
        formats
    }

    /// Deterministic pseudo-random bytes (valid for every sample type: floats come out as
    /// ordinary finite values or, rarely, NaN/inf, which the pyramid must handle alike).
    fn noise(len: usize, seed: u64) -> Vec<u8> {
        let mut state = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        (0..len)
            .map(|_| {
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                (state >> 33) as u8
            })
            .collect()
    }

    fn all_tiles(img: &RasterImage) -> Vec<Vec<Arc<[u8]>>> {
        img.levels().iter().map(|l| l.tiles().to_vec()).collect()
    }

    #[test]
    fn tile_by_tile_pyramid_matches_from_pixels() {
        // Odd sizes at every level, one exact multiple of the tile size, and a thin strip.
        let sizes = [
            Size::new(600, 530),
            Size::new(513, 257),
            Size::new(1024, 512),
            Size::new(1100, 3),
        ];
        for (i, format) in formats().into_iter().enumerate() {
            for (j, size) in sizes.into_iter().enumerate() {
                let len = size.pixel_count() as usize * format.bytes_per_pixel() as usize;
                let pixels = noise(len, (i * 10 + j) as u64);
                let reference = RasterImage::from_pixels(size, format, &pixels).unwrap();
                let level0 = reference.levels()[0].tiles().to_vec();
                let rebuilt = RasterImage::from_level0_tiles(size, format, level0).unwrap();
                let (a, b) = (all_tiles(&reference), all_tiles(&rebuilt));
                assert_eq!(a.len(), b.len(), "{format:?} {size:?}");
                for (level, (a, b)) in a.iter().zip(&b).enumerate() {
                    assert!(
                        a.iter().zip(b).all(|(x, y)| x[..] == y[..]),
                        "{format:?} {size:?}: level {level} differs"
                    );
                }
                assert_eq!(
                    reference
                        .average_color(&WORKING_SPACE)
                        .to_srgb_encoded()
                        .map(f32::to_bits),
                    rebuilt
                        .average_color(&WORKING_SPACE)
                        .to_srgb_encoded()
                        .map(f32::to_bits),
                );
            }
        }
    }

    #[test]
    fn replaced_tiles_rebuild_only_the_pyramid_above_them() {
        let size = Size::new(1100, 530);
        for (i, format) in formats().into_iter().enumerate() {
            let len = size.pixel_count() as usize * format.bytes_per_pixel() as usize;
            let original = RasterImage::from_pixels(size, format, &noise(len, i as u64)).unwrap();
            let other = RasterImage::from_pixels(size, format, &noise(len, 99)).unwrap();
            let coords = [TileCoord { col: 0, row: 0 }, TileCoord { col: 4, row: 2 }];
            let replaced = coords
                .iter()
                .map(|&c| (c, Arc::clone(other.levels()[0].tile(c).unwrap())))
                .collect();
            let painted = original.with_tiles(replaced).unwrap();
            let mut level0 = original.levels()[0].tiles().to_vec();
            for c in coords {
                level0[(c.row * 5 + c.col) as usize] =
                    Arc::clone(other.levels()[0].tile(c).unwrap());
            }
            let reference = RasterImage::from_level0_tiles(size, format, level0).unwrap();
            for (level, (a, b)) in all_tiles(&reference)
                .iter()
                .zip(&all_tiles(&painted))
                .enumerate()
            {
                assert!(
                    a.iter().zip(b).all(|(x, y)| x[..] == y[..]),
                    "{format:?}: level {level} differs"
                );
            }
            // Untouched tiles are shared, at every level.
            let shared = |level: usize, index: usize| {
                Arc::ptr_eq(
                    &original.levels()[level].tiles()[index],
                    &painted.levels()[level].tiles()[index],
                )
            };
            assert!(shared(0, 1) && !shared(0, 0));
            assert!(shared(1, 2) && !shared(1, 0));
            assert_ne!(original.id(), painted.id());
        }
        let rgba = format(ChannelLayout::Rgba, SampleType::U8, ColorSpace::SRGB);
        let image = RasterImage::from_pixels(size, rgba, &noise(1100 * 530 * 4, 1)).unwrap();
        let tile = Arc::clone(
            image.levels()[0]
                .tile(TileCoord { col: 0, row: 0 })
                .unwrap(),
        );
        assert_eq!(
            image
                .with_tiles(vec![(TileCoord { col: 5, row: 0 }, Arc::clone(&tile))])
                .err(),
            Some(RasterError::TileOutside(TileCoord { col: 5, row: 0 }))
        );
        assert!(matches!(
            image.with_tiles(vec![(TileCoord { col: 0, row: 0 }, Arc::from(&tile[1..]))]),
            Err(RasterError::TileLengthMismatch { .. })
        ));
    }

    #[test]
    fn changed_areas_rebuild_the_same_pyramid() {
        // Odd sizes, so that areas reach the padding of every level.
        let size = Size::new(1100, 531);
        let rgba = format(ChannelLayout::Rgba, SampleType::U16, ColorSpace::SRGB);
        let len = size.pixel_count() as usize * 8;
        let original = RasterImage::from_pixels(size, rgba, &noise(len, 3)).unwrap();
        let other = RasterImage::from_pixels(size, rgba, &noise(len, 4)).unwrap();
        // Tiles whose pixels outside an area are the original's.
        let cases = [
            (TileCoord { col: 1, row: 0 }, [10, 20, 200, 90]),
            (TileCoord { col: 4, row: 2 }, [0, 0, 76, 19]),
            (TileCoord { col: 4, row: 1 }, [70, 250, 256, 256]),
        ];
        let bpp = 8;
        let mixed: Vec<(TileCoord, Arc<[u8]>, [u32; 4])> = cases
            .iter()
            .map(|&(coord, [x0, y0, x1, y1])| {
                let mut tile = original.levels()[0].tile(coord).unwrap().to_vec();
                let new = other.levels()[0].tile(coord).unwrap();
                for y in y0..y1 {
                    for x in x0..x1 {
                        let i = ((y * TILE_SIZE + x) * bpp) as usize;
                        tile[i..i + bpp as usize].copy_from_slice(&new[i..i + bpp as usize]);
                    }
                }
                (coord, Arc::from(tile), [x0, y0, x1, y1])
            })
            .collect();
        let whole = original
            .with_tiles(mixed.iter().map(|(c, t, _)| (*c, Arc::clone(t))).collect())
            .unwrap();
        let partial = original.with_changed_tiles(mixed).unwrap();
        for (level, (a, b)) in all_tiles(&whole)
            .iter()
            .zip(&all_tiles(&partial))
            .enumerate()
        {
            for (index, (x, y)) in a.iter().zip(b).enumerate() {
                assert!(x[..] == y[..], "level {level}, tile {index} differs");
            }
        }
    }

    #[test]
    fn alpha_is_added_without_changing_pixels() {
        let size = Size::new(300, 260);
        let rgb = format(ChannelLayout::Rgb, SampleType::U8, ColorSpace::SRGB);
        let image = RasterImage::from_pixels(size, rgb, &noise(300 * 260 * 3, 5)).unwrap();
        let with = image.with_alpha().unwrap().unwrap();
        assert_eq!(with.format().layout, ChannelLayout::Rgba);
        assert!(Arc::ptr_eq(
            &image.levels()[1].tiles()[0],
            &with.levels()[1].tiles()[0]
        ));
        assert!(with.with_alpha().is_none());

        let gray = format(ChannelLayout::Gray, SampleType::U16, ColorSpace::SRGB);
        let image = RasterImage::from_pixels(size, gray, &noise(300 * 260 * 2, 6)).unwrap();
        let with = image.with_alpha().unwrap().unwrap();
        assert_eq!(with.format().layout, ChannelLayout::GrayAlpha);
        for (x, y) in [(0, 0), (299, 259), (17, 140)] {
            assert_eq!(with.gray_at(x, y), image.gray_at(x, y));
            assert_eq!(with.alpha_at(x, y), 1.0);
        }
    }

    #[test]
    fn placed_images_match_their_full_buffer_and_share_the_background() {
        let size = Size::new(1100, 700);
        let rects = [
            Rect::new(300, 260, 200, 150),
            // Touching the right and bottom edges (padded tiles).
            Rect::new(900, 500, 200, 200),
            Rect::new(0, 0, 1100, 700),
            Rect::new(10, 10, 0, 0),
        ];
        for (i, format) in formats().into_iter().enumerate() {
            let bpp = format.bytes_per_pixel() as usize;
            let background = noise(bpp, 100 + i as u64);
            for (j, rect) in rects.into_iter().enumerate() {
                let pixels = noise(
                    rect.size().pixel_count() as usize * bpp,
                    (i * 10 + j) as u64,
                );
                let mut full = background.repeat(size.pixel_count() as usize);
                for y in 0..rect.height as usize {
                    let row = &pixels[y * rect.width as usize * bpp..][..rect.width as usize * bpp];
                    let at = ((rect.y as usize + y) * size.width as usize + rect.x as usize) * bpp;
                    full[at..at + row.len()].copy_from_slice(row);
                }
                let reference = RasterImage::from_pixels(size, format, &full).unwrap();
                let placed =
                    RasterImage::from_placed(size, format, rect, &pixels, &background).unwrap();
                let (a, b) = (all_tiles(&reference), all_tiles(&placed));
                assert_eq!(a.len(), b.len());
                for (level, (a, b)) in a.iter().zip(&b).enumerate() {
                    assert!(
                        a.iter().zip(b).all(|(x, y)| x[..] == y[..]),
                        "{format:?} {rect:?}: level {level} differs"
                    );
                }
                if j == 0 {
                    // Level 0 (5 × 3 tiles): every tile but the one the rectangle covers is the same.
                    let level0 = &b[0];
                    let shared = level0.iter().filter(|t| Arc::ptr_eq(t, &level0[0])).count();
                    assert_eq!(shared, level0.len() - 1, "{format:?}");
                    // Level 1 (3 × 2 tiles): the last column comes from background only.
                    assert!(Arc::ptr_eq(&b[1][2], &b[1][5]), "{format:?}");
                }
            }
        }
    }

    #[test]
    fn placed_images_check_their_rectangle_and_buffers() {
        let size = Size::new(100, 50);
        let format = PixelFormat::RGBA8_SRGB;
        let rect = Rect::new(90, 40, 20, 5);
        assert_eq!(
            RasterImage::from_placed(size, format, rect, &[0; 400], &[0; 4]).unwrap_err(),
            RasterError::RectOutside { rect, size }
        );
        let rect = Rect::new(0, 0, 2, 2);
        assert!(matches!(
            RasterImage::from_placed(size, format, rect, &[0; 15], &[0; 4]),
            Err(RasterError::SizeMismatch { .. })
        ));
        assert!(matches!(
            RasterImage::from_placed(size, format, rect, &[0; 16], &[0; 3]),
            Err(RasterError::SizeMismatch { .. })
        ));
    }

    #[test]
    fn from_tiles_shares_the_tiles_it_is_given() {
        let size = Size::new(700, 300);
        let format = PixelFormat::RGBA8_SRGB;
        let pixels = noise(size.pixel_count() as usize * 4, 7);
        let reference = RasterImage::from_pixels(size, format, &pixels).unwrap();
        let tiles = all_tiles(&reference);
        let rebuilt = RasterImage::from_tiles(size, format, tiles.clone()).unwrap();
        assert_ne!(rebuilt.id(), reference.id());
        for (level, original) in tiles.iter().enumerate() {
            for (a, b) in original.iter().zip(rebuilt.levels()[level].tiles()) {
                assert!(Arc::ptr_eq(a, b), "tiles must not be copied");
            }
        }
        assert_eq!(
            RasterImage::level_sizes(size),
            reference
                .levels()
                .iter()
                .map(|l| l.size())
                .collect::<Vec<_>>()
        );
        assert_eq!(RasterImage::tile_bytes(format), 256 * 256 * 4);
    }

    #[test]
    fn from_tiles_rejects_inconsistent_tiles() {
        let size = Size::new(300, 20);
        let format = PixelFormat::RGBA8_SRGB;
        let reference =
            RasterImage::from_pixels(size, format, &solid(size, [1, 2, 3, 255])).unwrap();
        let tiles = all_tiles(&reference);
        assert_eq!(tiles.len(), 2);
        assert_eq!(
            RasterImage::from_tiles(size, format, tiles[..1].to_vec()).unwrap_err(),
            RasterError::LevelCountMismatch {
                expected: 2,
                actual: 1
            }
        );
        let mut missing = tiles.clone();
        missing[0].pop();
        assert!(matches!(
            RasterImage::from_tiles(size, format, missing).unwrap_err(),
            RasterError::TileCountMismatch { level: 0, .. }
        ));
        let mut short = tiles.clone();
        short[1][0] = Arc::from(vec![0u8; 10]);
        assert!(matches!(
            RasterImage::from_tiles(size, format, short).unwrap_err(),
            RasterError::TileLengthMismatch {
                level: 1,
                index: 0,
                ..
            }
        ));
        assert!(matches!(
            RasterImage::from_level0_tiles(Size::new(0, 5), format, vec![]).unwrap_err(),
            RasterError::EmptyImage
        ));
    }

    #[test]
    fn pyramid_goes_down_to_one_tile() {
        let size = Size::new(1000, 600);
        let img =
            RasterImage::from_pixels(size, PixelFormat::RGBA8_SRGB, &solid(size, [9, 9, 9, 255]))
                .unwrap();
        let sizes: Vec<_> = img.levels().iter().map(|l| l.size()).collect();
        assert_eq!(
            sizes,
            [
                Size::new(1000, 600),
                Size::new(500, 300),
                Size::new(250, 150)
            ]
        );
        assert!(img.memory_bytes() > 0);
    }

    #[test]
    fn downsampling_averages_in_linear_light() {
        // Black and white stripes average to linear 0.5, i.e. sRGB ~188, not 128.
        let size = Size::new(512, 2);
        let px: Vec<u8> = (0..size.pixel_count())
            .flat_map(|i| {
                if i % 2 == 0 {
                    [0, 0, 0, 255]
                } else {
                    [255, 255, 255, 255]
                }
            })
            .collect();
        let img = RasterImage::from_pixels(size, PixelFormat::RGBA8_SRGB, &px).unwrap();
        assert_eq!(tile_pixel(&img, 1, 0, 0), [188, 188, 188, 255]);
    }

    #[test]
    fn float_pyramid_keeps_hdr_values() {
        // Linear float stripes of 0 and 4: the average is 2.0, far above 1.0, not clipped.
        let size = Size::new(512, 2);
        let px: Vec<u8> = (0..size.pixel_count())
            .flat_map(|i| {
                let v = if i % 2 == 0 { 0.0f32 } else { 4.0 };
                [v, v, v, 1.0]
            })
            .flat_map(|v| v.to_ne_bytes())
            .collect();
        let fmt = format(
            ChannelLayout::Rgba,
            SampleType::F32,
            ColorSpace::LINEAR_REC2020,
        );
        let img = RasterImage::from_pixels(size, fmt, &px).unwrap();
        let level1: Vec<f32> = tile_pixel(&img, 1, 0, 0)
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_ne_bytes(*b))
            .collect();
        assert_eq!(level1, [2.0, 2.0, 2.0, 1.0]);
    }

    #[test]
    fn half_float_pyramid() {
        let size = Size::new(514, 1);
        let px: Vec<u8> = (0..size.pixel_count())
            .flat_map(|i| {
                let v = if i % 2 == 0 { 1.0f32 } else { 3.0 };
                [v, 1.0]
            })
            .flat_map(|v| f32_to_f16(v).to_ne_bytes())
            .collect();
        let fmt = format(
            ChannelLayout::GrayAlpha,
            SampleType::F16,
            ColorSpace::LINEAR_SRGB,
        );
        let img = RasterImage::from_pixels(size, fmt, &px).unwrap();
        let level1 = tile_pixel(&img, 1, 0, 0);
        assert_eq!(f16_to_f32(u16::from_ne_bytes([level1[0], level1[1]])), 2.0);
    }

    #[test]
    fn transparent_pixels_do_not_darken_averages() {
        // Premultiplied averaging: a transparent black neighbor must not pull red toward black.
        let size = Size::new(514, 2);
        let px: Vec<u8> = (0..size.pixel_count())
            .flat_map(|i| {
                if i % 2 == 0 {
                    [255, 0, 0, 255]
                } else {
                    [0, 0, 0, 0]
                }
            })
            .collect();
        let img = RasterImage::from_pixels(size, PixelFormat::RGBA8_SRGB, &px).unwrap();
        assert_eq!(tile_pixel(&img, 1, 0, 0), [255, 0, 0, 128]);
    }

    #[test]
    fn premultiplied_sources_stay_premultiplied() {
        // Premultiplied linear f32: (0.5, 0, 0, 0.5) next to (0, 0, 0, 0) averages to
        // (0.25, 0, 0, 0.25), still premultiplied.
        let size = Size::new(512, 1);
        let px: Vec<u8> = (0..size.pixel_count())
            .flat_map(|i| {
                if i % 2 == 0 {
                    [0.5f32, 0.0, 0.0, 0.5]
                } else {
                    [0.0; 4]
                }
            })
            .flat_map(|v| v.to_ne_bytes())
            .collect();
        let fmt = PixelFormat {
            alpha: AlphaMode::Premultiplied,
            ..format(
                ChannelLayout::Rgba,
                SampleType::F32,
                ColorSpace::LINEAR_SRGB,
            )
        };
        let img = RasterImage::from_pixels(size, fmt, &px).unwrap();
        let level1: Vec<f32> = tile_pixel(&img, 1, 0, 0)
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_ne_bytes(*b))
            .collect();
        assert_eq!(level1, [0.25, 0.0, 0.0, 0.25]);
    }

    #[test]
    fn average_color_is_converted_to_the_target_space() {
        let size = Size::new(600, 300);
        let img = RasterImage::from_pixels(
            size,
            PixelFormat::RGBA8_SRGB,
            &solid(size, [255, 0, 0, 255]),
        )
        .unwrap();
        let in_srgb = img.average_color(&ColorSpace::LINEAR_SRGB);
        assert!((in_srgb.r - 1.0).abs() < 1e-3 && in_srgb.g.abs() < 1e-3);
        let in_working = img.average_color(&WORKING_SPACE);
        let expected = LinearRgba::new(1.0, 0.0, 0.0, 1.0)
            .transform(&ColorSpace::LINEAR_SRGB.matrix_to(&WORKING_SPACE));
        assert!((in_working.r - expected.r).abs() < 1e-3);
        assert!((in_working.g - expected.g).abs() < 1e-3);
    }

    #[test]
    fn premultiplied_encoded_samples_are_unpremultiplied_before_decoding() {
        // 8-bit sRGB with associated alpha: (128, 128, 128, 128) is white at 50 %.
        let size = Size::new(512, 1);
        let px: Vec<u8> = (0..size.pixel_count())
            .flat_map(|i| {
                if i % 2 == 0 {
                    [128, 128, 128, 128]
                } else {
                    [0, 0, 0, 0]
                }
            })
            .collect();
        let fmt = PixelFormat {
            alpha: AlphaMode::Premultiplied,
            ..PixelFormat::RGBA8_SRGB
        };
        let img = RasterImage::from_pixels(size, fmt, &px).unwrap();
        let average = img.average_color(&ColorSpace::LINEAR_SRGB);
        assert!((average.r - 1.0).abs() < 1e-3, "{average:?}");
        // Still premultiplied, still white: color equals alpha.
        assert_eq!(tile_pixel(&img, 1, 0, 0), [64, 64, 64, 64]);
    }

    #[test]
    fn flat_rec2020_16_bit_areas_do_not_drift() {
        let size = Size::new(512, 2);
        let px: Vec<u8> = (0..size.pixel_count() * 3)
            .flat_map(|_| 5309u16.to_ne_bytes())
            .collect();
        let fmt = format(ChannelLayout::Rgb, SampleType::U16, ColorSpace::REC2020);
        let img = RasterImage::from_pixels(size, fmt, &px).unwrap();
        let level1 = tile_pixel(&img, 1, 0, 0);
        assert_eq!(&level1[..2], 5309u16.to_ne_bytes());
    }

    #[test]
    fn non_finite_samples_do_not_spread() {
        let size = Size::new(512, 1);
        let px: Vec<u8> = (0..size.pixel_count())
            .flat_map(|i| match i % 4 {
                0 => [f32::INFINITY, 0.0, 0.0, 1.0],
                1 => [f32::NAN, 1.0, 1.0, 1.0],
                2 => [1.0, f32::NEG_INFINITY, 1.0, f32::NAN],
                _ => [1.0, 1.0, 1.0, 1.0],
            })
            .flat_map(|v| v.to_ne_bytes())
            .collect();
        let fmt = format(
            ChannelLayout::Rgba,
            SampleType::F32,
            ColorSpace::LINEAR_REC2020,
        );
        let img = RasterImage::from_pixels(size, fmt, &px).unwrap();
        for x in 0..2 {
            let level1: Vec<f32> = tile_pixel(&img, 1, x, 0)
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_ne_bytes(*b))
                .collect();
            assert!(level1.iter().all(|v| v.is_finite()), "{level1:?}");
        }
        let average = img.average_color(&ColorSpace::LINEAR_SRGB);
        assert!(average.is_finite(), "{average:?}");
    }

    #[test]
    fn memory_estimate_bounds_the_real_usage() {
        for (size, layout) in [
            (Size::new(1000, 600), ChannelLayout::Rgba),
            (Size::new(3, 700), ChannelLayout::Rgb),
            (Size::new(257, 1), ChannelLayout::Gray),
        ] {
            let fmt = format(layout, SampleType::U8, ColorSpace::SRGB);
            let bpp = fmt.bytes_per_pixel() as usize;
            let px = vec![7u8; size.pixel_count() as usize * bpp];
            let img = RasterImage::from_pixels(size, fmt, &px).unwrap();
            let estimate = RasterImage::estimated_memory_bytes(size, fmt).unwrap();
            assert!(estimate >= img.memory_bytes(), "{size:?}");
        }
        // A thin strip: padding makes the tiles ~256× the pixel data.
        let strip =
            RasterImage::estimated_memory_bytes(Size::new(1, 50_000_000), PixelFormat::RGBA8_SRGB)
                .unwrap();
        assert!(strip > 50 << 30, "{strip}");
    }

    #[test]
    fn ids_are_unique() {
        let size = Size::new(1, 1);
        let px = solid(size, [0, 0, 0, 0]);
        let a = RasterImage::from_pixels(size, PixelFormat::RGBA8_SRGB, &px).unwrap();
        let b = RasterImage::from_pixels(size, PixelFormat::RGBA8_SRGB, &px).unwrap();
        assert_ne!(a.id(), b.id());
    }
}
