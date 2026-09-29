use std::path::{Path, PathBuf};

use ::exr::image::FlatSamples;
use ::exr::meta::MetaData;
use half::f16;
use slopshop_core::color::ColorSpace;
use slopshop_core::convert::{ConvertOptions, Converter};
use slopshop_core::raster::TILE_SIZE;
use slopshop_core::tile::TileCoord;
use slopshop_core::{CancelToken, RasterImage, Rect};

use super::*;
use crate::export::{
    ExportFormat, ExportNotice, ExportReport, ExportSpec, ExrSample, export_image,
};
use crate::open_image;

fn temp_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("slopshop-exr-{}-{name}", std::process::id()))
}

/// Heights not multiple of the band nor of the block height, widths spanning several tiles.
const ODD_SIZE: Size = Size::new(300, 520);

/// Premultiplied working-space pixel (x, y), with negative and HDR values.
fn source_pixel(x: u32, y: u32, opaque: bool) -> [f32; 4] {
    let a = if opaque {
        1.0
    } else {
        ((x * 7 + y * 3) % 10 + 1) as f32 / 10.0
    };
    [
        (x as f32 * 0.013 - 1.0) * a,
        y as f32 * 0.021 * a,
        (x ^ y) as f32 / 97.0 * a,
        a,
    ]
}

fn export_exr(path: &Path, size: Size, spec: &ExportSpec) -> ExportReport {
    let opaque = !spec.keep_alpha;
    let source = |region: Rect, out: &mut [f32]| {
        for (i, px) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let (x, y) = (i as u32 % region.width, i as u32 / region.width);
            *px = source_pixel(region.x + x, region.y + y, opaque);
        }
        Ok(())
    };
    export_image(path, size, spec, source, &CancelToken::new(), &mut |_| {}).unwrap()
}

fn exr_spec(sample: ExrSample, space: ColorSpace, keep_alpha: bool) -> ExportSpec {
    ExportSpec {
        format: ExportFormat::Exr { sample },
        space,
        keep_alpha,
        dither: false,
    }
}

/// What an F32 file must hold: the converter's output for the source, interleaved.
fn expected_f32(size: Size, spec: &ExportSpec) -> Vec<f32> {
    let options = ConvertOptions {
        dither: false,
        big_endian: false,
    };
    let converter = Converter::new(spec.target_format(), options).unwrap();
    let mut samples = Vec::new();
    for y in 0..size.height {
        let src: Vec<f32> = (0..size.width)
            .flat_map(|x| source_pixel(x, y, !spec.keep_alpha))
            .collect();
        let mut dst = vec![0; size.width as usize * converter.bytes_per_pixel()];
        converter
            .convert_row(&src, 0, y, &mut dst, &mut Default::default())
            .unwrap();
        samples.extend(
            dst.as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b)),
        );
    }
    samples
}

/// Level-0 samples of an imported F32 image (stored as RGBA), interleaved.
fn imported_f32(image: &RasterImage) -> Vec<f32> {
    let size = image.size();
    let mut samples = Vec::new();
    for y in 0..size.height {
        for x in 0..size.width {
            let coord = TileCoord {
                col: x / TILE_SIZE,
                row: y / TILE_SIZE,
            };
            let tile = image.levels()[0].tile(coord).unwrap();
            let i = ((y % TILE_SIZE) * TILE_SIZE + x % TILE_SIZE) as usize * 16;
            samples.extend(
                tile[i..i + 16]
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|b| f32::from_ne_bytes(*b)),
            );
        }
    }
    samples
}

/// Channels of the file as exr's own reader decodes them: name and samples (row-major).
fn read_channels(path: &Path) -> Vec<(String, FlatSamples)> {
    let image = ::exr::prelude::read_all_flat_layers_from_file(path).unwrap();
    assert_eq!(image.layer_data.len(), 1);
    image.layer_data[0]
        .channel_data
        .list
        .iter()
        .map(|c| (c.name.to_string(), c.sample_data.clone()))
        .collect()
}

fn names(channels: &[(String, FlatSamples)]) -> Vec<&str> {
    channels.iter().map(|(name, _)| name.as_str()).collect()
}

/// Interleaved samples of channel `index` (0 = R … 3 = A) out of `per_pixel`.
fn channel(samples: &[f32], index: usize, per_pixel: usize) -> Vec<f32> {
    samples
        .iter()
        .skip(index)
        .step_by(per_pixel)
        .copied()
        .collect()
}

fn bits(samples: &[f32]) -> Vec<u32> {
    samples.iter().map(|v| v.to_bits()).collect()
}

