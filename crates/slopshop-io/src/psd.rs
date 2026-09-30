//! Photoshop documents (PSD, and PSB for large documents), stage 1 of the formats plan
//! (docs/formats.md): the flattened composite image stored in the file, at its native depth.
//!
//! Read from Adobe's "Photoshop File Formats Specification": a header, the color mode data
//! (the palette of indexed images), image resources (8BIM blocks: the ICC profile, 1039, and
//! the version info, 1057, whose "has real merged data" flag tells whether the composite is
//! real), the layer and mask information (only the layer count is read: negative means the
//! first extra channel of the composite is its transparency), then the composite: planar
//! channels, raw, PackBits RLE, or zlib with or without prediction. PSB differs by 64-bit
//! section lengths and 32-bit RLE row counts. Big-endian throughout.
//!
//! Conventions this reader follows:
//! - 32-bit documents hold linear light: their space is the profile's primaries with a linear
//!   transfer.
//! - A composite with transparency is matted against white by Photoshop: the white is removed
//!   (`c = (c' - (1 - a)) / a`, on encoded values) so that colors come back straight.
//! - Bitmap images store 1 for black; they open as 8-bit gray.
//!
//! Supported: bitmap, grayscale, duotone (as its grayscale data, with a warning), indexed, RGB;
//! 1/8/16/32-bit. CMYK, Lab and multichannel are refused until the engine has them (ADR 0006).
//! The file is untrusted: every size is checked before it is used.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use flate2::read::ZlibDecoder;
use slopshop_core::color::{AlphaMode, ChannelLayout, ColorSpace, SampleType, TransferFunction};
use slopshop_core::geom::Size;

use crate::orient::Orientation;
use crate::{
    Decoded, ImportError, ImportWarning, Opened, check_budget, finish, icc, resolve_color_space,
};

mod layers;

const SIGNATURE: &[u8; 4] = b"8BPS";
const RESOURCE_SIGNATURE: &[u8; 4] = b"8BIM";
const RESOURCE_ICC: u16 = 1039;
const RESOURCE_TRANSPARENCY_INDEX: u16 = 1047;
const RESOURCE_VERSION_INFO: u16 = 1057;

/// Photoshop's limits: 30,000 px per side for PSD, 300,000 for PSB; up to 56 channels.
const MAX_SIDE_PSD: u32 = 30_000;
const MAX_SIDE_PSB: u32 = 300_000;
const MAX_CHANNELS: u16 = 56;
/// Image resources and color mode data are small (profiles, palettes, thumbnails).
const MAX_SECTION_BYTES: u64 = 256 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Bitmap,
    Grayscale,
    Indexed,
    Rgb,
    Duotone,
}

pub(crate) fn is_psd(head: &[u8]) -> bool {
    head.starts_with(SIGNATURE)
}

fn corrupt(what: &str) -> ImportError {
    ImportError::Decode(format!("Photoshop document: {what}"))
}

/// Big-endian readers over the file.
struct Input<R> {
    r: R,
    big: bool,
}

impl<R: Read + Seek> Input<R> {
    fn bytes<const N: usize>(&mut self) -> Result<[u8; N], ImportError> {
        let mut b = [0u8; N];
        self.r.read_exact(&mut b)?;
        Ok(b)
    }

    fn u16(&mut self) -> Result<u16, ImportError> {
        Ok(u16::from_be_bytes(self.bytes()?))
    }

    fn u32(&mut self) -> Result<u32, ImportError> {
        Ok(u32::from_be_bytes(self.bytes()?))
    }

    /// A section length: 32-bit in PSD, 64-bit in PSB for the large sections.
    fn length(&mut self) -> Result<u64, ImportError> {
        if self.big {
            Ok(u64::from_be_bytes(self.bytes()?))
        } else {
            Ok(u64::from(self.u32()?))
        }
    }

