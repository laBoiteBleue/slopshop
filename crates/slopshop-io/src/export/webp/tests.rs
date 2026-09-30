use std::path::{Path, PathBuf};

use slopshop_core::BlendSpace;
use slopshop_core::color::{ColorSpace, WORKING_SPACE, mat_vec};
use slopshop_core::convert::{ConversionReport, ConvertOptions, Converter, WHITE_MATTE};
use slopshop_core::{CancelToken, Rect};

use super::*;
use crate::export::{ExportFormat, ExportSpec, WebpCompression, export_image, temp_files};
use crate::open_image;

fn temp_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("slopshop-webp-{}-{name}", std::process::id()))
}

/// Odd sizes: an odd last row and column for chroma, heights not multiple of the band.
const ODD_SIZE: Size = Size::new(301, 523);

/// Smooth sRGB-gamut gradients with translucency (alpha 0.2 to 1), as premultiplied
/// working-space values.
fn pattern(region: Rect, out: &mut [f32]) -> Result<u64, String> {
    let to_working = ColorSpace::LINEAR_SRGB.matrix_to(&WORKING_SPACE);
    for (i, px) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let x = (region.x + i as u32 % region.width) as f32;
        let y = (region.y + i as u32 / region.width) as f32;
        let alpha = 0.2 + 0.8 * (x / 300.0).min(1.0);
        let r = (x / 300.0).min(1.0);
        let g = (y / 520.0).min(1.0);
        let b = 0.5 + 0.4 * ((x + y) / 60.0).sin();
        let [r, g, b] = mat_vec(&to_working, [r, g, b].map(f64::from)).map(|c| c as f32);
        *px = [r * alpha, g * alpha, b * alpha, alpha];
    }
    Ok(0)
}

fn webp_spec(compression: WebpCompression, keep_alpha: bool) -> ExportSpec {
    ExportSpec {
        format: ExportFormat::Webp { compression },
        space: ColorSpace::SRGB,
        keep_alpha,
        matte: WHITE_MATTE,
        dither: false,
        gray: false,
        // A synthetic source, not a document: flattened in linear light, as the expected
        // samples are computed.
        blend_space: BlendSpace::Linear,
    }
}

/// The 8-bit samples the writer receives: the pattern through the export's conversion.
fn expected(size: Size, spec: &ExportSpec) -> Vec<u8> {
    let options = ConvertOptions {
        dither: spec.dither,
        big_endian: false,
        matte: spec.matte,
        blend_space: spec.blend_space,
    };
    let converter = Converter::new(spec.target_format(), options).unwrap();
    let row_bytes = size.width as usize * converter.bytes_per_pixel();
    let mut out = vec![0; row_bytes * size.height as usize];
    let mut src = vec![0.0; size.width as usize * 4];
    for (y, dst) in out.chunks_exact_mut(row_bytes).enumerate() {
        let y = y as u32;
        pattern(Rect::new(0, y, size.width, 1), &mut src).unwrap();
        let mut report = ConversionReport::default();
        converter.convert_row(&src, 0, y, dst, &mut report).unwrap();
    }
    out
}

fn export_pattern(path: &Path, size: Size, spec: &ExportSpec) {
    export_image(path, size, spec, pattern, &CancelToken::new(), &mut |_| {}).unwrap();
    assert!(temp_files(path).is_empty());
}

fn psnr(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len());
    let mse = a
        .iter()
        .zip(b)
        .map(|(&x, &y)| (f64::from(x) - f64::from(y)).powi(2))
        .sum::<f64>()
        / a.len() as f64;
    10.0 * (255.0 * 255.0 / mse.max(1e-12)).log10()
}

/// Samples decoded by `image` (its own WebP decoder), in the layout written.
fn decode(path: &Path, alpha: bool) -> Vec<u8> {
    let image = image::open(path).unwrap();
    if alpha {
        image.to_rgba8().into_raw()
    } else {
        image.to_rgb8().into_raw()
    }
}