#[test]
fn f32_round_trip_is_exact_and_tagged() {
    for space in [ColorSpace::LINEAR_SRGB, ColorSpace::LINEAR_REC2020] {
        let path = temp_path("f32.exr");
        let spec = exr_spec(ExrSample::F32, space, true);
        let report = export_exr(&path, ODD_SIZE, &spec);
        assert_eq!(report, ExportReport::default(), "{space:?}");
        let expected = expected_f32(ODD_SIZE, &spec);
        if space == ColorSpace::LINEAR_REC2020 {
            // The working space: the file holds the source values themselves.
            let source: Vec<f32> = (0..ODD_SIZE.height)
                .flat_map(|y| (0..ODD_SIZE.width).flat_map(move |x| source_pixel(x, y, false)))
                .collect();
            assert!(bits(&expected) == bits(&source));
        }

        // Our importer: same values, same space, premultiplied.
        let imported = open_image(&path).unwrap();
        let format = imported.image.format();
        assert_eq!(format, spec.target_format(), "{space:?}");
        assert!(imported.warnings.is_empty(), "{space:?}");
        assert!(
            bits(&imported_f32(&imported.image)) == bits(&expected),
            "{space:?}: F32 round trip is not exact"
        );

        // exr's own reader: sorted channels, same values.
        let channels = read_channels(&path);
        assert_eq!(names(&channels), ["A", "B", "G", "R"]);
        for ((_, samples), index) in channels.iter().zip([3, 2, 1, 0]) {
            let FlatSamples::F32(samples) = samples else {
                panic!("{space:?}: not F32");
            };
            assert!(bits(samples) == bits(&channel(&expected, index, 4)));
        }

        // Header: chromaticities of the space, ZIP16 scan lines in increasing order.
        let meta = MetaData::read_from_file(&path, true).unwrap();
        let header = &meta.headers[0];
        let c = header.shared_attributes.chromaticities.unwrap();
        let p = space.primaries;
        let xy = |[x, y]: [f64; 2]| Vec2(x as f32, y as f32);
        assert_eq!(
            [c.red, c.green, c.blue, c.white],
            [xy(p.red), xy(p.green), xy(p.blue), xy(p.white)]
        );
        assert_eq!(header.compression, Compression::ZIP16);
        assert_eq!(header.blocks, BlockDescription::ScanLines);
        assert_eq!(header.line_order, LineOrder::Increasing);
        assert_eq!(header.own_attributes.layer_name, None);
        std::fs::remove_file(&path).ok();
    }
}

#[test]
fn half_floats_report_overflow_and_precision() {
    let size = Size::new(10, 20);
    let path = temp_path("f16.exr");
    let spec = exr_spec(ExrSample::F16, ColorSpace::LINEAR_REC2020, true);
    let report = export_image(
        &path,
        size,
        &spec,
        |_, out| {
            out.as_chunks_mut::<4>().0.fill([1.0e6, 0.1, -2.5, 1.0]);
            Ok(())
        },
        &CancelToken::new(),
        &mut |_| {},
    )
    .unwrap();
    // Red only: the working space needs no matrix.
    assert_eq!(
        report.notices,
        [
            ExportNotice::HalfOverflow(size.pixel_count()),
            ExportNotice::PrecisionReduced
        ]
    );

    let channels = read_channels(&path);
    assert_eq!(names(&channels), ["A", "B", "G", "R"]);
    for ((_, samples), value) in channels.iter().zip([1.0, -2.5, 0.1, 65504.0]) {
        let FlatSamples::F16(samples) = samples else {
            panic!("not F16");
        };
        assert_eq!(samples.len() as u64, size.pixel_count());
        assert!(
            samples.iter().all(|s| *s == f16::from_f32(value)),
            "{value}"
        );
    }
    let imported = open_image(&path).unwrap();
    assert_eq!(
        imported.image.format().color_space,
        ColorSpace::LINEAR_REC2020
    );
    std::fs::remove_file(&path).ok();
}

#[test]
fn rgb_targets_have_no_alpha_channel() {
    let size = Size::new(40, 37);
    let path = temp_path("rgb.exr");
    let spec = exr_spec(ExrSample::F32, ColorSpace::LINEAR_REC2020, false);
    assert_eq!(export_exr(&path, size, &spec), ExportReport::default());
    let expected = expected_f32(size, &spec);

    let channels = read_channels(&path);
    assert_eq!(names(&channels), ["B", "G", "R"]);
    for ((_, samples), index) in channels.iter().zip([2, 1, 0]) {
        let FlatSamples::F32(samples) = samples else {
            panic!("not F32");
        };
        assert!(bits(samples) == bits(&channel(&expected, index, 3)));
    }

    let imported = open_image(&path).unwrap();
    assert_eq!(imported.image.format().layout, ChannelLayout::Rgb);
    let stored = imported_f32(&imported.image);
    for index in 0..3 {
        assert!(bits(&channel(&stored, index, 4)) == bits(&channel(&expected, index, 3)));
    }
    std::fs::remove_file(&path).ok();
}

