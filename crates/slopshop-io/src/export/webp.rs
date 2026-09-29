//! WebP writers (ADR 0010): lossless through `image-webp` (pure Rust), lossy through libwebp.
//!
//! - 8-bit RGB or RGBA with straight alpha; always tagged with the ICC profile of the space (an
//!   `ICCP` chunk, sRGB included). No EXIF or XMP.
//! - At most [`MAX_SIDE`] (16383) pixels per side, the format's limit.
//! - **A WebP file cannot be written incrementally**: both writers keep the whole frame, an
//!   explicit exception to the bounded memory of ADR 0008, capped by the size limit. The frame
//!   is reserved up front (`tooLarge` when the memory is not available) and filled band by band;
//!   the encoding happens in `finish`.
//!   - Lossless keeps the RGB(A) samples (3 or 4 bytes per pixel, 1 GiB at most).
//!   - Lossy keeps YUV 4:2:0 planes, plus alpha (1.5 or 2.5 bytes per pixel), converted from
//!     each band as it arrives with libwebp's BT.601 coefficients. Chroma is the average of each
//!     2 × 2 block, weighted by alpha so that transparent pixels do not darken visible edges.
//!     libwebp itself averages in a gamma-compressed space: our planes are close to, not
//!     identical with, what `cwebp` would produce from the same pixels.
//! - Lossy encoding can be cancelled (libwebp's progress hook reads the export's cancel token);
//!   lossless encoding cannot, once started (a few seconds at most at the size limit).
//! - Lossy WebP stores its modes in a first partition limited to 512 KiB, which very detailed
//!   content overflows before the size limit. Large frames are encoded with a degraded first
//!   partition from the start (`partition_limit`, and libwebp's low-memory mode), then once more
//!   with a single segment, and the export fails with `contentTooComplex` if it still does not
//!   fit (lossless WebP and JPEG have no such limit).

mod ffi;
mod riff;

use std::fs::File;
use std::io::{BufWriter, Write};

use slopshop_core::color::{AlphaMode, ChannelLayout, PixelFormat, SampleType};
use slopshop_core::{CancelToken, Size};

use self::ffi::{EncodeError, LossySettings, Planes};
use super::{ExportError, ExportNotice};
use crate::icc;

/// The largest side of a WebP image (`WEBP_MAX_DIMENSION`).
pub const MAX_SIDE: u32 = libwebp_sys::WEBP_MAX_DIMENSION;

/// WebP sizes, checked before anything is written.
pub(super) fn check_size(size: Size) -> Result<(), ExportError> {
    if size.width > MAX_SIDE || size.height > MAX_SIDE {
        return Err(too_large(size));
    }
    Ok(())
}

fn too_large(size: Size) -> ExportError {
    ExportError::TooLarge {
        width: size.width,
        height: size.height,
    }
}

/// What both writers check in `new`: the target, the size, and the profile to embed.
fn validate(size: Size, target: &PixelFormat) -> Result<Vec<u8>, ExportError> {
    let supported = matches!(target.layout, ChannelLayout::Rgb | ChannelLayout::Rgba)
        && target.sample == SampleType::U8
        && (target.layout == ChannelLayout::Rgb || target.alpha == AlphaMode::Straight);
    if !supported {
        return Err(ExportError::InvalidSpec(format!(
            "WebP export cannot store {target:?}"
        )));
    }
    if size.is_empty() {
        return Err(ExportError::InvalidSpec(format!(
            "empty image ({}×{})",
            size.width, size.height
        )));
    }
    check_size(size)?;
    icc::write_matrix_trc(&target.color_space)
        .map_err(|_| ExportError::UnsupportedSpace(target.color_space))
}

/// Check that `rows` are the next rows expected; their count.
fn next_rows(
    size: Size,
    next_row: u32,
    first_row: u32,
    rows: &[u8],
    row_bytes: usize,
) -> Result<u32, ExportError> {
    let count = rows.len() / row_bytes;
    if first_row != next_row
        || !rows.len().is_multiple_of(row_bytes)
        || count > (size.height - next_row) as usize
    {
        return Err(ExportError::Encode(format!(
            "rows out of order: {} bytes at row {first_row}, expected row {next_row} of {}",
            rows.len(),
            size.height
        )));
    }
    Ok(count as u32)
}

fn all_rows(size: Size, next_row: u32) -> Result<(), ExportError> {
    if next_row != size.height {
        return Err(ExportError::Encode(format!(
            "{next_row} rows written out of {}",
            size.height
        )));
    }
    Ok(())
}

/// Lossless WebP (VP8L) through `image-webp`.
pub(super) struct WebpLosslessWriter {
    file: File,
    size: Size,
    alpha: bool,
    profile: Vec<u8>,
    frame: Vec<u8>,
    next_row: u32,
    cancel: CancelToken,
}

