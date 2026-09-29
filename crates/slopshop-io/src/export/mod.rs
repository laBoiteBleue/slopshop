//! Writing images to files (ADR 0008).
//!
//! Export is a view of a document: [`export_image`] pulls working-space pixels from a source
//! closure (the GPU renderer or the CPU compositor; `slopshop-io` depends on neither), converts
//! them with [`Converter`] and streams them to a format writer, with bounded memory:
//! - the image is processed in full-width bands of [`BAND_ROWS`] rows (the last one shorter),
//!   aligned to the tile grid;
//! - a producer thread calls the source and hands bands over a channel of capacity 1, while the
//!   calling thread converts the previous band (rows in parallel) and feeds the writer. Band
//!   buffers are recycled;
//! - cancellation is checked between bands, progress is reported after each band (in rows);
//! - the file is written to a temporary file of its own in the same directory
//!   (`.<name>.<pid>-<n>.slopshop-tmp`, created exclusively, so that concurrent exports, even
//!   to the same destination, never share one), synced, then renamed over `path`. On error or
//!   cancellation that temporary file is deleted and `path` is left untouched. When several
//!   exports to one destination succeed, the last one to finish replaces the others, whole.
//!
//! Every lossy event is counted and returned in an [`ExportReport`] of stable ids (including
//! the non-finite samples the source replaced, which it counts); nothing is converted silently,
//! and files are always color-tagged ([`supports_space`]).
//!
//! # Format writers
//!
//! Each format has a writer in its own module (`png`, `tiff`, `exr`), driven the same way by
//! [`export_image`]. A writer is a struct with three methods:
//!
//! - `new(file: File, size: Size, target: PixelFormat, options) -> Result<Self, ExportError>`
//!   (`options` is the format's compression setting, when it has one). `file` is the temporary
//!   file, just created, empty, opened for reading and writing (so `Write + Seek`, unbuffered:
//!   wrap it in a `BufWriter` if needed). `size` is not empty. `target` is
//!   [`ExportSpec::target_format`]: RGB or RGBA; for TIFF U8/U16 with straight alpha or F32 with
//!   premultiplied alpha, for EXR F32/F16 with premultiplied alpha; its color space passed
//!   [`supports_space`] for the format. Tags and headers are written here or in `finish`.
//! - `write_rows(&mut self, first_row: u32, rows: &[u8]) -> Result<(), ExportError>`: called in
//!   order, from top to bottom, without gaps: `first_row` is 0, then the previous `first_row`
//!   plus the previous row count. `rows` holds a whole number of rows (a band: [`BAND_ROWS`]
//!   rows, fewer for the last one) of exactly `size.width × target.bytes_per_pixel()` bytes each,
//!   interleaved samples in the `target` format. 16/32-bit samples are little-endian for TIFF
//!   and EXR (big-endian for PNG only).
//! - `finish(self) -> Result<Vec<ExportNotice>, ExportError>`: called once after the last row.
//!   Writes what remains (trailers, offsets), flushes everything and reports its notices (e.g.
//!   [`ExportNotice::BigTiff`]); it must surface every write error (beware of encoders that
//!   write or flush in `Drop` and ignore errors there). It neither syncs, renames nor deletes
//!   the file: [`export_image`] does.
//!
//! A writer dropped without `finish` (error, cancellation) may leave a partial file behind:
//! [`export_image`] deletes it. Writer errors: [`ExportError::Io`] for I/O,
//! [`ExportError::Encode`] for encoder failures, [`ExportError::TooLarge`] for sizes the format
//! cannot store, [`ExportError::InvalidSpec`] for targets it does not handle. Inside the format
//! modules, refer to the encoder crates with a leading `::` (`::png`, `::tiff`, `::exr`), since
//! the modules have the same names.

mod exr;
mod png;
mod tiff;

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread;

use slopshop_core::color::{
    AlphaMode, ChannelLayout, ColorSpace, PixelFormat, SampleType, TransferFunction,
};
use slopshop_core::convert::{ConversionReport, ConvertError, ConvertOptions, Converter};
use slopshop_core::document::{Document, LayerContent};
use slopshop_core::{CancelToken, Progress, Rect, Size};

use self::exr::ExrWriter;
use self::png::PngWriter;
use self::tiff::TiffWriter;
use crate::icc;

