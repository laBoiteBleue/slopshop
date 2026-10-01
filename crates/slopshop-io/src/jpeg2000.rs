//! JPEG 2000 import (hayro-jpeg2000): JP2 files and raw codestreams, at the file's precision
//! (samples of up to 8 bits as 8-bit, up to 16 bits as 16-bit, deeper ones as 32-bit float,
//! each scaled from its own bit depth), gray or RGB, with straight or premultiplied alpha.
//! Color comes from the JP2 header: sRGB, gray and sYCC (converted to RGB by the decoder) as
//! sRGB, ROMM-RGB and embedded ICC profiles as profiles. Raw codestreams have no color
//! information: they are read as sRGB. CMYK and CIELab are refused; e-sRGB and e-sYCC are read
//! as sRGB with a warning.

use std::path::Path;

use hayro_jpeg2000::{
    ColorSpace as J2kSpace, ComponentData, DecodeSettings, DecoderContext, Image,
};
use slopshop_core::Size;
use slopshop_core::color::{AlphaMode, ChannelLayout, SampleType};

use crate::orient::Orientation;
use crate::{Decoded, ImportError, ImportWarning, check_budget};

/// The JP2 signature box, and the start of a raw codestream (SOC and SIZ markers).
const JP2_SIGNATURE: &[u8] = b"\0\0\0\x0cjP  \r\n\x87\n";
const CODESTREAM_SIGNATURE: &[u8] = &[0xFF, 0x4F, 0xFF, 0x51];

/// Enumerated color spaces of the JP2 `colr` box (ISO/IEC 15444-2, table M.25) that the
/// decoder does not report faithfully.
const CIELAB: u32 = 14;
const E_SRGB: u32 = 20;
const E_SYCC: u32 = 24;

/// Whether `head` starts like a JPEG 2000 file: the JP2 container or a raw codestream.
pub(crate) fn is_jpeg2000(head: &[u8]) -> bool {
    head.starts_with(JP2_SIGNATURE) || head.starts_with(CODESTREAM_SIGNATURE)
}

pub(crate) fn decode(path: &Path) -> Result<Decoded, ImportError> {
    let data = std::fs::read(path)?;
    let decode_error = |e| ImportError::Decode(format!("JPEG 2000: {e}"));
    let header = jp2_header(&data);
    let mut warnings = Vec::new();
    match header.enumerated {
        Some(CIELAB) => return Err(ImportError::UnsupportedPixels("CIELab".to_owned())),
        Some(E_SRGB | E_SYCC) => warnings.push(ImportWarning::ColorInfoUnsupported),
        _ => {}
    }

    let image = Image::new(&data, &DecodeSettings::default()).map_err(decode_error)?;
    let (gray, icc) = match image.color_space() {
        J2kSpace::Gray => (true, None),
        J2kSpace::RGB => (false, None),
        J2kSpace::Icc {
            profile,
            num_channels: 1,
        } => (true, Some(profile.clone())),
        J2kSpace::Icc {
            profile,
            num_channels: 3,
        } => (false, Some(profile.clone())),
        J2kSpace::CMYK
        | J2kSpace::Icc {
            num_channels: 4, ..
        } => {
            return Err(ImportError::UnsupportedPixels("CMYK".to_owned()));
        }
        other => {
            return Err(ImportError::UnsupportedPixels(format!(
                "JPEG 2000 with {} color channels",
                other.num_channels()
            )));
        }
    };
    let layout = match (gray, image.has_alpha()) {
        (true, false) => ChannelLayout::Gray,
        (true, true) => ChannelLayout::GrayAlpha,
        (false, false) => ChannelLayout::Rgb,
        (false, true) => ChannelLayout::Rgba,
    };
    let (width, height) = (image.width(), image.height());
    // The decoder holds every component as 32-bit floats, at the size of the image.
    check_budget(
        width,
        height,
        layout,
        sample_type(image.original_bit_depth()),
        layout.channels() * 4,
    )?;

    let mut context = DecoderContext::default();
    let decoded = image.decode(&mut context).map_err(decode_error)?;
    let components = decoded.components();
    let size = Size::new(width, height);
    if components.len() != layout.channels() as usize
        || components
            .iter()
            .any(|c| c.samples().len() as u64 != size.pixel_count())
    {
        return Err(ImportError::Decode(
            "JPEG 2000: components do not match the image".to_owned(),
        ));
    }
    let deepest = components.iter().map(ComponentData::bit_depth).max();
    let sample = sample_type(deepest.unwrap_or(8));
    Ok(Decoded {
        size,
        layout,
        sample,
        alpha: if header.premultiplied {
            AlphaMode::Premultiplied
        } else {
            AlphaMode::Straight
        },
        icc,
        space: None,
        orientation: Orientation::Normal,
        pixels: interleave(components, sample),
        warnings,
    })
}

