//! TIFF writer (ADR 0008): baseline RGB/RGBA strips of 8/16-bit integer or 32-bit float
//! samples, little-endian, always tagged with an ICC profile.
//!
//! The strips are ours, written through `::tiff`'s low-level `DirectoryEncoder`: tiff 0.11.3's
//! streaming `ImageEncoder::write_strip` ignores the compression it declares (corrupt LZW and
//! Deflate files, see the regression test). So this writer:
//! - cuts each band into strips of at most [`STRIP_BYTES`] and compresses them in parallel
//!   (Deflate with flate2's zlib, LZW with weezl, as tiff does internally), with the horizontal
//!   predictor for integer samples;
//! - hands them to a writer thread, which owns the file and the encoder (a `DirectoryEncoder`
//!   borrows its encoder, so both cannot be kept in this struct between calls) and writes them
//!   while the next band is converted;
//! - writes every tag explicitly once all strips are written: dimensions, samples, compression,
//!   predictor, strip tables, `ExtraSamples` (2 = straight alpha, 1 = premultiplied), and the
//!   ICC profile with the `UNDEFINED` type, as the TIFF/ICC specifications ask.
//!
//! Classic TIFF stores 32-bit offsets. Whether the file needs BigTIFF is decided before
//! anything is written, from a worst-case bound of its size ([`size_bound`]); BigTIFF is
//! reported ([`ExportNotice::BigTiff`]) since some older software cannot read it.

use std::fs::File;
use std::io::{BufWriter, Seek, Write};
use std::mem;
use std::sync::mpsc;
use std::thread;

use ::tiff::encoder::{
    DirectoryEncoder, Rational, TiffEncoder, TiffKind, TiffKindBig, TiffKindStandard, TiffValue,
};
use ::tiff::tags::{
    CompressionMethod, PhotometricInterpretation, PlanarConfiguration, Predictor, ResolutionUnit,
    SampleFormat, Tag, Type,
};
use ::tiff::{Directory, TiffError};
use slopshop_core::Size;
use slopshop_core::color::{AlphaMode, ChannelLayout, PixelFormat, SampleType};

use super::{BAND_ROWS, ExportError, ExportNotice, TiffCompression};
use crate::icc;

/// Largest strip, uncompressed: large enough for good compression and small strip tables,
/// small enough to compress several strips of a band in parallel.
const STRIP_BYTES: u64 = 1 << 20;

/// Most rows per strip: a full band gives at least 8 strips to compress in parallel, even when
/// the image is narrow.
const MAX_STRIP_ROWS: u32 = BAND_ROWS / 8;

// Rows per strip are powers of two up to `MAX_STRIP_ROWS`: they divide the band height, so
// strips never straddle two bands.
const _: () = assert!(BAND_ROWS.is_power_of_two() && MAX_STRIP_ROWS >= 1);

/// Files whose size bound exceeds this are written as BigTIFF (classic TIFF offsets are 32-bit).
/// The margin covers what the bound might have missed.
pub(crate) const BIG_TIFF_THRESHOLD: u64 = u32::MAX as u64 - (64 << 20);

/// zlib level of Deflate strips. Measured on noisy 16-bit samples (miniz_oxide): level 2 is as
/// fast as level 1 with files 15% smaller, and 9 times faster than level 6 (zlib's default) for
/// files 1% larger.
const DEFLATE_LEVEL: u32 = 2;

/// Bands of compressed strips waiting for the writer thread, besides the one it writes: bounds
/// memory while letting disk writes overlap the next band.
const QUEUED_BANDS: usize = 1;

pub(super) struct TiffWriter {
    layout: Layout,
    next_row: u32,
    big: bool,
    /// Compressed strips of each band, in order, to the writer thread. `None` once closed.
    strips: Option<mpsc::SyncSender<Vec<Vec<u8>>>>,
    thread: Option<thread::JoinHandle<Result<(), ExportError>>>,
}

impl TiffWriter {
    pub(super) fn new(
        file: File,
        size: Size,
        target: PixelFormat,
        compression: TiffCompression,
    ) -> Result<Self, ExportError> {
        Self::with_big_tiff_threshold(file, size, target, compression, BIG_TIFF_THRESHOLD)
    }