#[test]
fn lossless_round_trips_bit_exact() {
    for keep_alpha in [false, true] {
        let spec = webp_spec(WebpCompression::Lossless, keep_alpha);
        let path = temp_path("lossless.webp");
        export_pattern(&path, ODD_SIZE, &spec);
        assert!(
            decode(&path, keep_alpha) == expected(ODD_SIZE, &spec),
            "lossless, alpha {keep_alpha}: not bit-exact"
        );
        std::fs::remove_file(&path).ok();
    }
}

#[test]
fn lossy_round_trips_within_the_quality_asked() {
    for (quality, keep_alpha, min_psnr) in [(95, false, 38.0), (90, true, 36.0), (50, false, 30.0)]
    {
        let case = format!("q{quality} alpha {keep_alpha}");
        let spec = webp_spec(WebpCompression::Lossy { quality }, keep_alpha);
        let path = temp_path("lossy.webp");
        export_pattern(&path, ODD_SIZE, &spec);
        let decoded = decode(&path, keep_alpha);
        let wanted = expected(ODD_SIZE, &spec);
        let channels = if keep_alpha { 4 } else { 3 };
        let color = |samples: &[u8]| -> Vec<u8> {
            samples
                .chunks_exact(channels)
                .flat_map(|px| px[..3].to_vec())
                .collect()
        };
        let quality_db = psnr(&color(&decoded), &color(&wanted));
        assert!(quality_db > min_psnr, "{case}: PSNR {quality_db:.1} dB");
        if keep_alpha {
            // Alpha is compressed losslessly (alpha quality 100).
            let alpha = |samples: &[u8]| -> Vec<u8> {
                samples.as_chunks::<4>().0.iter().map(|px| px[3]).collect()
            };
            assert!(alpha(&decoded) == alpha(&wanted), "{case}: alpha changed");
        }
        std::fs::remove_file(&path).ok();
    }
}

#[test]
fn files_are_tagged_and_read_back_as_the_same_space() {
    let size = Size::new(24, 16);
    for compression in [
        WebpCompression::Lossless,
        WebpCompression::Lossy { quality: 90 },
    ] {
        for space in [
            ColorSpace::SRGB,
            ColorSpace::DISPLAY_P3,
            ColorSpace::ADOBE_RGB,
            ColorSpace::REC2020,
        ] {
            for keep_alpha in [false, true] {
                let spec = ExportSpec {
                    space,
                    ..webp_spec(compression, keep_alpha)
                };
                let path = temp_path("icc.webp");
                export_pattern(&path, size, &spec);
                let bytes = std::fs::read(&path).unwrap();
                let chunks = riff::chunks(&bytes).unwrap();
                let fourccs: Vec<&[u8; 4]> = chunks.iter().map(|(f, _)| f).collect();
                let case = format!("{compression:?} {space:?} alpha {keep_alpha}");
                assert_eq!(fourccs[..2], [b"VP8X", b"ICCP"], "{case}");
                let flags = chunks[0].1[0];
                assert_eq!(flags & 0x20, 0x20, "{case}: ICC flag");
                assert_eq!(flags & 0x10 != 0, keep_alpha, "{case}: alpha flag");
                let imported = open_image(&path).unwrap();
                assert_eq!(imported.image.format().color_space, space, "{case}");
                assert_eq!(imported.image.format().layout.has_alpha(), keep_alpha);
                std::fs::remove_file(&path).ok();
            }
        }
    }
}

#[test]
fn lossy_is_smaller_than_lossless_and_follows_quality() {
    let size = Size::new(256, 256);
    let path = temp_path("sizes.webp");
    let mut lengths = Vec::new();
    for compression in [
        WebpCompression::Lossless,
        WebpCompression::Lossy { quality: 95 },
        WebpCompression::Lossy { quality: 40 },
    ] {
        export_pattern(&path, size, &webp_spec(compression, false));
        lengths.push(std::fs::metadata(&path).unwrap().len());
    }
    std::fs::remove_file(&path).ok();
    assert!(
        lengths[0] > lengths[1] && lengths[1] > lengths[2],
        "{lengths:?}"
    );
}

