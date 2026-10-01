//! JPEG 2000 writer: openjp2 (the Rust port of OpenJPEG), through its safe API, writes a JP2
//! file: 8 or 16 bits per sample, gray or RGB, with straight alpha (a `cdef` box), sRGB (the
//! `colr` box enumerates sRGB, or greyscale: the sRGB curve). Lossless (the reversible 5/3
//! wavelet) or lossy (the irreversible 9/7 wavelet at a compression ratio from the quality).
//! No ICC profile: openjp2 takes it only through raw pointers.
//!
//! The image is held whole, one 32-bit integer per sample (openjp2's image planes, filled as
//! the rows come): at most [`MAX_SIDE`] pixels per side and [`MAX_SAMPLES`] samples. It is
//! encoded in tiles of [`TILE`] pixels, so the encoder works on one tile at a time.
//!
//! openjp2's only safe output stream is a file it creates by path: the file is encoded to a
//! temporary file of the system's temporary directory, then copied into the export's file
//! (the compressed bytes only) and deleted, on error too.

use std::fs::File;
use std::io::{Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use openjp2::{
    CODEC_FORMAT, COLOR_SPACE, Codec, Stream, opj_cparameters_t, opj_image, opj_image_comptparm,
};
use slopshop_core::color::{AlphaMode, ColorSpace, PixelFormat, SampleType};
use slopshop_core::{CancelToken, Size};

use super::bmp::rows_out_of_order;
use super::{ExportError, ExportNotice, Jpeg2000Compression};

/// Largest side.
pub const MAX_SIDE: u32 = 65535;

/// At most this many samples (pixels × channels): 1 GiB of 32-bit samples held while the
/// image is encoded (e.g. 16384 × 16384 pixels in RGB, 8192 × 8192 in RGBA... at most).
pub(super) const MAX_SAMPLES: u64 = 1 << 28;

/// Tile side: the encoder copies and transforms one tile at a time.
const TILE: u32 = 1024;

/// Buffer of openjp2's file stream.
const STREAM_BUFFER: usize = 1 << 20;

/// The compression ratio of a lossy quality (1 to 100): 2^((100 − quality) / 12.5), from
/// about 1:1 at 100 to 16:1 at 50 and about 240:1 at 1.
pub(super) fn ratio(quality: u8) -> f32 {
    2f32.powf((100.0 - f32::from(quality.clamp(1, 100))) / 12.5)
}

pub(super) struct Jpeg2000Writer {
    file: File,
    image: Box<opj_image>,
    size: Size,
    channels: usize,
    /// Bytes per sample.
    bytes: usize,
    compression: Jpeg2000Compression,
    next_row: u32,
    cancel: CancelToken,
}

impl Jpeg2000Writer {
    pub(super) fn new(
        file: File,
        size: Size,
        target: PixelFormat,
        compression: Jpeg2000Compression,
        cancel: CancelToken,
    ) -> Result<Self, ExportError> {
        let invalid = || ExportError::InvalidSpec(format!("JPEG 2000 cannot store {target:?}"));
        let (bytes, precision) = match target.sample {
            SampleType::U8 => (1, 8),
            SampleType::U16 => (2, 16),
            SampleType::F16 | SampleType::F32 => return Err(invalid()),
        };
        let alpha = target.layout.has_alpha();
        if alpha && target.alpha != AlphaMode::Straight {
            return Err(invalid());
        }
        if target.color_space != ColorSpace::SRGB {
            return Err(ExportError::UnsupportedSpace(target.color_space));
        }
        if let Jpeg2000Compression::Lossy { quality } = compression
            && !(1..=100).contains(&quality)
        {
            return Err(ExportError::InvalidSpec(format!(
                "JPEG 2000 quality {quality} is not between 1 and 100"
            )));
        }
        let channels = target.layout.channels() as usize;
        let too_large = || ExportError::TooLarge {
            width: size.width,
            height: size.height,
        };
        if size.width > MAX_SIDE
            || size.height > MAX_SIDE
            || size.pixel_count() * channels as u64 > MAX_SAMPLES
        {
            return Err(too_large());
        }
        let space = if target.layout.is_gray() {
            COLOR_SPACE::OPJ_CLRSPC_GRAY
        } else {
            COLOR_SPACE::OPJ_CLRSPC_SRGB
        };
        let component = opj_image_comptparm {
            dx: 1,
            dy: 1,
            w: size.width,
            h: size.height,
            x0: 0,
            y0: 0,
            prec: precision,
            bpp: precision,
            sgnd: 0,
        };
        let mut image =
            opj_image::create(&vec![component; channels], space).ok_or_else(too_large)?;
        image.x0 = 0;
        image.y0 = 0;
        image.x1 = size.width;
        image.y1 = size.height;
        if alpha && let Some(last) = image.comps_mut().and_then(|c| c.last_mut()) {
            last.alpha = 1;
        }
        Ok(Self {
            file,
            image,
            size,
            channels,
            bytes,
            compression,
            next_row: 0,
            cancel,
        })
    }

    pub(super) fn write_rows(&mut self, first_row: u32, rows: &[u8]) -> Result<(), ExportError> {
        let width = self.size.width as usize;
        let row_bytes = width * self.channels * self.bytes;
        if first_row != self.next_row || !rows.len().is_multiple_of(row_bytes) {
            return Err(rows_out_of_order(first_row, self.next_row, rows.len()));
        }
        let start = first_row as usize * width;
        let count = rows.len() / self.bytes;
        let (channels, bytes) = (self.channels, self.bytes);
        let planes = self
            .image
            .comps_data_mut_iter()
            .ok_or_else(|| ExportError::Encode("JPEG 2000: no image planes".to_owned()))?;
        // Interleaved samples to planes, one per component.
        for (channel, plane) in planes.enumerate() {
            let plane = &mut plane[start..start + count / channels];
            let samples = (channel..count).step_by(channels);
            for (out, i) in plane.iter_mut().zip(samples) {
                *out = if bytes == 1 {
                    i32::from(rows[i])
                } else {
                    i32::from(u16::from_le_bytes([rows[2 * i], rows[2 * i + 1]]))
                };
            }
        }
        self.next_row += (rows.len() / row_bytes) as u32;
        Ok(())
    }

    pub(super) fn finish(mut self) -> Result<Vec<ExportNotice>, ExportError> {
        let Size { width, height } = self.size;
        if self.next_row != height {
            return Err(ExportError::Encode(format!(
                "{} rows written out of {height}",
                self.next_row
            )));
        }
        if self.cancel.is_cancelled() {
            return Err(ExportError::Cancelled);
        }
        // One quality layer; lossless: no rate (0), lossy: the ratio.
        let (irreversible, rate) = match self.compression {
            Jpeg2000Compression::Lossless => (0, 0.0),
            Jpeg2000Compression::Lossy { quality } => (1, ratio(quality)),
        };
        let mut tcp_rates = [0.0; 100];
        tcp_rates[0] = rate;
        let tiled = width > TILE || height > TILE;
        // As many resolutions as the smallest tile side allows (small images fail otherwise),
        // at most 6 (5 decompositions).
        let side = width.min(height).min(TILE);
        let mut parameters = opj_cparameters_t {
            tcp_numlayers: 1,
            cp_disto_alloc: 1,
            irreversible,
            tcp_rates,
            // The color transform, for RGB (alpha aside).
            tcp_mct: std::ffi::c_char::from(self.channels >= 3),
            tile_size_on: i32::from(tiled),
            cp_tdx: if tiled { TILE as i32 } else { 0 },
            cp_tdy: if tiled { TILE as i32 } else { 0 },
            numresolution: (side.ilog2() as i32 + 1).clamp(1, 6),
            ..Default::default()
        };

        let temp = TempPath::new();
        let encoded = encode(&temp.0, &mut parameters, &mut self.image);
        // The samples are no longer needed: free them before the copy.
        drop(self.image);
        encoded?;
        let mut encoded = File::open(&temp.0)?;
        self.file.seek(SeekFrom::Start(0))?;
        std::io::copy(&mut encoded, &mut self.file)?;
        Ok(Vec::new())
    }
}

/// Encode `image` as a JP2 file at `path`.
fn encode(
    path: &std::path::Path,
    parameters: &mut opj_cparameters_t,
    image: &mut opj_image,
) -> Result<(), ExportError> {
    let failed =
        |step: &str| ExportError::Encode(format!("JPEG 2000: the encoder failed ({step})"));
    let mut codec =
        Codec::new_encoder(CODEC_FORMAT::OPJ_CODEC_JP2).ok_or_else(|| failed("creation"))?;
    if codec.setup_encoder(parameters, image) == 0 {
        return Err(failed("setup"));
    }
    let mut stream = Stream::new_file(path, STREAM_BUFFER, false)?;
    if codec.start_compress(image, &mut stream) == 0 {
        return Err(failed("start"));
    }
    if codec.encode(&mut stream) == 0 {
        return Err(failed("encoding"));
    }
    if codec.end_compress(&mut stream) == 0 {
        return Err(failed("end"));
    }
    // Dropping the stream would flush it silently.
    stream.flush()?;
    Ok(())
}

/// A file name in the system's temporary directory, deleted when dropped.
struct TempPath(PathBuf);

impl TempPath {
    fn new() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let name = format!("slopshop-jp2-{}-{n}.jp2", std::process::id());
        Self(std::env::temp_dir().join(name))
    }
}

impl Drop for TempPath {
    fn drop(&mut self) {
        // It may not exist (the encoder failed before creating it).
        std::fs::remove_file(&self.0).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quality_maps_to_a_compression_ratio() {
        assert_eq!(ratio(100), 1.0);
        assert_eq!(ratio(50), 16.0);
        assert!((ratio(1) - 240.0).abs() < 5.0, "{}", ratio(1));
        assert!(ratio(90) < ratio(80));
    }
}