    fn vec(&mut self, len: u64) -> Result<Vec<u8>, ImportError> {
        if len > MAX_SECTION_BYTES {
            return Err(corrupt("section too large"));
        }
        let mut v = vec![0u8; len as usize];
        self.r.read_exact(&mut v)?;
        Ok(v)
    }

    fn skip(&mut self, len: u64) -> Result<(), ImportError> {
        let len = i64::try_from(len).map_err(|_| corrupt("section too large"))?;
        self.r.seek(SeekFrom::Current(len))?;
        Ok(())
    }
}

/// What the header, the color mode data and the image resources say.
struct Header {
    channels: u16,
    width: u32,
    height: u32,
    depth: u16,
    mode: Mode,
    /// The palette of indexed images (256 reds, then greens, then blues).
    color_data: Vec<u8>,
    icc_profile: Option<Vec<u8>>,
    transparent_index: Option<u16>,
    /// The composite is real (else Photoshop wrote a blank one: "Maximize Compatibility" off).
    real_merged_data: bool,
}

impl Header {
    /// Channels holding colors: 3 for RGB, 1 for the others.
    fn color_channels(&self) -> u16 {
        if self.mode == Mode::Rgb { 3 } else { 1 }
    }

    /// The largest side Photoshop allows, which layers cannot exceed either.
    fn max_side(big: bool) -> u32 {
        if big { MAX_SIDE_PSB } else { MAX_SIDE_PSD }
    }

    /// The color space of the samples: 32-bit documents hold linear light in the profile's
    /// primaries; others are what their profile says (sRGB without one).
    fn color_space(&self, warnings: &mut Vec<ImportWarning>) -> ColorSpace {
        let space = (self.depth == 32).then(|| {
            let primaries = self
                .icc_profile
                .as_deref()
                .and_then(|bytes| icc::parse(bytes).ok())
                .map_or(ColorSpace::LINEAR_SRGB.primaries, |c| c.space.primaries);
            ColorSpace {
                primaries,
                transfer: TransferFunction::Linear,
            }
        });
        resolve_color_space(
            self.depth == 32,
            space,
            self.icc_profile.as_deref(),
            warnings,
        )
    }
}

/// Open a Photoshop document: its layers as a document when it has layers the engine can hold,
/// else its flattened image (with a warning when layers were there but could not be read).
pub(crate) fn open(path: &Path) -> Result<Opened, ImportError> {
    let mut input = Input {
        r: BufReader::new(File::open(path)?),
        big: false,
    };
    let header = read_header(&mut input)?;
    let layered = matches!(header.mode, Mode::Grayscale | Mode::Rgb | Mode::Duotone);
    if layered && let Ok(Some(layers)) = layers::read(&mut input, &header) {
        return Ok(Opened::Layers(layers));
    }
    // No layers, or unreadable ones: the composite, which says whether layers were there.
    drop(input);
    finish(decode(path)?).map(Opened::Image)
}

