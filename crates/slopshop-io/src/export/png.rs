//! PNG writer (ADR 0008): rows streamed through `png::StreamWriter`, 8/16-bit RGB/RGBA with
//! straight alpha, 16-bit samples big-endian.
//!
//! Color is always declared: sRGB with the sRGB chunk; spaces with H.273 code points with a cICP
//! chunk (png 0.18 never writes it: it is written by hand, before the image data), plus an iCCP
//! profile when ICC can describe the space too, for readers that ignore cICP (not PQ or HLG);
//! other spaces with an iCCP profile only.

use std::borrow::Cow;
use std::cell::Cell;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::rc::Rc;

use slopshop_core::Size;
use slopshop_core::color::{
    AlphaMode, ChannelLayout, ColorSpace, PixelFormat, RgbPrimaries, SampleType, TransferFunction,
};

use super::{ExportError, ExportNotice, PngCompression};
use crate::icc;

/// Largest compressed chunk: fewer, larger IDAT chunks.
const CHUNK_SIZE: usize = 1 << 20;

/// PNG stores dimensions as 31-bit integers.
pub(super) fn check_size(size: Size) -> Result<(), ExportError> {
    let limit = i32::MAX as u32;
    if size.width > limit || size.height > limit {
        return Err(ExportError::TooLarge {
            width: size.width,
            height: size.height,
        });
    }
    Ok(())
}

/// ITU-T H.273 color primaries and transfer characteristics of `space`, if it has some that
/// our importer reads back (see `cicp_space` in the crate root).
pub(super) fn cicp_code(space: &ColorSpace) -> Option<[u8; 2]> {
    const PRIMARIES: [(RgbPrimaries, u8); 3] = [
        (RgbPrimaries::REC709, 1),
        (RgbPrimaries::REC2020, 9),
        (RgbPrimaries::DISPLAY_P3, 12),
    ];
    let (_, primaries) = PRIMARIES
        .iter()
        .find(|(primaries, _)| *primaries == space.primaries)?;
    let transfer = match space.transfer {
        TransferFunction::Rec709 => 1,
        TransferFunction::Gamma(2.2) => 4,
        TransferFunction::Gamma(2.8) => 5,
        TransferFunction::Linear => 8,
        TransferFunction::Srgb => 13,
        TransferFunction::Pq => 16,
        TransferFunction::Hlg => 18,
        _ => return None,
    };
    Some([*primaries, transfer])
}

pub(super) struct PngWriter {
    stream: ::png::StreamWriter<'static, Sink>,
    /// First I/O error of the file, including those png ignores in `Drop`.
    error: Rc<Cell<Option<io::Error>>>,
    row_bytes: usize,
    next_row: u32,
    height: u32,
}

impl PngWriter {
    pub(super) fn new(
        file: File,
        size: Size,
        target: PixelFormat,
        compression: PngCompression,
    ) -> Result<Self, ExportError> {
        check_size(size)?;
        let color_type = match target.layout {
            ChannelLayout::Rgb => ::png::ColorType::Rgb,
            ChannelLayout::Rgba if target.alpha == AlphaMode::Straight => ::png::ColorType::Rgba,
            _ => return Err(invalid_target(&target)),
        };
        let bit_depth = match target.sample {
            SampleType::U8 => ::png::BitDepth::Eight,
            SampleType::U16 => ::png::BitDepth::Sixteen,
            SampleType::F16 | SampleType::F32 => return Err(invalid_target(&target)),
        };
        let space = target.color_space;
        let srgb = space == ColorSpace::SRGB;
        let cicp = if srgb { None } else { cicp_code(&space) };
        let profile = if srgb {
            None
        } else {
            icc::write_matrix_trc(&space).ok()
        };
        if !srgb && cicp.is_none() && profile.is_none() {
            return Err(ExportError::UnsupportedSpace(space));
        }

        let mut info = ::png::Info::with_size(size.width, size.height);
        info.color_type = color_type;
        info.bit_depth = bit_depth;
        if srgb {
            info.srgb = Some(::png::SrgbRenderingIntent::Perceptual);
        }
        info.icc_profile = profile.map(Cow::Owned);

        let error = Rc::new(Cell::new(None));
        let sink = Sink {
            file: BufWriter::new(file),
            error: Rc::clone(&error),
        };
        let mut encoder = ::png::Encoder::with_info(sink, info).map_err(encoding_error)?;
        encoder.set_compression(match compression {
            PngCompression::Fast => ::png::Compression::Fast,
            PngCompression::Small => ::png::Compression::Balanced,
        });
        let mut writer = encoder.write_header().map_err(encoding_error)?;
        if let Some([primaries, transfer]) = cicp {
            // RGB (matrix 0), full range.
            writer
                .write_chunk(::png::chunk::cICP, &[primaries, transfer, 0, 1])
                .map_err(encoding_error)?;
        }
        let stream = writer
            .into_stream_writer_with_size(CHUNK_SIZE)
            .map_err(encoding_error)?;
        Ok(Self {
            stream,
            error,
            row_bytes: size.width as usize * target.bytes_per_pixel() as usize,
            next_row: 0,
            height: size.height,
        })
    }

    pub(super) fn write_rows(&mut self, first_row: u32, rows: &[u8]) -> Result<(), ExportError> {
        if first_row != self.next_row || !rows.len().is_multiple_of(self.row_bytes) {
            return Err(ExportError::Encode(format!(
                "rows out of order: {} bytes at row {first_row}, expected row {}",
                rows.len(),
                self.next_row
            )));
        }
        self.stream.write_all(rows)?;
        // Fits: the encoder refuses more rows than the image has.
        self.next_row += (rows.len() / self.row_bytes) as u32;
        Ok(())
    }

    pub(super) fn finish(self) -> Result<Vec<ExportNotice>, ExportError> {
        if self.next_row != self.height {
            return Err(ExportError::Encode(format!(
                "{} rows written out of {}",
                self.next_row, self.height
            )));
        }
        // Also drops the inner writer, which writes IEND and flushes the file.
        self.stream.finish().map_err(encoding_error)?;
        match self.error.take() {
            Some(e) => Err(ExportError::Io(e)),
            None => Ok(Vec::new()),
        }
    }
}

fn invalid_target(target: &PixelFormat) -> ExportError {
    ExportError::InvalidSpec(format!("PNG cannot store {target:?}"))
}

fn encoding_error(e: ::png::EncodingError) -> ExportError {
    match e {
        ::png::EncodingError::IoError(e) => ExportError::Io(e),
        other => ExportError::Encode(other.to_string()),
    }
}

/// The file as png sees it. png writes the last chunks (IEND) and flushes in `Drop`, ignoring
/// errors there: the sink keeps the first error so that [`PngWriter::finish`] can report it.
struct Sink {
    file: BufWriter<File>,
    error: Rc<Cell<Option<io::Error>>>,
}

impl Sink {
    fn remember<T>(&self, result: io::Result<T>) -> io::Result<T> {
        if let Err(e) = &result {
            let first = self.error.take();
            self.error.set(Some(
                first.unwrap_or_else(|| io::Error::new(e.kind(), e.to_string())),
            ));
        }
        result
    }
}

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let result = self.file.write(buf);
        self.remember(result)
    }

    fn flush(&mut self) -> io::Result<()> {
        let result = self.file.flush();
        self.remember(result)
    }
}

impl Drop for Sink {
    fn drop(&mut self) {
        let result = self.file.flush();
        let _ = self.remember(result);
    }
}
