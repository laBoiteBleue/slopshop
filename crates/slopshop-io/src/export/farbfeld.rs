//! farbfeld writer: the magic `farbfeld`, the width and height (32-bit big-endian), then 16-bit
//! big-endian RGBA rows with straight alpha. Always RGBA: without alpha, every pixel is written
//! opaque (65535). sRGB by convention (the format has no color tagging). Streamed.

use std::fs::File;
use std::io::{BufWriter, Write};

use slopshop_core::Size;
use slopshop_core::color::{AlphaMode, ChannelLayout, ColorSpace, PixelFormat, SampleType};

use super::bmp::rows_out_of_order;
use super::{ExportError, ExportNotice};

pub(super) struct FarbfeldWriter {
    out: BufWriter<File>,
    /// Bytes per pixel of the rows given: 6 (RGB) or 8 (RGBA).
    pixel_bytes: usize,
    row_bytes: usize,
    next_row: u32,
    height: u32,
    /// A row with alpha, when the rows given have none.
    row: Vec<u8>,
}

impl FarbfeldWriter {
    pub(super) fn new(file: File, size: Size, target: PixelFormat) -> Result<Self, ExportError> {
        let pixel_bytes = match target.layout {
            ChannelLayout::Rgb => 6,
            ChannelLayout::Rgba if target.alpha == AlphaMode::Straight => 8,
            _ => return Err(invalid_target(&target)),
        };
        if target.sample != SampleType::U16 {
            return Err(invalid_target(&target));
        }
        if target.color_space != ColorSpace::SRGB {
            return Err(ExportError::UnsupportedSpace(target.color_space));
        }
        let mut out = BufWriter::new(file);
        out.write_all(b"farbfeld")?;
        out.write_all(&size.width.to_be_bytes())?;
        out.write_all(&size.height.to_be_bytes())?;
        let width = size.width as usize;
        Ok(Self {
            out,
            pixel_bytes,
            row_bytes: width * pixel_bytes,
            next_row: 0,
            height: size.height,
            row: Vec::new(),
        })
    }

    pub(super) fn write_rows(&mut self, first_row: u32, rows: &[u8]) -> Result<(), ExportError> {
        if first_row != self.next_row || !rows.len().is_multiple_of(self.row_bytes) {
            return Err(rows_out_of_order(first_row, self.next_row, rows.len()));
        }
        if self.pixel_bytes == 8 {
            self.out.write_all(rows)?;
        } else {
            for src in rows.chunks_exact(self.row_bytes) {
                self.row.clear();
                for px in src.as_chunks::<6>().0 {
                    self.row.extend_from_slice(px);
                    self.row.extend([0xff, 0xff]);
                }
                self.out.write_all(&self.row)?;
            }
        }
        self.next_row += (rows.len() / self.row_bytes) as u32;
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

fn invalid_target(target: &PixelFormat) -> ExportError {
    ExportError::InvalidSpec(format!("farbfeld cannot store {target:?}"))
}