fn read_header<R: Read + Seek>(input: &mut Input<R>) -> Result<Header, ImportError> {
    if &input.bytes::<4>()? != SIGNATURE {
        return Err(ImportError::Unrecognized);
    }
    let version = input.u16()?;
    input.big = match version {
        1 => false,
        2 => true,
        _ => return Err(corrupt("unknown version")),
    };
    input.bytes::<6>()?;
    let channels = input.u16()?;
    let height = input.u32()?;
    let width = input.u32()?;
    let depth = input.u16()?;
    let mode = match input.u16()? {
        0 => Mode::Bitmap,
        1 => Mode::Grayscale,
        2 => Mode::Indexed,
        3 => Mode::Rgb,
        8 => Mode::Duotone,
        4 => return Err(ImportError::NotYetSupported("CMYK Photoshop document")),
        7 => {
            return Err(ImportError::NotYetSupported(
                "multichannel Photoshop document",
            ));
        }
        9 => return Err(ImportError::NotYetSupported("Lab Photoshop document")),
        _ => return Err(corrupt("unknown color mode")),
    };
    if channels == 0 || channels > MAX_CHANNELS || width == 0 || height == 0 {
        return Err(corrupt("invalid header"));
    }
    let max_side = Header::max_side(input.big);
    if width > max_side || height > max_side {
        return Err(ImportError::TooLarge { width, height });
    }
    let valid_depth = match mode {
        Mode::Bitmap => depth == 1,
        Mode::Indexed | Mode::Duotone => depth == 8,
        Mode::Grayscale | Mode::Rgb => matches!(depth, 8 | 16 | 32),
    };
    if !valid_depth {
        return Err(corrupt("depth not valid for its color mode"));
    }

    // Color mode data: the palette of indexed images.
    let color_data = {
        let len = u64::from(input.u32()?);
        input.vec(len)?
    };
    if mode == Mode::Indexed && color_data.len() < 768 {
        return Err(corrupt("missing palette"));
    }

    // Image resources.
    let resources = {
        let len = u64::from(input.u32()?);
        input.vec(len)?
    };
    let mut header = Header {
        channels,
        width,
        height,
        depth,
        mode,
        color_data,
        icc_profile: None,
        transparent_index: None,
        real_merged_data: true,
    };
    if channels < header.color_channels() {
        return Err(corrupt("too few channels"));
    }
    for (id, data) in resource_blocks(&resources)? {
        match id {
            RESOURCE_ICC => header.icc_profile = Some(data.to_vec()),
            RESOURCE_TRANSPARENCY_INDEX if data.len() >= 2 => {
                header.transparent_index = Some(u16::from_be_bytes([data[0], data[1]]));
            }
            // Version info: a 4-byte version, then "has real merged data".
            RESOURCE_VERSION_INFO if data.len() >= 5 => header.real_merged_data = data[4] != 0,
            _ => {}
        }
    }
    Ok(header)
}

/// The flattened composite image.
pub(crate) fn decode(path: &Path) -> Result<Decoded, ImportError> {
    let mut input = Input {
        r: BufReader::new(File::open(path)?),
        big: false,
    };
    let header = read_header(&mut input)?;
    if !header.real_merged_data {
        return Err(ImportError::PsdWithoutComposite);
    }
    let Header {
        channels,
        width,
        height,
        depth,
        mode,
        ..
    } = header;
    let color_channels = header.color_channels();

    // Layer and mask information: only the layer count, then skip to the composite.
    let layer_count = layers::layer_count(&mut input)?;
    // Bitmap documents have no transparency (their extra channels would be 1-bit selections).
    let transparency = layer_count < 0 && channels > color_channels && depth >= 8;
    let mut warnings = Vec::new();
    if layer_count != 0 {
        warnings.push(ImportWarning::LayersFlattened);
    }
    if mode == Mode::Duotone {
        warnings.push(ImportWarning::ColorInfoUnsupported);
    }

    // What the composite becomes.
    let (layout, sample) = match (mode, depth, transparency) {
        (Mode::Rgb | Mode::Indexed, 8, false) => (ChannelLayout::Rgb, SampleType::U8),
        (Mode::Rgb | Mode::Indexed, 8, true) => (ChannelLayout::Rgba, SampleType::U8),
        (Mode::Rgb, 16, t) => (rgb(t), SampleType::U16),
        (Mode::Rgb, 32, t) => (rgb(t), SampleType::F32),
        (_, 16, t) => (gray(t), SampleType::U16),
        (_, 32, t) => (gray(t), SampleType::F32),
        (_, _, t) => (gray(t), SampleType::U8),
    };
    let indexed_alpha = mode == Mode::Indexed && header.transparent_index.is_some();
    let layout = if indexed_alpha {
        ChannelLayout::Rgba
    } else {
        layout
    };
    let out_channels = layout.channels() as usize;
    let out_sample = sample.bytes() as usize;
    check_budget(
        width,
        height,
        layout,
        sample,
        (out_channels * out_sample) as u32,
    )?;

    // The composite's channels read: the colors, then its transparency when it has one.
    let read_channels = usize::from(color_channels) + usize::from(transparency);
    let samples = Samples {
        width: width as usize,
        height: height as usize,
        depth,
    };
    let planes = read_composite(&mut input, channels, read_channels, &samples)?;

    let pixels = assemble(
        &planes,
        &samples,
        mode,
        &header.color_data,
        header.transparent_index,
        transparency,
        out_channels,
    );

    let space = header.color_space(&mut warnings);
    Ok(Decoded {
        size: Size::new(width, height),
        layout,
        sample,
        alpha: AlphaMode::Straight,
        icc: None,
        space: Some(space),
        orientation: Orientation::Normal,
        pixels,
        warnings,
    })
}