    /// [`Self::new`], BigTIFF being chosen above `threshold` bytes (tests force it with 0).
    pub(super) fn with_big_tiff_threshold(
        file: File,
        size: Size,
        target: PixelFormat,
        compression: TiffCompression,
        threshold: u64,
    ) -> Result<Self, ExportError> {
        // Samples come little-endian from the converter, and tiff declares the byte order of
        // the machine in the header.
        if cfg!(target_endian = "big") {
            return Err(ExportError::InvalidSpec(
                "TIFF export needs a little-endian machine".to_owned(),
            ));
        }
        let layout = Layout::new(size, target, compression)?;
        let profile = icc::write_matrix_trc(&target.color_space)
            .map_err(|_| ExportError::UnsupportedSpace(target.color_space))?;
        let bound = size_bound(&layout, profile.len()).ok_or_else(|| layout.too_large())?;
        let big = bound > threshold;

        let (sender, receiver) = mpsc::sync_channel(QUEUED_BANDS);
        let tags = layout.clone();
        let thread = thread::Builder::new()
            .name("slopshop-tiff-writer".to_owned())
            .spawn(move || {
                if big {
                    write_file::<TiffKindBig>(file, &tags, &profile, receiver)
                } else {
                    write_file::<TiffKindStandard>(file, &tags, &profile, receiver)
                }
            })?;
        Ok(Self {
            layout,
            next_row: 0,
            big,
            strips: Some(sender),
            thread: Some(thread),
        })
    }

    pub(super) fn write_rows(&mut self, first_row: u32, rows: &[u8]) -> Result<(), ExportError> {
        let layout = &self.layout;
        let count = rows.len() / layout.row_bytes;
        let end = u32::try_from(count)
            .ok()
            .and_then(|count| first_row.checked_add(count))
            .filter(|end| *end <= layout.height);
        let valid = first_row == self.next_row && rows.len().is_multiple_of(layout.row_bytes);
        // Only the last rows of the image may fill a strip partially.
        let whole_strips = |end| end == layout.height || count.is_multiple_of(layout.strip_rows());
        let Some(end) = end.filter(|end| valid && whole_strips(*end)) else {
            return Err(ExportError::Encode(format!(
                "rows out of order: {} bytes at row {first_row}, expected whole strips from \
                 row {}",
                rows.len(),
                self.next_row
            )));
        };
        let strips = layout.encode_strips(rows)?;
        let sent = match &self.strips {
            Some(sender) => sender.send(strips).is_ok(),
            None => false,
        };
        if !sent {
            // The writer thread stopped: its error says why.
            return Err(self.join().err().unwrap_or_else(|| {
                ExportError::Encode("the TIFF writer thread stopped early".to_owned())
            }));
        }
        self.next_row = end;
        Ok(())
    }

    pub(super) fn finish(mut self) -> Result<Vec<ExportNotice>, ExportError> {
        if self.next_row != self.layout.height {
            return Err(ExportError::Encode(format!(
                "{} rows written out of {}",
                self.next_row, self.layout.height
            )));
        }
        // The thread writes the tags once it has every strip, then flushes the file.
        self.join()?;
        Ok(if self.big {
            vec![ExportNotice::BigTiff]
        } else {
            Vec::new()
        })
    }

    /// Close the channel and wait for the writer thread.
    fn join(&mut self) -> Result<(), ExportError> {
        self.strips = None;
        match self.thread.take() {
            Some(thread) => thread
                .join()
                .map_err(|_| ExportError::Encode("the TIFF writer thread panicked".to_owned()))?,
            None => Ok(()),
        }
    }
}

impl Drop for TiffWriter {
    /// Dropped without `finish` (error, cancellation): the thread must close the file before
    /// `export_image` deletes it (Windows cannot delete an open file).
    fn drop(&mut self) {
        let _ = self.join();
    }
}

/// What the strips and the tags of an image depend on.
#[derive(Debug, Clone)]
struct Layout {
    width: u32,
    height: u32,
    channels: u16,
    sample: SampleType,
    /// `None` without an alpha channel.
    alpha: Option<AlphaMode>,
    compression: TiffCompression,
    row_bytes: usize,
    strip_rows: u32,
}

