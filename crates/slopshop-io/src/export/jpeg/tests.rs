use std::path::{Path, PathBuf};

use slopshop_core::BlendSpace;
use slopshop_core::color::{AlphaMode, ColorSpace, WORKING_SPACE, mat_vec};
use slopshop_core::convert::{ConversionReport, ConvertOptions, Converter, WHITE_MATTE};
use slopshop_core::{CancelToken, Rect};

use super::*;
use crate::export::{
    ExportFormat, ExportNotice, ExportSpec, JpegSubsampling, export_image, temp_files,
};
use crate::open_image;

fn temp_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("slopshop-jpeg-{}-{name}", std::process::id()))
}

/// Heights not multiple of the band or of a row of blocks, widths spanning several tiles and
/// not multiple of a block.
const ODD_SIZE: Size = Size::new(301, 523);

/// A deterministic image: smooth gradients (JPEG's home ground) and translucency, within the
/// sRGB gamut (nothing to clip in an sRGB file).
fn pattern(region: Rect, out: &mut [f32]) -> Result<u64, String> {
    let to_working = ColorSpace::LINEAR_SRGB.matrix_to(&WORKING_SPACE);
    for (i, px) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let x = (region.x + i as u32 % region.width) as f32;
        let y = (region.y + i as u32 / region.width) as f32;
        let alpha = 0.6 + 0.4 * (x / 300.0).min(1.0);
        let r = (x / 300.0).min(1.0);
        let g = (y / 520.0).min(1.0);
        let b = 0.5 + 0.4 * ((x + y) / 60.0).sin();
        let [r, g, b] = mat_vec(&to_working, [r, g, b].map(f64::from)).map(|c| c as f32);
        *px = [r * alpha, g * alpha, b * alpha, alpha];
    }
    Ok(0)
}

fn jpeg_spec(quality: u8, subsampling: JpegSubsampling) -> ExportSpec {
    ExportSpec {
        format: ExportFormat::Jpeg {
            quality,
            subsampling,
        },
        space: ColorSpace::SRGB,
        keep_alpha: false,
        matte: WHITE_MATTE,
        dither: false,
        gray: false,
        // A synthetic source, not a document: flattened in linear light, as the expected
        // samples are computed.
        blend_space: BlendSpace::Linear,
        resolution: None,
    }
}