fn rgb(alpha: bool) -> ChannelLayout {
    if alpha {
        ChannelLayout::Rgba
    } else {
        ChannelLayout::Rgb
    }
}

fn gray(alpha: bool) -> ChannelLayout {
    if alpha {
        ChannelLayout::GrayAlpha
    } else {
        ChannelLayout::Gray
    }
}

/// The 8BIM blocks of the image resources section: id and data.
fn resource_blocks(section: &[u8]) -> Result<Vec<(u16, &[u8])>, ImportError> {
    let mut blocks = Vec::new();
    let mut at = 0usize;
    while at + 6 <= section.len() {
        if &section[at..at + 4] != RESOURCE_SIGNATURE {
            // Other signatures exist (e.g. MeSa); stop at anything unexpected.
            break;
        }
        let id = u16::from_be_bytes([section[at + 4], section[at + 5]]);
        at += 6;
        // Pascal string name, padded to an even length (count byte included).
        let name_len = *section
            .get(at)
            .ok_or_else(|| corrupt("truncated resource"))? as usize;
        at += (name_len + 1).next_multiple_of(2);
        let size_bytes = section
            .get(at..at + 4)
            .ok_or_else(|| corrupt("truncated resource"))?;
        let size = u32::from_be_bytes([size_bytes[0], size_bytes[1], size_bytes[2], size_bytes[3]])
            as usize;
        at += 4;
        let data = section
            .get(at..at.saturating_add(size))
            .ok_or_else(|| corrupt("truncated resource"))?;
        blocks.push((id, data));
        at = at.saturating_add(size.next_multiple_of(2));
    }
    Ok(blocks)
}

/// Dimensions of the composite's planes.
struct Samples {
    width: usize,
    height: usize,
    /// 1, 8, 16 or 32 bits.
    depth: u16,
}

impl Samples {
    /// Bytes of one row of one channel, as stored (1-bit rows are padded to a byte).
    fn row_bytes(&self) -> usize {
        match self.depth {
            1 => self.width.div_ceil(8),
            d => self.width * usize::from(d / 8),
        }
    }
}

