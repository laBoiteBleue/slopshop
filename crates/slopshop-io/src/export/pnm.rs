//! Netpbm writers. PNM: 8/16-bit (big-endian) gray (PGM, P5), RGB (PPM, P6), or with straight
//! alpha (PAM, P7), sRGB by convention. PFM: 32-bit float gray (`Pf`) or RGB (`PF`),
//! little-endian, linear sRGB primaries by convention; its rows are stored bottom to top, so
//! each row is written at its place in the file.

use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};

use slopshop_core::Size;
use slopshop_core::color::{AlphaMode, ChannelLayout, ColorSpace, PixelFormat, SampleType};

use super::bmp::rows_out_of_order;
use super::{ExportError, ExportNotice};

pub(super) struct PnmWriter {
    out: BufWriter<File>,
    row_bytes: usize,
    next_row: u32,
    height: u32,
}

impl PnmWriter {
    pub(super) fn new(file: File, size: Size, target: PixelFormat) -> Result<Self, ExportError> {
        let max = match target.sample {
            SampleType::U8 => 255,
            SampleType::U16 => 65_535,
            SampleType::F16 | SampleType::F32 => return Err(invalid_target("PNM", &target)),
        };
        let alpha = matches!(
            target.layout,
            ChannelLayout::GrayAlpha | ChannelLayout::Rgba
        );
        if alpha && target.alpha != AlphaMode::Straight {
            return Err(invalid_target("PNM", &target));
        }
        if target.color_space != ColorSpace::SRGB {
            return Err(ExportError::UnsupportedSpace(target.color_space));
        }
        let (w, h) = (size.width, size.height);
        let header = match target.layout {
            ChannelLayout::Gray => format!("P5\n{w} {h}\n{max}\n"),
            ChannelLayout::Rgb => format!("P6\n{w} {h}\n{max}\n"),
            ChannelLayout::GrayAlpha | ChannelLayout::Rgba => {
                let (depth, tuple) = if target.layout == ChannelLayout::Rgba {
                    (4, "RGB_ALPHA")
                } else {
                    (2, "GRAYSCALE_ALPHA")
                };
                format!(
                    "P7\nWIDTH {w}\nHEIGHT {h}\nDEPTH {depth}\nMAXVAL {max}\nTUPLTYPE {tuple}\nENDHDR\n"
                )
            }
        };
        let mut out = BufWriter::new(file);
        out.write_all(header.as_bytes())?;
        Ok(Self {
            out,
            row_bytes: w as usize * target.bytes_per_pixel() as usize,
            next_row: 0,
            height: h,
        })
    }

    pub(super) fn write_rows(&mut self, first_row: u32, rows: &[u8]) -> Result<(), ExportError> {
        if first_row != self.next_row || !rows.len().is_multiple_of(self.row_bytes) {
            return Err(rows_out_of_order(first_row, self.next_row, rows.len()));
        }
        self.out.write_all(rows)?;
        self.next_row += (rows.len() / self.row_bytes) as u32;
        Ok(())
    }

    pub(super) fn finish(mut self) -> Result<Vec<ExportNotice>, ExportError> {
        finish_rows(self.next_row, self.height)?;
        self.out.flush()?;
        Ok(Vec::new())
    }
}

pub(super) struct PfmWriter {
    file: File,
    header_bytes: u64,
    row_bytes: usize,
    next_row: u32,
    height: u32,
}

impl PfmWriter {
    pub(super) fn new(
        mut file: File,
        size: Size,
        target: PixelFormat,
    ) -> Result<Self, ExportError> {
        let magic = match target.layout {
            ChannelLayout::Gray => "Pf",
            ChannelLayout::Rgb => "PF",
            _ => return Err(invalid_target("PFM", &target)),
        };
        if target.sample != SampleType::F32 {
            return Err(invalid_target("PFM", &target));
        }
        if target.color_space != ColorSpace::LINEAR_SRGB {
            return Err(ExportError::UnsupportedSpace(target.color_space));
        }
        // A negative scale: little-endian samples.
        let header = format!("{magic}\n{} {}\n-1.0\n", size.width, size.height);
        file.write_all(header.as_bytes())?;
        let row_bytes = size.width as usize * target.bytes_per_pixel() as usize;
        Ok(Self {
            file,
            header_bytes: header.len() as u64,
            row_bytes,
            next_row: 0,
            height: size.height,
        })
    }

    pub(super) fn write_rows(&mut self, first_row: u32, rows: &[u8]) -> Result<(), ExportError> {
        if first_row != self.next_row || !rows.len().is_multiple_of(self.row_bytes) {
            return Err(rows_out_of_order(first_row, self.next_row, rows.len()));
        }
        for row in rows.chunks_exact(self.row_bytes) {
            // Bottom to top: the first image row is the last in the file.
            let index = u64::from(self.height - 1 - self.next_row);
            self.file.seek(SeekFrom::Start(
                self.header_bytes + index * self.row_bytes as u64,
            ))?;
            self.file.write_all(row)?;
            self.next_row += 1;
        }
        Ok(())
    }

    pub(super) fn finish(self) -> Result<Vec<ExportNotice>, ExportError> {
        finish_rows(self.next_row, self.height)?;
        Ok(Vec::new())
    }
}

fn finish_rows(written: u32, height: u32) -> Result<(), ExportError> {
    if written != height {
        return Err(ExportError::Encode(format!(
            "{written} rows written out of {height}"
        )));
    }
    Ok(())
}

fn invalid_target(format: &str, target: &PixelFormat) -> ExportError {
    ExportError::InvalidSpec(format!("{format} cannot store {target:?}"))
}