impl Layout {
    fn new(
        size: Size,
        target: PixelFormat,
        compression: TiffCompression,
    ) -> Result<Self, ExportError> {
        if size.is_empty() {
            return Err(ExportError::InvalidSpec("empty image".to_owned()));
        }
        let invalid = || ExportError::InvalidSpec(format!("TIFF cannot store {target:?}"));
        let alpha = match target.layout {
            ChannelLayout::Rgb => None,
            ChannelLayout::Rgba => Some(target.alpha),
            ChannelLayout::Gray | ChannelLayout::GrayAlpha => return Err(invalid()),
        };
        if !matches!(
            target.sample,
            SampleType::U8 | SampleType::U16 | SampleType::F32
        ) {
            return Err(invalid());
        }
        let too_large = ExportError::TooLarge {
            width: size.width,
            height: size.height,
        };
        let row_bytes = usize::try_from(size.width)
            .ok()
            .and_then(|width| width.checked_mul(target.bytes_per_pixel() as usize))
            .ok_or(too_large)?;
        Ok(Self {
            width: size.width,
            height: size.height,
            // 3 or 4.
            channels: target.layout.channels() as u16,
            sample: target.sample,
            alpha,
            compression,
            row_bytes,
            strip_rows: strip_rows(row_bytes as u64),
        })
    }

    fn too_large(&self) -> ExportError {
        ExportError::TooLarge {
            width: self.width,
            height: self.height,
        }
    }

    fn strip_rows(&self) -> usize {
        self.strip_rows as usize
    }

    fn strip_count(&self) -> u64 {
        u64::from(self.height.div_ceil(self.strip_rows))
    }

    fn bits_per_sample(&self) -> u16 {
        // 8, 16 or 32.
        self.sample.bytes() as u16 * 8
    }

    /// The horizontal predictor, for compressed integer samples only (floats would need the
    /// floating-point predictor, which fewer readers support).
    fn predictor(&self) -> Predictor {
        let integer = matches!(self.sample, SampleType::U8 | SampleType::U16);
        if integer && self.compression != TiffCompression::None {
            Predictor::Horizontal
        } else {
            Predictor::None
        }
    }

    fn compression_method(&self) -> CompressionMethod {
        match self.compression {
            TiffCompression::None => CompressionMethod::None,
            TiffCompression::Deflate => CompressionMethod::Deflate,
            TiffCompression::Lzw => CompressionMethod::LZW,
        }
    }

    /// Cut whole rows into strips and encode them, strips split across threads.
    fn encode_strips(&self, rows: &[u8]) -> Result<Vec<Vec<u8>>, ExportError> {
        let strips: Vec<&[u8]> = rows.chunks(self.strip_rows() * self.row_bytes).collect();
        let threads = thread::available_parallelism().map_or(1, |n| n.get());
        let per_thread = strips.len().div_ceil(threads).max(1);
        thread::scope(|scope| {
            let workers: Vec<_> = strips
                .chunks(per_thread)
                .map(|chunk| {
                    scope.spawn(move || {
                        let mut scratch = Vec::new();
                        chunk
                            .iter()
                            .map(|strip| self.encode_strip(strip, &mut scratch))
                            .collect::<Result<Vec<_>, _>>()
                    })
                })
                .collect();
            let mut encoded = Vec::with_capacity(strips.len());
            for worker in workers {
                let chunk = worker.join().map_err(|_| {
                    ExportError::Encode("a TIFF compression thread panicked".to_owned())
                })??;
                encoded.extend(chunk);
            }
            Ok(encoded)
        })
    }

    /// One strip as stored in the file. `scratch` receives the predicted samples.
    fn encode_strip(&self, strip: &[u8], scratch: &mut Vec<u8>) -> Result<Vec<u8>, ExportError> {
        let data = if self.predictor() == Predictor::Horizontal {
            scratch.clear();
            scratch.extend_from_slice(strip);
            let stride = usize::from(self.channels) * self.sample.bytes() as usize;
            for row in scratch.chunks_exact_mut(self.row_bytes) {
                predict_row(row, self.sample, stride);
            }
            &scratch[..]
        } else {
            strip
        };
        let encode_error = |e: &dyn std::fmt::Display| ExportError::Encode(e.to_string());
        match self.compression {
            // A copy: the writer thread needs owned data.
            TiffCompression::None => Ok(data.to_vec()),
            TiffCompression::Deflate => {
                let out = Vec::with_capacity(data.len() / 2);
                let level = flate2::Compression::new(DEFLATE_LEVEL);
                let mut encoder = flate2::write::ZlibEncoder::new(out, level);
                encoder.write_all(data).map_err(|e| encode_error(&e))?;
                encoder.finish().map_err(|e| encode_error(&e))
            }
            TiffCompression::Lzw => {
                let mut out = Vec::with_capacity(data.len() / 2);
                // As tiff does: MSB-first codes, code size switched one code early (TIFF).
                let mut encoder =
                    weezl::encode::Encoder::with_tiff_size_switch(weezl::BitOrder::Msb, 8);
                let result = encoder.into_vec(&mut out).encode_all(data);
                result.status.map_err(|e| encode_error(&e))?;
                Ok(out)
            }
        }
    }
}

