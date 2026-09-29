//! OpenEXR writer (ADR 0008): scanline blocks streamed through `::exr::block::write`.
//!
//! - One part, scan lines in increasing Y order, lossless ZIP compression of 16 lines per block.
//! - Channels `A`, `B`, `G`, `R` (`B`, `G`, `R` without alpha), sorted by name as EXR requires,
//!   of 32-bit or 16-bit floats. Color is scene-linear and premultiplied by alpha (the OpenEXR
//!   convention).
//! - The `chromaticities` attribute always declares the primaries and white point of the file
//!   (readers assume Rec.709 without it), so only linear spaces with valid primaries are
//!   accepted.
//!
//! exr's block writer is driven by a closure that must produce every block before it returns,
//! so it runs on a thread of its own, which owns the file: [`ExrWriter::write_rows`] rearranges
//! the interleaved rows into EXR blocks (planar per line: all the samples of a line for one
//! channel, then the next channel; native-endian) and sends each complete block over a bounded
//! channel. The thread compresses the blocks on exr's thread pool (sequentially if the pool
//! cannot be created) and writes them in order, then the offset table.
//!
//! Memory is bounded and does not depend on the height: a block is [`BLOCK_ROWS`] rows, that is
//! `16 × width × 16` bytes for RGBA F32 (4 MiB at 16384 px wide). At most [`QUEUED_BLOCKS`]
//! blocks (one band) wait in the channel, one is being filled, and exr compresses at most
//! `threads + 2` at a time.
//!
//! In debug builds, exr decompresses every block again to check it (a `debug_assert`), so EXR
//! export is much slower there than in release builds.

use std::fs::File;
use std::io::BufWriter;
use std::mem;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread::{self, JoinHandle};

use ::exr::block::UncompressedBlock;
use ::exr::block::writer::ChunksWriter;
use ::exr::compression::Compression;
use ::exr::math::Vec2;
use ::exr::meta::BlockDescription;
use ::exr::meta::attribute::{
    ChannelDescription, Chromaticities, LineOrder, SampleType as ExrSampleType, Text,
};
use ::exr::meta::header::{Header, LayerAttributes};
use slopshop_core::Size;
use slopshop_core::color::{AlphaMode, ChannelLayout, PixelFormat, RgbPrimaries, SampleType};

use super::{BAND_ROWS, ExportError, ExportNotice};

/// Lossless, fast, and good on smooth or rendered content.
const COMPRESSION: Compression = Compression::ZIP16;

/// Rows of one block (fixed by the compression).
const BLOCK_ROWS: u32 = COMPRESSION.scan_lines_per_block() as u32;

/// Capacity of the channel to the encoding thread: one band, so that the next band can be
/// converted while this one is compressed.
const QUEUED_BLOCKS: usize = (BAND_ROWS / BLOCK_ROWS) as usize;

/// EXR windows must stay below `i32::MAX / 2` (as in the reference C++ library).
const MAX_SIDE: u32 = (i32::MAX / 2 - 1) as u32;

/// File channels in the order EXR requires (sorted by name), with the index of their sample in
/// our interleaved RGBA pixels. Without alpha: all but the first.
const CHANNELS: [(&str, usize); 4] = [("A", 3), ("B", 2), ("G", 1), ("R", 0)];

pub(super) struct ExrWriter {
    /// Complete blocks, to the encoding thread. `None` once closed.
    blocks: Option<SyncSender<Vec<u8>>>,
    /// The encoding thread. `None` once joined.
    encoder: Option<JoinHandle<Result<(), ExportError>>>,
    /// File channels, with their sample index in the input pixels.
    channels: &'static [(&'static str, usize)],
    /// Bytes of one sample: 2 (F16) or 4 (F32).
    sample_bytes: usize,
    /// Bytes of one row: interleaved in the input, planar in the file (same size).
    row_bytes: usize,
    size: Size,
    next_row: u32,
    /// The block being filled: rows from the last multiple of [`BLOCK_ROWS`] to `next_row`.
    block: Vec<u8>,
}

