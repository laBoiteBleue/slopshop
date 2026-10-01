//! Radiance HDR writer: RGBE pixels (three 8-bit mantissas sharing an 8-bit exponent), linear
//! sRGB primaries by convention (the format's color tagging, `PRIMARIES`, is rarely honored),
//! no alpha. The header is `#?RADIANCE`, `FORMAT=32-bit_rle_rgbe`, a blank line and
//! `-Y height +X width` (rows top to bottom); scanlines are written flat (uncompressed), as the
//! rows come, which every reader accepts.
//!
//! RGBE cannot store negative values, nor values above about 1.7 × 10³⁸: they are clipped and
//! counted ([`ExportNotice::ClippedLow`], [`ExportNotice::ClippedHigh`], in samples). Each
//! mantissa is rounded to the nearest step of its pixel's exponent; the brightest channel keeps
//! at least 7 significant bits.

use std::fs::File;
use std::io::{BufWriter, Write};

use slopshop_core::Size;
use slopshop_core::color::{ChannelLayout, ColorSpace, PixelFormat, SampleType};

use super::bmp::rows_out_of_order;
use super::{ExportError, ExportNotice};

/// The largest value RGBE stores: mantissa 255 at the largest exponent (255, i.e. 2¹²⁷).
const RGBE_MAX: f64 = 255.0 * f64::from_bits((1023 + 119) << 52); // 255 × 2^(255 − 136)

pub(super) struct HdrWriter {
    out: BufWriter<File>,
    row_bytes: usize,
    next_row: u32,
    height: u32,
    row: Vec<u8>,
    clipped_high: u64,
    clipped_low: u64,
}

impl HdrWriter {
    pub(super) fn new(file: File, size: Size, target: PixelFormat) -> Result<Self, ExportError> {
        if target.layout != ChannelLayout::Rgb || target.sample != SampleType::F32 {
            return Err(ExportError::InvalidSpec(format!(
                "Radiance HDR cannot store {target:?}"
            )));
        }
        if target.color_space != ColorSpace::LINEAR_SRGB {
            return Err(ExportError::UnsupportedSpace(target.color_space));
        }
        let header = format!(
            "#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y {} +X {}\n",
            size.height, size.width
        );
        let mut out = BufWriter::new(file);
        out.write_all(header.as_bytes())?;
        let width = size.width as usize;
        Ok(Self {
            out,
            row_bytes: width * 12,
            next_row: 0,
            height: size.height,
            row: Vec::with_capacity(width * 4),
            clipped_high: 0,
            clipped_low: 0,
        })
    }

    /// `rows`: little-endian `f32` RGB.
    pub(super) fn write_rows(&mut self, first_row: u32, rows: &[u8]) -> Result<(), ExportError> {
        if first_row != self.next_row || !rows.len().is_multiple_of(self.row_bytes) {
            return Err(rows_out_of_order(first_row, self.next_row, rows.len()));
        }
        for src in rows.chunks_exact(self.row_bytes) {
            self.row.clear();
            for px in src.as_chunks::<12>().0 {
                let mut rgb = [0f64; 3];
                for (c, bytes) in rgb.iter_mut().zip(px.as_chunks::<4>().0) {
                    let v = f64::from(f32::from_le_bytes(*bytes));
                    *c = if v.is_nan() {
                        // Never given (the source replaces NaN), but defined: black.
                        0.0
                    } else if v > RGBE_MAX {
                        self.clipped_high += 1;
                        RGBE_MAX
                    } else if v < 0.0 {
                        self.clipped_low += 1;
                        0.0
                    } else {
                        v
                    };
                }
                self.row.extend(rgbe(rgb));
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
        let mut notices = Vec::new();
        if self.clipped_high > 0 {
            notices.push(ExportNotice::ClippedHigh(self.clipped_high));
        }
        if self.clipped_low > 0 {
            notices.push(ExportNotice::ClippedLow(self.clipped_low));
        }
        Ok(notices)
    }
}

/// The RGBE encoding of finite values in `[0, RGBE_MAX]`: the exponent of the largest channel,
/// mantissas rounded to the nearest. Values decode as `mantissa × 2^(exponent − 136)`. Too
/// small to be stored (below 2⁻¹²⁸ or so): black.
fn rgbe(rgb: [f64; 3]) -> [u8; 4] {
    let max = rgb[0].max(rgb[1]).max(rgb[2]);
    if max <= 0.0 {
        return [0; 4];
    }
    // `max` in [2^(e − 1), 2^e): its mantissa, max × 2^(8 − e), is in [128, 256).
    let mut e = max.log2().floor() as i32 + 1;
    if max * 2f64.powi(8 - e) >= 256.0 {
        e += 1;
    } else if max * 2f64.powi(8 - e) < 128.0 {
        e -= 1;
    }
    // Rounding may carry the largest mantissa to 256: one exponent up.
    if (max * 2f64.powi(8 - e)).round() >= 256.0 {
        e += 1;
    }
    let biased = e + 128;
    if biased < 1 {
        return [0; 4];
    }
    // Only reached by values rounding above RGBE_MAX's mantissa, kept at the top.
    let biased = biased.min(255);
    let scale = 2f64.powi(8 - (biased - 128));
    let mantissa = |c: f64| (c * scale).round().min(255.0) as u8;
    [
        mantissa(rgb[0]),
        mantissa(rgb[1]),
        mantissa(rgb[2]),
        biased as u8,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode([r, g, b, e]: [u8; 4]) -> [f64; 3] {
        if e == 0 {
            return [0.0; 3];
        }
        let scale = 2f64.powi(i32::from(e) - 136);
        [r, g, b].map(|c| f64::from(c) * scale)
    }

    #[test]
    fn rgbe_round_trips_within_half_a_step() {
        for rgb in [
            [1.0, 0.5, 0.25],
            [8.0, 0.001, 0.0],
            [0.999_99, 0.999_99, 0.999_99],
            [1e-30, 3e-31, 0.0],
            [6.5e4, 1e5, 2.0],
            [RGBE_MAX, 1.0, 0.0],
        ] {
            let encoded = rgbe(rgb);
            let max_mantissa = encoded[..3].iter().max().copied().unwrap_or(0);
            assert!(
                max_mantissa >= 128,
                "{rgb:?} → {encoded:?} is not normalized"
            );
            let back = decode(encoded);
            let step = 2f64.powi(i32::from(encoded[3]) - 136);
            for (v, w) in rgb.iter().zip(back) {
                assert!(
                    (v - w).abs() <= step / 2.0 + f64::EPSILON,
                    "{rgb:?} → {back:?}"
                );
            }
        }
        assert_eq!(rgbe([0.0; 3]), [0; 4]);
        // Far below the smallest exponent: black.
        assert_eq!(rgbe([1e-45, 0.0, 0.0]), [0; 4]);
    }
}
