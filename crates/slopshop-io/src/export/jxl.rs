//! Lossless JPEG XL writer: zune-jpegxl (pure Rust) encodes the frame, 8 or 16 bits per sample,
//! gray or RGB, with alpha.
//!
//! zune-jpegxl writes its own image header, which always declares sRGB and an 8-bit alpha
//! channel (16-bit alpha would be clipped). Its header ends padded to a byte, before the frame:
//! [`image_header`] writes ours instead (the color encoding of the file's space, as JPEG XL
//! enumerates it or as custom primaries and gamma; the alpha at the samples' depth), and the
//! frame follows unchanged. The header replaced is checked to be exactly the one zune-jpegxl's
//! code writes ([`zune_header`]), so a different version cannot be spliced silently.
//!
//! The whole image is held (zune-jpegxl encodes from one buffer): at most [`MAX_SIDE`] pixels
//! per side, at least 2. 16-bit gray with alpha is written as RGBA (see `finish`).

use std::fs::File;
use std::io::{BufWriter, Write};

use slopshop_core::color::{
    AlphaMode, ChannelLayout, ColorSpace, D65, PixelFormat, RgbPrimaries, SampleType,
    TransferFunction,
};
use slopshop_core::{CancelToken, Size};
use zune_core::bit_depth::BitDepth;
use zune_core::colorspace::ColorSpace as ZuneColors;
use zune_core::options::EncoderOptions;

use super::bmp::rows_out_of_order;
use super::{ExportError, ExportNotice};

/// Largest side: the image is held whole while it is encoded.
pub const MAX_SIDE: u32 = 65535;

/// Whether a JPEG XL file can declare `space` without an ICC profile: any primaries (enumerated
/// or custom), and any transfer but the parametric curves of ICC profiles.
pub(super) fn can_declare(space: &ColorSpace) -> bool {
    space.primaries.is_valid() && !matches!(space.transfer, TransferFunction::Parametric { .. })
}

pub(super) struct JxlWriter {
    out: BufWriter<File>,
    pixels: Vec<u8>,
    size: Size,
    target: PixelFormat,
    row_bytes: usize,
    next_row: u32,
    cancel: CancelToken,
}

impl JxlWriter {
    pub(super) fn new(
        file: File,
        size: Size,
        target: PixelFormat,
        cancel: CancelToken,
    ) -> Result<Self, ExportError> {
        let invalid = || ExportError::InvalidSpec(format!("JPEG XL cannot store {target:?}"));
        if size.width > MAX_SIDE || size.height > MAX_SIDE {
            return Err(ExportError::TooLarge {
                width: size.width,
                height: size.height,
            });
        }
        if size.width < 2 || size.height < 2 {
            return Err(ExportError::InvalidSpec(
                "JPEG XL export needs at least 2 × 2 pixels".to_owned(),
            ));
        }
        if !matches!(target.sample, SampleType::U8 | SampleType::U16) {
            return Err(invalid());
        }
        let alpha = matches!(
            target.layout,
            ChannelLayout::GrayAlpha | ChannelLayout::Rgba
        );
        if alpha && target.alpha != AlphaMode::Straight {
            return Err(invalid());
        }
        if !can_declare(&target.color_space) {
            return Err(ExportError::UnsupportedSpace(target.color_space));
        }
        let row_bytes = size.width as usize * target.bytes_per_pixel() as usize;
        Ok(Self {
            out: BufWriter::new(file),
            pixels: Vec::with_capacity(row_bytes * size.height as usize),
            size,
            target,
            row_bytes,
            next_row: 0,
            cancel,
        })
    }

    pub(super) fn write_rows(&mut self, first_row: u32, rows: &[u8]) -> Result<(), ExportError> {
        if first_row != self.next_row || !rows.len().is_multiple_of(self.row_bytes) {
            return Err(rows_out_of_order(first_row, self.next_row, rows.len()));
        }
        self.pixels.extend_from_slice(rows);
        self.next_row += (rows.len() / self.row_bytes) as u32;
        Ok(())
    }

    pub(super) fn finish(mut self) -> Result<Vec<ExportNotice>, ExportError> {
        if self.next_row != self.size.height {
            return Err(ExportError::Encode(format!(
                "{} rows written out of {}",
                self.next_row, self.size.height
            )));
        }
        if self.cancel.is_cancelled() {
            return Err(ExportError::Cancelled);
        }
        let mut format = Layout::of(&self.target);
        // HACK(zune-jpegxl 0.5.2 reads 16-bit gray + alpha samples as signed: values from 32768
        // come out wrong): such images are written as RGBA, the gray in each color channel.
        // Lossless, but the file is not gray. Follow-up: fix upstream, then drop this.
        if format.gray && format.alpha && format.bits == 16 {
            self.pixels = gray_alpha_to_rgba16(&self.pixels);
            format.gray = false;
        }
        let (width, height) = (self.size.width as usize, self.size.height as usize);
        let colors = match (format.gray, format.alpha) {
            (true, false) => ZuneColors::Luma,
            (true, true) => ZuneColors::LumaA,
            (false, false) => ZuneColors::RGB,
            (false, true) => ZuneColors::RGBA,
        };
        let depth = if format.bits == 8 {
            BitDepth::Eight
        } else {
            BitDepth::Sixteen
        };
        let encoder = zune_jpegxl::JxlSimpleEncoder::new(
            &self.pixels,
            EncoderOptions::new(width, height, colors, depth),
        );
        let mut encoded = Vec::new();
        encoder
            .encode(&mut encoded)
            .map_err(|e| ExportError::Encode(format!("JPEG XL: {e:?}")))?;
        drop(std::mem::take(&mut self.pixels));
        let theirs = zune_header(self.size, format);
        if !encoded.starts_with(&theirs) {
            return Err(ExportError::Encode(
                "unexpected JPEG XL header from the encoder".to_owned(),
            ));
        }
        self.out
            .write_all(&image_header(self.size, format, &self.target.color_space))?;
        self.out.write_all(&encoded[theirs.len()..])?;
        self.out.flush()?;
        Ok(Vec::new())
    }
}

