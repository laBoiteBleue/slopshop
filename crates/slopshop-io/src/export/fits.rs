//! FITS writer (in-house): one primary HDU, big-endian samples. Gray is a 2-axis image, RGB a
//! cube of three planes (NAXIS3 = 3: red, green, blue, as our importer and astronomy software
//! read them). Samples: 8-bit (BITPIX 8), 16-bit (BITPIX 16 with BZERO 32768, the unsigned
//! convention: the sign bit flipped), 32-bit float (BITPIX −32). FITS has no color tagging:
//! the samples are display values, sRGB-encoded by convention, as our importer declares them.
//! No alpha (flattened over the matte).
//!
//! FITS stores the bottom row first, and planes one after the other: each band is written at
//! its place in the file (one seek and one write per plane), so rows stream with bounded
//! memory whatever the image size, without holding a plane. The header and the data are padded
//! to 2880-byte blocks (spaces and zeros).

use std::fs::File;
use std::io::{Seek, SeekFrom, Write};

use slopshop_core::Size;
use slopshop_core::color::{ChannelLayout, ColorSpace, PixelFormat, SampleType};

use super::bmp::rows_out_of_order;
use super::{ExportError, ExportNotice};

/// FITS files are made of blocks of this many bytes.
const BLOCK: u64 = 2880;
/// Header cards are 80 characters.
const CARD: usize = 80;

pub(super) struct FitsWriter {
    file: File,
    width: usize,
    height: u32,
    channels: usize,
    /// Bytes per sample.
    bytes: usize,
    /// Flip the sign bit of 16-bit samples (BZERO 32768).
    offset_binary: bool,
    header_bytes: u64,
    next_row: u32,
    /// One plane of a band, rows in the file's order.
    plane: Vec<u8>,
}

impl FitsWriter {
    pub(super) fn new(
        mut file: File,
        size: Size,
        target: PixelFormat,
    ) -> Result<Self, ExportError> {
        let channels = match target.layout {
            ChannelLayout::Gray => 1,
            ChannelLayout::Rgb => 3,
            _ => return Err(invalid_target(&target)),
        };
        let (bitpix, bytes) = match target.sample {
            SampleType::U8 => (8, 1),
            SampleType::U16 => (16, 2),
            SampleType::F32 => (-32, 4),
            SampleType::F16 => return Err(invalid_target(&target)),
        };
        if target.color_space != ColorSpace::SRGB {
            return Err(ExportError::UnsupportedSpace(target.color_space));
        }
        let mut cards = vec![
            card_value("SIMPLE", "T", "conforms to the FITS standard"),
            card_value("BITPIX", &bitpix.to_string(), ""),
            card_value("NAXIS", if channels == 1 { "2" } else { "3" }, ""),
            card_value("NAXIS1", &size.width.to_string(), "width"),
            card_value("NAXIS2", &size.height.to_string(), "height"),
        ];
        if channels == 3 {
            cards.push(card_value("NAXIS3", "3", "red, green, blue planes"));
        }
        cards.push(card_value("EXTEND", "F", ""));
        if bitpix == 16 {
            cards.push(card_value("BZERO", "32768", "unsigned 16-bit samples"));
            cards.push(card_value("BSCALE", "1", ""));
        }
        cards.push(card_comment(
            "Written by SlopShop: display values, sRGB-encoded.",
        ));
        cards.push(format!("{:CARD$}", "END"));
        let mut header = cards.concat().into_bytes();
        header.resize(padded(header.len() as u64) as usize, b' ');
        file.write_all(&header)?;
        Ok(Self {
            file,
            width: size.width as usize,
            height: size.height,
            channels,
            bytes,
            offset_binary: bitpix == 16,
            header_bytes: header.len() as u64,
            next_row: 0,
            plane: Vec::new(),
        })
    }

    pub(super) fn write_rows(&mut self, first_row: u32, rows: &[u8]) -> Result<(), ExportError> {
        let row_bytes = self.width * self.channels * self.bytes;
        if first_row != self.next_row || !rows.len().is_multiple_of(row_bytes) {
            return Err(rows_out_of_order(first_row, self.next_row, rows.len()));
        }
        let count = rows.len() / row_bytes;
        if count == 0 {
            return Ok(());
        }
        let plane_row = self.width * self.bytes;
        let plane_bytes = plane_row as u64 * u64::from(self.height);
        // The band's last row is the first of its rows in the file (bottom to top).
        let last = u64::from(self.height - 1 - (self.next_row + count as u32 - 1));
        for channel in 0..self.channels {
            self.plane.clear();
            for row in rows.chunks_exact(row_bytes).rev() {
                if self.channels == 1 {
                    self.plane.extend_from_slice(row);
                } else {
                    let samples = row.chunks_exact(self.bytes);
                    for sample in samples.skip(channel).step_by(self.channels) {
                        self.plane.extend_from_slice(sample);
                    }
                }
            }
            if self.offset_binary {
                // Big-endian: the sign bit is in the first byte.
                for sample in self.plane.as_chunks_mut::<2>().0 {
                    sample[0] ^= 0x80;
                }
            }
            let offset = self.header_bytes + channel as u64 * plane_bytes + last * plane_row as u64;
            self.file.seek(SeekFrom::Start(offset))?;
            self.file.write_all(&self.plane)?;
        }
        self.next_row += count as u32;
        Ok(())
    }

    pub(super) fn finish(self) -> Result<Vec<ExportNotice>, ExportError> {
        if self.next_row != self.height {
            return Err(ExportError::Encode(format!(
                "{} rows written out of {}",
                self.next_row, self.height
            )));
        }
        let data = (self.width * self.channels * self.bytes) as u64 * u64::from(self.height);
        // The last block padded with zeros.
        self.file.set_len(self.header_bytes + padded(data))?;
        Ok(Vec::new())
    }
}

/// `len` rounded up to whole blocks.
fn padded(len: u64) -> u64 {
    len.div_ceil(BLOCK) * BLOCK
}

/// A fixed-format card: the keyword, `= `, the value right-aligned to column 30, a comment.
fn card_value(keyword: &str, value: &str, comment: &str) -> String {
    let mut card = format!("{keyword:<8}= {value:>20}");
    if !comment.is_empty() {
        card += " / ";
        card += comment;
    }
    card.truncate(CARD);
    format!("{card:CARD$}")
}

/// A COMMENT card.
fn card_comment(text: &str) -> String {
    let mut card = format!("COMMENT {text}");
    card.truncate(CARD);
    format!("{card:CARD$}")
}

fn invalid_target(target: &PixelFormat) -> ExportError {
    ExportError::InvalidSpec(format!("FITS cannot store {target:?}"))
}