/// The sample type that holds `bits`-bit integer samples without loss.
fn sample_type(bits: u8) -> SampleType {
    match bits {
        0..=8 => SampleType::U8,
        9..=16 => SampleType::U16,
        _ => SampleType::F32,
    }
}

/// What the decoder does not expose from the JP2 header.
#[derive(Debug, Default, PartialEq)]
struct Jp2Header {
    /// The enumerated color space of the first `colr` box, if it has one.
    enumerated: Option<u32>,
    /// Whether the opacity channel is premultiplied (`cdef` type 2).
    premultiplied: bool,
}

/// The `jp2h` box's color and channel definitions; nothing for a raw codestream or a damaged
/// header (the decoder reports the latter).
fn jp2_header(data: &[u8]) -> Jp2Header {
    let mut header = Jp2Header::default();
    let Some((_, jp2h)) = boxes(data).find(|(kind, _)| kind == b"jp2h") else {
        return header;
    };
    let mut first_colr = true;
    for (kind, content) in boxes(jp2h) {
        match &kind {
            b"colr" if first_colr => {
                first_colr = false;
                // METH, PREC, APPROX, then EnumCS when METH is 1.
                if content.first() == Some(&1) {
                    header.enumerated = content
                        .get(3..7)
                        .and_then(|b| b.try_into().ok())
                        .map(u32::from_be_bytes);
                }
            }
            b"cdef" => {
                // N, then N times (Cn, Typ, Asoc), all 16-bit.
                header.premultiplied = content
                    .get(2..)
                    .unwrap_or_default()
                    .as_chunks::<6>()
                    .0
                    .iter()
                    .any(|&[_, _, typ_high, typ_low, _, _]| [typ_high, typ_low] == [0, 2]);
            }
            _ => {}
        }
    }
    header
}

/// The boxes of an ISO base media sequence, as (type, content); stops at the first box that
/// does not fit.
fn boxes(mut data: &[u8]) -> impl Iterator<Item = ([u8; 4], &[u8])> {
    std::iter::from_fn(move || {
        let length = u32::from_be_bytes(data.get(0..4)?.try_into().ok()?);
        let kind: [u8; 4] = data.get(4..8)?.try_into().ok()?;
        let (start, end) = match length {
            0 => (8, data.len()),
            1 => {
                let long = u64::from_be_bytes(data.get(8..16)?.try_into().ok()?);
                (16, usize::try_from(long).ok()?)
            }
            n => (8, usize::try_from(n).ok()?),
        };
        let content = data.get(start..end)?;
        data = &data[end..];
        Some((kind, content))
    })
}

