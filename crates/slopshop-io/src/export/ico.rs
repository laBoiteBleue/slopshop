//! ICO writer: one image, stored as a PNG inside the icon container (the format of Windows
//! Vista and later; every current reader takes it). The PNG is always 8-bit RGBA, as the format
//! requires (opaque without alpha), with the sRGB chunk. At most 256 pixels per side, so the
//! image is held in memory (256 KiB at most) and compressed in `finish`.

use std::fs::File;
use std::io::{BufWriter, Write};

use slopshop_core::Size;
use slopshop_core::color::{AlphaMode, ChannelLayout, ColorSpace, PixelFormat, SampleType};

use super::bmp::rows_out_of_order;
use super::{ExportError, ExportNotice};

/// The directory entry stores sides in a byte (0 for 256).
pub(super) const MAX_SIDE: u32 = 256;

/// ICONDIR (6 bytes) and one ICONDIRENTRY (16 bytes).
const HEADER_BYTES: u32 = 6 + 16;

pub(super) struct IcoWriter {
    out: BufWriter<File>,
    size: Size,
    channels: usize,
    /// RGBA rows received so far.
    pixels: Vec<u8>,
    next_row: u32,
}

impl IcoWriter {
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
        if size.width > MAX_SIDE || size.height > MAX_SIDE {
            return Err(ExportError::TooLarge {
                width: size.width,
                height: size.height,
            });
        }
        Ok(Self {
            out: BufWriter::new(file),
            size,
            channels,
            pixels: Vec::with_capacity(size.pixel_count() as usize * 4),
            next_row: 0,
        })
    }

    pub(super) fn write_rows(&mut self, first_row: u32, rows: &[u8]) -> Result<(), ExportError> {
        let row_bytes = self.size.width as usize * self.channels;
        if first_row != self.next_row || !rows.len().is_multiple_of(row_bytes) {
            return Err(rows_out_of_order(first_row, self.next_row, rows.len()));
        }
        if self.channels == 4 {
            self.pixels.extend_from_slice(rows);
        } else {
            for px in rows.as_chunks::<3>().0 {
                self.pixels.extend([px[0], px[1], px[2], 255]);
            }
        }
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
        let mut png = Vec::new();
        let mut info = ::png::Info::with_size(width, height);
        info.color_type = ::png::ColorType::Rgba;
        info.bit_depth = ::png::BitDepth::Eight;
        info.srgb = Some(::png::SrgbRenderingIntent::Perceptual);
        let mut encoder = ::png::Encoder::with_info(&mut png, info).map_err(encoding_error)?;
        encoder.set_compression(::png::Compression::Balanced);
        let mut writer = encoder.write_header().map_err(encoding_error)?;
        writer
            .write_image_data(&self.pixels)
            .map_err(encoding_error)?;
        writer.finish().map_err(encoding_error)?;
        // At most 256 × 256 pixels: far below 4 GiB.
        let png_bytes = u32::try_from(png.len())
            .map_err(|_| ExportError::Encode("the PNG image exceeds 4 GiB".to_owned()))?;

        let mut header = Vec::with_capacity(HEADER_BYTES as usize);
        // ICONDIR: reserved, type 1 (icon), one image.
        for v in [0u16, 1, 1] {
            header.extend(v.to_le_bytes());
        }
        // ICONDIRENTRY: sides (0 for 256), no palette, reserved, 1 plane, 32 bits per pixel.
        header.extend([(width % 256) as u8, (height % 256) as u8, 0, 0]);
        header.extend(1u16.to_le_bytes());
        header.extend(32u16.to_le_bytes());
        header.extend(png_bytes.to_le_bytes());
        header.extend(HEADER_BYTES.to_le_bytes());
        self.out.write_all(&header)?;
        self.out.write_all(&png)?;
        self.out.flush()?;
        Ok(Vec::new())
    }
}

fn encoding_error(e: ::png::EncodingError) -> ExportError {
    match e {
        ::png::EncodingError::IoError(e) => ExportError::Io(e),
        other => ExportError::Encode(other.to_string()),
    }
}

fn invalid_target(target: &PixelFormat) -> ExportError {
    ExportError::InvalidSpec(format!("ICO cannot store {target:?}"))
}
