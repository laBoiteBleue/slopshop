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
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::color::{
    AlphaMode, ChannelLayout, ColorSpace, IDENTITY, LinearRgba, PixelFormat, SampleType,
    TransferFunction, f16_to_f32, f32_to_f16,
};
use crate::geom::Size;
use crate::tile::{TileCoord, TileGrid};

/// Tile edge in pixels. 256 × 256 RGBA8 = 256 KiB, a common GPU-friendly size.
pub const TILE_SIZE: u32 = 256;

/// Magnitude that non-finite float samples (±inf) are read as: the largest half float, so that
/// sums and matrices stay finite. NaN is read as 0. The stored samples are not modified.
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

impl RasterLevel {
    pub fn size(&self) -> Size {
        self.size
    }

    pub fn grid(&self) -> TileGrid {
        self.grid
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
        if size.is_empty() {
            return Err(RasterError::EmptyImage);
        }
        if !format.layout.is_gray() && !format.color_space.primaries.is_valid() {
            return Err(RasterError::InvalidColorSpace(format.color_space));
        }
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
        })
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

/// Reads and writes pixels of one format, converting to and from linear premultiplied light.
struct Codec {
    sample: SampleType,
    channels: usize,
    color_channels: usize,
    has_alpha: bool,
    premultiplied: bool,
    transfer: TransferFunction,
    bytes_per_pixel: usize,
    /// Raw integer sample → linear, for 8/16-bit data.
    decode_lut: Vec<f32>,
}

impl Codec {
    fn new(format: PixelFormat) -> Self {
        let sample = format.sample;
        let transfer = format.color_space.transfer;
        let decode_lut = match sample {
            SampleType::U8 => (0..=255u32)
                .map(|v| transfer.decode(v as f32 / 255.0))
                .collect(),
            SampleType::U16 => (0..=65535u32)
                .map(|v| transfer.decode(v as f32 / 65535.0))
                .collect(),
            SampleType::F16 | SampleType::F32 => Vec::new(),
        };
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

    fn raw(&self, px: &[u8], channel: usize) -> RawSample {
        let b = self.sample.bytes() as usize;
        let s = &px[channel * b..][..b];
        match self.sample {
            SampleType::U8 => RawSample::Int(u32::from(s[0])),
            SampleType::U16 => RawSample::Int(u32::from(u16::from_ne_bytes([s[0], s[1]]))),
            SampleType::F16 => {
                RawSample::Float(finite(f16_to_f32(u16::from_ne_bytes([s[0], s[1]]))))
            }
            SampleType::F32 => {
                RawSample::Float(finite(f32::from_ne_bytes([s[0], s[1], s[2], s[3]])))
            }
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

    fn linear(&self, raw: RawSample) -> f32 {
        match raw {
            RawSample::Int(v) => self.decode_lut[v as usize],
            RawSample::Float(v) if self.transfer.is_linear() => v,
            // A finite sample can still decode out of range (HLG grows exponentially).
            RawSample::Float(v) => finite(self.transfer.decode(v)),
        }
    }

    /// Linear premultiplied color (gray replicated) and alpha of one pixel.
    fn read(&self, px: &[u8]) -> ([f32; 3], f32) {
        let alpha = if self.has_alpha {
            self.unit(self.raw(px, self.channels - 1)).clamp(0.0, 1.0)
        } else {
            1.0
        };
        // Premultiplied with a non-linear transfer: the file multiplied *encoded* values by
        // alpha, so decoding needs the straight encoded value first.
        let unpremultiply_first = self.premultiplied && !self.transfer.is_linear();
        let mut color = [0.0; 3];
        for (c, value) in color.iter_mut().enumerate().take(self.color_channels) {
            let raw = self.raw(px, c);
            *value = if unpremultiply_first {
                if alpha > 0.0 {
                    finite(self.transfer.decode(self.unit(raw) / alpha)) * alpha
                } else {
                    0.0
                }
            } else if self.premultiplied {
                self.linear(raw)
            } else {
                self.linear(raw) * alpha
            };
        }
        if self.color_channels == 1 {
            color = [color[0]; 3];
        }
        (color, alpha)
    }

    /// Write a linear premultiplied pixel (gray uses the first component), in the same alpha
    /// convention as the source (see [`Self::read`]).
    fn write(&self, color: [f32; 3], alpha: f32, out: &mut [u8]) {
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

/// Non-finite float samples as read for averaging and display (see [`MAX_FINITE_SAMPLE`]).
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
            .chunks_exact(4)
            .map(|b| f32::from_ne_bytes(b.try_into().unwrap()))
            .collect();
        assert_eq!(floats, [2.5, -0.25, 1e-6, 1.0]);
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
            .chunks_exact(4)
            .map(|b| f32::from_ne_bytes(b.try_into().unwrap()))
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
            .chunks_exact(4)
            .map(|b| f32::from_ne_bytes(b.try_into().unwrap()))
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
                .chunks_exact(4)
                .map(|b| f32::from_ne_bytes(b.try_into().unwrap()))
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