/// The components as interleaved native-endian samples of `sample`, each scaled from its own
/// bit depth to the full range of `sample` (rounded and clamped: lossy files overshoot), pixels
/// in parallel.
fn interleave(components: &[ComponentData], sample: SampleType) -> Vec<u8> {
    let full = match sample {
        SampleType::U8 => f32::from(u8::MAX),
        SampleType::U16 => f32::from(u16::MAX),
        SampleType::F16 | SampleType::F32 => 1.0,
    };
    let scales: Vec<f32> = components
        .iter()
        .map(|c| full / ((1u64 << c.bit_depth().min(63)) - 1).max(1) as f32)
        .collect();
    let width = sample.bytes() as usize;
    let pixel_bytes = components.len() * width;
    let pixels = components.first().map_or(0, |c| c.samples().len());
    let mut out = vec![0u8; pixels * pixel_bytes];
    if pixel_bytes == 0 {
        return out;
    }
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let per_thread = pixels.div_ceil(threads).max(1 << 14);
    std::thread::scope(|scope| {
        for (chunk, part) in out.chunks_mut(per_thread * pixel_bytes).enumerate() {
            let scales = &scales;
            scope.spawn(move || {
                let first = chunk * per_thread;
                for (i, pixel) in part.chunks_exact_mut(pixel_bytes).enumerate() {
                    for (c, target) in pixel.chunks_exact_mut(width).enumerate() {
                        let v = components[c].samples()[first + i] * scales[c];
                        // Float-to-integer `as` saturates: rounding then casting clamps.
                        match sample {
                            SampleType::U8 => target[0] = v.round() as u8,
                            SampleType::U16 => {
                                target.copy_from_slice(&(v.round() as u16).to_ne_bytes());
                            }
                            SampleType::F16 | SampleType::F32 => {
                                target.copy_from_slice(&v.clamp(0.0, 1.0).to_ne_bytes());
                            }
                        }
                    }
                }
            });
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use slopshop_core::color::ColorSpace;

    fn fixture(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/jpeg2000")
            .join(name)
    }

    fn u16s(bytes: &[u8]) -> Vec<u16> {
        bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u16::from_ne_bytes(*b))
            .collect()
    }

    /// The fixtures' pattern, for each pixel in reading order.
    fn pattern<T>(f: impl Fn(u32, u32) -> Vec<T>) -> Vec<T> {
        let mut out = Vec::new();
        for y in 0..48 {
            for x in 0..64 {
                out.extend(f(x, y));
            }
        }
        out
    }

    fn rgb8() -> Vec<u8> {
        pattern(|x, y| {
            vec![
                (x * 4 % 256) as u8,
                (y * 5 % 256) as u8,
                ((x + y) * 3 % 256) as u8,
            ]
        })
    }

    #[test]
    fn lossless_files_come_back_exactly_at_their_depth() {
        for name in ["rgb8.jp2", "tiled.j2k"] {
            let rgb = decode(&fixture(name)).unwrap();
            assert_eq!(
                (rgb.layout, rgb.sample, rgb.size),
                (ChannelLayout::Rgb, SampleType::U8, Size::new(64, 48)),
                "{name}"
            );
            assert_eq!(rgb.pixels, rgb8(), "{name}");
        }

        let gray = decode(&fixture("gray8.j2k")).unwrap();
        assert_eq!(gray.layout, ChannelLayout::Gray);
        assert_eq!(
            gray.pixels,
            pattern(|x, y| vec![((x * 3 + y * 2) % 256) as u8])
        );

        let rgba = decode(&fixture("rgba16.jp2")).unwrap();
        assert_eq!(
            (rgba.layout, rgba.sample, rgba.alpha),
            (ChannelLayout::Rgba, SampleType::U16, AlphaMode::Straight)
        );
        let expected = pattern(|x, y| {
            vec![
                (x * 1000 % 65536) as u16,
                (y * 1300 % 65536) as u16,
                (x * y * 37 % 65536) as u16,
                (1000 + (x + y) * 500) as u16,
            ]
        });
        assert_eq!(u16s(&rgba.pixels), expected);
    }

    #[test]
    fn twelve_bit_samples_fill_the_sixteen_bit_range() {
        let rgb = decode(&fixture("rgb12.jp2")).unwrap();
        assert_eq!(rgb.sample, SampleType::U16);
        let scale = |v: u32| (f64::from(v) * 65535.0 / 4095.0).round() as u16;
        let expected = pattern(|x, y| {
            vec![
                scale((x * 64 + y) % 4096),
                scale(y * 85 % 4096),
                scale(x * y * 3 % 4096),
            ]
        });
        assert_eq!(u16s(&rgb.pixels), expected);
    }

    #[test]
    fn lossy_files_decode_close_to_their_source_in_srgb() {
        let lossy = decode(&fixture("lossy.jp2")).unwrap();
        assert_eq!(lossy.sample, SampleType::U8);
        let mean = lossy
            .pixels
            .iter()
            .zip(rgb8())
            .map(|(a, b)| f64::from(a.abs_diff(b)))
            .sum::<f64>()
            / lossy.pixels.len() as f64;
        assert!(mean < 6.0, "{mean}");
        let imported = crate::open_image(&fixture("lossy.jp2")).unwrap();
        assert_eq!(imported.image.format().color_space, ColorSpace::SRGB);
        assert!(imported.warnings.is_empty(), "{:?}", imported.warnings);
    }

    /// A JP2 header: the signature, then `jp2h` with the given boxes.
    fn jp2(children: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let wrap = |kind: &[u8], content: &[u8]| {
            let mut out = u32::try_from(content.len() + 8)
                .unwrap()
                .to_be_bytes()
                .to_vec();
            out.extend(kind);
            out.extend(content);
            out
        };
        let inner: Vec<u8> = children
            .iter()
            .flat_map(|(kind, content)| wrap(*kind, content))
            .collect();
        let mut out = JP2_SIGNATURE.to_vec();
        out.extend(wrap(b"ftyp", b"jp2 \0\0\0\0jp2 "));
        out.extend(wrap(b"jp2h", &inner));
        out
    }

    #[test]
    fn the_header_gives_the_enumerated_space_and_premultiplied_alpha() {
        let colr = |cs: u32| [&[1, 0, 0][..], &cs.to_be_bytes()].concat();
        // Three color channels, then a premultiplied opacity for the whole image.
        let cdef = [
            0, 4, //
            0, 0, 0, 0, 0, 1, //
            0, 1, 0, 0, 0, 2, //
            0, 2, 0, 0, 0, 3, //
            0, 3, 0, 2, 0, 0,
        ];
        let header = jp2_header(&jp2(&[
            (b"ihdr", vec![0; 14]),
            (b"colr", colr(E_SRGB)),
            (b"colr", colr(16)),
            (b"cdef", cdef.to_vec()),
        ]));
        assert_eq!(
            header,
            Jp2Header {
                enumerated: Some(E_SRGB),
                premultiplied: true
            }
        );
        // An ICC profile (METH 2) has no enumerated space; straight opacity is type 1.
        let mut straight = cdef;
        straight[23] = 1;
        let header = jp2_header(&jp2(&[
            (b"colr", vec![2, 0, 0, 1, 2, 3]),
            (b"cdef", straight.to_vec()),
        ]));
        assert_eq!(header, Jp2Header::default());
        // Raw codestreams and truncated boxes give nothing.
        assert_eq!(jp2_header(CODESTREAM_SIGNATURE), Jp2Header::default());
        let mut cut = jp2(&[(b"colr", colr(CIELAB))]);
        cut.truncate(cut.len() - 1);
        assert_eq!(jp2_header(&cut), Jp2Header::default());
    }

    #[test]
    fn depths_beyond_sixteen_bits_become_float() {
        assert_eq!(sample_type(1), SampleType::U8);
        assert_eq!(sample_type(12), SampleType::U16);
        assert_eq!(sample_type(20), SampleType::F32);
    }

    #[test]
    fn damaged_files_are_errors() {
        let bytes = std::fs::read(fixture("rgba16.jp2")).unwrap();
        assert!(is_jpeg2000(&bytes));
        let path =
            std::env::temp_dir().join(format!("slopshop-j2k-{}-cut.jp2", std::process::id()));
        std::fs::write(&path, &bytes[..bytes.len() / 3]).unwrap();
        let result = decode(&path);
        std::fs::remove_file(&path).ok();
        assert!(result.is_err());
    }
}