/// Rows per band: one tile row of the engine's grid.
pub const BAND_ROWS: u32 = slopshop_core::raster::TILE_SIZE;

/// Suffix of the temporary files written next to the destination.
const TEMP_SUFFIX: &str = ".slopshop-tmp";

/// Characters of the destination's name kept in a temporary file's name: enough to recognize
/// it, few enough to stay within file-name length limits.
const TEMP_NAME_CHARS: usize = 64;

/// Existing files skipped before giving up on creating a temporary file.
const TEMP_ATTEMPTS: u32 = 1000;

/// Numbers the temporary files of this process.
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// The file formats export can write, without their settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExportFormatKind {
    Png,
    Tiff,
    Exr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PngDepth {
    U8,
    U16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PngCompression {
    /// fdeflate: several times faster, files somewhat larger.
    Fast,
    /// zlib level 6: smaller files, slower (single-threaded).
    Small,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TiffSample {
    U8,
    U16,
    F32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TiffCompression {
    None,
    /// Deflate (zlib), with the horizontal predictor for integer samples.
    Deflate,
    /// LZW, for older readers.
    Lzw,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExrSample {
    F32,
    /// Half float: smaller, less precise (reported as a precision reduction).
    F16,
}

/// A file format with its settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExportFormat {
    Png {
        depth: PngDepth,
        compression: PngCompression,
    },
    Tiff {
        sample: TiffSample,
        compression: TiffCompression,
    },
    Exr {
        sample: ExrSample,
    },
}

impl ExportFormat {
    pub fn kind(&self) -> ExportFormatKind {
        match self {
            ExportFormat::Png { .. } => ExportFormatKind::Png,
            ExportFormat::Tiff { .. } => ExportFormatKind::Tiff,
            ExportFormat::Exr { .. } => ExportFormatKind::Exr,
        }
    }

    /// Type of the samples written to the file.
    pub fn sample_type(&self) -> SampleType {
        match self {
            ExportFormat::Png { depth, .. } => match depth {
                PngDepth::U8 => SampleType::U8,
                PngDepth::U16 => SampleType::U16,
            },
            ExportFormat::Tiff { sample, .. } => match sample {
                TiffSample::U8 => SampleType::U8,
                TiffSample::U16 => SampleType::U16,
                TiffSample::F32 => SampleType::F32,
            },
            ExportFormat::Exr { sample } => match sample {
                ExrSample::F32 => SampleType::F32,
                ExrSample::F16 => SampleType::F16,
            },
        }
    }
}

/// Everything an export needs to know besides the pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExportSpec {
    pub format: ExportFormat,
    /// Color space of the file; must pass [`supports_space`] for the format.
    pub space: ColorSpace,
    /// Write an alpha channel. Without it, the color written is the image over black: only
    /// drop alpha for opaque images.
    pub keep_alpha: bool,
    /// Blue-noise dither for 8-bit samples (ignored for the other sample types).
    pub dither: bool,
}

impl ExportSpec {
    /// Pixel format written to the file: RGB or RGBA, the format's sample type, the spec's
    /// space. Alpha is straight, except where the format stores it premultiplied (float TIFF,
    /// EXR; ADR 0008).
    pub fn target_format(&self) -> PixelFormat {
        let premultiplied = matches!(
            self.format,
            ExportFormat::Tiff {
                sample: TiffSample::F32,
                ..
            } | ExportFormat::Exr { .. }
        );
        PixelFormat {
            layout: if self.keep_alpha {
                ChannelLayout::Rgba
            } else {
                ChannelLayout::Rgb
            },
            sample: self.format.sample_type(),
            color_space: self.space,
            alpha: if premultiplied {
                AlphaMode::Premultiplied
            } else {
                AlphaMode::Straight
            },
        }
    }
}

/// Something the user should know about what the export did. Counts are in samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportNotice {
    /// Values above the format's range were clipped.
    ClippedHigh(u64),
    /// Negative or out-of-gamut values were clipped.
    ClippedLow(u64),
    /// Infinite or NaN values were replaced (by the pixel source or the conversion).
    NonFinite(u64),
    /// Finite values beyond the half-float range were written as ±65504.
    HalfOverflow(u64),
    /// The samples are half floats: less precise than the working space.
    PrecisionReduced,
    /// The file was written as BigTIFF (over 4 GiB): some older software cannot read it.
    BigTiff,
}

impl ExportNotice {
    /// Stable identifier, translated by the UI.
    pub fn id(self) -> &'static str {
        match self {
            ExportNotice::ClippedHigh(_) => "clippedHigh",
            ExportNotice::ClippedLow(_) => "clippedLow",
            ExportNotice::NonFinite(_) => "nonFinite",
            ExportNotice::HalfOverflow(_) => "halfOverflow",
            ExportNotice::PrecisionReduced => "precisionReduced",
            ExportNotice::BigTiff => "bigTiff",
        }
    }

    /// Number of samples concerned, for the notices that count something.
    pub fn count(self) -> Option<u64> {
        match self {
            ExportNotice::ClippedHigh(n)
            | ExportNotice::ClippedLow(n)
            | ExportNotice::NonFinite(n)
            | ExportNotice::HalfOverflow(n) => Some(n),
            ExportNotice::PrecisionReduced | ExportNotice::BigTiff => None,
        }
    }
}

