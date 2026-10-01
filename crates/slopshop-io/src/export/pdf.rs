//! PDF writer (in-house): one page holding the image, 8-bit gray or RGB, color-managed with an
//! ICC-based color space (our profile of the target space, the device space as its alternate),
//! alpha as a soft mask (`/SMask`, an 8-bit gray image).
//!
//! The page measures one point per pixel (72 pixels per inch), so that it opens at the
//! image's size at 72 dpi; images larger than [`MAX_PAGE_POINTS`] get a page scaled down to
//! that size (Acrobat's largest page, 200 inches), at a higher resolution: every pixel is kept.
//!
//! The image is Flate-compressed with the PNG Up predictor, and streamed: its compressed length
//! is unknown until the last row, so the image dictionary refers to an indirect `/Length`
//! object written after the stream. The color samples go straight to the file through the
//! compressor; the soft mask, a second stream, is compressed in memory meanwhile (one byte per
//! pixel at most, before compression) and written after the image. Then the cross-reference
//! table and the trailer.

use std::fs::File;
use std::io::{BufWriter, Write};

use flate2::write::ZlibEncoder;
use slopshop_core::Size;
use slopshop_core::color::{AlphaMode, ChannelLayout, PixelFormat, SampleType};

use super::bmp::rows_out_of_order;
use super::{ExportError, ExportNotice};
use crate::icc;

/// The largest page side, in points (Acrobat's limit, 200 inches).
const MAX_PAGE_POINTS: f64 = 14_400.0;

/// zlib level of the image streams (see the TIFF writer's measurements: level 2 is as fast as
/// level 1 and gives smaller files).
const DEFLATE_LEVEL: u32 = 2;

/// The cross-reference table stores offsets in 10 digits.
const MAX_OFFSET: u64 = 9_999_999_999;

// Object numbers.
const CATALOG: usize = 1;
const PAGES: usize = 2;
const PAGE: usize = 3;
const CONTENTS: usize = 4;
const INFO: usize = 5;
const PROFILE: usize = 6;
const IMAGE: usize = 7;
const IMAGE_LENGTH: usize = 8;
const SOFT_MASK: usize = 9;

/// Counts the bytes written through it: the objects' offsets.
struct Counted<W> {
    inner: W,
    written: u64,
}

impl<W: Write> Write for Counted<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.written += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

pub(super) struct PdfWriter {
    /// The color samples' stream, compressed into the file.
    image: ZlibEncoder<Counted<BufWriter<File>>>,
    /// The alpha samples, compressed in memory.
    mask: Option<ZlibEncoder<Vec<u8>>>,
    /// Offset of each object, by number (0 unused).
    offsets: [u64; SOFT_MASK + 1],
    /// Offset of the image stream's first byte.
    stream_start: u64,
    size: Size,
    /// Color channels: 1 (gray) or 3 (RGB).
    colors: usize,
    next_row: u32,
    /// The previous color and alpha rows, for the Up predictor.
    previous: Vec<u8>,
    previous_alpha: Vec<u8>,
    /// One predicted row, its filter byte first.
    row: Vec<u8>,
}

