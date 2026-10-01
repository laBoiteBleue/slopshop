//! BMP writer: 8-bit RGB (24 bits per pixel) or RGBA with straight alpha (32 bits, BITFIELDS
//! masks), with a BITMAPV5HEADER declaring sRGB. Rows are written top-down (negative height)
//! as they come, blue first, each padded to 4 bytes.
//!
//! BMP has no other color tagging that readers honor: only sRGB is written.

use std::fs::File;
use std::io::{BufWriter, Write};

use slopshop_core::Size;
use slopshop_core::color::{AlphaMode, ChannelLayout, ColorSpace, PixelFormat, SampleType};

use super::{ExportError, ExportNotice};

/// BMP stores dimensions as 32-bit signed integers.
pub(super) const MAX_SIDE: u32 = i32::MAX as u32;

/// File header and BITMAPV5HEADER.
const HEADER_BYTES: u64 = 14 + 124;

pub(super) struct BmpWriter {
    out: BufWriter<File>,
    channels: usize,
    width: usize,
    padding: usize,
    next_row: u32,
    height: u32,
    row: Vec<u8>,
}

impl BmpWriter {
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
        if size.width > MAX_SIDE || size.height > MAX_SIDE {
            return Err(too_large());
        }
        let stride = (u64::from(size.width) * channels as u64).next_multiple_of(4);
        // Sizes are 32-bit in the file.
        let image_bytes = stride * u64::from(size.height);
        let file_bytes = u32::try_from(HEADER_BYTES + image_bytes).map_err(|_| too_large())?;

        let mut header = Vec::with_capacity(HEADER_BYTES as usize);
        header.extend(b"BM");
        header.extend(file_bytes.to_le_bytes());
        header.extend([0; 4]);
        header.extend((HEADER_BYTES as u32).to_le_bytes());
        // BITMAPV5HEADER.
        header.extend(124u32.to_le_bytes());
        header.extend((size.width as i32).to_le_bytes());
        // Negative: rows top-down.
        header.extend((-(size.height as i32)).to_le_bytes());
        header.extend(1u16.to_le_bytes());
        header.extend((channels as u16 * 8).to_le_bytes());
        // BI_RGB (0) or BI_BITFIELDS (3).
        let bitfields = channels == 4;
        header.extend(if bitfields { 3u32 } else { 0 }.to_le_bytes());
        header.extend((image_bytes as u32).to_le_bytes());
        // 72 dpi, no palette.
        header.extend(2835u32.to_le_bytes());
        header.extend(2835u32.to_le_bytes());
        header.extend([0; 8]);
        let masks: [u32; 4] = if bitfields {
            [0x00ff_0000, 0x0000_ff00, 0x0000_00ff, 0xff00_0000]
        } else {
            [0; 4]
        };
        for mask in masks {
            header.extend(mask.to_le_bytes());
        }
        // LCS_sRGB, no endpoints or gamma (36 + 12 bytes), LCS_GM_IMAGES, no profile.
        header.extend(b"BGRs");
        header.extend([0; 48]);
        header.extend(4u32.to_le_bytes());
        header.extend([0; 12]);
        debug_assert_eq!(header.len() as u64, HEADER_BYTES);

        let mut out = BufWriter::new(file);
        out.write_all(&header)?;
        let width = size.width as usize;
        Ok(Self {
            out,
            channels,
            width,
            padding: stride as usize - width * channels,
            next_row: 0,
            height: size.height,
            row: Vec::with_capacity(stride as usize),
        })
    }

    pub(super) fn write_rows(&mut self, first_row: u32, rows: &[u8]) -> Result<(), ExportError> {
        let row_bytes = self.width * self.channels;
        if first_row != self.next_row || !rows.len().is_multiple_of(row_bytes) {
            return Err(rows_out_of_order(first_row, self.next_row, rows.len()));
        }
        for src in rows.chunks_exact(row_bytes) {
            self.row.clear();
            for px in src.chunks_exact(self.channels) {
                // Blue, green, red (, alpha).
                self.row.extend([px[2], px[1], px[0]]);
                if self.channels == 4 {
                    self.row.push(px[3]);
                }
            }
            self.row.resize(row_bytes + self.padding, 0);
            self.out.write_all(&self.row)?;
            self.next_row += 1;
        }
        Ok(())
    }

    pub(super) fn finish(mut self) -> Result<Vec<ExportNotice>, ExportError> {
        if self.next_row != self.height {
            return Err(ExportError::Encode(format!(
                "{} rows written out of {}",
                self.next_row, self.height
            )));
        }
        self.out.flush()?;
        Ok(Vec::new())
    }
}

pub(super) fn rows_out_of_order(first_row: u32, expected: u32, len: usize) -> ExportError {
    ExportError::Encode(format!(
        "rows out of order: {len} bytes at row {first_row}, expected row {expected}"
    ))
}

fn invalid_target(target: &PixelFormat) -> ExportError {
    ExportError::InvalidSpec(format!("BMP cannot store {target:?}"))
}
