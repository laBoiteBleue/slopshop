use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;

use slopshop_core::color::LinearRgba;
use slopshop_core::composite::composite_region;
use slopshop_core::document::{Layer, LayerId};
use slopshop_core::{Edit, RasterImage};

use super::*;
use crate::open_image;

fn temp_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("slopshop-export-{}-{name}", std::process::id()))
}

fn temp_of(path: &Path) -> PathBuf {
    TempFile::new(path).unwrap().path.clone()
}

fn push_layer(doc: &mut Document, content: LayerContent, opacity: f32) -> LayerId {
    let id = doc.allocate_layer_id();
    let index = doc.layers().len();
    let layer = Layer {
        id,
        name: "layer".into(),
        visible: true,
        opacity,
        content,
    };
    Edit::InsertLayer { index, layer }.apply(doc).unwrap();
    id
}

fn raster_document(image: RasterImage) -> Document {
    let mut doc = Document::new(image.size());
    let image = Arc::new(image);
    push_layer(&mut doc, LayerContent::Raster { image }, 1.0);
    doc
}

fn fill_document(size: Size, color: LinearRgba) -> Document {
    let mut doc = Document::new(size);
    push_layer(&mut doc, LayerContent::Fill { color }, 1.0);
    doc
}

fn png_spec(depth: PngDepth, space: ColorSpace, keep_alpha: bool) -> ExportSpec {
    ExportSpec {
        format: ExportFormat::Png {
            depth,
            compression: PngCompression::Fast,
        },
        space,
        keep_alpha,
        dither: true,
    }
}

/// Export `doc` with the CPU compositor as the source.
fn export(doc: &Document, path: &Path, spec: &ExportSpec) -> Result<ExportReport, ExportError> {
    let source = |region: Rect, out: &mut [f32]| {
        composite_region(doc, region, out)
            .map(|_| ())
            .map_err(|e| e.to_string())
    };
    export_image(
        path,
        doc.size(),
        spec,
        source,
        &CancelToken::new(),
        &mut |_| {},
    )
}

/// Write an image with the `image` crate, import it, export it again and decode the result.
fn round_trip(
    name: &str,
    input: image::DynamicImage,
    spec: &ExportSpec,
) -> (image::DynamicImage, ExportReport) {
    let source_path = temp_path(&format!("{name}-in.png"));
    input.save(&source_path).unwrap();
    let imported = open_image(&source_path).unwrap();
    std::fs::remove_file(&source_path).ok();
    assert_eq!(imported.image.format().color_space, ColorSpace::SRGB);
    let doc = raster_document(imported.image);

    let path = temp_path(&format!("{name}-out.png"));
    let report = export(&doc, &path, spec).unwrap();
    assert!(!temp_of(&path).exists());
    let output = image::open(&path).unwrap();
    // Our importer reads the tag back.
    let reimported = open_image(&path).unwrap();
    assert_eq!(reimported.image.format().color_space, spec.space);
    std::fs::remove_file(&path).ok();
    (output, report)
}

/// Heights not multiple of the band height, widths spanning several tiles.
const ODD_SIZE: Size = Size::new(300, 520);

#[test]
fn png_8_bit_round_trip_is_bit_exact_with_dither() {
    let (w, h) = (ODD_SIZE.width, ODD_SIZE.height);
    let rgb = image::RgbImage::from_fn(w, h, |x, y| {
        image::Rgb([(x * 7 + y) as u8, (y * 3) as u8, (x ^ y) as u8])
    });
    let spec = png_spec(PngDepth::U8, ColorSpace::SRGB, false);
    let (output, report) = round_trip("rgb8", rgb.clone().into(), &spec);
    assert_eq!(output.color(), image::ColorType::Rgb8);
    assert!(output.to_rgb8() == rgb, "8-bit RGB is not bit-exact");
    assert_eq!(report, ExportReport::default());

    // Straight alpha, every alpha > 0 (color under alpha 0 is not kept, by design).
    let rgba = image::RgbaImage::from_fn(w, h, |x, y| {
        image::Rgba([
            (x * 5 + y) as u8,
            (y * 11) as u8,
            (x + 2 * y) as u8,
            1 + ((x * 13 + y * 7) % 255) as u8,
        ])
    });
    let spec = png_spec(PngDepth::U8, ColorSpace::SRGB, true);
    let (output, report) = round_trip("rgba8", rgba.clone().into(), &spec);
    assert_eq!(output.color(), image::ColorType::Rgba8);
    assert!(output.to_rgba8() == rgba, "8-bit RGBA is not bit-exact");
    assert_eq!(report, ExportReport::default());
}