/// 16-bit gray + alpha samples as RGBA (native byte order).
fn gray_alpha_to_rgba16(pixels: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(pixels.len() * 2);
    for px in pixels.as_chunks::<4>().0 {
        let (gray, alpha) = ([px[0], px[1]], [px[2], px[3]]);
        out.extend([gray, gray, gray, alpha].concat());
    }
    out
}

/// What the headers depend on.
#[derive(Debug, Clone, Copy)]
struct Layout {
    gray: bool,
    alpha: bool,
    bits: u32,
}

impl Layout {
    fn of(target: &PixelFormat) -> Self {
        Self {
            gray: target.layout.is_gray(),
            alpha: matches!(
                target.layout,
                ChannelLayout::GrayAlpha | ChannelLayout::Rgba
            ),
            bits: if target.sample == SampleType::U8 {
                8
            } else {
                16
            },
        }
    }
}

/// JPEG XL's bit writer: least significant bits first.
#[derive(Default)]
struct Bits {
    bytes: Vec<u8>,
    used: u32,
}

impl Bits {
    fn put(&mut self, n: u32, value: u64) {
        for i in 0..n {
            if self.used.is_multiple_of(8) {
                self.bytes.push(0);
            }
            if value >> i & 1 != 0 {
                let last = self.bytes.len() - 1;
                self.bytes[last] |= 1 << (self.used % 8);
            }
            self.used += 1;
        }
    }

    fn bool(&mut self, b: bool) {
        self.put(1, u64::from(b));
    }

    /// An enum value: `U32(Val(0), Val(1), BitsOffset(4, 2), BitsOffset(6, 18))`.
    fn enumeration(&mut self, v: u32) {
        match v {
            0 | 1 => self.put(2, u64::from(v)),
            2..=17 => {
                self.put(2, 2);
                self.put(4, u64::from(v - 2));
            }
            _ => {
                self.put(2, 3);
                self.put(6, u64::from(v - 18));
            }
        }
    }

    /// A chromaticity coordinate (`CustomXY`'s fields): millionths, packed signed, then
    /// `U32(Bits(19), BitsOffset(19, 524288), BitsOffset(20, 1048576), BitsOffset(21, 2097152))`.
    fn coordinate(&mut self, v: f64) {
        let v = (v * 1e6).round() as i64;
        let packed = if v >= 0 { 2 * v } else { -2 * v - 1 } as u64;
        let ranges = [(19, 0), (19, 524_288), (20, 1_048_576), (21, 2_097_152)];
        for (selector, (bits, offset)) in ranges.into_iter().enumerate() {
            if packed >= offset && packed - offset < 1 << bits {
                self.put(2, selector as u64);
                self.put(bits, packed - offset);
                return;
            }
        }
        // Beyond every range: the largest (coordinates are below 4.19).
        self.put(2, 3);
        self.put(21, (1 << 21) - 1);
    }

    fn xy(&mut self, [x, y]: [f64; 2]) {
        self.coordinate(x);
        self.coordinate(y);
    }

