//! JPEG XL import (jxl-oxide): the first frame, at the file's precision (integer samples of up
//! to 8 bits as 8-bit, up to 16 bits as 16-bit, float or deeper samples as 32-bit float), in the
//! color encoding the file declares: H.273 code points when it has them, else the ICC profile
//! describing the rendered image. Orientation is applied by the decoder. CMYK is refused.

use std::path::Path;

use jxl_oxide::image::BitDepth;
use jxl_oxide::{AllocTracker, JxlImage, PixelFormat as JxlFormat};
use slopshop_core::Size;
use slopshop_core::color::{AlphaMode, ChannelLayout, SampleType};

use crate::orient::Orientation;
use crate::{Decoded, ImportError, ImportWarning, MAX_IMPORT_BYTES, check_budget, cicp_code_space};

/// Whether `head` starts like a JPEG XL file: a bare codestream or the ISOBMFF container.
pub(crate) fn is_jxl(head: &[u8]) -> bool {
    head.starts_with(&[0xFF, 0x0A]) || head.starts_with(b"\0\0\0\x0cJXL \r\n\x87\n")
}

pub(crate) fn decode(path: &Path) -> Result<Decoded, ImportError> {
    let decode_error =
        |e: Box<dyn std::error::Error + Send + Sync>| ImportError::Decode(format!("JPEG XL: {e}"));
    let image = JxlImage::builder()
        .alloc_tracker(AllocTracker::with_limit(
            usize::try_from(MAX_IMPORT_BYTES).unwrap_or(usize::MAX),
        ))
        .open(path)
        .map_err(decode_error)?;
    let (layout, channels) = match image.pixel_format() {
        JxlFormat::Gray => (ChannelLayout::Gray, 1),
        JxlFormat::Graya => (ChannelLayout::GrayAlpha, 2),
        JxlFormat::Rgb => (ChannelLayout::Rgb, 3),
        JxlFormat::Rgba => (ChannelLayout::Rgba, 4),
        JxlFormat::Cmyk | JxlFormat::Cmyka => {
            return Err(ImportError::UnsupportedPixels("CMYK".to_owned()));
        }
    };
    let sample = match image.image_header().metadata.bit_depth {
        BitDepth::IntegerSample { bits_per_sample } if bits_per_sample <= 8 => SampleType::U8,
        BitDepth::IntegerSample { bits_per_sample } if bits_per_sample <= 16 => SampleType::U16,
        _ => SampleType::F32,
    };
    let (width, height) = (image.width(), image.height());
    check_budget(width, height, layout, sample, channels * 4)?;

    let mut warnings = Vec::new();
    if image.num_loaded_keyframes() > 1 {
        warnings.push(ImportWarning::FirstFrameOnly);
    }
    let render = image.render_frame(0).map_err(decode_error)?;
    let mut stream = render.stream();
    // The stream applies the orientation: its size may be the header's, transposed.
    let size = Size::new(stream.width(), stream.height());
    let count = size.pixel_count() as usize * channels as usize;
    let pixels: Vec<u8> = match sample {
        SampleType::U8 => {
            let mut buffer = vec![0u8; count];
            stream.write_to_buffer(&mut buffer);
            buffer
        }
        SampleType::U16 => read_samples(&mut stream, count, 0u16, u16::to_ne_bytes),
        SampleType::F16 | SampleType::F32 => {
            read_samples(&mut stream, count, 0f32, f32::to_ne_bytes)
        }
    };
    let associated = image
        .image_header()
        .metadata
        .ec_info
        .iter()
        .find_map(|ec| ec.alpha_associated())
        .unwrap_or(false);
    let space = image
        .rendered_cicp()
        .and_then(|[primaries, transfer, matrix, full]| {
            cicp_code_space(primaries, transfer, matrix, full != 0)
        });
    let icc = if space.is_none() {
        Some(image.rendered_icc())
    } else {
        None
    };
    Ok(Decoded {
        size,
        layout,
        sample,
        alpha: if associated {
            AlphaMode::Premultiplied
        } else {
            AlphaMode::Straight
        },
        icc,
        space,
        orientation: Orientation::Normal,
        pixels,
        warnings,
    })
}