/// What a successful export reports. Empty when nothing was lost.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExportReport {
    pub notices: Vec<ExportNotice>,
}

impl ExportReport {
    fn new(format: &ExportFormat, conversion: &ConversionReport) -> Self {
        let counted = [
            (
                conversion.clipped_high,
                ExportNotice::ClippedHigh as fn(u64) -> ExportNotice,
            ),
            (conversion.clipped_low, ExportNotice::ClippedLow),
            (conversion.non_finite, ExportNotice::NonFinite),
            (conversion.half_overflow, ExportNotice::HalfOverflow),
        ];
        let mut notices: Vec<ExportNotice> = counted
            .into_iter()
            .filter(|(count, _)| *count > 0)
            .map(|(count, notice)| notice(count))
            .collect();
        if format.sample_type() == SampleType::F16 {
            notices.push(ExportNotice::PrecisionReduced);
        }
        Self { notices }
    }
}

#[derive(Debug)]
pub enum ExportError {
    Io(std::io::Error),
    /// The pixel source failed (rendering error), or stopped early.
    Source(String),
    Cancelled,
    /// The format cannot store (and tag) this color space.
    UnsupportedSpace(ColorSpace),
    /// The image is too large for the format, or for the memory of one band.
    TooLarge {
        width: u32,
        height: u32,
    },
    /// Settings that make no sense (empty image, path without a file name) or that a writer
    /// does not handle.
    InvalidSpec(String),
    /// The encoder failed for another reason than I/O.
    Encode(String),
}

impl ExportError {
    /// Stable identifier, translated by the UI (`export.error.<code>`).
    pub fn code(&self) -> &'static str {
        match self {
            ExportError::Io(_) => "io",
            ExportError::Source(_) => "source",
            ExportError::Cancelled => "cancelled",
            ExportError::UnsupportedSpace(_) => "unsupportedSpace",
            ExportError::TooLarge { .. } => "tooLarge",
            ExportError::InvalidSpec(_) => "invalidSpec",
            ExportError::Encode(_) => "encode",
        }
    }
}

/// The technical detail only: the UI puts it inside a translated message chosen by
/// [`ExportError::code`].
impl fmt::Display for ExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExportError::Io(e) => write!(f, "{e}"),
            ExportError::Source(e) | ExportError::InvalidSpec(e) | ExportError::Encode(e) => {
                write!(f, "{e}")
            }
            ExportError::Cancelled => write!(f, "cancelled"),
            ExportError::UnsupportedSpace(space) => match space.id() {
                Some(id) => write!(f, "{id}"),
                None => write!(f, "{space:?}"),
            },
            ExportError::TooLarge { width, height } => write!(f, "{width}×{height}"),
        }
    }
}

impl std::error::Error for ExportError {}

impl From<std::io::Error> for ExportError {
    fn from(e: std::io::Error) -> Self {
        ExportError::Io(e)
    }
}

