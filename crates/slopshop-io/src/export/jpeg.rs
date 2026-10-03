//! JPEG writer (ADR 0010): baseline JPEG streamed through `jpeg_encoder`.
//!
//! - Interleaved baseline (sequential DCT, standard Annex K tables and Huffman codes), 8-bit
//!   YCbCr, chroma subsampled 4:4:4, 4:2:2 or 4:2:0 as the spec says, chroma averaged over each
//!   block (the crate's default, nearest, costs quality and bytes); or a single gray component
//!   for gray targets (subsampling does not apply).
//! - Always tagged: the ICC profile of the space in APP2 segments (sRGB included; a gray profile
//!   for gray files). No EXIF or
//!   XMP: none is carried, and an orientation tag must never be written.
//! - No alpha: JPEG cannot store it, so the export flattens it over the matte first.
//! - At most [`MAX_SIDE`] pixels per side: the file format allows 65535, but the libjpeg family
//!   of decoders refuses anything above 65500.
//!
//! `jpeg_encoder` pulls rows from an [`ImageBuffer`] until the image is done, so it runs on a
//! thread of its own, which owns the file: [`JpegWriter::write_rows`] converts each band from
//! RGB to YCbCr planes (on the export's thread, in parallel with the encoding) and sends it over
//! a channel of capacity 1; band buffers come back to be reused. Memory is bounded: a few bands
//! (`3 × width × 256` bytes each) plus the encoder's buffers of one row of blocks.
//!
//! The encoder cannot be told to stop from its row source: when the export gives up (error or
//! cancellation), the source pads with the last row and the file sink fails on the next write,
//! which stops the encoder; the thread then closes the file so that the export can delete it.

use std::cell::{Cell, RefCell};
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::thread::{self, JoinHandle};

use jpeg_encoder::{
    ChromaSubsamplingMethod, Encoder, EncodingError, ImageBuffer, JpegColorType, SamplingFactor,
    rgb_to_ycbcr,
};
use slopshop_core::Size;
use slopshop_core::color::{ChannelLayout, PixelFormat, SampleType};

use super::{ExportError, ExportNotice, JpegSubsampling};
use crate::icc;

/// Largest side the libjpeg-family decoders accept (`JPEG_MAX_DIMENSION`).
pub const MAX_SIDE: u32 = 65_500;

/// JPEG sizes, checked before anything is written.
pub(super) fn check_size(size: Size) -> Result<(), ExportError> {
    if size.width > MAX_SIDE || size.height > MAX_SIDE {
        return Err(ExportError::TooLarge {
            width: size.width,
            height: size.height,
        });
    }
    Ok(())
}

/// An 8-bit sRGB RGB image (`size.width × size.height × 3` bytes, rows top to bottom) as a
/// JPEG in memory, 4:4:4 and tagged sRGB: for small pages such as File > Print's, not for
/// exports (which stream to their file).
pub fn encode_srgb8(rgb: &[u8], size: Size, quality: u8) -> Result<Vec<u8>, ExportError> {
    check_size(size)?;
    if rgb.len() != size.width as usize * size.height as usize * 3 {
        return Err(ExportError::Encode(
            "the pixels do not match the size".to_owned(),
        ));
    }
    let profile = icc::write_matrix_trc(&slopshop_core::color::ColorSpace::SRGB)
        .map_err(|_| ExportError::UnsupportedSpace(slopshop_core::color::ColorSpace::SRGB))?;
    let mut bytes = Vec::new();
    let mut encoder = Encoder::new(&mut bytes, quality);
    encoder.set_sampling_factor(SamplingFactor::F_1_1);
    encoder.add_icc_profile(&profile).map_err(encode_error)?;
    // Checked above: both sides fit in `u16`.
    let (width, height) = (size.width as u16, size.height as u16);
    encoder
        .encode(rgb, width, height, jpeg_encoder::ColorType::Rgb)
        .map_err(encode_error)?;
    Ok(bytes)
}

/// A band of rows as YCbCr planes (or one gray plane), from `first_row`.
struct Band {
    first_row: u32,
    rows: u32,
    /// Y, Cb and Cr, `width × rows` bytes each; only the first for gray images.
    planes: [Vec<u8>; 3],
}