fn target(layout: ChannelLayout, sample: SampleType, space: ColorSpace) -> PixelFormat {
    PixelFormat {
        layout,
        sample,
        color_space: space,
        alpha: AlphaMode::Premultiplied,
    }
}

#[test]
fn rows_are_regrouped_into_blocks_whatever_the_band_height() {
    // 7-row bands: blocks straddle them, the last one is partial (50 = 3 × 16 + 2).
    let size = Size::new(13, 50);
    let path = temp_path("blocks.exr");
    let format = target(
        ChannelLayout::Rgba,
        SampleType::F16,
        ColorSpace::LINEAR_REC2020,
    );
    let value = |x: u32, y: u32, c: u32| f16::from_f32((x * 1000 + y * 10 + c) as f32);
    let mut writer = ExrWriter::new(File::create(&path).unwrap(), size, format).unwrap();
    let mut y = 0;
    while y < size.height {
        let rows = 7.min(size.height - y);
        let bytes: Vec<u8> = (y..y + rows)
            .flat_map(|y| (0..size.width).flat_map(move |x| (0..4).map(move |c| (x, y, c))))
            .flat_map(|(x, y, c)| value(x, y, c).to_le_bytes())
            .collect();
        writer.write_rows(y, &bytes).unwrap();
        y += rows;
    }
    assert_eq!(writer.finish().unwrap(), []);

    let channels = read_channels(&path);
    assert_eq!(names(&channels), ["A", "B", "G", "R"]);
    for ((_, samples), c) in channels.iter().zip([3, 2, 1, 0]) {
        let FlatSamples::F16(samples) = samples else {
            panic!("not F16");
        };
        let expected: Vec<f16> = (0..size.height)
            .flat_map(|y| (0..size.width).map(move |x| value(x, y, c)))
            .collect();
        assert!(*samples == expected, "channel {c}");
    }
    std::fs::remove_file(&path).ok();
}

#[test]
fn invalid_targets_are_rejected() {
    let path = temp_path("invalid.exr");
    let size = Size::new(4, 4);
    let new = |size: Size, format: PixelFormat| {
        ExrWriter::new(File::create(&path).unwrap(), size, format)
            .map(|_| ())
            .unwrap_err()
            .code()
    };
    let linear = ColorSpace::LINEAR_REC2020;
    let rgba = |sample| target(ChannelLayout::Rgba, sample, linear);
    assert_eq!(new(size, rgba(SampleType::U16)), "invalidSpec");
    let straight = PixelFormat {
        alpha: AlphaMode::Straight,
        ..rgba(SampleType::F32)
    };
    assert_eq!(new(size, straight), "invalidSpec");
    let gray = target(ChannelLayout::Gray, SampleType::F32, linear);
    assert_eq!(new(size, gray), "invalidSpec");
    let srgb = target(ChannelLayout::Rgb, SampleType::F32, ColorSpace::SRGB);
    assert_eq!(new(size, srgb), "unsupportedSpace");
    assert_eq!(
        new(Size::new(1 << 30, 1), rgba(SampleType::F32)),
        "tooLarge"
    );
    std::fs::remove_file(&path).ok();
}

#[test]
fn an_unfinished_writer_releases_the_file() {
    let size = Size::new(3, 40);
    let format = target(
        ChannelLayout::Rgb,
        SampleType::F32,
        ColorSpace::LINEAR_REC2020,
    );
    let row = vec![0u8; 3 * 12];

    // Dropped after one complete block and part of the next one.
    let path = temp_path("dropped.exr");
    let mut writer = ExrWriter::new(File::create(&path).unwrap(), size, format).unwrap();
    writer.write_rows(0, &row.repeat(20)).unwrap();
    drop(writer);
    // Fails on Windows while a handle is still open.
    std::fs::remove_file(&path).unwrap();

    // Rows out of order, then finished early.
    let path = temp_path("early.exr");
    let mut writer = ExrWriter::new(File::create(&path).unwrap(), size, format).unwrap();
    writer.write_rows(0, &row.repeat(16)).unwrap();
    assert_eq!(writer.write_rows(20, &row).unwrap_err().code(), "encode");
    assert_eq!(
        writer.write_rows(16, &row[1..]).unwrap_err().code(),
        "encode"
    );
    assert_eq!(
        writer.write_rows(16, &row.repeat(25)).unwrap_err().code(),
        "encode"
    );
    assert_eq!(writer.finish().unwrap_err().code(), "encode");
    std::fs::remove_file(&path).unwrap();
}