#[test]
fn sizes_are_checked_before_anything_is_written() {
    assert!(check_size(Size::new(MAX_SIDE, MAX_SIDE)).is_ok());
    let path = temp_path("too-large.webp");
    for size in [Size::new(MAX_SIDE + 1, 1), Size::new(1, MAX_SIDE + 1)] {
        let never = |_: Rect, _: &mut [f32]| -> Result<u64, String> {
            panic!("the source must not be called")
        };
        let result = export_image(
            &path,
            size,
            &webp_spec(WebpCompression::Lossless, false),
            never,
            &CancelToken::new(),
            &mut |_| {},
        );
        assert_eq!(result.unwrap_err().code(), "tooLarge", "{size:?}");
        assert!(!path.exists());
        assert!(temp_files(&path).is_empty());
    }
}

#[test]
fn cancelling_during_lossy_encoding_stops_libwebp() {
    let size = Size::new(512, 512);
    let spec = webp_spec(WebpCompression::Lossy { quality: 90 }, true);
    let rows = expected(size, &spec);
    let path = temp_path("cancel.webp");
    let cancel = CancelToken::new();
    let file = File::create(&path).unwrap();
    let mut writer =
        WebpLossyWriter::new(file, size, spec.target_format(), 90, cancel.clone()).unwrap();
    for (band, rows) in rows.chunks(256 * 512 * 4).enumerate() {
        writer.write_rows(band as u32 * 256, rows).unwrap();
    }
    cancel.cancel();
    assert_eq!(writer.finish().unwrap_err().code(), "cancelled");
    std::fs::remove_file(&path).unwrap();
}

#[test]
fn cancelling_before_lossless_encoding_stops_it() {
    let size = Size::new(16, 16);
    let spec = webp_spec(WebpCompression::Lossless, false);
    let path = temp_path("cancel-lossless.webp");
    let cancel = CancelToken::new();
    let file = File::create(&path).unwrap();
    let mut writer =
        WebpLosslessWriter::new(file, size, spec.target_format(), cancel.clone()).unwrap();
    writer.write_rows(0, &expected(size, &spec)).unwrap();
    cancel.cancel();
    assert_eq!(writer.finish().unwrap_err().code(), "cancelled");
    std::fs::remove_file(&path).unwrap();
}

#[test]
fn rows_must_come_in_order_and_in_pairs_for_lossy() {
    let size = Size::new(8, 10);
    let target = webp_spec(WebpCompression::Lossy { quality: 90 }, false).target_format();
    let rows = |count: usize| vec![128; count * 8 * 3];
    let path = temp_path("rows.webp");
    let new_writer = || {
        let file = File::create(&path).unwrap();
        WebpLossyWriter::new(file, size, target, 90, CancelToken::new()).unwrap()
    };
    let mut writer = new_writer();
    assert!(writer.write_rows(2, &rows(2)).is_err(), "gap");
    assert!(
        writer.write_rows(0, &rows(3)).is_err(),
        "odd count before the end"
    );
    writer.write_rows(0, &rows(4)).unwrap();
    assert!(writer.write_rows(4, &rows(7)).is_err(), "past the end");
    // The last rows may be odd.
    let mut odd = new_writer();
    odd.write_rows(0, &rows(4)).unwrap();
    odd.write_rows(4, &rows(6)).unwrap();
    odd.finish().unwrap();
    // Finishing early is an error.
    assert_eq!(writer.finish().unwrap_err().code(), "encode");
    std::fs::remove_file(&path).unwrap();
}