/// Rows per strip: the largest power of two whose strip fits in [`STRIP_BYTES`], within
/// `1..=MAX_STRIP_ROWS` (a single row may be larger than `STRIP_BYTES`).
fn strip_rows(row_bytes: u64) -> u32 {
    let fit = (STRIP_BYTES / row_bytes.max(1)).clamp(1, u64::from(MAX_STRIP_ROWS));
    // Fits: at most MAX_STRIP_ROWS.
    1 << (fit as u32).ilog2()
}

/// Horizontal predictor (TIFF predictor 2), in place on one row of little-endian samples: each
/// sample becomes its difference (wrapping) with the same channel of the previous pixel,
/// `stride` bytes before.
fn predict_row(row: &mut [u8], sample: SampleType, stride: usize) {
    match sample {
        SampleType::U8 => {
            for i in (stride..row.len()).rev() {
                row[i] = row[i].wrapping_sub(row[i - stride]);
            }
        }
        SampleType::U16 => {
            let at = |row: &[u8], i: usize| u16::from_le_bytes([row[i], row[i + 1]]);
            for i in (stride..row.len()).step_by(2).rev() {
                let delta = at(row, i).wrapping_sub(at(row, i - stride));
                row[i..i + 2].copy_from_slice(&delta.to_le_bytes());
            }
        }
        // Never predicted (see `Layout::predictor`).
        SampleType::F16 | SampleType::F32 => {}
    }
}

/// Upper bound of the size of the file as classic TIFF, in bytes (`None` beyond `u64`).
fn size_bound(layout: &Layout, profile_len: usize) -> Option<u64> {
    let data = (layout.row_bytes as u64).checked_mul(u64::from(layout.height))?;
    let strips = layout.strip_count();
    let expansion = match layout.compression {
        TiffCompression::None => 0,
        // Incompressible data is stored: 5 bytes per block of at most 64 KiB, plus the zlib
        // header and checksum (6 bytes) of each strip.
        TiffCompression::Deflate => data / 1024 + strips * 64,
        // Each LZW code stands for at least one byte and takes at most 12 bits, plus the clear
        // and end codes.
        TiffCompression::Lzw => data / 2 + data / 1024 + strips * 64,
    };
    // Header, directory (fewer than 20 entries) and its values, strip tables (8 bytes per
    // strip), ICC profile, word padding.
    let tags = 4096 + strips * 8 + profile_len as u64;
    data.checked_add(expansion)?.checked_add(tags)
}

/// The writer thread: writes the strips as they come, then the directory once all are written.
fn write_file<K: TiffKind>(
    file: File,
    layout: &Layout,
    profile: &[u8],
    bands: mpsc::Receiver<Vec<Vec<u8>>>,
) -> Result<(), ExportError> {
    let mut out = BufWriter::new(file);
    let mut encoder = TiffEncoder::<_, K>::new_generic(&mut out).map_err(tiff_error)?;
    let mut dir = encoder.image_directory().map_err(tiff_error)?;
    let expected = layout.strip_count();
    let mut offsets = Vec::new();
    let mut byte_counts = Vec::new();
    // Ends when every strip is written, or when the writer is dropped (channel closed).
    for strip in bands.iter().flatten() {
        offsets.push(dir.write_data(&strip[..]).map_err(tiff_error)?);
        byte_counts.push(strip.len() as u64);
        if offsets.len() as u64 == expected {
            break;
        }
    }
    if offsets.len() as u64 != expected {
        // Error or cancellation: the file is discarded.
        return Err(ExportError::Encode(format!(
            "{} TIFF strips written out of {expected}",
            offsets.len()
        )));
    }
    write_tags(&mut dir, layout, profile, &offsets, &byte_counts)?;
    dir.finish().map_err(tiff_error)?;
    // BufWriter ignores errors when dropped.
    out.flush()?;
    Ok(())
}

