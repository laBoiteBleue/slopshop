//! QOI writer ("Quite OK Image" format, specification 1.0): 8-bit RGB or RGBA with straight
//! alpha, encoded as the rows come (the format is a single stream of pixel operations, with no
//! row structure). The header's color space byte declares sRGB (0, "sRGB with linear alpha") or
//! linear sRGB (1, "all channels linear"); QOI has no other color tagging.
//!
//! In-house: the `qoi` crate (in the tree through `image`) encodes whole buffers only.

use std::fs::File;
use std::io::{BufWriter, Write};

use slopshop_core::Size;
use slopshop_core::color::{AlphaMode, ChannelLayout, ColorSpace, PixelFormat, SampleType};

use super::bmp::rows_out_of_order;
use super::{ExportError, ExportNotice};

/// The reference decoder refuses images of more pixels (`QOI_PIXELS_MAX`), and so does the
/// `qoi` crate our importer reads through.
pub(super) const MAX_PIXELS: u64 = 400_000_000;

const OP_INDEX: u8 = 0x00;
const OP_DIFF: u8 = 0x40;
const OP_LUMA: u8 = 0x80;
const OP_RUN: u8 = 0xc0;
const OP_RGB: u8 = 0xfe;
const OP_RGBA: u8 = 0xff;
/// Longest run of one `OP_RUN` (62: 63 and 64 would collide with `OP_RGB` and `OP_RGBA`).
const MAX_RUN: u8 = 62;
const END_MARKER: [u8; 8] = [0, 0, 0, 0, 0, 0, 0, 1];

pub(super) struct QoiWriter {
    out: BufWriter<File>,
    channels: usize,
    row_bytes: usize,
    next_row: u32,
    height: u32,
    /// The encoder state: previous pixel, the running array of seen pixels, the pending run.
    previous: [u8; 4],
    seen: [[u8; 4]; 64],
    run: u8,
}

impl QoiWriter {
    pub(super) fn new(file: File, size: Size, target: PixelFormat) -> Result<Self, ExportError> {
        let channels = match target.layout {
            ChannelLayout::Rgb => 3,
            ChannelLayout::Rgba if target.alpha == AlphaMode::Straight => 4,
            _ => return Err(invalid_target(&target)),
        };
        if target.sample != SampleType::U8 {
            return Err(invalid_target(&target));
        }
        let linear = if target.color_space == ColorSpace::SRGB {
            0
        } else if target.color_space == ColorSpace::LINEAR_SRGB {
            1
        } else {
            return Err(ExportError::UnsupportedSpace(target.color_space));
        };
        if size.pixel_count() > MAX_PIXELS {
            return Err(ExportError::TooLarge {
                width: size.width,
                height: size.height,
            });
        }
        let mut header = Vec::with_capacity(14);
        header.extend(b"qoif");
        header.extend(size.width.to_be_bytes());
        header.extend(size.height.to_be_bytes());
        header.extend([channels as u8, linear]);
        let mut out = BufWriter::new(file);
        out.write_all(&header)?;
        Ok(Self {
            out,
            channels,
            row_bytes: size.width as usize * channels,
            next_row: 0,
            height: size.height,
            previous: [0, 0, 0, 255],
            seen: [[0; 4]; 64],
            run: 0,
        })
    }

    pub(super) fn write_rows(&mut self, first_row: u32, rows: &[u8]) -> Result<(), ExportError> {
        if first_row != self.next_row || !rows.len().is_multiple_of(self.row_bytes) {
            return Err(rows_out_of_order(first_row, self.next_row, rows.len()));
        }
        // At most 5 bytes per pixel: encoded in memory, written once per band.
        let mut encoded = Vec::with_capacity(rows.len() / self.channels * 5);
        for px in rows.chunks_exact(self.channels) {
            let pixel = [
                px[0],
                px[1],
                px[2],
                if self.channels == 4 { px[3] } else { 255 },
            ];
            self.encode(pixel, &mut encoded);
        }
        self.out.write_all(&encoded)?;
        self.next_row += (rows.len() / self.row_bytes) as u32;
        Ok(())
    }

    fn encode(&mut self, pixel: [u8; 4], out: &mut Vec<u8>) {
        if pixel == self.previous {
            self.run += 1;
            if self.run == MAX_RUN {
                out.push(OP_RUN | (self.run - 1));
                self.run = 0;
            }
            return;
        }
        if self.run > 0 {
            out.push(OP_RUN | (self.run - 1));
            self.run = 0;
        }
        let [r, g, b, a] = pixel;
        let hash =
            (usize::from(r) * 3 + usize::from(g) * 5 + usize::from(b) * 7 + usize::from(a) * 11)
                % 64;
        if self.seen[hash] == pixel {
            out.push(OP_INDEX | hash as u8);
        } else {
            self.seen[hash] = pixel;
            let [pr, pg, pb, pa] = self.previous;
            if a == pa {
                // Differences wrap around, as the decoder adds them modulo 256.
                let dr = r.wrapping_sub(pr) as i8;
                let dg = g.wrapping_sub(pg) as i8;
                let db = b.wrapping_sub(pb) as i8;
                let (dr_dg, db_dg) = (dr.wrapping_sub(dg), db.wrapping_sub(dg));
                let small = |d: i8| (-2..=1).contains(&d);
                if small(dr) && small(dg) && small(db) {
                    out.push(
                        OP_DIFF | ((dr + 2) as u8) << 4 | ((dg + 2) as u8) << 2 | (db + 2) as u8,
                    );
                } else if (-32..=31).contains(&dg)
                    && (-8..=7).contains(&dr_dg)
                    && (-8..=7).contains(&db_dg)
                {
                    out.push(OP_LUMA | (dg + 32) as u8);
                    out.push(((dr_dg + 8) as u8) << 4 | (db_dg + 8) as u8);
                } else {
                    out.extend([OP_RGB, r, g, b]);
                }
            } else {
                out.extend([OP_RGBA, r, g, b, a]);
            }
        }
        self.previous = pixel;
    }

    pub(super) fn finish(mut self) -> Result<Vec<ExportNotice>, ExportError> {
        if self.next_row != self.height {
            return Err(ExportError::Encode(format!(
                "{} rows written out of {}",
                self.next_row, self.height
            )));
        }
        if self.run > 0 {
            self.out.write_all(&[OP_RUN | (self.run - 1)])?;
        }
        self.out.write_all(&END_MARKER)?;
        self.out.flush()?;
        Ok(Vec::new())
    }
}

fn invalid_target(target: &PixelFormat) -> ExportError {
    ExportError::InvalidSpec(format!("QOI cannot store {target:?}"))
}