/// Whether a file of this format can store `space` and declare it, so that it reads back as
/// the same space:
/// - PNG: sRGB (sRGB chunk), spaces with H.273 code points (cICP chunk: Rec.709/sRGB, Display
///   P3 and Rec.2020 primaries with sRGB, Rec.709, gamma 2.2/2.8, linear, PQ or HLG transfers),
///   or spaces an ICC profile can describe (iCCP chunk);
/// - TIFF: spaces an ICC profile can describe (matrix/TRC; not PQ or HLG);
/// - EXR: linear spaces with valid primaries (`chromaticities` attribute), since EXR samples are
///   scene-linear.
pub fn supports_space(kind: ExportFormatKind, space: &ColorSpace) -> bool {
    let icc_writable = || icc::write_matrix_trc(space).is_ok();
    match kind {
        ExportFormatKind::Png => {
            *space == ColorSpace::SRGB || png::cicp_code(space).is_some() || icc_writable()
        }
        ExportFormatKind::Tiff => icc_writable(),
        ExportFormatKind::Exr => {
            space.transfer == TransferFunction::Linear && space.primaries.is_valid()
        }
    }
}

/// The settings an export of `document` to `kind` starts with (ADR 0008, "Defaults per format"):
/// - PNG: 8-bit if every visible raster is 8-bit, else 16-bit; 8-bit in sRGB, 16-bit in the
///   source space when all visible rasters share one that PNG can tag, else sRGB; fast
///   compression;
/// - TIFF: the deepest source sample type (8/16-bit, float → 32-bit float); the source space
///   when unique and taggable, else Rec.2020 (integers) or linear Rec.2020 (float); Deflate;
/// - EXR: 32-bit float, linear Rec.709 (with chromaticities);
/// - alpha kept unless the document is structurally opaque (its bottom visible layer is an
///   opaque fill, or an alpha-less raster covering the canvas, at opacity 1);
/// - dither on for PNG and TIFF (it only applies to 8-bit samples).
pub fn default_spec(kind: ExportFormatKind, document: &Document) -> ExportSpec {
    let rasters: Vec<PixelFormat> = document
        .layers()
        .iter()
        .filter(|layer| layer.visible)
        .filter_map(|layer| match &layer.content {
            LayerContent::Raster { image } => Some(image.format()),
            LayerContent::Fill { .. } => None,
        })
        .collect();
    let unique_space = rasters
        .first()
        .map(|format| format.color_space)
        .filter(|space| rasters.iter().all(|format| format.color_space == *space));
    let (format, space) = match kind {
        ExportFormatKind::Png => {
            let depth = if rasters.iter().all(|format| format.sample == SampleType::U8) {
                PngDepth::U8
            } else {
                PngDepth::U16
            };
            let space = match depth {
                PngDepth::U8 => ColorSpace::SRGB,
                PngDepth::U16 => unique_space
                    .filter(|space| supports_space(kind, space))
                    .unwrap_or(ColorSpace::SRGB),
            };
            let compression = PngCompression::Fast;
            (ExportFormat::Png { depth, compression }, space)
        }
        ExportFormatKind::Tiff => {
            let sample = if rasters.iter().any(|format| format.sample.is_float()) {
                TiffSample::F32
            } else if rasters
                .iter()
                .any(|format| format.sample == SampleType::U16)
            {
                TiffSample::U16
            } else {
                TiffSample::U8
            };
            let fallback = match sample {
                TiffSample::F32 => ColorSpace::LINEAR_REC2020,
                TiffSample::U8 | TiffSample::U16 => ColorSpace::REC2020,
            };
            let space = unique_space
                .filter(|space| supports_space(kind, space))
                .unwrap_or(fallback);
            let compression = TiffCompression::Deflate;
            (
                ExportFormat::Tiff {
                    sample,
                    compression,
                },
                space,
            )
        }
        ExportFormatKind::Exr => (
            ExportFormat::Exr {
                sample: ExrSample::F32,
            },
            ColorSpace::LINEAR_SRGB,
        ),
    };
    ExportSpec {
        format,
        space,
        keep_alpha: !is_structurally_opaque(document),
        // It only applies to 8-bit samples, which EXR never has.
        dither: kind != ExportFormatKind::Exr,
    }
}