/// Every tag of the image, including the strip tables and the ICC profile.
fn write_tags<W: Write + Seek, K: TiffKind>(
    dir: &mut DirectoryEncoder<'_, W, K>,
    layout: &Layout,
    profile: &[u8],
    offsets: &[u64],
    byte_counts: &[u64],
) -> Result<(), ExportError> {
    let channels = usize::from(layout.channels);
    let format = if layout.sample.is_float() {
        SampleFormat::IEEEFP
    } else {
        SampleFormat::Uint
    };
    write_tag(dir, Tag::ImageWidth, layout.width)?;
    write_tag(dir, Tag::ImageLength, layout.height)?;
    write_tag(
        dir,
        Tag::BitsPerSample,
        &vec![layout.bits_per_sample(); channels][..],
    )?;
    write_tag(dir, Tag::Compression, layout.compression_method())?;
    write_tag(
        dir,
        Tag::PhotometricInterpretation,
        PhotometricInterpretation::RGB,
    )?;
    write_tag(dir, Tag::SamplesPerPixel, layout.channels)?;
    write_tag(dir, Tag::RowsPerStrip, layout.strip_rows)?;
    // Baseline TIFF requires a resolution: the document has no physical size (yet), so only
    // the (square) aspect ratio is declared.
    write_tag(dir, Tag::XResolution, Rational { n: 1, d: 1 })?;
    write_tag(dir, Tag::YResolution, Rational { n: 1, d: 1 })?;
    write_tag(dir, Tag::ResolutionUnit, ResolutionUnit::None)?;
    write_tag(dir, Tag::PlanarConfiguration, PlanarConfiguration::Chunky)?;
    write_tag(dir, Tag::Predictor, layout.predictor())?;
    if let Some(alpha) = layout.alpha {
        let extra: u16 = match alpha {
            AlphaMode::Premultiplied => 1,
            AlphaMode::Straight => 2,
        };
        write_tag(dir, Tag::ExtraSamples, &[extra][..])?;
    }
    write_tag(dir, Tag::SampleFormat, &vec![format; channels][..])?;

    // Strip tables: LONG in classic TIFF (checked, although the size bound guarantees it),
    // LONG8 in BigTIFF.
    let big = mem::size_of::<K::OffsetType>() == 8;
    for (tag, values) in [
        (Tag::StripOffsets, offsets),
        (Tag::StripByteCounts, byte_counts),
    ] {
        let (ty, bytes) = if big {
            let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_ne_bytes()).collect();
            (Type::LONG8, bytes)
        } else {
            let mut bytes = Vec::with_capacity(values.len() * 4);
            for &value in values {
                let value = u32::try_from(value).map_err(|_| layout.too_large())?;
                bytes.extend_from_slice(&value.to_ne_bytes());
            }
            (Type::LONG, bytes)
        };
        write_entry(dir, tag, ty, &bytes)?;
    }
    write_entry(dir, Tag::IccProfile, Type::UNDEFINED, profile)
}

fn write_tag<W: Write + Seek, K: TiffKind, T: TiffValue>(
    dir: &mut DirectoryEncoder<'_, W, K>,
    tag: Tag,
    value: T,
) -> Result<(), ExportError> {
    dir.write_tag(tag, value).map_err(tiff_error)
}

/// A tag of an explicit type, from bytes in the file's (native) byte order.
fn write_entry<W: Write + Seek, K: TiffKind>(
    dir: &mut DirectoryEncoder<'_, W, K>,
    tag: Tag,
    ty: Type,
    bytes: &[u8],
) -> Result<(), ExportError> {
    let entry = dir.write_entry_bytes(ty, bytes).map_err(tiff_error)?;
    dir.extend_from(&Directory::from_iter([(tag, entry)]));
    Ok(())
}

fn tiff_error(e: TiffError) -> ExportError {
    match e {
        TiffError::IoError(e) => ExportError::Io(e),
        other => ExportError::Encode(other.to_string()),
    }
}

#[cfg(test)]
mod tests;
