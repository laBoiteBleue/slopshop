//! GIF writer: one frame of at most 256 colors (a global palette), sRGB by convention, with
//! on/off transparency. The palette needs every pixel, so the image is held in memory (3 or 4
//! bytes per pixel, plus one per index), bounded by [`MAX_PIXELS`]; it is built in `finish`:
//! - with alpha, pixels under half opacity (alpha < 128) become the transparent index, the
//!   others opaque;
//! - when the opaque pixels have few enough colors (256, or 255 beside the transparent index),
//!   the palette is exactly theirs;
//! - otherwise NeuQuant (`color_quant`) learns a palette from an even sample of the opaque
//!   pixels ([`TRAINING_PIXELS`] at most), and each pixel takes its nearest color. There is no
//!   error diffusion yet.
//!
//! Every pixel not written exactly (a color replaced by its palette entry, an alpha other than
//! 0 or 255) is counted and reported as [`ExportNotice::ColorsQuantized`]. Frames are written by
//! the `gif` crate (LZW).

use std::borrow::Cow;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::thread;

use slopshop_core::Size;
use slopshop_core::color::{AlphaMode, ChannelLayout, ColorSpace, PixelFormat, SampleType};

use super::bmp::rows_out_of_order;
use super::{ExportError, ExportNotice};

/// GIF stores sides as 16-bit integers.
pub(super) const MAX_SIDE: u32 = u16::MAX as u32;

/// The image is held whole in memory: at most this many pixels (16384²: up to 1 GiB with
/// alpha).
pub(super) const MAX_PIXELS: u64 = 1 << 28;

/// The palette is learned from at most this many pixels: a few tenths of a second whatever
/// the image size.
const TRAINING_PIXELS: usize = 1 << 20;

pub(super) struct GifWriter {
    out: BufWriter<File>,
    size: Size,
    channels: usize,
    pixels: Vec<u8>,
    next_row: u32,
}

impl GifWriter {
    pub(super) fn new(file: File, size: Size, target: PixelFormat) -> Result<Self, ExportError> {
        let channels = match target.layout {
            ChannelLayout::Rgb => 3,
            ChannelLayout::Rgba if target.alpha == AlphaMode::Straight => 4,
            _ => return Err(invalid_target(&target)),
        };
        if target.sample != SampleType::U8 {
            return Err(invalid_target(&target));
        }
        if target.color_space != ColorSpace::SRGB {
            return Err(ExportError::UnsupportedSpace(target.color_space));
        }
        let too_large = || ExportError::TooLarge {
            width: size.width,
            height: size.height,
        };
        if size.width > MAX_SIDE || size.height > MAX_SIDE || size.pixel_count() > MAX_PIXELS {
            return Err(too_large());
        }
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(size.pixel_count() as usize * channels)
            .map_err(|_| too_large())?;
        Ok(Self {
            out: BufWriter::new(file),
            size,
            channels,
            pixels,
            next_row: 0,
        })
    }

    pub(super) fn write_rows(&mut self, first_row: u32, rows: &[u8]) -> Result<(), ExportError> {
        let row_bytes = self.size.width as usize * self.channels;
        if first_row != self.next_row || !rows.len().is_multiple_of(row_bytes) {
            return Err(rows_out_of_order(first_row, self.next_row, rows.len()));
        }
        self.pixels.extend_from_slice(rows);
        self.next_row += (rows.len() / row_bytes) as u32;
        Ok(())
    }