impl WebpLosslessWriter {
    pub(super) fn new(
        file: File,
        size: Size,
        target: PixelFormat,
        cancel: CancelToken,
    ) -> Result<Self, ExportError> {
        let profile = validate(size, &target)?;
        let alpha = target.layout.has_alpha();
        let len = (size.width as usize)
            .checked_mul(size.height as usize)
            .and_then(|pixels| pixels.checked_mul(if alpha { 4 } else { 3 }))
            .ok_or_else(|| too_large(size))?;
        let mut frame = Vec::new();
        frame.try_reserve_exact(len).map_err(|_| too_large(size))?;
        Ok(Self {
            file,
            size,
            alpha,
            profile,
            frame,
            next_row: 0,
            cancel,
        })
    }

    pub(super) fn write_rows(&mut self, first_row: u32, rows: &[u8]) -> Result<(), ExportError> {
        let row_bytes = self.size.width as usize * if self.alpha { 4 } else { 3 };
        let count = next_rows(self.size, self.next_row, first_row, rows, row_bytes)?;
        self.frame.extend_from_slice(rows);
        self.next_row += count;
        Ok(())
    }

    pub(super) fn finish(self) -> Result<Vec<ExportNotice>, ExportError> {
        all_rows(self.size, self.next_row)?;
        if self.cancel.is_cancelled() {
            return Err(ExportError::Cancelled);
        }
        let mut out = BufWriter::new(self.file);
        let mut encoder = image_webp::WebPEncoder::new(&mut out);
        encoder.set_icc_profile(self.profile);
        let color = if self.alpha {
            image_webp::ColorType::Rgba8
        } else {
            image_webp::ColorType::Rgb8
        };
        // The frame length matches the size (every row was checked), as `encode` requires.
        encoder
            .encode(&self.frame, self.size.width, self.size.height, color)
            .map_err(|e| match e {
                image_webp::EncodingError::IoError(e) => ExportError::Io(e),
                other => ExportError::Encode(other.to_string()),
            })?;
        out.flush()?;
        Ok(Vec::new())
    }
}

/// Lossy WebP (VP8, plus ALPH) through libwebp.
pub(super) struct WebpLossyWriter {
    file: File,
    size: Size,
    quality: u8,
    profile: Vec<u8>,
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
    /// With alpha only.
    a: Option<Vec<u8>>,
    next_row: u32,
    cancel: CancelToken,
}

impl WebpLossyWriter {
    pub(super) fn new(
        file: File,
        size: Size,
        target: PixelFormat,
        quality: u8,
        cancel: CancelToken,
    ) -> Result<Self, ExportError> {
        let profile = validate(size, &target)?;
        if quality > 100 {
            return Err(ExportError::InvalidSpec(format!(
                "WebP quality {quality} is above 100"
            )));
        }
        // Fits: at most 16383 × 16383.
        let luma = size.width as usize * size.height as usize;
        let chroma = size.width.div_ceil(2) as usize * size.height.div_ceil(2) as usize;
        let plane = |len: usize| -> Result<Vec<u8>, ExportError> {
            let mut plane = Vec::new();
            plane.try_reserve_exact(len).map_err(|_| too_large(size))?;
            Ok(plane)
        };
        Ok(Self {
            file,
            size,
            quality,
            profile,
            y: plane(luma)?,
            u: plane(chroma)?,
            v: plane(chroma)?,
            a: if target.layout.has_alpha() {
                Some(plane(luma)?)
            } else {
                None
            },
            next_row: 0,
            cancel,
        })
    }

    pub(super) fn write_rows(&mut self, first_row: u32, rows: &[u8]) -> Result<(), ExportError> {
        let channels = if self.a.is_some() { 4 } else { 3 };
        let row_bytes = self.size.width as usize * channels;
        let count = next_rows(self.size, self.next_row, first_row, rows, row_bytes)?;
        // Chroma covers row pairs: a band must start on an even row, and only the last one may
        // have an odd number of rows (bands are 256 rows).
        if first_row % 2 == 1 || (count % 2 == 1 && first_row + count != self.size.height) {
            return Err(ExportError::Encode(format!(
                "WebP rows must come in pairs: {count} rows at row {first_row}"
            )));
        }
        let rows: Vec<&[u8]> = rows.chunks_exact(row_bytes).collect();
        for pair in rows.chunks(2) {
            // An odd last row pairs with itself.
            let (top, bottom) = (pair[0], pair[pair.len() - 1]);
            self.convert_pair(top, bottom, pair.len() == 2, channels);
        }
        self.next_row += count;
        Ok(())
    }