/// Whether every pixel of the document is opaque by construction: its bottom contributing layer
/// is an opaque fill, or a raster without alpha covering the canvas, at opacity 1.
fn is_structurally_opaque(document: &Document) -> bool {
    let Some(bottom) = document
        .layers()
        .iter()
        .find(|layer| layer.visible && layer.opacity > 0.0)
    else {
        return false;
    };
    if bottom.opacity < 1.0 {
        return false;
    }
    match &bottom.content {
        LayerContent::Fill { color } => color.a >= 1.0,
        LayerContent::Raster { image } => {
            let (image_size, canvas) = (image.size(), document.size());
            !image.format().layout.has_alpha()
                && image_size.width >= canvas.width
                && image_size.height >= canvas.height
        }
    }
}

/// Export an image of `size` pixels to `path` (see the module documentation).
///
/// `source(region, out)` fills `out` with `region` (full-width bands, top to bottom) as
/// premultiplied RGBA `f32` in the working space, row-major, and returns the number of
/// non-finite (NaN, ±inf) samples it replaced to produce it (reported as
/// [`ExportNotice::NonFinite`], with those the conversion meets); it runs on another thread.
/// `progress` is called on the calling thread after each band, in rows. Blocking and CPU-heavy:
/// call it off the UI thread.
pub fn export_image(
    path: &Path,
    size: Size,
    spec: &ExportSpec,
    source: impl FnMut(Rect, &mut [f32]) -> Result<u64, String> + Send,
    cancel: &CancelToken,
    progress: &mut dyn FnMut(Progress),
) -> Result<ExportReport, ExportError> {
    // Everything that can be checked is checked before the file is created.
    if size.is_empty() {
        return Err(ExportError::InvalidSpec(format!(
            "empty image ({}×{})",
            size.width, size.height
        )));
    }
    if !supports_space(spec.format.kind(), &spec.space) {
        return Err(ExportError::UnsupportedSpace(spec.space));
    }
    if let ExportFormat::Png { .. } = spec.format {
        png::check_size(size)?;
    }
    let target = spec.target_format();
    let converter = Converter::new(
        target,
        ConvertOptions {
            dither: spec.dither,
            big_endian: spec.format.kind() == ExportFormatKind::Png,
        },
    )
    .map_err(|e| match e {
        ConvertError::InvalidColorSpace(space) => ExportError::UnsupportedSpace(space),
        other => ExportError::InvalidSpec(other.to_string()),
    })?;
    let too_large = || ExportError::TooLarge {
        width: size.width,
        height: size.height,
    };
    let band_values = band_len(size.width, 4).ok_or_else(too_large)?;
    let band_bytes = band_len(size.width, converter.bytes_per_pixel()).ok_or_else(too_large)?;

    let (temp, file) = TempFile::create(path)?;
    // A second handle to sync the data once the writer has finished (and dropped its own).
    let sync = file.try_clone()?;
    let mut writer = match spec.format {
        ExportFormat::Png { compression, .. } => {
            FormatWriter::Png(Box::new(PngWriter::new(file, size, target, compression)?))
        }
        ExportFormat::Tiff { compression, .. } => {
            FormatWriter::Tiff(Box::new(TiffWriter::new(file, size, target, compression)?))
        }
        ExportFormat::Exr { .. } => {
            FormatWriter::Exr(Box::new(ExrWriter::new(file, size, target)?))
        }
    };
    let bands = Bands {
        size,
        band_values,
        band_bytes,
        converter: &converter,
        cancel,
    };
    let conversion = bands.run(source, &mut writer, progress)?;
    let mut report = ExportReport::new(&spec.format, &conversion);
    report.notices.extend(writer.finish()?);
    // Durable before it replaces the destination: a crash must not leave a truncated file
    // under the final name.
    sync.sync_all()?;
    drop(sync);
    temp.persist(path)?;
    Ok(report)
}

/// Number of samples (`per_pixel` = 4) or bytes of one full band, if it fits in memory
/// addressing.
fn band_len(width: u32, per_pixel: usize) -> Option<usize> {
    usize::try_from(width)
        .ok()?
        .checked_mul(BAND_ROWS as usize)?
        .checked_mul(per_pixel)
}

/// Boxed: writers hold encoder state of very different sizes.
enum FormatWriter {
    Png(Box<PngWriter>),
    Tiff(Box<TiffWriter>),
    Exr(Box<ExrWriter>),
}