#[test]
fn png_16_bit_round_trip_is_bit_exact() {
    let rgba = image::ImageBuffer::<image::Rgba<u16>, _>::from_fn(260, 300, |x, y| {
        image::Rgba([
            (x * 251 + y * 13) as u16,
            (y * 211) as u16,
            ((x * y) % 65536) as u16,
            1 + ((x * 257 + y * 97) % 65535) as u16,
        ])
    });
    let spec = png_spec(PngDepth::U16, ColorSpace::SRGB, true);
    let (output, report) = round_trip("rgba16", rgba.clone().into(), &spec);
    assert_eq!(output.color(), image::ColorType::Rgba16);
    assert!(output.to_rgba16() == rgba, "16-bit RGBA is not bit-exact");
    assert_eq!(report, ExportReport::default());
}

/// Chunks of a PNG file, as the png decoder reports them.
fn png_tags(path: &Path) -> (Option<[u8; 2]>, bool, bool) {
    let file = std::io::BufReader::new(File::open(path).unwrap());
    let reader = ::png::Decoder::new(file).read_info().unwrap();
    let info = reader.info();
    (
        info.coding_independent_code_points
            .map(|c| [c.color_primaries, c.transfer_function]),
        info.icc_profile.is_some(),
        info.srgb.is_some(),
    )
}

#[test]
fn png_color_tags_read_back() {
    let doc = fill_document(Size::new(8, 4), LinearRgba::new(0.2, 0.3, 0.1, 1.0));
    // Space, expected cICP code points, expected iCCP.
    let cases = [
        (ColorSpace::DISPLAY_P3, Some([12, 13]), true),
        (ColorSpace::REC2020, Some([9, 1]), true),
        (ColorSpace::LINEAR_REC2020, Some([9, 8]), true),
        (ColorSpace::REC2100_PQ, Some([9, 16]), false),
        (ColorSpace::REC2100_HLG, Some([9, 18]), false),
        (ColorSpace::ADOBE_RGB, None, true),
        (ColorSpace::PROPHOTO, None, true),
    ];
    for (space, cicp, icc) in cases {
        let path = temp_path("tags.png");
        export(&doc, &path, &png_spec(PngDepth::U16, space, false)).unwrap();
        assert_eq!(png_tags(&path), (cicp, icc, false), "{space:?}");
        let imported = open_image(&path).unwrap();
        assert_eq!(imported.image.format().color_space, space);
        assert!(imported.warnings.is_empty(), "{space:?}");
        std::fs::remove_file(&path).ok();
    }
    let path = temp_path("tags-srgb.png");
    export(
        &doc,
        &path,
        &png_spec(PngDepth::U8, ColorSpace::SRGB, false),
    )
    .unwrap();
    assert_eq!(png_tags(&path), (None, false, true));
    std::fs::remove_file(&path).ok();
}

#[test]
fn hdr_content_to_8_bit_reports_clipping() {
    let size = Size::new(10, 3);
    let doc = fill_document(size, LinearRgba::new(4.0, 4.0, 4.0, 1.0));
    let path = temp_path("hdr.png");
    let report = export(&doc, &path, &png_spec(PngDepth::U8, ColorSpace::SRGB, true)).unwrap();
    std::fs::remove_file(&path).ok();
    // R, G and B of every pixel.
    assert_eq!(
        report.notices,
        [ExportNotice::ClippedHigh(size.pixel_count() * 3)]
    );
    assert_eq!(report.notices[0].id(), "clippedHigh");
    assert_eq!(report.notices[0].count(), Some(90));
}