#[test]
fn unsupported_targets_are_rejected() {
    let path = temp_path("unsupported.webp");
    let rgb8 = webp_spec(WebpCompression::Lossless, false).target_format();
    let premultiplied = PixelFormat {
        layout: ChannelLayout::Rgba,
        alpha: AlphaMode::Premultiplied,
        ..rgb8
    };
    let rgb16 = PixelFormat {
        sample: SampleType::U16,
        ..rgb8
    };
    let pq = PixelFormat {
        color_space: ColorSpace::REC2100_PQ,
        ..rgb8
    };
    for (format, code) in [
        (premultiplied, "invalidSpec"),
        (rgb16, "invalidSpec"),
        (pq, "unsupportedSpace"),
    ] {
        let file = File::create(&path).unwrap();
        let lossless = WebpLosslessWriter::new(file, Size::new(2, 2), format, CancelToken::new());
        assert_eq!(lossless.err().map(|e| e.code()), Some(code), "{format:?}");
        let file = File::create(&path).unwrap();
        let lossy = WebpLossyWriter::new(file, Size::new(2, 2), format, 90, CancelToken::new());
        assert_eq!(lossy.err().map(|e| e.code()), Some(code), "{format:?}");
    }
    let file = File::create(&path).unwrap();
    let quality = WebpLossyWriter::new(file, Size::new(2, 2), rgb8, 101, CancelToken::new());
    assert_eq!(quality.err().map(|e| e.code()), Some("invalidSpec"));
    std::fs::remove_file(&path).ok();
}

#[test]
fn yuv_conversion_matches_libwebp_levels() {
    // Limited range: black 16, white 235, grays have neutral chroma.
    assert_eq!(rgb_to_y(0, 0, 0), 16);
    assert_eq!(rgb_to_y(255, 255, 255), 235);
    for gray in [0u8, 77, 128, 255] {
        let sum = i32::from(gray) * 4;
        assert_eq!(
            (rgb_to_u(sum, sum, sum), rgb_to_v(sum, sum, sum)),
            (128, 128)
        );
    }
    // Saturated colors reach the chroma extremes without wrapping.
    assert_eq!(rgb_to_u(0, 0, 255 * 4), 240);
    assert_eq!(rgb_to_v(255 * 4, 0, 0), 240);
}

#[test]
fn transparent_pixels_do_not_pull_the_chroma() {
    let red = [255, 0, 0, 255];
    let transparent_black = [0, 0, 0, 0];
    assert_eq!(
        sum_of_four(&[red, transparent_black, transparent_black, transparent_black]),
        [1020, 0, 0]
    );
    // Partial blocks are scaled to four samples.
    assert_eq!(sum_of_four(&[red, red]), [1020, 0, 0]);
    // Fully transparent: plain average.
    assert_eq!(sum_of_four(&[[40, 0, 0, 0], [0, 0, 0, 0]]), [80, 0, 0]);
}

#[test]
fn riff_chunks_are_padded_to_even_sizes() {
    // A minimal "libwebp" file: RIFF with a 3-byte VP8 chunk (padded).
    let mut input = b"RIFF\x0e\0\0\0WEBPVP8 \x03\0\0\0abc\0".to_vec();
    let file = riff::with_icc_profile(&input, 2, 3, b"ICC").unwrap();
    let chunks = riff::chunks(&file).unwrap();
    let fourccs: Vec<&[u8; 4]> = chunks.iter().map(|(f, _)| f).collect();
    assert_eq!(fourccs, [b"VP8X", b"ICCP", b"VP8 "]);
    assert_eq!(chunks[1].1, b"ICC");
    assert_eq!(chunks[2].1, b"abc");
    assert_eq!(chunks[0].1, [0x20, 0, 0, 0, 1, 0, 0, 2, 0, 0]);
    assert_eq!(file.len() % 2, 0);
    let declared = u32::from_le_bytes(file[4..8].try_into().unwrap()) as usize;
    assert_eq!(declared + 8, file.len());
    // Truncated input is an error, not a panic.
    input.truncate(20);
    assert!(riff::with_icc_profile(&input, 2, 3, b"ICC").is_err());
}