/// The RGB8 samples the encoder receives: the pattern through the export's conversion.
fn expected(size: Size, spec: &ExportSpec) -> Vec<u8> {
    let options = ConvertOptions {
        dither: spec.dither,
        big_endian: false,
        matte: spec.matte,
        blend_space: spec.blend_space,
    };
    let converter = Converter::new(spec.target_format(), options).unwrap();
    let row_bytes = size.width as usize * 3;
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

fn export_pattern(path: &Path, size: Size, spec: &ExportSpec) -> Vec<ExportNotice> {
    export_image(path, size, spec, pattern, &CancelToken::new(), &mut |_| {})
        .unwrap()
        .notices
}

/// Peak signal-to-noise ratio of 8-bit samples, in dB.
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

#[test]
fn round_trips_within_the_quality_asked() {
    for (quality, subsampling, min_psnr) in [
        (100, JpegSubsampling::S444, 45.0),
        (90, JpegSubsampling::S444, 38.0),
        (90, JpegSubsampling::S422, 36.0),
        (85, JpegSubsampling::S420, 34.0),
    ] {
        let case = format!("q{quality} {subsampling:?}");
        let spec = jpeg_spec(quality, subsampling);
        let path = temp_path("round-trip.jpg");
        let notices = export_pattern(&path, ODD_SIZE, &spec);
        // The translucent pattern is flattened over the matte, and reported.
        assert!(
            matches!(notices.as_slice(), [ExportNotice::AlphaFlattened(_)]),
            "{case}: {notices:?}"
        );
        assert!(temp_files(&path).is_empty());

        let decoded = image::open(&path).unwrap();
        assert_eq!(
            (decoded.width(), decoded.height()),
            (ODD_SIZE.width, ODD_SIZE.height)
        );
        let quality_db = psnr(decoded.to_rgb8().as_raw(), &expected(ODD_SIZE, &spec));
        assert!(quality_db > min_psnr, "{case}: PSNR {quality_db:.1} dB");
        std::fs::remove_file(&path).ok();
    }
}

#[test]
fn smaller_settings_give_smaller_files() {
    let size = Size::new(256, 256);
    let path = temp_path("sizes.jpg");
    let mut lengths = Vec::new();
    for (quality, subsampling) in [
        (95, JpegSubsampling::S444),
        (95, JpegSubsampling::S420),
        (60, JpegSubsampling::S420),
    ] {
        export_pattern(&path, size, &jpeg_spec(quality, subsampling));
        lengths.push(std::fs::metadata(&path).unwrap().len());
    }
    std::fs::remove_file(&path).ok();
    assert!(
        lengths[0] > lengths[1] && lengths[1] > lengths[2],
        "{lengths:?}"
    );
}

#[test]
fn icc_profile_reads_back_as_the_same_space() {
    let size = Size::new(24, 16);
    for space in [
        ColorSpace::SRGB,
        ColorSpace::DISPLAY_P3,
        ColorSpace::ADOBE_RGB,
        ColorSpace::PROPHOTO,
        ColorSpace::REC2020,
        ColorSpace::LINEAR_SRGB,
    ] {
        let spec = ExportSpec {
            space,
            ..jpeg_spec(90, JpegSubsampling::S444)
        };
        let path = temp_path("icc.jpg");
        export_pattern(&path, size, &spec);
        let imported = open_image(&path).unwrap();
        assert_eq!(imported.image.format().color_space, space, "{space:?}");
        assert!(imported.warnings.is_empty(), "{space:?}");
        std::fs::remove_file(&path).ok();
    }
}

#[test]
fn rows_must_come_in_order() {
    let size = Size::new(64, 300);
    let target = jpeg_spec(90, JpegSubsampling::S420).target_format();
    let rows = |count: usize| vec![128; count * 64 * 3];
    let path = temp_path("rows.jpg");
    let new_writer = || {
        let file = File::create(&path).unwrap();
        JpegWriter::new(file, size, target, 90, JpegSubsampling::S420, None).unwrap()
    };

    let mut writer = new_writer();
    assert!(writer.write_rows(10, &rows(10)).is_err());
    writer.write_rows(0, &rows(256)).unwrap();
    assert!(writer.write_rows(256, &[0; 10]).is_err());
    assert!(writer.write_rows(256, &rows(45)).is_err());
    writer.write_rows(256, &rows(44)).unwrap();
    writer.finish().unwrap();
    assert!(image::open(&path).is_ok());

    // Finishing early is an error, and closes the file so that it can be deleted.
    let mut writer = new_writer();
    writer.write_rows(0, &rows(256)).unwrap();
    assert_eq!(writer.finish().unwrap_err().code(), "encode");
    std::fs::remove_file(&path).unwrap();

    // So does dropping the writer, even before any row.
    drop(new_writer());
    std::fs::remove_file(&path).unwrap();
    let mut writer = new_writer();
    writer.write_rows(0, &rows(256)).unwrap();
    drop(writer);
    std::fs::remove_file(&path).unwrap();
}

#[test]
fn cancelling_a_jpeg_export_leaves_no_file() {
    let path = temp_path("cancel.jpg");
    let cancel = CancelToken::new();
    let result = export_image(
        &path,
        Size::new(16, 1000),
        &jpeg_spec(90, JpegSubsampling::S444),
        pattern,
        &cancel,
        // Cancel after the first band.
        &mut |_| cancel.cancel(),
    );
    assert_eq!(result.unwrap_err().code(), "cancelled");
    assert!(!path.exists());
    assert!(temp_files(&path).is_empty());
}

#[test]
fn a_failing_source_keeps_the_existing_file() {
    let path = temp_path("failing.jpg");
    std::fs::write(&path, b"previous").unwrap();
    let mut calls = 0;
    let source = move |region: Rect, out: &mut [f32]| {
        calls += 1;
        if calls > 1 {
            return Err("render failed".to_owned());
        }
        pattern(region, out)
    };
    let result = export_image(
        &path,
        Size::new(16, 1000),
        &jpeg_spec(90, JpegSubsampling::S444),
        source,
        &CancelToken::new(),
        &mut |_| {},
    );
    assert_eq!(result.unwrap_err().code(), "source");
    assert_eq!(std::fs::read(&path).unwrap(), b"previous");
    assert!(temp_files(&path).is_empty());
    std::fs::remove_file(&path).ok();
}

#[test]
fn sizes_are_checked_before_anything_is_written() {
    assert!(check_size(Size::new(MAX_SIDE, MAX_SIDE)).is_ok());
    let path = temp_path("too-large.jpg");
    for size in [Size::new(MAX_SIDE + 1, 1), Size::new(1, MAX_SIDE + 1)] {
        let never = |_: Rect, _: &mut [f32]| -> Result<u64, String> {
            panic!("the source must not be called")
        };
        let result = export_image(
            &path,
            size,
            &jpeg_spec(90, JpegSubsampling::S444),
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
fn alpha_is_refused_rather_than_dropped() {
    let path = temp_path("alpha.jpg");
    let spec = ExportSpec {
        keep_alpha: true,
        ..jpeg_spec(90, JpegSubsampling::S444)
    };
    let result = export_image(
        &path,
        Size::new(8, 8),
        &spec,
        pattern,
        &CancelToken::new(),
        &mut |_| {},
    );
    assert_eq!(result.unwrap_err().code(), "invalidSpec");
    assert!(!path.exists());
}

#[test]
fn unsupported_targets_are_rejected() {
    let path = temp_path("unsupported.jpg");
    let rgb8 = jpeg_spec(90, JpegSubsampling::S444).target_format();
    let rgba8 = PixelFormat {
        layout: ChannelLayout::Rgba,
        alpha: AlphaMode::Straight,
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
    for (format, size, quality, code) in [
        (rgba8, Size::new(2, 2), 90, "invalidSpec"),
        (rgb16, Size::new(2, 2), 90, "invalidSpec"),
        (pq, Size::new(2, 2), 90, "unsupportedSpace"),
        (rgb8, Size::new(0, 2), 90, "invalidSpec"),
        (rgb8, Size::new(2, 2), 0, "invalidSpec"),
        (rgb8, Size::new(2, 2), 101, "invalidSpec"),
    ] {
        let file = File::create(&path).unwrap();
        let result = JpegWriter::new(file, size, format, quality, JpegSubsampling::S444, None);
        assert_eq!(
            result.err().map(|e| e.code()),
            Some(code),
            "{format:?} q{quality}"
        );
    }
    std::fs::remove_file(&path).ok();
}

#[test]
fn small_images_encode_in_memory() {
    let size = Size::new(40, 24);
    let rgb: Vec<u8> = (0..size.height)
        .flat_map(|y| (0..size.width).flat_map(move |x| [(x * 6) as u8, (y * 10) as u8, 128]))
        .collect();
    let bytes = encode_srgb8(&rgb, size, 95).unwrap();
    let mut decoder = zune_jpeg::JpegDecoder::new(zune_core::bytestream::ZCursor::new(&bytes));
    let decoded = decoder.decode().unwrap();
    assert_eq!(decoder.dimensions(), Some((40, 24)));
    assert!(psnr(&rgb, &decoded) > 35.0, "{}", psnr(&rgb, &decoded));
    assert!(encode_srgb8(&rgb[3..], size, 95).is_err());
}