pub(super) struct JpegWriter {
    /// Bands to the encoding thread. `None` once closed.
    bands: Option<SyncSender<Band>>,
    /// Band buffers the encoder is done with.
    spares: Receiver<[Vec<u8>; 3]>,
    /// The encoding thread. `None` once joined.
    encoder: Option<JoinHandle<Result<(), ExportError>>>,
    size: Size,
    next_row: u32,
    /// One gray component instead of YCbCr.
    gray: bool,
}

impl JpegWriter {
    pub(super) fn new(
        file: File,
        size: Size,
        target: PixelFormat,
        quality: u8,
        subsampling: JpegSubsampling,
    ) -> Result<Self, ExportError> {
        let gray = target.layout == ChannelLayout::Gray;
        if !(gray || target.layout == ChannelLayout::Rgb) || target.sample != SampleType::U8 {
            return Err(ExportError::InvalidSpec(format!(
                "JPEG export cannot store {target:?}"
            )));
        }
        if !(1..=100).contains(&quality) {
            return Err(ExportError::InvalidSpec(format!(
                "JPEG quality {quality} is not between 1 and 100"
            )));
        }
        if size.is_empty() {
            return Err(ExportError::InvalidSpec(format!(
                "empty image ({}×{})",
                size.width, size.height
            )));
        }
        check_size(size)?;
        let profile = if gray {
            icc::write_gray_trc(&target.color_space)
        } else {
            icc::write_matrix_trc(&target.color_space)
        }
        .map_err(|_| ExportError::UnsupportedSpace(target.color_space))?;
        let (bands, received) = mpsc::sync_channel(1);
        let (spares_back, spares) = mpsc::channel();
        let settings = Settings {
            size,
            quality,
            subsampling,
            profile,
            gray,
        };
        let encoder = thread::Builder::new()
            .name("jpeg-writer".to_owned())
            .spawn(move || encode(file, settings, received, spares_back))?;
        Ok(Self {
            bands: Some(bands),
            spares,
            encoder: Some(encoder),
            size,
            next_row: 0,
            gray,
        })
    }

    pub(super) fn write_rows(&mut self, first_row: u32, rows: &[u8]) -> Result<(), ExportError> {
        let width = self.size.width as usize;
        let row_bytes = width * if self.gray { 1 } else { 3 };
        let count = rows.len() / row_bytes;
        if first_row != self.next_row
            || !rows.len().is_multiple_of(row_bytes)
            || count > (self.size.height - self.next_row) as usize
        {
            return Err(ExportError::Encode(format!(
                "rows out of order: {} bytes at row {first_row}, expected row {} of {}",
                rows.len(),
                self.next_row,
                self.size.height
            )));
        }
        if count == 0 {
            return Ok(());
        }
        let mut planes = self.spares.try_recv().unwrap_or_default();
        for plane in &mut planes {
            plane.clear();
            plane
                .try_reserve_exact(width * count)
                .map_err(|_| ExportError::TooLarge {
                    width: self.size.width,
                    height: self.size.height,
                })?;
        }
        let [y, cb, cr] = &mut planes;
        if self.gray {
            y.extend_from_slice(rows);
        } else {
            for pixel in rows.as_chunks::<3>().0 {
                let (luma, blue, red) = rgb_to_ycbcr(pixel[0], pixel[1], pixel[2]);
                y.push(luma);
                cb.push(blue);
                cr.push(red);
            }
        }
        let band = Band {
            first_row,
            rows: count as u32,
            planes,
        };
        if let Some(bands) = &self.bands
            && bands.send(band).is_ok()
        {
            self.next_row += count as u32;
            return Ok(());
        }
        // The thread only hangs up before the last band when it failed.
        Err(match self.join() {
            Err(e) => e,
            Ok(()) => ExportError::Encode("the JPEG encoder stopped early".to_owned()),
        })
    }

    pub(super) fn finish(mut self) -> Result<Vec<ExportNotice>, ExportError> {
        if self.next_row != self.size.height {
            return Err(ExportError::Encode(format!(
                "{} rows written out of {}",
                self.next_row, self.size.height
            )));
        }
        self.join()?;
        Ok(Vec::new())
    }

    /// Close the channel and wait for the encoding thread; its result.
    fn join(&mut self) -> Result<(), ExportError> {
        self.bands = None;
        match self.encoder.take() {
            Some(encoder) => encoder.join().unwrap_or_else(|_| {
                Err(ExportError::Encode("the JPEG encoder panicked".to_owned()))
            }),
            None => Ok(()),
        }
    }
}

