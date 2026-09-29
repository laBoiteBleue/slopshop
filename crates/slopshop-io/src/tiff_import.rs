//! TIFF import through the `tiff` crate: native bit depths, ICC profile, orientation and
//! associated alpha, instead of `image`'s adapter (which maps CMYK to RGB silently).

use std::fs::File;
use std::io::BufReader;

use slopshop_core::Size;
use slopshop_core::color::{AlphaMode, ChannelLayout, SampleType};
use tiff::decoder::{Decoder, DecodingResult, Limits};
use tiff::tags::Tag;
use tiff::{TiffError, TiffResult};

use crate::orient::Orientation;
use crate::{Decoded, ImportError, ImportWarning, check_budget};

const TAG_ICC_PROFILE: u16 = 34675;

/// Largest tag value read before the pixels (an ICC profile is at most a few MiB). The `tiff`
/// crate bounds tag allocations with the decoding buffer limit.
const TAG_BYTES_LIMIT: usize = 64 * 1024 * 1024;

pub(crate) fn decode(file: File) -> Result<Decoded, ImportError> {
    let file_len = file.metadata()?.len();
    let mut tag_limits = Limits::default();
    tag_limits.decoding_buffer_size = TAG_BYTES_LIMIT;
    let mut decoder = Decoder::new(BufReader::new(file))
        .map_err(tiff_error)?
        .with_limits(tag_limits);
    let (width, height) = decoder.dimensions().map_err(tiff_error)?;
    let too_large = |e: TiffError| match e {
        TiffError::LimitsExceeded => ImportError::TooLarge { width, height },
        other => tiff_error(other),
    };
    let color = decoder.colortype().map_err(tiff_error)?;
    let photometric = optional(decoder.find_tag_unsigned::<u16>(Tag::PhotometricInterpretation))?;
    // ExtraSamples: 1 = associated (premultiplied) alpha, 2 = unassociated alpha.
    let extra = optional(decoder.find_tag_unsigned_vec::<u16>(Tag::ExtraSamples))?;
    let extra_alpha = extra.as_ref().and_then(|v| v.first().copied());
    let (layout, bits) = match color {
        tiff::ColorType::Gray(b) => (ChannelLayout::Gray, b),
        tiff::ColorType::GrayA(b) => (ChannelLayout::GrayAlpha, b),
        // `tiff` reports gray + alpha as two bands: accept them when the second one really is
        // alpha over BlackIsZero gray (WhiteIsZero would need inverting).
        tiff::ColorType::Multiband {
            bit_depth: b,
            num_samples: 2,
        } if photometric == Some(1) && matches!(extra_alpha, Some(1 | 2)) => {
            (ChannelLayout::GrayAlpha, b)
        }
        tiff::ColorType::RGB(b) => (ChannelLayout::Rgb, b),
        tiff::ColorType::RGBA(b) => (ChannelLayout::Rgba, b),
        other => {
            return Err(ImportError::UnsupportedPixels(format!("TIFF {other:?}")));
        }
    };
    // 1/2/4-bit (bilevel scans, indexed-like gray) and 12/24-bit samples come packed.
    if !matches!(bits, 8 | 16 | 32 | 64) {
        return Err(ImportError::UnsupportedPixels(format!(
            "TIFF {bits}-bit samples"
        )));
    }
    // `read_image` only returns the first plane of planar data.
    let planar = optional(decoder.find_tag_unsigned::<u16>(Tag::PlanarConfiguration))? == Some(2);
    if planar && layout.channels() > 1 {
        return Err(ImportError::UnsupportedPixels("planar TIFF".into()));
    }

    // Budget from the header, before decoding anything.
    let channels = layout.channels();
    let sample_bytes = u32::from(bits) / 8;
    let stored_sample = match bits {
        8 => SampleType::U8,
        16 => SampleType::U16,
        // 64-bit floats are stored as 32-bit floats, converted from a second buffer.
        _ => SampleType::F32,
    };
    let converted_bytes = if bits == 64 { 4 } else { 0 };
    check_budget(
        width,
        height,
        layout,
        stored_sample,
        channels * (sample_bytes + converted_bytes),
    )?;

    let mut warnings = Vec::new();
    let icc = match optional(decoder.find_tag(Tag::Unknown(TAG_ICC_PROFILE)))? {
        None => None,
        Some(value) => match value.into_u8_vec() {
            Ok(bytes) => Some(bytes),
            Err(_) => {
                warnings.push(ImportWarning::IccProfileUnsupported);
                None
            }
        },
    };
    let orientation = optional(decoder.find_tag_unsigned::<u16>(Tag::Orientation))?
        .map_or(Orientation::Normal, Orientation::from_exif);
    let associated = layout.has_alpha() && extra_alpha == Some(1);
    if decoder.more_images() {
        warnings.push(ImportWarning::FirstPageOnly);
    }

    // Exact limits for the pixels: the output buffer, and chunks no larger than the file.
    let mut pixel_limits = Limits::default();
    pixel_limits.decoding_buffer_size =
        usize::try_from(u64::from(width) * u64::from(height) * u64::from(channels * sample_bytes))
            .map_err(|_| ImportError::TooLarge { width, height })?;
    pixel_limits.intermediate_buffer_size = usize::try_from(file_len).unwrap_or(usize::MAX);
    let mut decoder = decoder.with_limits(pixel_limits);

    let (sample, bytes) = match decoder.read_image().map_err(too_large)? {
        DecodingResult::U8(v) => (SampleType::U8, v),
        DecodingResult::U16(v) => (
            SampleType::U16,
            v.iter().flat_map(|s| s.to_ne_bytes()).collect(),
        ),
        DecodingResult::F16(v) => (
            SampleType::F16,
            v.iter().flat_map(|s| s.to_bits().to_ne_bytes()).collect(),
        ),
        DecodingResult::F32(v) => (
            SampleType::F32,
            v.iter().flat_map(|s| s.to_ne_bytes()).collect(),
        ),
        DecodingResult::F64(v) => {
            // No f64 storage: f32 keeps 24 significant bits, far beyond display needs, but the
            // conversion is reported.
            warnings.push(ImportWarning::PrecisionReduced);
            (
                SampleType::F32,
                v.iter().flat_map(|s| (*s as f32).to_ne_bytes()).collect(),
            )
        }
        other => {
            return Err(ImportError::UnsupportedPixels(format!(
                "TIFF {} samples",
                decoding_kind(&other)
            )));
        }
    };

    Ok(Decoded {
        size: Size::new(width, height),
        layout,
        sample,
        alpha: if associated {
            AlphaMode::Premultiplied
        } else {
            AlphaMode::Straight
        },
        icc,
        space: None,
        orientation,
        pixels: bytes,
        warnings,
    })
}

/// An optional tag: absent or unreadable gives `None` (defaults apply), but exhausted limits
/// and I/O errors are errors.
fn optional<T>(result: TiffResult<Option<T>>) -> Result<Option<T>, ImportError> {
    match result {
        Ok(value) => Ok(value),
        Err(e @ (TiffError::LimitsExceeded | TiffError::IoError(_))) => Err(tiff_error(e)),
        Err(_) => Ok(None),
    }
}

fn decoding_kind(result: &DecodingResult) -> &'static str {
    match result {
        DecodingResult::U32(_) => "32-bit integer",
        DecodingResult::U64(_) => "64-bit integer",
        DecodingResult::I8(_)
        | DecodingResult::I16(_)
        | DecodingResult::I32(_)
        | DecodingResult::I64(_) => "signed integer",
        _ => "unknown",
    }
}

fn tiff_error(e: TiffError) -> ImportError {
    match e {
        TiffError::IoError(io) => ImportError::Io(io),
        TiffError::UnsupportedError(u) => ImportError::UnsupportedPixels(format!("TIFF: {u}")),
        other => ImportError::Decode(other.to_string()),
    }
}