/// `count` samples of `stream` as native-endian bytes, through a small buffer: the image is
/// not held twice.
fn read_samples<S: jxl_oxide::FrameBufferSample + Copy, const N: usize>(
    stream: &mut jxl_oxide::ImageStream<'_>,
    count: usize,
    zero: S,
    bytes: fn(S) -> [u8; N],
) -> Vec<u8> {
    let mut out = Vec::with_capacity(count * N);
    let mut chunk = vec![zero; 1 << 16];
    loop {
        let written = stream.write_to_buffer(&mut chunk);
        if written == 0 {
            break;
        }
        for &v in &chunk[..written] {
            out.extend(bytes(v));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use slopshop_core::color::ColorSpace;

    fn fixture(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/jxl")
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

    fn f32s(bytes: &[u8]) -> Vec<f32> {
        bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_ne_bytes(*b))
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
        let rgb = decode(&fixture("rgb8.jxl")).unwrap();
        assert_eq!(
            (rgb.layout, rgb.sample, rgb.size),
            (ChannelLayout::Rgb, SampleType::U8, Size::new(64, 48))
        );
        assert_eq!(rgb.pixels, rgb8());

        let gray = decode(&fixture("gray8.jxl")).unwrap();
        assert_eq!(gray.layout, ChannelLayout::Gray);
        assert_eq!(
            gray.pixels,
            pattern(|x, y| vec![((x * 3 + y * 2) % 256) as u8])
        );

        let rgba = decode(&fixture("rgba16.jxl")).unwrap();
        assert_eq!(
            (rgba.layout, rgba.sample),
            (ChannelLayout::Rgba, SampleType::U16)
        );
        assert_eq!(rgba.alpha, AlphaMode::Straight);
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
    fn float_samples_stay_float() {
        let float = decode(&fixture("float.jxl")).unwrap();
        assert_eq!(float.sample, SampleType::F32);
        let expected = pattern(|x, y| {
            vec![
                x as f32 / 16.0,
                y as f32 / 12.0,
                (x + y) as f32 / 40.0 - 0.5,
            ]
        });
        let worst = f32s(&float.pixels)
            .iter()
            .zip(&expected)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(worst < 1e-3, "{worst}");
    }

    #[test]
    fn lossy_files_decode_close_to_their_source_in_srgb() {
        let lossy = decode(&fixture("lossy.jxl")).unwrap();
        assert_eq!(lossy.sample, SampleType::U8);
        let mean = lossy
            .pixels
            .iter()
            .zip(rgb8())
            .map(|(a, b)| f64::from(a.abs_diff(b)))
            .sum::<f64>()
            / lossy.pixels.len() as f64;
        assert!(mean < 6.0, "{mean}");
        let imported = crate::open_image(&fixture("lossy.jxl")).unwrap();
        assert_eq!(imported.image.format().color_space, ColorSpace::SRGB);
        assert!(imported.warnings.is_empty(), "{:?}", imported.warnings);
    }

    #[test]
    fn animations_give_their_first_frame_with_a_warning() {
        let anim = decode(&fixture("anim.jxl")).unwrap();
        assert_eq!(anim.warnings, [ImportWarning::FirstFrameOnly]);
        assert_eq!(anim.pixels, rgb8());
    }

    #[test]
    fn damaged_files_are_errors() {
        let bytes = std::fs::read(fixture("rgba16.jxl")).unwrap();
        let path =
            std::env::temp_dir().join(format!("slopshop-jxl-{}-cut.jxl", std::process::id()));
        std::fs::write(&path, &bytes[..bytes.len() / 2]).unwrap();
        assert!(is_jxl(&bytes));
        assert!(decode(&path).is_err());
        std::fs::remove_file(&path).ok();
    }
}