impl Drop for JpegWriter {
    /// Without `finish` (error, cancellation): the source sees the channel close before the last
    /// band, the encoder stops at its next write, and the thread closes the file, so that the
    /// export can delete it.
    fn drop(&mut self) {
        // The export is already failing: its own error is the one to report.
        let _ = self.join();
    }
}

struct Settings {
    size: Size,
    quality: u8,
    subsampling: JpegSubsampling,
    profile: Vec<u8>,
    gray: bool,
}

/// The encoding thread: header, ICC profile, then every row pulled from the bands received.
fn encode(
    file: File,
    settings: Settings,
    bands: Receiver<Band>,
    spares: Sender<[Vec<u8>; 3]>,
) -> Result<(), ExportError> {
    let aborted = Rc::new(Cell::new(false));
    let mut sink = Sink {
        file: BufWriter::new(file),
        aborted: aborted.clone(),
    };
    let mut encoder = Encoder::new(&mut sink, settings.quality);
    encoder.set_sampling_factor(match settings.subsampling {
        JpegSubsampling::S444 => SamplingFactor::F_1_1,
        JpegSubsampling::S422 => SamplingFactor::F_2_1,
        JpegSubsampling::S420 => SamplingFactor::F_2_2,
    });
    encoder.set_chroma_subsampling_method(ChromaSubsamplingMethod::Average);
    encoder
        .add_icc_profile(&settings.profile)
        .map_err(encode_error)?;
    let source = Source {
        size: settings.size,
        components: if settings.gray { 1 } else { 3 },
        state: RefCell::new(SourceState {
            bands,
            spares,
            band: None,
        }),
        aborted: aborted.clone(),
    };
    let result = encoder.encode_image(source);
    if aborted.get() {
        return Err(ExportError::Encode(
            "the export stopped before the last row".to_owned(),
        ));
    }
    result.map_err(encode_error)?;
    sink.file.flush()?;
    Ok(())
}

fn encode_error(e: EncodingError) -> ExportError {
    match e {
        EncodingError::IoError(e) => ExportError::Io(e),
        other => ExportError::Encode(other.to_string()),
    }
}

/// The file, failing every write once the source has run out of bands early.
struct Sink {
    file: BufWriter<File>,
    aborted: Rc<Cell<bool>>,
}

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.aborted.get() {
            return Err(io::Error::other("JPEG export aborted"));
        }
        self.file.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

/// The rows of the image, pulled by the encoder in increasing order (the last one several
/// times, to pad the last row of blocks).
struct Source {
    size: Size,
    /// 1 (gray) or 3 (YCbCr).
    components: usize,
    state: RefCell<SourceState>,
    aborted: Rc<Cell<bool>>,
}

struct SourceState {
    bands: Receiver<Band>,
    spares: Sender<[Vec<u8>; 3]>,
    /// The band holding the rows last asked for.
    band: Option<Band>,
}

impl ImageBuffer for Source {
    fn get_jpeg_color_type(&self) -> JpegColorType {
        if self.components == 1 {
            JpegColorType::Luma
        } else {
            JpegColorType::Ycbcr
        }
    }

    fn width(&self) -> u16 {
        // Fits: at most `MAX_SIDE`, checked in `JpegWriter::new`.
        self.size.width as u16
    }

    fn height(&self) -> u16 {
        self.size.height as u16
    }

    fn fill_buffers(&self, y: u16, buffers: &mut [Vec<u8>; 4]) {
        let y = u32::from(y);
        let width = self.size.width as usize;
        let mut state = self.state.borrow_mut();
        while !self.aborted.get()
            && state
                .band
                .as_ref()
                .is_none_or(|band| y >= band.first_row + band.rows)
        {
            match state.bands.recv() {
                Ok(next) => {
                    if let Some(done) = state.band.replace(next) {
                        // The writer may be gone already: nothing to recycle then.
                        state.spares.send(done.planes).ok();
                    }
                }
                Err(_) => self.aborted.set(true),
            }
        }
        match (&state.band, self.aborted.get()) {
            (Some(band), false) => {
                let start = (y - band.first_row) as usize * width;
                let planes = &band.planes[..self.components];
                for (buffer, plane) in buffers.iter_mut().zip(planes) {
                    buffer.extend_from_slice(&plane[start..start + width]);
                }
            }
            // Aborted: any bytes will do, the next write fails.
            _ => {
                for buffer in &mut buffers[..self.components] {
                    buffer.resize(buffer.len() + width, 0);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