    pub(super) fn finish(mut self) -> Result<Vec<ExportNotice>, ExportError> {
        let Size { width, height } = self.size;
        if self.next_row != height {
            return Err(ExportError::Encode(format!(
                "{} rows written out of {height}",
                self.next_row
            )));
        }
        let channels = self.channels;
        let pixels = &self.pixels;
        let transparent = channels == 4 && pixels.as_chunks::<4>().0.iter().any(|p| p[3] < 128);
        let colors = if transparent { 255 } else { 256 };
        let opaque = |px: &[u8]| channels == 3 || px[3] >= 128;
        let rgb = |px: &[u8]| [px[0], px[1], px[2]];

        let mut indices = Vec::new();
        indices
            .try_reserve_exact(pixels.len() / channels)
            .map_err(|_| ExportError::TooLarge { width, height })?;
        indices.resize(pixels.len() / channels, 0);
        let (palette, changed) = match exact_palette(pixels, channels, colors) {
            Some(exact) => {
                let mut palette = vec![[0u8; 3]; exact.len()];
                for (color, &index) in &exact {
                    palette[usize::from(index)] = *color;
                }
                let index_of = |px: &[u8]| exact.get(&rgb(px)).copied().unwrap_or(0);
                let changed = map(pixels, channels, &palette, colors, &index_of, &mut indices)?;
                (palette, changed)
            }
            None => {
                // An even sample of the opaque pixels: a bounded copy, so that learning takes
                // the same time whatever the size, and skips transparent pixels.
                let count = pixels
                    .chunks_exact(channels)
                    .filter(|px| opaque(px))
                    .count();
                let stride = count.div_ceil(TRAINING_PIXELS).max(1);
                let sample: Vec<u8> = pixels
                    .chunks_exact(channels)
                    .filter(|px| opaque(px))
                    .step_by(stride)
                    .flat_map(|px| [px[0], px[1], px[2], 255])
                    .collect();
                // Sample factor 1: every sampled pixel is learned from.
                let quantizer = color_quant::NeuQuant::new(1, colors, &sample);
                let palette: Vec<[u8; 3]> = quantizer.color_map_rgb().as_chunks::<3>().0.to_vec();
                let index_of = |px: &[u8]| quantizer.index_of(&[px[0], px[1], px[2], 255]) as u8;
                let changed = map(pixels, channels, &palette, colors, &index_of, &mut indices)?;
                (palette, changed)
            }
        };

        // The transparent entry follows the colors.
        let mut table: Vec<u8> = palette.iter().flatten().copied().collect();
        let transparent_index = if transparent {
            table.extend([0, 0, 0]);
            Some(palette.len() as u8)
        } else {
            None
        };
        // Fits: both sides are at most MAX_SIDE.
        let (w, h) = (width as u16, height as u16);
        let mut encoder = ::gif::Encoder::new(&mut self.out, w, h, &table).map_err(gif_error)?;
        let frame = ::gif::Frame {
            width: w,
            height: h,
            transparent: transparent_index,
            buffer: Cow::Borrowed(&indices),
            ..::gif::Frame::default()
        };
        encoder.write_frame(&frame).map_err(gif_error)?;
        // Writes the trailer, with its errors (dropping the encoder would ignore them).
        encoder.into_inner().map_err(gif_error)?;
        self.out.flush()?;
        Ok(if changed > 0 {
            vec![ExportNotice::ColorsQuantized(changed)]
        } else {
            Vec::new()
        })
    }
}

/// The colors of the opaque pixels, each with its index, if there are at most `colors`.
fn exact_palette(pixels: &[u8], channels: usize, colors: usize) -> Option<HashMap<[u8; 3], u8>> {
    let mut palette = HashMap::new();
    for px in pixels.chunks_exact(channels) {
        if channels == 4 && px[3] < 128 {
            continue;
        }
        let next = palette.len();
        if let std::collections::hash_map::Entry::Vacant(entry) =
            palette.entry([px[0], px[1], px[2]])
        {
            if next == colors {
                return None;
            }
            // Fits: `next` < `colors` ≤ 256.
            entry.insert(next as u8);
        }
    }
    Some(palette)
}

/// Write the palette index of every pixel into `indices` (the transparent ones get `colors`,
/// the index after the palette), on all cores, and count the pixels not written exactly.
fn map(
    pixels: &[u8],
    channels: usize,
    palette: &[[u8; 3]],
    colors: usize,
    index_of: &(dyn Fn(&[u8]) -> u8 + Sync),
    indices: &mut [u8],
) -> Result<u64, ExportError> {
    let count = indices.len();
    let threads = thread::available_parallelism().map_or(1, |n| n.get());
    let per_thread = count.div_ceil(threads).max(1);
    // Fits: `colors` is 255 when there is a transparent index.
    let transparent = colors.min(255) as u8;
    thread::scope(|scope| {
        let workers: Vec<_> = pixels
            .chunks(per_thread * channels)
            .zip(indices.chunks_mut(per_thread))
            .map(|(pixels, indices)| {
                scope.spawn(move || {
                    let mut changed = 0u64;
                    for (px, index) in pixels.chunks_exact(channels).zip(indices) {
                        let alpha = if channels == 4 { px[3] } else { 255 };
                        if alpha < 128 {
                            *index = transparent;
                            changed += u64::from(alpha != 0);
                        } else {
                            *index = index_of(px);
                            let color = palette.get(usize::from(*index));
                            let exact = color == Some(&[px[0], px[1], px[2]]);
                            changed += u64::from(alpha != 255 || !exact);
                        }
                    }
                    changed
                })
            })
            .collect();
        let mut changed = 0;
        for worker in workers {
            changed += worker
                .join()
                .map_err(|_| ExportError::Encode("a palette thread panicked".to_owned()))?;
        }
        Ok(changed)
    })
}

fn gif_error(e: ::gif::EncodingError) -> ExportError {
    match e {
        ::gif::EncodingError::Io(e) => ExportError::Io(e),
        other => ExportError::Encode(other.to_string()),
    }
}

fn invalid_target(target: &PixelFormat) -> ExportError {
    ExportError::InvalidSpec(format!("GIF cannot store {target:?}"))
}