impl PdfWriter {
    pub(super) fn new(file: File, size: Size, target: PixelFormat) -> Result<Self, ExportError> {
        let (colors, alpha) = match target.layout {
            ChannelLayout::Gray => (1, false),
            ChannelLayout::GrayAlpha => (1, true),
            ChannelLayout::Rgb => (3, false),
            ChannelLayout::Rgba => (3, true),
        };
        if target.sample != SampleType::U8 || (alpha && target.alpha != AlphaMode::Straight) {
            return Err(ExportError::InvalidSpec(format!(
                "PDF cannot store {target:?}"
            )));
        }
        let profile = if colors == 1 {
            icc::write_gray_trc(&target.color_space)
        } else {
            icc::write_matrix_trc(&target.color_space)
        }
        .map_err(|_| ExportError::UnsupportedSpace(target.color_space))?;

        let Size { width, height } = size;
        let longer = f64::from(width.max(height));
        let scale = if longer > MAX_PAGE_POINTS {
            MAX_PAGE_POINTS / longer
        } else {
            1.0
        };
        let (page_width, page_height) = (
            number(f64::from(width) * scale),
            number(f64::from(height) * scale),
        );

        let mut out = Counted {
            inner: BufWriter::new(file),
            written: 0,
        };
        let mut offsets = [0; SOFT_MASK + 1];
        // A comment of bytes above 127 after the header: the file is binary.
        out.write_all(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")?;
        let mut object = |out: &mut Counted<_>, number: usize, body: &[u8]| {
            offsets[number] = out.written;
            out.write_all(format!("{number} 0 obj\n").as_bytes())?;
            out.write_all(body)?;
            out.write_all(b"\nendobj\n")
        };
        object(
            &mut out,
            CATALOG,
            format!("<< /Type /Catalog /Pages {PAGES} 0 R >>").as_bytes(),
        )?;
        object(
            &mut out,
            PAGES,
            format!("<< /Type /Pages /Kids [{PAGE} 0 R] /Count 1 >>").as_bytes(),
        )?;
        object(
            &mut out,
            PAGE,
            format!(
                "<< /Type /Page /Parent {PAGES} 0 R /MediaBox [0 0 {page_width} {page_height}] \
                 /Resources << /XObject << /Im0 {IMAGE} 0 R >> >> /Contents {CONTENTS} 0 R >>"
            )
            .as_bytes(),
        )?;
        // The image fills the page (an image is drawn in the unit square).
        let contents = format!("q {page_width} 0 0 {page_height} 0 0 cm /Im0 Do Q");
        object(
            &mut out,
            CONTENTS,
            &stream(
                &format!("<< /Length {} >>", contents.len()),
                contents.as_bytes(),
            ),
        )?;
        object(&mut out, INFO, b"<< /Producer (SlopShop) >>")?;
        let device = if colors == 1 {
            "/DeviceGray"
        } else {
            "/DeviceRGB"
        };
        object(
            &mut out,
            PROFILE,
            &stream(
                &format!(
                    "<< /N {colors} /Alternate {device} /Length {} >>",
                    profile.len()
                ),
                &profile,
            ),
        )?;

        offsets[IMAGE] = out.written;
        let mask = if alpha {
            format!(" /SMask {SOFT_MASK} 0 R")
        } else {
            String::new()
        };
        out.write_all(
            format!(
                "{IMAGE} 0 obj\n<< /Type /XObject /Subtype /Image /Width {width} \
                 /Height {height} /ColorSpace [/ICCBased {PROFILE} 0 R] /BitsPerComponent 8 \
                 {}{mask} /Length {IMAGE_LENGTH} 0 R >>\nstream\n",
                flate(colors, width)
            )
            .as_bytes(),
        )?;
        let stream_start = out.written;
        let level = flate2::Compression::new(DEFLATE_LEVEL);
        let row_bytes = width as usize * colors;
        Ok(Self {
            image: ZlibEncoder::new(out, level),
            mask: alpha.then(|| ZlibEncoder::new(Vec::new(), level)),
            offsets,
            stream_start,
            size,
            colors,
            next_row: 0,
            previous: vec![0; row_bytes],
            previous_alpha: if alpha {
                vec![0; width as usize]
            } else {
                Vec::new()
            },
            row: Vec::with_capacity(row_bytes + 1),
        })
    }

    pub(super) fn write_rows(&mut self, first_row: u32, rows: &[u8]) -> Result<(), ExportError> {
        let width = self.size.width as usize;
        let channels = self.colors + usize::from(self.mask.is_some());
        let row_bytes = width * channels;
        if first_row != self.next_row || !rows.len().is_multiple_of(row_bytes) {
            return Err(rows_out_of_order(first_row, self.next_row, rows.len()));
        }
        for src in rows.chunks_exact(row_bytes) {
            // PNG Up: each sample minus the one above it (the first row: minus zero).
            self.row.clear();
            self.row.push(2);
            let mut i = 0;
            for pixel in src.chunks_exact(channels) {
                for &sample in &pixel[..self.colors] {
                    self.row.push(sample.wrapping_sub(self.previous[i]));
                    self.previous[i] = sample;
                    i += 1;
                }
            }
            self.image.write_all(&self.row)?;
            if let Some(mask) = &mut self.mask {
                self.row.clear();
                self.row.push(2);
                for (pixel, above) in src
                    .chunks_exact(channels)
                    .zip(self.previous_alpha.iter_mut())
                {
                    let alpha = pixel[self.colors];
                    self.row.push(alpha.wrapping_sub(*above));
                    *above = alpha;
                }
                mask.write_all(&self.row)?;
            }
            self.next_row += 1;
        }
        Ok(())
    }

    pub(super) fn finish(self) -> Result<Vec<ExportNotice>, ExportError> {
        let Size { width, height } = self.size;
        if self.next_row != height {
            return Err(ExportError::Encode(format!(
                "{} rows written out of {height}",
                self.next_row
            )));
        }
        let mut out = self.image.finish()?;
        let mut offsets = self.offsets;
        let length = out.written - self.stream_start;
        out.write_all(b"\nendstream\nendobj\n")?;
        offsets[IMAGE_LENGTH] = out.written;
        out.write_all(format!("{IMAGE_LENGTH} 0 obj\n{length}\nendobj\n").as_bytes())?;
        let objects = if let Some(mask) = self.mask {
            let mask = mask.finish()?;
            offsets[SOFT_MASK] = out.written;
            out.write_all(format!("{SOFT_MASK} 0 obj\n").as_bytes())?;
            out.write_all(&stream(
                &format!(
                    "<< /Type /XObject /Subtype /Image /Width {width} /Height {height} \
                     /ColorSpace /DeviceGray /BitsPerComponent 8 {} /Length {} >>",
                    flate(1, width),
                    mask.len()
                ),
                &mask,
            ))?;
            out.write_all(b"\nendobj\n")?;
            SOFT_MASK
        } else {
            IMAGE_LENGTH
        };
        let xref = out.written;
        if xref > MAX_OFFSET {
            return Err(ExportError::TooLarge { width, height });
        }
        // Each entry is 20 bytes: offset, generation, in use, a two-byte end of line.
        let mut table = format!("xref\n0 {}\n0000000000 65535 f \n", objects + 1);
        for offset in &offsets[1..=objects] {
            table += &format!("{offset:010} 00000 n \n");
        }
        table += &format!(
            "trailer\n<< /Size {} /Root {CATALOG} 0 R /Info {INFO} 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects + 1
        );
        out.write_all(table.as_bytes())?;
        out.flush()?;
        Ok(Vec::new())
    }
}

/// A stream object's body: its dictionary, then its data.
fn stream(dictionary: &str, data: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(dictionary.len() + data.len() + 32);
    body.extend(dictionary.as_bytes());
    body.extend(b"\nstream\n");
    body.extend(data);
    body.extend(b"\nendstream");
    body
}

/// The filter of an image stream: Flate with the PNG predictors.
fn flate(colors: usize, width: u32) -> String {
    format!(
        "/Filter /FlateDecode /DecodeParms << /Predictor 12 /Colors {colors} \
         /BitsPerComponent 8 /Columns {width} >>"
    )
}

/// A PDF number: at most four decimals, without trailing zeros.
fn number(v: f64) -> String {
    let text = format!("{v:.4}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_short() {
        assert_eq!(number(300.0), "300");
        assert_eq!(number(14_400.0 * 300.0 / 20_000.0), "216");
        assert_eq!(number(14_400.0 / 3.0 * 2.0), "9600");
        assert_eq!(number(1.0 / 3.0), "0.3333");
    }
}