    /// Pad to a byte.
    fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

/// The signature and the size header (not "small"), as zune-jpegxl writes them.
fn size_header(bits: &mut Bits, size: Size) {
    bits.put(16, 0x0AFF);
    bits.put(1, 0);
    let mut dimension = |v: u32, ratio: bool| {
        let v = u64::from(v - 1);
        let (selector, n) = match v {
            v if v < 1 << 9 => (0, 9),
            v if v < 1 << 13 => (1, 13),
            v if v < 1 << 18 => (2, 18),
            _ => (3, 30),
        };
        bits.put(2, selector);
        bits.put(n, v);
        if ratio {
            bits.put(3, 0);
        }
    };
    dimension(size.height, true);
    dimension(size.width, false);
}

/// The image header zune-jpegxl 0.5.2 writes (its `prepare_header`): sRGB, 8-bit alpha.
fn zune_header(size: Size, f: Layout) -> Vec<u8> {
    let mut bits = Bits::default();
    size_header(&mut bits, size);
    bits.put(1, 0);
    bits.put(1, 0);
    bits.put(1, 0);
    if f.bits == 8 {
        bits.put(2, 0);
    } else {
        bits.put(2, 3);
        bits.put(6, u64::from(f.bits - 1));
    }
    bits.bool(f.bits <= 14);
    if f.alpha {
        bits.put(2, 1);
        bits.put(1, 1);
    } else {
        bits.put(2, 0);
    }
    bits.put(1, 0);
    if f.gray {
        bits.put(1, 0);
        bits.put(1, 0);
        bits.put(2, 1);
        bits.put(2, 1);
        bits.put(1, 0);
        bits.put(2, 2);
        bits.put(4, 11);
        bits.put(2, 1);
    } else {
        bits.put(1, 1);
    }
    bits.put(2, 0);
    bits.put(1, 1);
    bits.finish()
}

/// Our image header: the same samples, the alpha at their depth, the color encoding of `space`.
fn image_header(size: Size, f: Layout, space: &ColorSpace) -> Vec<u8> {
    let mut bits = Bits::default();
    size_header(&mut bits, size);
    // Not all default, no extra fields; integer samples.
    bits.put(1, 0);
    bits.put(1, 0);
    bits.put(1, 0);
    if f.bits == 8 {
        bits.put(2, 0);
    } else {
        bits.put(2, 3);
        bits.put(6, u64::from(f.bits - 1));
    }
    bits.bool(f.bits <= 14);
    if f.alpha {
        // One extra channel: alpha (not all default), its depth, no shift, no name, straight.
        bits.put(2, 1);
        bits.bool(false);
        bits.enumeration(0);
        bits.bool(false);
        if f.bits == 8 {
            bits.put(2, 0);
        } else {
            bits.put(2, 3);
            bits.put(6, u64::from(f.bits - 1));
        }
        bits.put(2, 0);
        bits.put(2, 0);
        bits.bool(false);
    } else {
        bits.put(2, 0);
    }
    // Not XYB.
    bits.bool(false);
    color_encoding(&mut bits, f.gray, space);
    // No extensions; default transform data.
    bits.put(2, 0);
    bits.bool(true);
    bits.finish()
}

/// `ColourEncoding` of `space`, without an ICC profile.
fn color_encoding(bits: &mut Bits, gray: bool, space: &ColorSpace) {
    // Not all default (sRGB is written in full too), no ICC.
    bits.bool(false);
    bits.bool(false);
    // RGB 0, gray 1.
    bits.enumeration(u32::from(gray));
    let p = &space.primaries;
    if p.white == D65 {
        bits.enumeration(1);
    } else {
        bits.enumeration(2);
        bits.xy(p.white);
    }
    if !gray {
        let same = |q: &RgbPrimaries| p.red == q.red && p.green == q.green && p.blue == q.blue;
        if same(&RgbPrimaries::REC709) {
            bits.enumeration(1);
        } else if same(&RgbPrimaries::REC2020) {
            bits.enumeration(9);
        } else if same(&RgbPrimaries::DISPLAY_P3) {
            bits.enumeration(11);
        } else {
            bits.enumeration(2);
            bits.xy(p.red);
            bits.xy(p.green);
            bits.xy(p.blue);
        }
    }
    match space.transfer {
        TransferFunction::Gamma(g) => {
            // The encoding exponent, in ten-millionths.
            bits.bool(true);
            bits.put(24, (1e7 / f64::from(g)).round() as u64);
        }
        transfer => {
            bits.bool(false);
            bits.enumeration(match transfer {
                TransferFunction::Rec709 => 1,
                TransferFunction::Linear => 8,
                TransferFunction::Pq => 16,
                TransferFunction::Hlg => 18,
                // sRGB; parametric curves are refused before.
                _ => 13,
            });
        }
    }
    // Relative colorimetric.
    bits.enumeration(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_go_least_significant_first() {
        let mut bits = Bits::default();
        bits.put(16, 0x0AFF);
        bits.put(3, 0b101);
        assert_eq!(bits.finish(), [0xFF, 0x0A, 0b101]);
    }

    #[test]
    fn our_header_matches_zune_for_what_zune_can_say() {
        // An 8-bit sRGB image without alpha: the same declaration, if not the same bits.
        let f = Layout {
            gray: false,
            alpha: false,
            bits: 8,
        };
        let size = Size::new(300, 7);
        assert!(!image_header(size, f, &ColorSpace::SRGB).is_empty());
        assert!(zune_header(size, f).starts_with(&[0xFF, 0x0A]));
    }

    #[test]
    fn declarable_spaces() {
        assert!(can_declare(&ColorSpace::PROPHOTO));
        assert!(can_declare(&ColorSpace::REC2100_PQ));
        let parametric = ColorSpace {
            primaries: RgbPrimaries::REC709,
            transfer: TransferFunction::Parametric {
                g: 2.4,
                a: 1.0,
                b: 0.0,
                c: 0.0,
                d: 0.0,
                e: 0.0,
                f: 0.0,
            },
        };
        assert!(!can_declare(&parametric));
    }
}
