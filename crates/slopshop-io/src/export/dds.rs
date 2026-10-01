//! DDS writer: an uncompressed surface with the legacy header (no DX10 extension, no mip
//! levels): 32-bit BGRA with straight alpha (`DDPF_RGB | DDPF_ALPHAPIXELS`, the D3DFMT
//! A8R8G8B8 masks), or 24-bit BGR without alpha (R8G8B8). sRGB by convention (the legacy header
//! has no color tagging). Rows are written top to bottom as they come, tightly packed.
//!
//! Uncompressed rather than block-compressed (BC1–BC3): lossless, any size, read by every DDS
//! reader, ours included (the `image` crate reads only block-compressed files: see
//! `crate::dds`).

use std::fs::File;
use std::io::{BufWriter, Write};

use slopshop_core::Size;
use slopshop_core::color::{AlphaMode, ChannelLayout, ColorSpace, PixelFormat, SampleType};

use super::bmp::rows_out_of_order;
use super::{ExportError, ExportNotice};

/// "DDS " and the 124-byte DDS_HEADER.
const HEADER_BYTES: usize = 128;
/// DDSD_CAPS, DDSD_HEIGHT, DDSD_WIDTH, DDSD_PITCH, DDSD_PIXELFORMAT.
const HEADER_FLAGS: u32 = 0x1 | 0x2 | 0x4 | 0x8 | 0x1000;
const DDPF_ALPHAPIXELS: u32 = 0x1;
const DDPF_RGB: u32 = 0x40;
const DDSCAPS_TEXTURE: u32 = 0x1000;

pub(super) struct DdsWriter {
    out: BufWriter<File>,
    channels: usize,
    row_bytes: usize,
    next_row: u32,
    height: u32,
    row: Vec<u8>,
}

impl DdsWriter {
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
        // The pitch (bytes per row) is a 32-bit field.
        let pitch = u32::try_from(u64::from(size.width) * channels as u64).map_err(|_| {
            ExportError::TooLarge {
                width: size.width,
                height: size.height,
            }
        })?;
        let alpha = channels == 4;
        let mut header = Vec::with_capacity(HEADER_BYTES);
        header.extend(b"DDS ");
        // Size, flags, height, width, pitch, depth, mip levels.
        for v in [124, HEADER_FLAGS, size.height, size.width, pitch, 0, 0] {
            header.extend(u32::to_le_bytes(v));
        }
        header.extend([0; 44]);
        // DDS_PIXELFORMAT: size, flags, no FourCC, bits per pixel, R, G, B, A masks.
        let flags = DDPF_RGB | if alpha { DDPF_ALPHAPIXELS } else { 0 };
        let alpha_mask = if alpha { 0xff00_0000 } else { 0 };
        for v in [
            32,
            flags,
            0,
            channels as u32 * 8,
            0x00ff_0000,
            0x0000_ff00,
            0x0000_00ff,
            alpha_mask,
        ] {
            header.extend(u32::to_le_bytes(v));
        }
        // Caps, caps 2 to 4, reserved.
        for v in [DDSCAPS_TEXTURE, 0, 0, 0, 0] {
            header.extend(u32::to_le_bytes(v));
        }
        debug_assert_eq!(header.len(), HEADER_BYTES);
        let mut out = BufWriter::new(file);
        out.write_all(&header)?;
        Ok(Self {
            out,
            channels,
            row_bytes: pitch as usize,
            next_row: 0,
            height: size.height,
            row: Vec::with_capacity(pitch as usize),
        })
    }

    pub(super) fn write_rows(&mut self, first_row: u32, rows: &[u8]) -> Result<(), ExportError> {
        if first_row != self.next_row || !rows.len().is_multiple_of(self.row_bytes) {
            return Err(rows_out_of_order(first_row, self.next_row, rows.len()));
        }
        for src in rows.chunks_exact(self.row_bytes) {
            self.row.clear();
            for px in src.chunks_exact(self.channels) {
                // Blue, green, red (, alpha): the masks above, little-endian.
                self.row.extend([px[2], px[1], px[0]]);
                if self.channels == 4 {
                    self.row.push(px[3]);
                }
            }
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

fn invalid_target(target: &PixelFormat) -> ExportError {
    ExportError::InvalidSpec(format!("DDS cannot store {target:?}"))
}