impl FormatWriter {
    fn write_rows(&mut self, first_row: u32, rows: &[u8]) -> Result<(), ExportError> {
        match self {
            FormatWriter::Png(w) => w.write_rows(first_row, rows),
            FormatWriter::Tiff(w) => w.write_rows(first_row, rows),
            FormatWriter::Exr(w) => w.write_rows(first_row, rows),
        }
    }

    fn finish(self) -> Result<Vec<ExportNotice>, ExportError> {
        match self {
            FormatWriter::Png(w) => (*w).finish(),
            FormatWriter::Tiff(w) => (*w).finish(),
            FormatWriter::Exr(w) => (*w).finish(),
        }
    }
}

/// A band of source pixels: rows `y..y + rows`, premultiplied RGBA `f32`.
struct Band {
    y: u32,
    rows: u32,
    pixels: Vec<f32>,
    /// Non-finite samples the source replaced in this band.
    non_finite: u64,
}

/// The band pipeline of one export.
struct Bands<'a> {
    size: Size,
    /// Samples of a full source band, bytes of a full converted band.
    band_values: usize,
    band_bytes: usize,
    converter: &'a Converter,
    cancel: &'a CancelToken,
}

impl Bands<'_> {
    /// Produce every band on a helper thread, convert and write them here.
    fn run(
        &self,
        source: impl FnMut(Rect, &mut [f32]) -> Result<u64, String> + Send,
        writer: &mut FormatWriter,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<ConversionReport, ExportError> {
        let (band_tx, band_rx) = mpsc::sync_channel::<Result<Band, ExportError>>(1);
        let (spare_tx, spare_rx) = mpsc::channel::<Vec<f32>>();
        thread::scope(|scope| {
            let producer = scope.spawn(move || self.produce(source, band_tx, spare_rx));
            // Returning drops the receiver, which stops the producer at its next band.
            let result = self.consume(band_rx, spare_tx, writer, progress);
            match (result, producer.join()) {
                (Err(ExportError::Source(_)), Err(_)) => {
                    Err(ExportError::Source("the pixel source panicked".to_owned()))
                }
                (result, _) => result,
            }
        })
    }

    fn produce(
        &self,
        mut source: impl FnMut(Rect, &mut [f32]) -> Result<u64, String>,
        bands: mpsc::SyncSender<Result<Band, ExportError>>,
        spares: mpsc::Receiver<Vec<f32>>,
    ) {
        let Size { width, height } = self.size;
        let mut y = 0;
        while y < height && !self.cancel.is_cancelled() {
            let rows = BAND_ROWS.min(height - y);
            let len = self.band_values / BAND_ROWS as usize * rows as usize;
            // Reuse a band the consumer is done with, if any.
            let mut pixels = spares.try_recv().unwrap_or_default();
            let band = match pixels.try_reserve_exact(len.saturating_sub(pixels.len())) {
                Err(_) => Err(ExportError::TooLarge { width, height }),
                Ok(()) => {
                    pixels.resize(len, 0.0);
                    source(Rect::new(0, y, width, rows), &mut pixels)
                        .map(|non_finite| Band {
                            y,
                            rows,
                            pixels,
                            non_finite,
                        })
                        .map_err(ExportError::Source)
                }
            };
            let failed = band.is_err();
            if bands.send(band).is_err() || failed {
                return;
            }
            y += rows;
        }
    }

    fn consume(
        &self,
        bands: mpsc::Receiver<Result<Band, ExportError>>,
        spares: mpsc::Sender<Vec<f32>>,
        writer: &mut FormatWriter,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<ConversionReport, ExportError> {
        let Size { width, height } = self.size;
        let mut report = ConversionReport::default();
        let mut converted: Vec<u8> = Vec::new();
        converted
            .try_reserve_exact(self.band_bytes)
            .map_err(|_| ExportError::TooLarge { width, height })?;
        let mut done = 0;
        for band in bands {
            if self.cancel.is_cancelled() {
                return Err(ExportError::Cancelled);
            }
            let band = band?;
            converted.resize(self.band_bytes / BAND_ROWS as usize * band.rows as usize, 0);
            report.merge(&self.convert(&band, &mut converted)?);
            report.non_finite = report.non_finite.saturating_add(band.non_finite);
            writer.write_rows(band.y, &converted)?;
            done = band.y + band.rows;
            // The producer may be gone already (last band): nothing to recycle then.
            spares.send(band.pixels).ok();
            progress(Progress {
                done: u64::from(done),
                total: u64::from(height),
            });
        }
        if self.cancel.is_cancelled() {
            return Err(ExportError::Cancelled);
        }
        if done < height {
            return Err(ExportError::Source(format!(
                "the pixel source stopped at row {done} of {height}"
            )));
        }
        Ok(report)
    }

    /// Convert a band into `out`, rows split across threads.
    fn convert(&self, band: &Band, out: &mut [u8]) -> Result<ConversionReport, ExportError> {
        let width = self.size.width as usize;
        let (src_row, dst_row) = (width * 4, width * self.converter.bytes_per_pixel());
        let threads = thread::available_parallelism().map_or(1, |n| n.get());
        let rows_per_chunk = (band.rows as usize).div_ceil(threads).max(1);
        let converter = self.converter;
        thread::scope(|scope| {
            let workers: Vec<_> = band
                .pixels
                .chunks(rows_per_chunk * src_row)
                .zip(out.chunks_mut(rows_per_chunk * dst_row))
                .enumerate()
                .map(|(chunk, (src, dst))| {
                    scope.spawn(move || {
                        let mut report = ConversionReport::default();
                        let rows = src.chunks_exact(src_row).zip(dst.chunks_exact_mut(dst_row));
                        for (i, (src, dst)) in rows.enumerate() {
                            // Fits: the row is inside the band, inside the image.
                            let y = band.y + (chunk * rows_per_chunk + i) as u32;
                            converter.convert_row(src, 0, y, dst, &mut report)?;
                        }
                        Ok::<_, ConvertError>(report)
                    })
                })
                .collect();
            let mut report = ConversionReport::default();
            for worker in workers {
                let chunk = worker
                    .join()
                    .map_err(|_| ExportError::Encode("a conversion thread panicked".to_owned()))?
                    .map_err(|e| ExportError::Encode(e.to_string()))?;
                report.merge(&chunk);
            }
            Ok(report)
        })
    }
}

/// A temporary file next to the destination, owned by one export: deleted unless it was renamed
/// over the destination.
struct TempFile {
    path: PathBuf,
    persisted: bool,
}

impl TempFile {
    /// Create `.<name>.<pid>-<n>.slopshop-tmp` next to `destination` (`<name>`: the start of the
    /// destination's name; `<n>`: a counter of this process), empty, for reading and writing.
    /// It is created exclusively: an existing file (another export's, or one a crash left
    /// behind) is skipped, never truncated, renamed or deleted.
    fn create(destination: &Path) -> Result<(Self, File), ExportError> {
        let name = destination.file_name().ok_or_else(|| {
            ExportError::InvalidSpec(format!("{} is not a file path", destination.display()))
        })?;
        // Only a hint for whoever finds the file: a lossy name is fine.
        let name: String = name
            .to_string_lossy()
            .chars()
            .take(TEMP_NAME_CHARS)
            .collect();
        let pid = std::process::id();
        let mut skipped = 0;
        loop {
            let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = destination.with_file_name(format!(".{name}.{pid}-{n}{TEMP_SUFFIX}"));
            let created = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&path);
            match created {
                Ok(file) => {
                    let temp = Self {
                        path,
                        persisted: false,
                    };
                    return Ok((temp, file));
                }
                Err(e) if e.kind() == ErrorKind::AlreadyExists && skipped < TEMP_ATTEMPTS => {
                    skipped += 1;
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// Replace `destination` with the temporary file. Every handle on it must be closed.
    fn persist(mut self, destination: &Path) -> std::io::Result<()> {
        fs::rename(&self.path, destination)?;
        self.persisted = true;
        Ok(())
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        if !self.persisted {
            // Best effort: the export's own error matters more than the clean-up's.
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// The temporary files of exports to `destination` that still exist (tests: none may be left).
#[cfg(test)]
fn temp_files(destination: &Path) -> Vec<PathBuf> {
    let (Some(dir), Some(name)) = (destination.parent(), destination.file_name()) else {
        return Vec::new();
    };
    let prefix = format!(".{}.", name.to_string_lossy());
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .is_some_and(|n| n.starts_with(&prefix) && n.ends_with(TEMP_SUFFIX))
        })
        .collect()
}

#[cfg(test)]
mod tests;