impl ExrWriter {
    pub(super) fn new(file: File, size: Size, target: PixelFormat) -> Result<Self, ExportError> {
        let channels = match (target.layout, target.alpha) {
            (ChannelLayout::Rgb, _) => &CHANNELS[1..],
            (ChannelLayout::Rgba, AlphaMode::Premultiplied) => &CHANNELS[..],
            _ => return Err(invalid_target(&target)),
        };
        let (sample, sample_bytes) = match target.sample {
            SampleType::F16 => (ExrSampleType::F16, 2),
            SampleType::F32 => (ExrSampleType::F32, 4),
            SampleType::U8 | SampleType::U16 => return Err(invalid_target(&target)),
        };
        let space = target.color_space;
        if !space.is_linear() || !space.primaries.is_valid() {
            return Err(ExportError::UnsupportedSpace(space));
        }
        if size.is_empty() {
            return Err(ExportError::InvalidSpec(format!(
                "empty image ({}×{})",
                size.width, size.height
            )));
        }
        let too_large = || ExportError::TooLarge {
            width: size.width,
            height: size.height,
        };
        if size.width > MAX_SIDE || size.height > MAX_SIDE {
            return Err(too_large());
        }
        let row_bytes = (size.width as usize)
            .checked_mul(channels.len() * sample_bytes)
            .filter(|bytes| bytes.checked_mul(BLOCK_ROWS as usize).is_some())
            .ok_or_else(too_large)?;

        let header = header(size, channels, sample, &space.primaries);
        let (blocks, received) = mpsc::sync_channel(QUEUED_BLOCKS);
        let encoder = thread::Builder::new()
            .name("exr-writer".to_owned())
            .spawn(move || encode(file, header, received))?;
        Ok(Self {
            blocks: Some(blocks),
            encoder: Some(encoder),
            channels,
            sample_bytes,
            row_bytes,
            size,
            next_row: 0,
            block: Vec::new(),
        })
    }

    pub(super) fn write_rows(&mut self, first_row: u32, rows: &[u8]) -> Result<(), ExportError> {
        let count = rows.len() / self.row_bytes;
        if first_row != self.next_row
            || !rows.len().is_multiple_of(self.row_bytes)
            || count > (self.size.height - self.next_row) as usize
        {
            return Err(ExportError::Encode(format!(
                "rows out of order: {} bytes at row {first_row}, expected row {} of {}",
                rows.len(),
                self.next_row,
                self.size.height
            )));
        }
        for row in rows.chunks_exact(self.row_bytes) {
            if self.block.is_empty() {
                let block_rows = BLOCK_ROWS.min(self.size.height - self.next_row) as usize;
                // Fits: checked in `new`.
                self.block
                    .try_reserve_exact(block_rows * self.row_bytes)
                    .map_err(|_| ExportError::TooLarge {
                        width: self.size.width,
                        height: self.size.height,
                    })?;
            }
            let start = self.block.len();
            self.block.resize(start + self.row_bytes, 0);
            let line = &mut self.block[start..];
            match self.sample_bytes {
                2 => to_planar::<2>(row, self.channels, line),
                _ => to_planar::<4>(row, self.channels, line),
            }
            self.next_row += 1;
            if self.next_row.is_multiple_of(BLOCK_ROWS) || self.next_row == self.size.height {
                self.send_block()?;
            }
        }
        Ok(())
    }

    pub(super) fn finish(mut self) -> Result<Vec<ExportNotice>, ExportError> {
        if self.next_row != self.size.height {
            return Err(ExportError::Encode(format!(
                "{} rows written out of {}",
                self.next_row, self.size.height
            )));
        }
        // Closing the channel lets the thread write the offset table and flush the file.
        self.join()?;
        Ok(Vec::new())
    }

    fn send_block(&mut self) -> Result<(), ExportError> {
        let block = mem::take(&mut self.block);
        if let Some(blocks) = &self.blocks
            && blocks.send(block).is_ok()
        {
            return Ok(());
        }
        // The thread only hangs up before the last block when it failed.
        Err(match self.join() {
            Err(e) => e,
            Ok(()) => ExportError::Encode("the EXR encoder stopped early".to_owned()),
        })
    }