/// The first `wanted` channels of the composite, each as `height` stored rows (big-endian).
fn read_composite<R: Read + Seek>(
    input: &mut Input<R>,
    channels: u16,
    wanted: usize,
    samples: &Samples,
) -> Result<Vec<Vec<u8>>, ImportError> {
    let row_bytes = samples.row_bytes();
    let plane_bytes = row_bytes
        .checked_mul(samples.height)
        .ok_or_else(|| corrupt("image too large"))?;
    let compression = input.u16()?;
    let mut planes = Vec::with_capacity(wanted);
    match compression {
        0 => {
            for _ in 0..wanted {
                let mut plane = vec![0u8; plane_bytes];
                input.r.read_exact(&mut plane)?;
                planes.push(plane);
            }
        }
        1 => {
            // Compressed length of every row of every channel, then the rows.
            let rows = usize::from(channels) * samples.height;
            let mut lengths = Vec::with_capacity(rows);
            for _ in 0..rows {
                lengths.push(if input.big {
                    input.u32()? as usize
                } else {
                    usize::from(input.u16()?)
                });
            }
            let mut packed = Vec::new();
            for channel in 0..wanted {
                let mut plane = vec![0u8; plane_bytes];
                for (y, row) in plane.chunks_exact_mut(row_bytes).enumerate() {
                    let len = lengths[channel * samples.height + y];
                    // A PackBits row never needs more than twice its unpacked size.
                    if len > 2 * row_bytes + 2 {
                        return Err(corrupt("RLE row too long"));
                    }
                    packed.resize(len, 0);
                    input.r.read_exact(&mut packed)?;
                    unpack_bits(&packed, row)?;
                }
                planes.push(plane);
            }
        }
        2 | 3 => {
            // One zlib stream over every channel.
            let mut z = ZlibDecoder::new(&mut input.r);
            for _ in 0..wanted {
                let mut plane = vec![0u8; plane_bytes];
                z.read_exact(&mut plane)?;
                if compression == 3 {
                    for row in plane.chunks_exact_mut(row_bytes) {
                        unpredict(row, samples);
                    }
                }
                planes.push(plane);
            }
            // Read the stream to its end (the remaining channels, then the checksum): a
            // truncated or damaged file is an error even when the wanted channels decoded.
            std::io::copy(&mut z, &mut std::io::sink())?;
        }
        _ => return Err(corrupt("unknown compression")),
    }
    Ok(planes)
}

/// PackBits: a count byte `n`; 0..=127 copies `n + 1` literal bytes, -127..=-1 repeats the next
/// byte `1 - n` times, -128 is skipped. `row` must come out exactly full.
fn unpack_bits(mut packed: &[u8], row: &mut [u8]) -> Result<(), ImportError> {
    let mut at = 0;
    while let Some((&n, rest)) = packed.split_first() {
        packed = rest;
        let n = n as i8;
        if n >= 0 {
            let count = n as usize + 1;
            let (literal, rest) = packed
                .split_at_checked(count)
                .ok_or_else(|| corrupt("truncated RLE row"))?;
            row.get_mut(at..at + count)
                .ok_or_else(|| corrupt("RLE row overflow"))?
                .copy_from_slice(literal);
            at += count;
            packed = rest;
        } else if n != -128 {
            let count = 1 - n as isize;
            let count = count as usize;
            let (&value, rest) = packed
                .split_first()
                .ok_or_else(|| corrupt("truncated RLE row"))?;
            row.get_mut(at..at + count)
                .ok_or_else(|| corrupt("RLE row overflow"))?
                .fill(value);
            at += count;
            packed = rest;
        }
    }
    if at != row.len() {
        return Err(corrupt("short RLE row"));
    }
    Ok(())
}

/// Undo zlib prediction on one row: running sums of 8-bit samples, of 16-bit big-endian samples,
/// or, for 32-bit floats, of the row's bytes stored as four planes (all first bytes, then all
/// second bytes…), which are then interleaved back.
fn unpredict(row: &mut [u8], samples: &Samples) {
    match samples.depth {
        16 => {
            let mut previous = 0u16;
            for pair in row.as_chunks_mut::<2>().0 {
                let value = u16::from_be_bytes([pair[0], pair[1]]).wrapping_add(previous);
                pair.copy_from_slice(&value.to_be_bytes());
                previous = value;
            }
        }
        32 => {
            for i in 1..row.len() {
                row[i] = row[i].wrapping_add(row[i - 1]);
            }
            let width = samples.width;
            let planar = row.to_vec();
            for x in 0..width {
                for byte in 0..4 {
                    row[x * 4 + byte] = planar[byte * width + x];
                }
            }
        }
        _ => {
            for i in 1..row.len() {
                row[i] = row[i].wrapping_add(row[i - 1]);
            }
        }
    }
}