    /// Convert two rows (the same one twice for an odd last row) into luma, alpha and one row
    /// of chroma.
    fn convert_pair(&mut self, top: &[u8], bottom: &[u8], two: bool, channels: usize) {
        let rows: &[&[u8]] = if two { &[top, bottom] } else { &[top] };
        for row in rows {
            for px in row.chunks_exact(channels) {
                self.y.push(rgb_to_y(px[0], px[1], px[2]));
                if let Some(a) = &mut self.a {
                    a.push(px[3]);
                }
            }
        }
        let width = self.size.width as usize;
        for x in (0..width).step_by(2) {
            let columns = if x + 1 < width { 2 } else { 1 };
            let mut block = [[0u32; 4]; 4];
            let mut n = 0;
            for row in [top, bottom] {
                for dx in 0..columns {
                    let px = &row[(x + dx) * channels..][..channels];
                    let alpha = if channels == 4 { px[3] } else { 255 };
                    block[n] = [
                        u32::from(px[0]),
                        u32::from(px[1]),
                        u32::from(px[2]),
                        u32::from(alpha),
                    ];
                    n += 1;
                }
            }
            let [r, g, b] = sum_of_four(&block[..n]);
            self.u.push(rgb_to_u(r, g, b));
            self.v.push(rgb_to_v(r, g, b));
        }
    }

    pub(super) fn finish(mut self) -> Result<Vec<ExportNotice>, ExportError> {
        all_rows(self.size, self.next_row)?;
        let megapixels = u64::from(self.size.width) * u64::from(self.size.height) / 1_000_000;
        let mut settings = LossySettings {
            quality: f32::from(self.quality),
            method: 4,
            segments: None,
            // Provisional thresholds (ADR 0010): the default wastes minutes before failing on
            // large detailed frames, while a degraded first partition costs almost nothing.
            partition_limit: match megapixels {
                0..24 => 0,
                24..100 => 75,
                _ => 100,
            },
            low_memory: megapixels >= 32,
        };
        let bitstream = loop {
            let planes = Planes {
                width: self.size.width,
                height: self.size.height,
                y: &mut self.y,
                u: &mut self.u,
                v: &mut self.v,
                a: self.a.as_deref_mut(),
            };
            match ffi::encode_lossy(planes, settings, &self.cancel) {
                Ok(bitstream) => break bitstream,
                Err(EncodeError::Partition0Overflow) if settings.segments.is_none() => {
                    settings.segments = Some(1);
                    settings.partition_limit = 100;
                }
                Err(EncodeError::Partition0Overflow) => return Err(ExportError::ContentTooComplex),
                Err(EncodeError::Cancelled) => return Err(ExportError::Cancelled),
                Err(EncodeError::OutOfMemory) => return Err(too_large(self.size)),
                Err(EncodeError::Other(e)) => return Err(ExportError::Encode(e)),
            }
        };
        // The planes are no longer needed: free them before building the file.
        drop((self.y, self.u, self.v, self.a));
        let file =
            riff::with_icc_profile(&bitstream, self.size.width, self.size.height, &self.profile)?;
        drop(bitstream);
        let mut out = BufWriter::new(self.file);
        out.write_all(&file)?;
        out.flush()?;
        Ok(Vec::new())
    }
}

/// libwebp's `VP8RGBToY` (BT.601, limited range, 16-bit fixed point, rounded).
fn rgb_to_y(r: u8, g: u8, b: u8) -> u8 {
    let luma = 16839 * i32::from(r) + 33059 * i32::from(g) + 6420 * i32::from(b);
    // At most 235: no clipping needed.
    ((luma + (1 << 15) + (16 << 16)) >> 16) as u8
}

/// libwebp's `VP8RGBToU`, on the sum of four samples of each channel.
fn rgb_to_u(r: i32, g: i32, b: i32) -> u8 {
    clip_uv(-9719 * r - 19081 * g + 28800 * b)
}

/// libwebp's `VP8RGBToV`, on the sum of four samples of each channel.
fn rgb_to_v(r: i32, g: i32, b: i32) -> u8 {
    clip_uv(28800 * r - 24116 * g - 4684 * b)
}

/// libwebp's `VP8ClipUV` with its rounding for four-sample sums.
fn clip_uv(uv: i32) -> u8 {
    let uv = (uv + (1 << 17) + (128 << 18)) >> 18;
    uv.clamp(0, 255) as u8
}

/// The sums of four samples of R, G and B of a block of 1 to 4 pixels (`[r, g, b, alpha]`):
/// weighted by alpha, so that transparent pixels do not pull the color of visible ones;
/// scaled to four samples for partial blocks (right or bottom edge).
fn sum_of_four(block: &[[u32; 4]]) -> [i32; 3] {
    let total_alpha: u32 = block.iter().map(|px| px[3]).sum();
    let mut sums = [0i32; 3];
    let n = block.len() as u32;
    for (channel, sum) in sums.iter_mut().enumerate() {
        let weighted: u32 = block.iter().map(|px| px[channel] * px[3]).sum();
        let value = (weighted * 4 + total_alpha / 2)
            .checked_div(total_alpha)
            .unwrap_or_else(|| {
                // Fully transparent: the color is invisible, keep a plain average.
                let plain: u32 = block.iter().map(|px| px[channel]).sum();
                (plain * 4 + n / 2) / n.max(1)
            });
        *sum = value as i32;
    }
    sums
}

#[cfg(test)]
mod tests;