    /// Close the channel and wait for the encoding thread; its result.
    fn join(&mut self) -> Result<(), ExportError> {
        self.blocks = None;
        match self.encoder.take() {
            Some(encoder) => encoder.join().unwrap_or_else(|_| {
                Err(ExportError::Encode("the EXR encoder panicked".to_owned()))
            }),
            None => Ok(()),
        }
    }
}

impl Drop for ExrWriter {
    /// Without `finish` (error, cancellation): the thread sees the channel close before the last
    /// block, gives up and closes the file, so that the export can delete it.
    fn drop(&mut self) {
        // The export is already failing: its own error is the one to report.
        let _ = self.join();
    }
}

/// The header of a single-part scanline file, with the chromaticities of the space.
fn header(
    size: Size,
    channels: &[(&str, usize)],
    sample: ExrSampleType,
    primaries: &RgbPrimaries,
) -> Header {
    let descriptions: Vec<ChannelDescription> = channels
        .iter()
        .map(|&(name, _)| ChannelDescription::named(name, sample))
        .collect();
    let mut header = Header::new(
        Text::default(),
        (size.width as usize, size.height as usize),
        descriptions.into(),
    )
    .with_encoding(
        COMPRESSION,
        BlockDescription::ScanLines,
        LineOrder::Increasing,
    )
    // No layer name: a plain single-part file.
    .with_attributes(LayerAttributes::default());
    let xy = |[x, y]: [f64; 2]| Vec2(x as f32, y as f32);
    header.shared_attributes.chromaticities = Some(Chromaticities {
        red: xy(primaries.red),
        green: xy(primaries.green),
        blue: xy(primaries.blue),
        white: xy(primaries.white),
    });
    header
}

/// The encoding thread: writes the header, every block received (in order, as exr expects them
/// from its own block enumeration), then the offset table.
fn encode(file: File, header: Header, blocks: Receiver<Vec<u8>>) -> Result<(), ExportError> {
    let headers = vec![header].into();
    ::exr::block::write(BufWriter::new(file), headers, true, |meta, chunks| {
        let mut received = ::exr::block::enumerate_ordered_header_block_indices(&meta.headers).map(
            |(chunk, index)| {
                // Closed before the last block: the export failed or was cancelled.
                let data = blocks.recv().map_err(|_| ::exr::error::Error::Aborted)?;
                Ok::<_, ::exr::error::Error>((chunk, UncompressedBlock { index, data }))
            },
        );
        match chunks.parallel_blocks_compressor(&meta) {
            Some(mut compressor) => received.try_for_each(|block| {
                let (chunk, block) = block?;
                compressor.add_block_to_compression_queue(chunk, block)
            }),
            None => {
                let mut compressor = chunks.sequential_blocks_compressor(&meta);
                received.try_for_each(|block| {
                    let (chunk, block) = block?;
                    compressor.compress_block(chunk, block)
                })
            }
        }
    })
    .map_err(|e| match e {
        ::exr::error::Error::Io(e) => ExportError::Io(e),
        other => ExportError::Encode(other.to_string()),
    })
}

/// One row of interleaved little-endian samples of `N` bytes, as one EXR line: every sample of
/// the first file channel, then of the second… in native byte order.
fn to_planar<const N: usize>(row: &[u8], channels: &[(&str, usize)], line: &mut [u8]) {
    let samples = row.as_chunks::<N>().0;
    let width = samples.len() / channels.len();
    let line = line.as_chunks_mut::<N>().0;
    for (&(_, source), out) in channels.iter().zip(line.chunks_exact_mut(width)) {
        let column = samples.iter().skip(source).step_by(channels.len());
        for (out, sample) in out.iter_mut().zip(column) {
            *out = *sample;
            if cfg!(target_endian = "big") {
                out.reverse();
            }
        }
    }
}

fn invalid_target(target: &PixelFormat) -> ExportError {
    ExportError::InvalidSpec(format!("OpenEXR export cannot store {target:?}"))
}

#[cfg(test)]
mod tests;