#[test]
fn progress_is_monotonic_and_ends_at_the_total() {
    let doc = fill_document(Size::new(5, 600), LinearRgba::new(0.5, 0.5, 0.5, 1.0));
    let path = temp_path("progress.png");
    let mut events = Vec::new();
    export_image(
        &path,
        doc.size(),
        &png_spec(PngDepth::U8, ColorSpace::SRGB, false),
        |region, out| {
            composite_region(&doc, region, out)
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
        &CancelToken::new(),
        &mut |p| events.push(p),
    )
    .unwrap();
    std::fs::remove_file(&path).ok();
    let done: Vec<u64> = events.iter().map(|p| p.done).collect();
    assert_eq!(done, [256, 512, 600]);
    assert!(events.iter().all(|p| p.total == 600));
}

#[test]
fn bands_are_full_width_and_aligned() {
    let size = Size::new(7, 513);
    let mut regions = Vec::new();
    let path = temp_path("bands.png");
    export_image(
        &path,
        size,
        &png_spec(PngDepth::U8, ColorSpace::SRGB, false),
        |region, out| {
            regions.push(region);
            out.fill(0.25);
            Ok(())
        },
        &CancelToken::new(),
        &mut |_| {},
    )
    .unwrap();
    std::fs::remove_file(&path).ok();
    assert_eq!(
        regions,
        [
            Rect::new(0, 0, 7, 256),
            Rect::new(0, 256, 7, 256),
            Rect::new(0, 512, 7, 1),
        ]
    );
}

#[test]
fn cancelling_leaves_neither_file_nor_temp_file() {
    let doc = fill_document(Size::new(16, 1000), LinearRgba::new(0.5, 0.5, 0.5, 1.0));
    let path = temp_path("cancel.png");
    let cancel = CancelToken::new();
    let result = export_image(
        &path,
        doc.size(),
        &png_spec(PngDepth::U8, ColorSpace::SRGB, false),
        |region, out| {
            composite_region(&doc, region, out)
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
        &cancel,
        // Cancel after the first band.
        &mut |_| cancel.cancel(),
    );
    assert_eq!(result.unwrap_err().code(), "cancelled");
    assert!(!path.exists());
    assert!(!temp_of(&path).exists());
}

#[test]
fn a_failing_source_keeps_the_existing_file() {
    let path = temp_path("source-error.png");
    std::fs::write(&path, b"previous").unwrap();
    let mut calls = 0;
    let result = export_image(
        &path,
        Size::new(4, 600),
        &png_spec(PngDepth::U8, ColorSpace::SRGB, false),
        |_, out| {
            calls += 1;
            if calls == 2 {
                return Err("device lost".to_owned());
            }
            out.fill(0.5);
            Ok(())
        },
        &CancelToken::new(),
        &mut |_| {},
    );
    let error = result.unwrap_err();
    assert_eq!(error.code(), "source");
    assert_eq!(error.to_string(), "device lost");
    assert_eq!(std::fs::read(&path).unwrap(), b"previous");
    assert!(!temp_of(&path).exists());
    std::fs::remove_file(&path).ok();
}

#[test]
fn a_panicking_source_is_an_error() {
    let path = temp_path("source-panic.png");
    let result = export_image(
        &path,
        Size::new(4, 4),
        &png_spec(PngDepth::U8, ColorSpace::SRGB, false),
        |_, _| panic!("source bug"),
        &CancelToken::new(),
        &mut |_| {},
    );
    assert_eq!(result.unwrap_err().code(), "source");
    assert!(!path.exists());
    assert!(!temp_of(&path).exists());
}

#[test]
fn export_replaces_an_existing_file() {
    let doc = fill_document(Size::new(3, 2), LinearRgba::new(1.0, 1.0, 1.0, 1.0));
    let path = temp_path("replace.png");
    std::fs::write(&path, b"previous").unwrap();
    export(
        &doc,
        &path,
        &png_spec(PngDepth::U8, ColorSpace::SRGB, false),
    )
    .unwrap();
    let output = image::open(&path).unwrap().to_rgb8();
    assert!(output.pixels().all(|p| p.0 == [255, 255, 255]));
    assert!(!temp_of(&path).exists());
    std::fs::remove_file(&path).ok();
}

#[test]
fn unsupported_spaces_are_rejected_before_writing() {
    let path = temp_path("unsupported.exr");
    let cases = [
        (
            ExportFormat::Exr {
                sample: ExrSample::F32,
            },
            ColorSpace::SRGB,
        ),
        (
            ExportFormat::Tiff {
                sample: TiffSample::U16,
                compression: TiffCompression::Deflate,
            },
            ColorSpace::REC2100_PQ,
        ),
    ];
    for (format, space) in cases {
        let spec = ExportSpec {
            format,
            space,
            keep_alpha: true,
            dither: false,
        };
        let result = export_image(
            &path,
            Size::new(4, 4),
            &spec,
            |_, _| panic!("the source must not be called"),
            &CancelToken::new(),
            &mut |_| panic!("no progress expected"),
        );
        assert!(matches!(result, Err(ExportError::UnsupportedSpace(s)) if s == space));
        assert!(!path.exists());
        assert!(!temp_of(&path).exists());
    }
}

#[test]
fn invalid_requests_are_rejected() {
    let spec = png_spec(PngDepth::U8, ColorSpace::SRGB, false);
    let run = |path: &Path, size: Size| {
        export_image(
            path,
            size,
            &spec,
            |_, _| Ok(()),
            &CancelToken::new(),
            &mut |_| {},
        )
        .unwrap_err()
        .code()
    };
    assert_eq!(run(&temp_path("empty.png"), Size::new(0, 5)), "invalidSpec");
    assert_eq!(run(Path::new("/"), Size::new(1, 1)), "invalidSpec");
    assert_eq!(
        run(&temp_path("huge.png"), Size::new(1 << 31, 1)),
        "tooLarge"
    );
}

#[test]
fn space_support_per_format() {
    use ExportFormatKind::{Exr, Png, Tiff};
    for space in [
        ColorSpace::SRGB,
        ColorSpace::DISPLAY_P3,
        ColorSpace::ADOBE_RGB,
        ColorSpace::PROPHOTO,
        ColorSpace::REC2020,
        ColorSpace::LINEAR_REC2020,
        ColorSpace::LINEAR_SRGB,
        ColorSpace::REC2100_PQ,
        ColorSpace::REC2100_HLG,
    ] {
        assert!(supports_space(Png, &space), "{space:?}");
        let hdr = matches!(space.transfer, TransferFunction::Pq | TransferFunction::Hlg);
        assert_eq!(supports_space(Tiff, &space), !hdr, "{space:?}");
        assert_eq!(supports_space(Exr, &space), space.is_linear(), "{space:?}");
    }
}

#[test]
fn target_formats_follow_the_format_conventions() {
    let spec = |format| ExportSpec {
        format,
        space: ColorSpace::LINEAR_SRGB,
        keep_alpha: true,
        dither: false,
    };
    let alpha = |format| spec(format).target_format().alpha;
    let tiff = |sample| ExportFormat::Tiff {
        sample,
        compression: TiffCompression::None,
    };
    assert_eq!(alpha(tiff(TiffSample::U16)), AlphaMode::Straight);
    assert_eq!(alpha(tiff(TiffSample::F32)), AlphaMode::Premultiplied);
    let half = ExportFormat::Exr {
        sample: ExrSample::F16,
    };
    assert_eq!(alpha(half), AlphaMode::Premultiplied);
    let target = spec(half).target_format();
    assert_eq!(
        (target.layout, target.sample),
        (ChannelLayout::Rgba, SampleType::F16)
    );
    let opaque = ExportSpec {
        keep_alpha: false,
        ..spec(half)
    };
    assert_eq!(opaque.target_format().layout, ChannelLayout::Rgb);
    assert_eq!(
        ExportReport::new(&half, &ConversionReport::default()).notices,
        [ExportNotice::PrecisionReduced]
    );
}

fn raster(size: Size, layout: ChannelLayout, sample: SampleType, space: ColorSpace) -> RasterImage {
    let format = PixelFormat {
        layout,
        sample,
        color_space: space,
        alpha: AlphaMode::Straight,
    };
    let len = size.pixel_count() as usize * format.bytes_per_pixel() as usize;
    RasterImage::from_pixels(size, format, &vec![0; len]).unwrap()
}

#[test]
fn default_specs_follow_the_sources() {
    use ChannelLayout::{Rgb, Rgba};
    use ExportFormatKind::{Exr, Png, Tiff};
    let size = Size::new(4, 4);

    // An opaque 8-bit sRGB photo.
    let doc = raster_document(raster(size, Rgb, SampleType::U8, ColorSpace::SRGB));
    let png = default_spec(Png, &doc);
    assert_eq!(png, png_spec(PngDepth::U8, ColorSpace::SRGB, false));
    let tiff = default_spec(Tiff, &doc);
    assert_eq!(
        (tiff.format, tiff.space, tiff.keep_alpha),
        (
            ExportFormat::Tiff {
                sample: TiffSample::U8,
                compression: TiffCompression::Deflate
            },
            ColorSpace::SRGB,
            false
        )
    );
    let exr = default_spec(Exr, &doc);
    assert_eq!(
        (exr.format, exr.space, exr.dither),
        (
            ExportFormat::Exr {
                sample: ExrSample::F32
            },
            ColorSpace::LINEAR_SRGB,
            false
        )
    );

    // 16-bit Display P3 with alpha: kept in P3 at 16 bits.
    let doc = raster_document(raster(size, Rgba, SampleType::U16, ColorSpace::DISPLAY_P3));
    let png = default_spec(Png, &doc);
    assert_eq!(png, png_spec(PngDepth::U16, ColorSpace::DISPLAY_P3, true));
    assert_eq!(default_spec(Tiff, &doc).space, ColorSpace::DISPLAY_P3);

    // 16-bit PQ: PNG can tag it (cICP), TIFF cannot.
    let doc = raster_document(raster(size, Rgb, SampleType::U16, ColorSpace::REC2100_PQ));
    assert_eq!(default_spec(Png, &doc).space, ColorSpace::REC2100_PQ);
    assert_eq!(default_spec(Tiff, &doc).space, ColorSpace::REC2020);

    // Float: 32-bit float TIFF in the source space.
    let doc = raster_document(raster(size, Rgba, SampleType::F16, ColorSpace::LINEAR_SRGB));
    let tiff = default_spec(Tiff, &doc);
    assert_eq!(tiff.format.sample_type(), SampleType::F32);
    assert_eq!(tiff.space, ColorSpace::LINEAR_SRGB);
    assert_eq!(
        default_spec(Png, &doc).format.sample_type(),
        SampleType::U16
    );

    // Mixed sources: no unique space.
    let mut doc = raster_document(raster(size, Rgb, SampleType::U8, ColorSpace::SRGB));
    let p3 = Arc::new(raster(size, Rgba, SampleType::F32, ColorSpace::DISPLAY_P3));
    push_layer(&mut doc, LayerContent::Raster { image: p3 }, 1.0);
    let tiff = default_spec(Tiff, &doc);
    assert_eq!(
        (tiff.format.sample_type(), tiff.space),
        (SampleType::F32, ColorSpace::LINEAR_REC2020)
    );
    assert_eq!(default_spec(Png, &doc).space, ColorSpace::SRGB);
    let mut doc = raster_document(raster(size, Rgb, SampleType::U8, ColorSpace::SRGB));
    let wide = Arc::new(raster(size, Rgba, SampleType::U16, ColorSpace::ADOBE_RGB));
    push_layer(&mut doc, LayerContent::Raster { image: wide }, 1.0);
    let tiff = default_spec(Tiff, &doc);
    assert_eq!(
        (tiff.format.sample_type(), tiff.space),
        (SampleType::U16, ColorSpace::REC2020)
    );
    // Still opaque: the bottom layer covers the canvas.
    assert!(!tiff.keep_alpha);
}

#[test]
fn alpha_is_dropped_only_for_structurally_opaque_documents() {
    use ChannelLayout::{Rgb, Rgba};
    let size = Size::new(4, 4);
    let keeps_alpha = |doc: &Document| default_spec(ExportFormatKind::Png, doc).keep_alpha;

    let opaque = LinearRgba::new(0.1, 0.2, 0.3, 1.0);
    assert!(!keeps_alpha(&fill_document(size, opaque)));
    let translucent = LinearRgba::new(0.1, 0.2, 0.3, 0.5);
    assert!(keeps_alpha(&fill_document(size, translucent)));
    assert!(keeps_alpha(&Document::new(size)));

    // An opaque fill at partial opacity.
    let mut doc = Document::new(size);
    push_layer(&mut doc, LayerContent::Fill { color: opaque }, 0.5);
    assert!(keeps_alpha(&doc));

    // A hidden or fully transparent bottom layer does not count.
    let mut doc = Document::new(size);
    let hidden = push_layer(&mut doc, LayerContent::Fill { color: translucent }, 1.0);
    Edit::SetLayerVisible {
        id: hidden,
        visible: false,
    }
    .apply(&mut doc)
    .unwrap();
    push_layer(&mut doc, LayerContent::Fill { color: translucent }, 0.0);
    push_layer(&mut doc, LayerContent::Fill { color: opaque }, 1.0);
    assert!(!keeps_alpha(&doc));

    // Rasters: with an alpha channel, or smaller than the canvas.
    let rgba = raster(size, Rgba, SampleType::U8, ColorSpace::SRGB);
    assert!(keeps_alpha(&raster_document(rgba)));
    let mut doc = Document::new(Size::new(8, 4));
    let small = Arc::new(raster(size, Rgb, SampleType::U8, ColorSpace::SRGB));
    push_layer(&mut doc, LayerContent::Raster { image: small }, 1.0);
    assert!(keeps_alpha(&doc));
}