/// Interleave the planes into native-endian pixels of the output layout: palette lookup,
/// 1-bit expansion, white matte removal.
fn assemble(
    planes: &[Vec<u8>],
    samples: &Samples,
    mode: Mode,
    palette: &[u8],
    transparent_index: Option<u16>,
    transparency: bool,
    out_channels: usize,
) -> Vec<u8> {
    let (width, height) = (samples.width, samples.height);
    let row_bytes = samples.row_bytes();
    let out_sample = match samples.depth {
        16 => 2,
        32 => 4,
        _ => 1,
    };
    let mut out = vec![0u8; width * height * out_channels * out_sample];
    let pixel_bytes = out_channels * out_sample;
    for y in 0..height {
        for x in 0..width {
            let px = &mut out[(y * width + x) * pixel_bytes..][..pixel_bytes];
            let stored = |plane: usize| &planes[plane][y * row_bytes..][..row_bytes];
            match (mode, samples.depth) {
                (Mode::Bitmap, _) => {
                    let bit = stored(0)[x / 8] >> (7 - x % 8) & 1;
                    // 1 is black.
                    px[0] = if bit == 1 { 0 } else { 255 };
                }
                (Mode::Indexed, _) => {
                    let index = usize::from(stored(0)[x]);
                    for c in 0..3 {
                        px[c] = palette[c * 256 + index];
                    }
                    if transparency {
                        px[3] = stored(1)[x];
                    } else if transparent_index.is_some() {
                        let clear = transparent_index == Some(index as u16);
                        px[3] = if clear { 0 } else { 255 };
                    }
                }
                (_, 8) => {
                    for (c, v) in px.iter_mut().enumerate() {
                        *v = stored(c)[x];
                    }
                }
                (_, 16) => {
                    for c in 0..out_channels {
                        let s = &stored(c)[x * 2..x * 2 + 2];
                        let v = u16::from_be_bytes([s[0], s[1]]);
                        px[c * 2..c * 2 + 2].copy_from_slice(&v.to_ne_bytes());
                    }
                }
                _ => {
                    for c in 0..out_channels {
                        let s = &stored(c)[x * 4..x * 4 + 4];
                        let v = f32::from_be_bytes([s[0], s[1], s[2], s[3]]);
                        px[c * 4..c * 4 + 4].copy_from_slice(&v.to_ne_bytes());
                    }
                }
            }
            if transparency {
                remove_white_matte(px, out_channels, samples.depth);
            }
        }
    }
    out
}

/// Photoshop mattes the composite of a transparent document against white: undo it on the
/// encoded values, `c = (c' - (1 - a)) / a`, clamped to the sample range (alpha 0: color 0).
fn remove_white_matte(px: &mut [u8], channels: usize, depth: u16) {
    let colors = channels - 1;
    match depth {
        16 => {
            let read = |px: &[u8], c: usize| {
                f64::from(u16::from_ne_bytes([px[c * 2], px[c * 2 + 1]])) / 65535.0
            };
            let a = read(px, colors);
            for c in 0..colors {
                let v = unmatte(read(px, c), a);
                let code = (v * 65535.0).round() as u16;
                px[c * 2..c * 2 + 2].copy_from_slice(&code.to_ne_bytes());
            }
        }
        32 => {
            let read = |px: &[u8], c: usize| {
                f64::from(f32::from_ne_bytes([
                    px[c * 4],
                    px[c * 4 + 1],
                    px[c * 4 + 2],
                    px[c * 4 + 3],
                ]))
            };
            let a = read(px, colors).clamp(0.0, 1.0);
            for c in 0..colors {
                // Float data is not clamped: only the matte is removed.
                let v = if a > 0.0 {
                    (read(px, c) - (1.0 - a)) / a
                } else {
                    0.0
                };
                px[c * 4..c * 4 + 4].copy_from_slice(&(v as f32).to_ne_bytes());
            }
        }
        _ => {
            let a = f64::from(px[colors]) / 255.0;
            for value in &mut px[..colors] {
                let v = unmatte(f64::from(*value) / 255.0, a);
                *value = (v * 255.0).round() as u8;
            }
        }
    }
}

fn unmatte(v: f64, a: f64) -> f64 {
    if a > 0.0 {
        ((v - (1.0 - a)) / a).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests;
