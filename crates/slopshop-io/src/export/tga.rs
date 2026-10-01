//! Targa writer: 8-bit RGB (24 bits per pixel) or RGBA with straight alpha (32 bits),
//! uncompressed or run-length encoded (packets never cross a row), rows top-down (origin at
//! the top left), blue first, with the TGA 2.0 footer.
//!
//! TGA has no color tagging: only sRGB, the convention, is written.

use std::fs::File;
use std::io::{BufWriter, Write};

use slopshop_core::Size;
use slopshop_core::color::{AlphaMode, ChannelLayout, ColorSpace, PixelFormat, SampleType};

use super::bmp::rows_out_of_order;
use super::{ExportError, ExportNotice, TgaCompression};

/// TGA stores dimensions as 16-bit integers.
pub(super) const MAX_SIDE: u32 = u16::MAX as u32;

/// Pixels of a run or literal packet at most.
const PACKET: usize = 128;

pub(super) struct TgaWriter {
    out: BufWriter<File>,
    channels: usize,
    width: usize,
    rle: bool,
    next_row: u32,
    height: u32,
    row: Vec<u8>,
    packed: Vec<u8>,
}

impl TgaWriter {
    pub(super) fn new(
        file: File,
        size: Size,
        target: PixelFormat,
        compression: TgaCompression,
    ) -> Result<Self, ExportError> {
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
        let rle = compression == TgaCompression::Rle;
        let mut header = vec![0u8; 18];
        // Truecolor, uncompressed (2) or run-length encoded (10).
        header[2] = if rle { 10 } else { 2 };
        header[12..14].copy_from_slice(&(size.width as u16).to_le_bytes());
        header[14..16].copy_from_slice(&(size.height as u16).to_le_bytes());
        header[16] = channels as u8 * 8;
        // Alpha bits, and the origin at the top left (bit 5).
        header[17] = if channels == 4 { 8 } else { 0 } | 0x20;
        let mut out = BufWriter::new(file);
        out.write_all(&header)?;
        Ok(Self {
            out,
            channels,
            width: size.width as usize,
            rle,
            next_row: 0,
            height: size.height,
            row: Vec::new(),
            packed: Vec::new(),
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
                self.row.extend([px[2], px[1], px[0]]);
                if self.channels == 4 {
                    self.row.push(px[3]);
                }
            }
            if self.rle {
                self.packed.clear();
                pack_row(&self.row, self.channels, &mut self.packed);
                self.out.write_all(&self.packed)?;
            } else {
                self.out.write_all(&self.row)?;
            }
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
        // TGA 2.0 footer: no extension or developer area.
        self.out.write_all(&[0; 8])?;
        self.out.write_all(b"TRUEVISION-XFILE.\0")?;
        self.out.flush()?;
        Ok(Vec::new())
    }
}

/// Run-length encode one row of `pixel`-byte pixels: runs of 2 to 128 equal pixels as
/// `0x80 | (n − 1)` and the pixel, the others as literal packets `n − 1` and the pixels.
fn pack_row(row: &[u8], pixel: usize, out: &mut Vec<u8>) {
    let pixels: Vec<&[u8]> = row.chunks_exact(pixel).collect();
    let mut i = 0;
    while i < pixels.len() {
        let mut run = 1;
        while i + run < pixels.len() && run < PACKET && pixels[i + run] == pixels[i] {
            run += 1;
        }
        if run > 1 {
            out.push(0x80 | (run - 1) as u8);
            out.extend(pixels[i]);
            i += run;
            continue;
        }
        // A literal packet, up to the next run of two.
        let start = i;
        while i < pixels.len()
            && i - start < PACKET
            && !(i + 1 < pixels.len() && pixels[i + 1] == pixels[i])
        {
            i += 1;
        }
        if i == start {
            // A run starts here.
            continue;
        }
        out.push((i - start - 1) as u8);
        for p in &pixels[start..i] {
            out.extend(*p);
        }
    }
}

fn invalid_target(target: &PixelFormat) -> ExportError {
    ExportError::InvalidSpec(format!("TGA cannot store {target:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decode what `pack_row` wrote.
    fn unpack(mut data: &[u8], pixel: usize) -> Vec<u8> {
        let mut out = Vec::new();
        while let Some((&header, rest)) = data.split_first() {
            let n = usize::from(header & 0x7f) + 1;
            if header & 0x80 != 0 {
                for _ in 0..n {
                    out.extend(&rest[..pixel]);
                }
                data = &rest[pixel..];
            } else {
                out.extend(&rest[..n * pixel]);
                data = &rest[n * pixel..];
            }
        }
        out
    }

    #[test]
    fn run_length_encoding_round_trips() {
        let mut row = Vec::new();
        for i in 0..400u32 {
            // Runs, literals, a run longer than a packet.
            let v = if i < 20 || (200..380).contains(&i) {
                7
            } else {
                (i * 37 % 251) as u8
            };
            row.extend([v, v / 2, 255 - v]);
        }
        let mut packed = Vec::new();
        pack_row(&row, 3, &mut packed);
        assert_eq!(unpack(&packed, 3), row);
        assert!(packed.len() < row.len());
        for pixel in [3, 4] {
            for len in [1, 2, 3, 129] {
                let row: Vec<u8> = (0..len * pixel).map(|i| (i % 5) as u8).collect();
                let mut packed = Vec::new();
                pack_row(&row, pixel, &mut packed);
                assert_eq!(unpack(&packed, pixel), row, "{pixel} {len}");
            }
        }
    }
}
