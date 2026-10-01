use std::fs::File;
use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Barrier};

use slopshop_core::BlendMode;
use slopshop_core::color::LinearRgba;
use slopshop_core::composite::composite_region;
use slopshop_core::document::{Layer, LayerId};
use slopshop_core::{Edit, RasterImage};

use super::*;
use crate::open_image;

fn temp_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("slopshop-export-{}-{name}", std::process::id()))
}

fn push_layer(doc: &mut Document, content: LayerContent, opacity: f32) -> LayerId {
    let id = doc.allocate_layer_id();
    let index = doc.layers().len();
    let layer = Layer {
        transform: slopshop_core::Affine::IDENTITY,
        clipped: false,
        id,
        name: "layer".into(),
        visible: true,
        opacity,
        blend_mode: BlendMode::Normal,
        mask: None,
        content,
    };
    Edit::InsertLayer {
        parent: None,
        index,
        layer,
    }
    .apply(doc)
    .unwrap();
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
        matte: WHITE_MATTE,
        dither: true,
        gray: false,
        blend_space: BlendSpace::default(),
    }
}

/// Export `doc` with the CPU compositor as the source.
fn export(doc: &Document, path: &Path, spec: &ExportSpec) -> Result<ExportReport, ExportError> {
    let source = |region: Rect, out: &mut [f32]| {
        composite_region(doc, region, out)
            .map(|report| report.non_finite)
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
    assert!(temp_files(&path).is_empty());
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
                .map(|report| report.non_finite)
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
            Ok(0)
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
                .map(|report| report.non_finite)
                .map_err(|e| e.to_string())
        },
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
            Ok(0)
        },
        &CancelToken::new(),
        &mut |_| {},
    );
    let error = result.unwrap_err();
    assert_eq!(error.code(), "source");
    assert_eq!(error.to_string(), "device lost");
    assert_eq!(std::fs::read(&path).unwrap(), b"previous");
    assert!(temp_files(&path).is_empty());
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
    assert!(temp_files(&path).is_empty());
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
    assert!(temp_files(&path).is_empty());
    std::fs::remove_file(&path).ok();
}

/// Size of the concurrent exports: three bands.
const RACE_SIZE: Size = Size::new(4, 600);

/// A gray source for the concurrent exports. Its first call waits at `start` (so that both
/// exports have created their temporary file), records how many temporary files exist, and
/// waits again (so that both have looked); its call for the second band runs
/// `on_second_band`; its call for the last band waits for `last_band` (if any).
fn gated_gray<'a>(
    gray: f32,
    path: &'a Path,
    start: &'a Barrier,
    seen: &'a AtomicUsize,
    last_band: Option<mpsc::Receiver<()>>,
    on_second_band: impl Fn() + Send + 'a,
) -> impl FnMut(Rect, &mut [f32]) -> Result<u64, String> + Send + 'a {
    move |region, out| {
        match region.y {
            0 => {
                start.wait();
                seen.fetch_max(temp_files(path).len(), Ordering::SeqCst);
                start.wait();
            }
            BAND_ROWS => on_second_band(),
            _ => {
                if let Some(go) = &last_band {
                    go.recv().map_err(|e| e.to_string())?;
                }
            }
        }
        out.as_chunks_mut::<4>().0.fill([gray, gray, gray, 1.0]);
        Ok(0)
    }
}

/// The 8-bit sRGB code of a linear gray.
fn gray_code_of(gray: f32) -> u8 {
    (TransferFunction::Srgb.encode(gray) * 255.0).round() as u8
}

/// The single gray code of an 8-bit gray PNG made by `gated_gray`.
fn gray_code(path: &Path) -> u8 {
    let pixels = image::open(path).unwrap().to_rgb8();
    let first = pixels.get_pixel(0, 0).0[0];
    assert!(pixels.pixels().all(|p| p.0 == [first; 3]), "mixed content");
    first
}

fn race_spec() -> ExportSpec {
    ExportSpec {
        dither: false,
        ..png_spec(PngDepth::U8, ColorSpace::SRGB, false)
    }
}

fn race_export(
    path: &Path,
    spec: &ExportSpec,
    source: impl FnMut(Rect, &mut [f32]) -> Result<u64, String> + Send,
    cancel: &CancelToken,
) -> Result<ExportReport, ExportError> {
    export_image(path, RACE_SIZE, spec, source, cancel, &mut |_| {})
}

#[test]
fn concurrent_exports_to_one_path_write_separate_files() {
    // Regression: both exports used `<name>.slopshop-tmp`. The second truncated the first's
    // file, the first renamed it (reporting success for what became the second's pixels), and
    // the second then failed.
    let path = temp_path("race.png");
    let spec = race_spec();
    let (start, seen) = (Barrier::new(2), AtomicUsize::new(0));
    thread::scope(|scope| {
        // Dropped if an assertion fails here, which stops the waiting export instead of
        // leaving the scope waiting for it.
        let (go, wait) = mpsc::channel();
        let first = gated_gray(0.2, &path, &start, &seen, None, || {});
        let second = gated_gray(0.6, &path, &start, &seen, Some(wait), || {});
        let (path, spec) = (&path, &spec);
        let first = scope.spawn(move || race_export(path, spec, first, &CancelToken::new()));
        let second = scope.spawn(move || race_export(path, spec, second, &CancelToken::new()));

        // The first finishes while the second is still writing: the file is the first's.
        assert_eq!(first.join().unwrap().unwrap(), ExportReport::default());
        assert_eq!(gray_code(path), gray_code_of(0.2));
        // Then the second finishes and replaces it, whole.
        go.send(()).unwrap();
        assert_eq!(second.join().unwrap().unwrap(), ExportReport::default());
        assert_eq!(gray_code(path), gray_code_of(0.6));
    });
    assert_eq!(seen.load(Ordering::SeqCst), 2, "one temporary file each");
    assert!(temp_files(&path).is_empty());
    std::fs::remove_file(&path).ok();
}

#[test]
fn cancelling_one_of_two_exports_to_one_path_spares_the_other() {
    // Regression: the cancelled export deleted the shared temporary file, and the other one
    // failed.
    let path = temp_path("race-cancel.png");
    std::fs::write(&path, b"previous").unwrap();
    let spec = race_spec();
    let (start, seen) = (Barrier::new(2), AtomicUsize::new(0));
    let cancel = CancelToken::new();
    thread::scope(|scope| {
        // See `concurrent_exports_to_one_path_write_separate_files`.
        let (go, wait) = mpsc::channel();
        let kept = gated_gray(0.2, &path, &start, &seen, Some(wait), || {});
        let cancelled = gated_gray(0.6, &path, &start, &seen, None, || cancel.cancel());
        let kept = scope.spawn(|| race_export(&path, &spec, kept, &CancelToken::new()));
        let cancelled = scope.spawn(|| race_export(&path, &spec, cancelled, &cancel));

        assert_eq!(cancelled.join().unwrap().unwrap_err().code(), "cancelled");
        // The destination is untouched, the other export still has its temporary file.
        assert_eq!(std::fs::read(&path).unwrap(), b"previous");
        assert_eq!(temp_files(&path).len(), 1);
        go.send(()).unwrap();
        assert_eq!(kept.join().unwrap().unwrap(), ExportReport::default());
    });
    assert_eq!(seen.load(Ordering::SeqCst), 2, "one temporary file each");
    assert_eq!(gray_code(&path), gray_code_of(0.2));
    assert!(temp_files(&path).is_empty());
    std::fs::remove_file(&path).ok();
}

#[test]
fn existing_temporary_files_are_left_alone() {
    // Files with the names the next exports would try (e.g. left by a crashed process that had
    // the same id) are skipped, neither truncated nor deleted.
    let path = temp_path("stale.png");
    let doc = fill_document(Size::new(3, 2), LinearRgba::new(1.0, 1.0, 1.0, 1.0));
    let spec = png_spec(PngDepth::U8, ColorSpace::SRGB, false);
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    let pid = std::process::id();
    // Other tests take numbers too: cover the next ones generously.
    let next = TEMP_COUNTER.load(Ordering::SeqCst);
    let stale: Vec<PathBuf> = (next..next + 64)
        .map(|n| path.with_file_name(format!(".{name}.{pid}-{n}{TEMP_SUFFIX}")))
        .collect();
    for file in &stale {
        std::fs::write(file, b"stale").unwrap();
    }
    export(&doc, &path, &spec).unwrap();
    assert!(image::open(&path).is_ok());
    for file in &stale {
        assert_eq!(std::fs::read(file).unwrap(), b"stale");
        std::fs::remove_file(file).unwrap();
    }
    assert!(temp_files(&path).is_empty());
    std::fs::remove_file(&path).ok();
}

#[test]
fn non_finite_samples_replaced_by_the_source_are_reported() {
    // The source reports what it replaced (1, 2 and 3 samples in the three bands); the
    // conversion adds what it meets (one NaN in the first band). Rec.2020, like the working
    // space: no matrix to turn the NaN (read as 0) into an out-of-gamut value.
    let path = temp_path("non-finite.png");
    let report = export_image(
        &path,
        RACE_SIZE,
        &png_spec(PngDepth::U8, ColorSpace::REC2020, false),
        |region, out| {
            out.as_chunks_mut::<4>().0.fill([0.5, 0.5, 0.5, 1.0]);
            if region.y == 0 {
                out[0] = f32::NAN;
            }
            Ok(u64::from(region.y / BAND_ROWS) + 1)
        },
        &CancelToken::new(),
        &mut |_| {},
    )
    .unwrap();
    std::fs::remove_file(&path).ok();
    assert_eq!(report.notices, [ExportNotice::NonFinite(7)]);
    assert_eq!(report.notices[0].id(), "nonFinite");

    // Through the CPU compositor: a NaN and an infinity in a float raster.
    let mut doc = Document::new(Size::new(2, 1));
    let pixels = [[f32::NAN, 0.5, 0.5, 1.0], [f32::INFINITY, 0.5, 0.5, 1.0]];
    let bytes: Vec<u8> = pixels
        .as_flattened()
        .iter()
        .flat_map(|v| v.to_ne_bytes())
        .collect();
    let format = PixelFormat {
        layout: ChannelLayout::Rgba,
        sample: SampleType::F32,
        color_space: ColorSpace::LINEAR_REC2020,
        alpha: AlphaMode::Straight,
    };
    let image = Arc::new(RasterImage::from_pixels(doc.size(), format, &bytes).unwrap());
    push_layer(&mut doc, LayerContent::Raster { image }, 1.0);
    let path = temp_path("non-finite.exr");
    let spec = ExportSpec {
        format: ExportFormat::Exr {
            sample: ExrSample::F32,
        },
        space: ColorSpace::LINEAR_REC2020,
        keep_alpha: true,
        matte: WHITE_MATTE,
        dither: false,
        gray: false,
        blend_space: BlendSpace::default(),
    };
    let report = export(&doc, &path, &spec).unwrap();
    std::fs::remove_file(&path).ok();
    assert_eq!(report.notices, [ExportNotice::NonFinite(2)]);
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
            matte: WHITE_MATTE,
            dither: false,
            gray: false,
            blend_space: BlendSpace::default(),
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
        assert!(temp_files(&path).is_empty());
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
            |_, _| Ok(0),
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
        matte: WHITE_MATTE,
        dither: false,
        gray: false,
        blend_space: BlendSpace::default(),
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

/// A translucent RGBA8 raster in sRGB, every alpha from 0 to 255.
fn translucent_raster(size: Size) -> Arc<RasterImage> {
    let format = PixelFormat {
        layout: ChannelLayout::Rgba,
        sample: SampleType::U8,
        color_space: ColorSpace::SRGB,
        alpha: AlphaMode::Straight,
    };
    let bytes: Vec<u8> = (0..size.pixel_count())
        .flat_map(|i| [(i * 7) as u8, (i * 3) as u8, (i * 11) as u8, i as u8])
        .collect();
    Arc::new(RasterImage::from_pixels(size, format, &bytes).unwrap())
}

#[test]
fn size_limits_per_format() {
    assert_eq!(max_side(ExportFormatKind::Webp), Some(16_383));
    assert_eq!(max_side(ExportFormatKind::Jpeg), Some(65_500));
    assert_eq!(max_side(ExportFormatKind::Png), Some(i32::MAX as u32));
    assert!(max_side(ExportFormatKind::Exr).is_some());
    assert_eq!(max_side(ExportFormatKind::Tiff), None);
}

#[test]
fn flattening_over_the_matte_equals_a_fill_below() {
    let size = Size::new(16, 16);
    let matte = LinearRgba::from_srgb_encoded_to_working(0.9, 0.5, 0.2, 1.0);
    for space in [BlendSpace::Linear, BlendSpace::Perceptual] {
        let spec = ExportSpec {
            matte,
            dither: false,
            blend_space: space,
            ..png_spec(PngDepth::U8, ColorSpace::SRGB, false)
        };
        let document = || {
            let mut doc = Document::new(size);
            Edit::SetBlendSpace { space }.apply(&mut doc).unwrap();
            doc
        };

        let mut flattened_doc = document();
        let image = translucent_raster(size);
        push_layer(
            &mut flattened_doc,
            LayerContent::Raster {
                image: image.clone(),
            },
            1.0,
        );
        let flattened_path = temp_path("matte-flattened.png");
        let report = export(&flattened_doc, &flattened_path, &spec).unwrap();
        // Every pixel but the one with alpha 255.
        assert_eq!(report.notices, [ExportNotice::AlphaFlattened(255)]);

        let mut fill_doc = document();
        push_layer(&mut fill_doc, LayerContent::Fill { color: matte }, 1.0);
        push_layer(&mut fill_doc, LayerContent::Raster { image }, 1.0);
        let fill_path = temp_path("matte-fill.png");
        assert_eq!(
            export(&fill_doc, &fill_path, &spec).unwrap(),
            ExportReport::default()
        );

        let flattened = image::open(&flattened_path).unwrap().to_rgb8();
        let fill = image::open(&fill_path).unwrap().to_rgb8();
        std::fs::remove_file(&flattened_path).ok();
        std::fs::remove_file(&fill_path).ok();
        // Exact in linear space; in perceptual space the composite goes through f32 between
        // the two blends, which may move a value by one code.
        let tolerance = match space {
            BlendSpace::Linear => 0,
            BlendSpace::Perceptual => 1,
        };
        let worst = flattened
            .as_raw()
            .iter()
            .zip(fill.as_raw())
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap_or(0);
        assert!(
            worst <= tolerance,
            "{space:?}: flattening differs from a fill layer below by {worst}"
        );
    }
}

#[test]
fn white_is_the_default_matte() {
    let size = Size::new(4, 4);
    let doc = Document::new(size);
    let spec = ExportSpec {
        keep_alpha: false,
        ..default_spec(ExportFormatKind::Png, &doc)
    };
    assert_eq!(spec.matte, WHITE_MATTE);
    let path = temp_path("default-matte.png");
    let report = export(&doc, &path, &spec).unwrap();
    assert_eq!(report.notices, [ExportNotice::AlphaFlattened(16)]);
    let output = image::open(&path).unwrap().to_rgb8();
    std::fs::remove_file(&path).ok();
    assert!(output.pixels().all(|p| p.0 == [255, 255, 255]));
}

fn gray_spec(format: ExportFormat, space: ColorSpace, keep_alpha: bool) -> ExportSpec {
    ExportSpec {
        format,
        space,
        keep_alpha,
        matte: WHITE_MATTE,
        dither: true,
        gray: true,
        blend_space: BlendSpace::default(),
    }
}

#[test]
fn gray_png_round_trips_bit_exact() {
    let (w, h) = (ODD_SIZE.width, ODD_SIZE.height);
    let luma = image::GrayImage::from_fn(w, h, |x, y| image::Luma([(x * 7 + y * 3) as u8]));
    let spec = ExportSpec {
        gray: true,
        blend_space: BlendSpace::default(),
        ..png_spec(PngDepth::U8, ColorSpace::SRGB, false)
    };
    let (output, report) = round_trip("gray8", luma.clone().into(), &spec);
    assert_eq!(output.color(), image::ColorType::L8);
    assert!(output.to_luma8() == luma, "8-bit gray is not bit-exact");
    assert_eq!(report, ExportReport::default());

    let luma_alpha = image::ImageBuffer::<image::LumaA<u16>, _>::from_fn(w, h, |x, y| {
        image::LumaA([
            (x * 211 + y * 13) as u16,
            1 + ((x * 257 + y * 97) % 65535) as u16,
        ])
    });
    let spec = ExportSpec {
        gray: true,
        blend_space: BlendSpace::default(),
        ..png_spec(PngDepth::U16, ColorSpace::SRGB, true)
    };
    let (output, report) = round_trip("gray16a", luma_alpha.clone().into(), &spec);
    assert_eq!(output.color(), image::ColorType::La16);
    assert!(
        output.to_luma_alpha16() == luma_alpha,
        "16-bit gray + alpha is not bit-exact"
    );
    assert_eq!(report, ExportReport::default());
}

/// A gray 16-bit document with a gradient, in the sRGB tone curve.
fn gray_gradient() -> (Document, Vec<u16>) {
    let size = Size::new(300, 40);
    let values: Vec<u16> = (0..size.pixel_count())
        .map(|i| ((i % 300) * 218) as u16)
        .collect();
    let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
    let format = PixelFormat {
        layout: ChannelLayout::Gray,
        sample: SampleType::U16,
        color_space: ColorSpace::SRGB,
        alpha: AlphaMode::Straight,
    };
    let image = RasterImage::from_pixels(size, format, &bytes).unwrap();
    (raster_document(image), values)
}

#[test]
fn gray_tiff_and_jpeg_are_gray_files_tagged_with_their_curve() {
    let (doc, values) = gray_gradient();
    let tiff = ExportFormat::Tiff {
        sample: TiffSample::U16,
        compression: TiffCompression::Deflate,
    };
    let path = temp_path("gray.tif");
    let report = export(&doc, &path, &gray_spec(tiff, ColorSpace::SRGB, false)).unwrap();
    assert_eq!(report, ExportReport::default());
    // Checked with the tiff crate (image is built without its TIFF codec).
    let mut decoder = ::tiff::decoder::Decoder::new(File::open(&path).unwrap()).unwrap();
    assert_eq!(decoder.colortype().unwrap(), ::tiff::ColorType::Gray(16));
    match decoder.read_image().unwrap() {
        ::tiff::decoder::DecodingResult::U16(decoded) => assert_eq!(decoded, values),
        _ => panic!("not 16-bit samples"),
    }
    let reimported = open_image(&path).unwrap().image.format();
    assert_eq!(reimported.layout, ChannelLayout::Gray);
    assert_eq!(reimported.color_space, ColorSpace::SRGB);

    // Another curve: declared by the gray profile, read back as the same curve.
    let report = export(&doc, &path, &gray_spec(tiff, ColorSpace::REC2020, false)).unwrap();
    assert_eq!(report, ExportReport::default());
    let reimported = open_image(&path).unwrap().image.format();
    assert_eq!(reimported.color_space.transfer, TransferFunction::Rec709);
    std::fs::remove_file(&path).ok();

    let jpeg = ExportFormat::Jpeg {
        quality: 100,
        subsampling: JpegSubsampling::S420,
    };
    let path = temp_path("gray.jpg");
    export(&doc, &path, &gray_spec(jpeg, ColorSpace::SRGB, false)).unwrap();
    let decoded = image::open(&path).unwrap();
    assert_eq!(decoded.color(), image::ColorType::L8);
    let worst = decoded
        .to_luma8()
        .pixels()
        .zip(&values)
        .map(|(p, &v)| p.0[0].abs_diff((f32::from(v) / 257.0).round() as u8))
        .max()
        .unwrap();
    assert!(worst <= 2, "gray JPEG off by {worst}");
    assert_eq!(
        open_image(&path).unwrap().image.format().layout,
        ChannelLayout::Gray
    );
    std::fs::remove_file(&path).ok();
}

#[test]
fn colors_exported_as_gray_become_their_luminance_and_are_reported() {
    let red = LinearRgba::from_srgb_encoded_to_working(1.0, 0.0, 0.0, 1.0);
    let size = Size::new(20, 10);
    let doc = fill_document(size, red);
    let path = temp_path("red-as-gray.png");
    let spec = ExportSpec {
        gray: true,
        blend_space: BlendSpace::default(),
        dither: false,
        ..png_spec(PngDepth::U8, ColorSpace::SRGB, false)
    };
    let report = export(&doc, &path, &spec).unwrap();
    assert_eq!(
        report.notices,
        [ExportNotice::ColorDiscarded(size.pixel_count())]
    );
    // Rec. 709 luminance of pure sRGB red, sRGB-encoded.
    let expected = (TransferFunction::Srgb.encode(0.2126) * 255.0).round() as u8;
    let decoded = image::open(&path).unwrap().to_luma8();
    assert!(decoded.pixels().all(|p| p.0[0].abs_diff(expected) <= 1));
    std::fs::remove_file(&path).ok();
}

#[test]
fn gray_support_per_format() {
    use ExportFormatKind::{Exr, Jpeg, Png, Tiff, Webp};
    for space in [
        ColorSpace::SRGB,
        ColorSpace::ADOBE_RGB,
        ColorSpace::REC2020,
        ColorSpace::LINEAR_SRGB,
    ] {
        for kind in [Png, Tiff, Jpeg] {
            assert!(supports_gray(kind, &space), "{kind:?} {space:?}");
        }
        assert!(!supports_gray(Exr, &space) && !supports_gray(Webp, &space));
    }
    // PQ and HLG have no ICC curve, and PNG declares them with cICP, for RGB only.
    assert!(!supports_gray(Png, &ColorSpace::REC2100_PQ));
    assert!(supports_space(Png, &ColorSpace::REC2100_PQ));
    assert!(has_gray(Png) && has_gray(Tiff) && has_gray(Jpeg));
    assert!(!has_gray(Exr) && !has_gray(Webp));

    let webp = gray_spec(
        ExportFormat::Webp {
            compression: WebpCompression::Lossless,
        },
        ColorSpace::SRGB,
        true,
    );
    let pq = gray_spec(
        ExportFormat::Png {
            depth: PngDepth::U16,
            compression: PngCompression::Fast,
        },
        ColorSpace::REC2100_PQ,
        true,
    );
    let path = temp_path("gray-refused.webp");
    for (spec, code) in [(webp, "invalidSpec"), (pq, "unsupportedSpace")] {
        let result = export_image(
            &path,
            Size::new(4, 4),
            &spec,
            |_, _| panic!("the source must not be called"),
            &CancelToken::new(),
            &mut |_| {},
        );
        assert_eq!(result.unwrap_err().code(), code);
        assert!(!path.exists() && temp_files(&path).is_empty());
    }
}

#[test]
fn gray_documents_export_as_gray_by_default() {
    use ExportFormatKind::{Exr, Jpeg, Png, Tiff, Webp};
    let size = Size::new(4, 4);
    let mut doc = Document::new(size);
    let neutral = LinearRgba::new(0.5, 0.5, 0.5, 1.0);
    push_layer(&mut doc, LayerContent::Fill { color: neutral }, 1.0);
    // A neutral fill alone is not a gray source.
    assert!(!default_spec(Png, &doc).gray);
    let gray = Arc::new(raster(
        size,
        ChannelLayout::Gray,
        SampleType::U16,
        ColorSpace::SRGB,
    ));
    push_layer(&mut doc, LayerContent::Raster { image: gray }, 1.0);
    for kind in [Png, Tiff, Jpeg] {
        let spec = default_spec(kind, &doc);
        assert!(spec.gray, "{kind:?}");
        assert!(supports_gray(kind, &spec.space), "{kind:?}");
    }
    assert!(!default_spec(Webp, &doc).gray && !default_spec(Exr, &doc).gray);

    // Any color makes it a color export...
    let tint = LinearRgba::new(0.5, 0.4, 0.5, 1.0);
    let id = push_layer(&mut doc, LayerContent::Fill { color: tint }, 0.1);
    assert!(!default_spec(Png, &doc).gray);
    // ...unless that layer is hidden.
    Edit::SetLayerVisible { id, visible: false }
        .apply(&mut doc)
        .unwrap();
    assert!(default_spec(Png, &doc).gray);
    let rgb = Arc::new(raster(
        size,
        ChannelLayout::Rgb,
        SampleType::U8,
        ColorSpace::SRGB,
    ));
    push_layer(&mut doc, LayerContent::Raster { image: rgb }, 1.0);
    assert!(!default_spec(Png, &doc).gray);
}

/// A document using what a layered PSD keeps: pixel layers (moved, translucent, in several
/// blend modes), an isolated masked group with a clipped layer, a hidden layer, a fill layer and
/// adjustment layers.
fn layered_document() -> Document {
    use slopshop_core::adjust::Adjustment;
    use slopshop_core::color::{AlphaMode, ChannelLayout, PixelFormat};
    use slopshop_core::document::LayerMask;
    let size = Size::new(64, 48);
    let mut doc = Document::new(size);
    let rgba = |w: u32, h: u32, f: &dyn Fn(u32, u32) -> [u8; 4]| {
        let pixels: Vec<u8> = (0..h)
            .flat_map(|y| (0..w).flat_map(move |x| f(x, y)))
            .collect();
        Arc::new(
            RasterImage::from_pixels(Size::new(w, h), PixelFormat::RGBA8_SRGB, &pixels).unwrap(),
        )
    };
    let gray_mask = |w: u32, h: u32| {
        let format = PixelFormat {
            layout: ChannelLayout::Gray,
            sample: SampleType::U8,
            color_space: ColorSpace::LINEAR_SRGB,
            alpha: AlphaMode::Straight,
        };
        let pixels: Vec<u8> = (0..h)
            .flat_map(|_| (0..w).map(|x| (x * 255 / (w - 1)) as u8))
            .collect();
        Arc::new(RasterImage::from_pixels(Size::new(w, h), format, &pixels).unwrap())
    };
    let add = |doc: &mut Document, parent: Option<LayerId>, name: &str, content: LayerContent| {
        let id = doc.allocate_layer_id();
        let index = doc.children_of(parent).unwrap().len();
        Edit::InsertLayer {
            parent,
            index,
            layer: Layer {
                transform: slopshop_core::Affine::IDENTITY,
                clipped: false,
                id,
                name: name.into(),
                visible: true,
                opacity: 1.0,
                blend_mode: BlendMode::Normal,
                mask: None,
                content,
            },
        }
        .apply(doc)
        .unwrap();
        id
    };
    let set = |doc: &mut Document, edit: Edit| {
        edit.apply(doc).unwrap();
    };
    add(
        &mut doc,
        None,
        "Background",
        LayerContent::Raster {
            image: rgba(64, 48, &|x, y| [(x * 4) as u8, (y * 5) as u8, 120, 255]),
        },
    );
    let moved = add(
        &mut doc,
        None,
        "Moved \u{e9}toile",
        LayerContent::Raster {
            image: rgba(20, 16, &|x, y| {
                [200, (x * 12) as u8, (y * 15) as u8, ((x + y) * 8) as u8]
            }),
        },
    );
    set(
        &mut doc,
        Edit::SetLayerTransform {
            id: moved,
            transform: slopshop_core::Affine::translation(10.0, 7.0),
        },
    );
    set(
        &mut doc,
        Edit::SetLayerBlendMode {
            id: moved,
            mode: BlendMode::Multiply,
        },
    );
    set(
        &mut doc,
        Edit::SetLayerOpacity {
            id: moved,
            opacity: 0.7,
        },
    );
    let group = add(
        &mut doc,
        None,
        "Group",
        LayerContent::Group {
            children: Vec::new(),
            pass_through: false,
        },
    );
    set(
        &mut doc,
        Edit::SetLayerBlendMode {
            id: group,
            mode: BlendMode::Screen,
        },
    );
    set(
        &mut doc,
        Edit::SetLayerMask {
            id: group,
            mask: Some(LayerMask {
                image: gray_mask(64, 48),
                enabled: true,
                replaces_alpha: false,
            }),
        },
    );
    add(
        &mut doc,
        Some(group),
        "Inside",
        LayerContent::Raster {
            image: rgba(30, 20, &|x, _| {
                [30, 90, (x * 8) as u8, if x < 20 { 255 } else { 0 }]
            }),
        },
    );
    let clipped = add(
        &mut doc,
        Some(group),
        "Clipped",
        // Within the sRGB gamut (the file's space): nothing is clipped.
        LayerContent::Fill {
            color: LinearRgba::from_srgb_encoded_to_working(0.8, 0.2, 0.2, 1.0),
        },
    );
    set(
        &mut doc,
        Edit::SetLayerClipped {
            id: clipped,
            clipped: true,
        },
    );
    let hidden = add(
        &mut doc,
        None,
        "Hidden",
        LayerContent::Raster {
            image: rgba(8, 8, &|_, _| [255, 255, 255, 255]),
        },
    );
    set(
        &mut doc,
        Edit::SetLayerVisible {
            id: hidden,
            visible: false,
        },
    );
    add(
        &mut doc,
        None,
        "Hue/Saturation 1",
        LayerContent::Adjustment {
            adjustment: Adjustment::HueSaturation {
                hue: 30.0,
                saturation: -20.0,
                lightness: 5.0,
            },
        },
    );
    let levels = add(
        &mut doc,
        None,
        "Levels 1",
        LayerContent::Adjustment {
            adjustment: Adjustment::Levels {
                input_black: 10.0 / 255.0,
                input_white: 240.0 / 255.0,
                gamma: 1.2,
                output_black: 0.0,
                output_white: 1.0,
            },
        },
    );
    set(
        &mut doc,
        Edit::SetLayerOpacity {
            id: levels,
            opacity: 0.5,
        },
    );
    doc
}

fn composite_all(doc: &Document) -> Vec<f32> {
    let mut out = vec![0.0; doc.size().pixel_count() as usize * 4];
    composite_region(doc, doc.size().bounds(), &mut out).unwrap();
    out
}

fn cpu_render(d: &Document, region: Rect, out: &mut [f32]) -> Result<u64, String> {
    composite_region(d, region, out)
        .map(|r| r.non_finite)
        .map_err(|e| e.to_string())
}

#[test]
fn layered_psd_round_trips_through_the_importer() {
    let cases = [
        (PsdDepth::U8, 0.03, false),
        (PsdDepth::U8, 0.03, true),
        (PsdDepth::U16, 0.004, false),
        (PsdDepth::U16, 0.004, true),
    ];
    // PSD, then PSB (wider lengths and row counts, read back by the same importer).
    for (large, (depth, tolerance, transparent)) in [false, true]
        .into_iter()
        .flat_map(|large| cases.map(|case| (large, case)))
    {
        let mut doc = layered_document();
        if transparent {
            // Without the opaque background, the merged composite (stored over white) has
            // transparency.
            let background = doc.layers()[0].id;
            Edit::SetLayerVisible {
                id: background,
                visible: false,
            }
            .apply(&mut doc)
            .unwrap();
        }
        let extension = if large { "psb" } else { "psd" };
        let path = temp_path(&format!("layered-{depth:?}-{transparent}.{extension}"));
        let options = PsdOptions {
            depth,
            space: ColorSpace::SRGB,
            dither: false,
            large,
        };
        let mut rows = 0;
        let report = export_psd(
            &path,
            &doc,
            &options,
            &mut cpu_render,
            &CancelToken::new(),
            &mut |p| rows = p.done,
        )
        .unwrap();
        assert!(rows > 0);
        assert!(report.notices.is_empty(), "{depth:?}: {:?}", report.notices);
        // Neither the spill file nor the temporary file is left.
        assert!(crate::atomic::temp_files(&path).is_empty());
        let crate::Opened::Layers(opened) = crate::open_file(&path).unwrap() else {
            panic!("{depth:?}: layers expected");
        };
        let back = &opened.document;
        // The same tree: names, visibility, opacity, modes, clipping, masks.
        type Summary = (String, bool, u8, BlendMode, bool, bool, bool);
        let summary = |d: &Document| -> Vec<Summary> {
            d.all_layers()
                .map(|l| {
                    (
                        l.name.clone(),
                        l.visible,
                        (l.opacity * 255.0).round() as u8,
                        l.blend_mode,
                        l.clipped,
                        l.mask.is_some(),
                        l.children().is_some(),
                    )
                })
                .collect()
        };
        assert_eq!(summary(back), summary(&doc), "{depth:?}");
        let adjustments = |d: &Document| -> Vec<slopshop_core::adjust::Adjustment> {
            d.all_layers()
                .filter_map(|l| match l.content {
                    LayerContent::Adjustment { adjustment } => Some(adjustment),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(adjustments(back), adjustments(&doc), "{depth:?}");
        // The same picture, within the file's precision.
        let (a, b) = (composite_all(&doc), composite_all(back));
        let worst = a
            .iter()
            .zip(&b)
            .map(|(x, y)| (x - y).abs())
            .fold(0.0f32, f32::max);
        assert!(worst < tolerance, "{depth:?}: composites differ by {worst}");
        // The merged composite, for readers without layers.
        let flat = raster_document(open_image(&path).unwrap().image);
        let c = composite_all(&flat);
        let worst = a
            .iter()
            .zip(&c)
            .map(|(x, y)| (x - y).abs())
            .fold(0.0f32, f32::max);
        assert!(
            worst < tolerance,
            "{depth:?}: merged composite differs by {worst}"
        );
        std::fs::remove_file(&path).ok();
    }
}

#[test]
fn adjustments_of_many_settings_round_trip_through_a_layered_psd() {
    use slopshop_core::adjust::Adjustment;
    let mut doc = Document::new(Size::new(8, 8));
    let written = [
        Adjustment::BlackWhite {
            weights: [30.0, -50.0, 120.0, 60.0, 250.0, 80.0],
            tint: true,
            tint_hue: 200.0,
            tint_saturation: 35.0,
        },
        Adjustment::ColorBalance {
            shadows: [-4.0, 2.0, -5.0],
            midtones: [10.0, 4.0, -9.0],
            highlights: [1.0, -9.0, -3.0],
            preserve_luminosity: false,
        },
        Adjustment::PhotoFilter {
            color: [0.2, 0.5, 0.9],
            density: 60.0,
            preserve_luminosity: true,
        },
        Adjustment::ChannelMixer {
            red: [40.0, 40.0, 20.0, -5.0],
            green: [0.0, 100.0, 0.0, 0.0],
            blue: [10.0, 0.0, 90.0, 12.0],
            monochrome: true,
        },
        Adjustment::Curves {
            rgb: slopshop_core::curve::Curve::new(&[[0, 10], [90, 130], [255, 250]]).unwrap(),
            red: slopshop_core::curve::Curve::IDENTITY,
            green: slopshop_core::curve::Curve::new(&[[30, 0], [255, 255]]).unwrap(),
            blue: slopshop_core::curve::Curve::new(&[[0, 0], [64, 40], [192, 220], [255, 255]])
                .unwrap(),
        },
    ];
    for (index, adjustment) in written.into_iter().enumerate() {
        let id = doc.allocate_layer_id();
        Edit::InsertLayer {
            parent: None,
            index,
            layer: Layer {
                transform: slopshop_core::Affine::IDENTITY,
                clipped: false,
                id,
                name: adjustment.id().into(),
                visible: true,
                opacity: 1.0,
                blend_mode: BlendMode::Normal,
                mask: None,
                content: LayerContent::Adjustment { adjustment },
            },
        }
        .apply(&mut doc)
        .unwrap();
    }
    let path = temp_path("many-settings.psd");
    let options = PsdOptions {
        depth: PsdDepth::U8,
        space: ColorSpace::SRGB,
        dither: false,
        large: false,
    };
    export_psd(
        &path,
        &doc,
        &options,
        &mut cpu_render,
        &CancelToken::new(),
        &mut |_| {},
    )
    .unwrap();
    let crate::Opened::Layers(opened) = crate::open_file(&path).unwrap() else {
        panic!("layers expected");
    };
    std::fs::remove_file(&path).ok();
    let read: Vec<Adjustment> = opened
        .document
        .layers()
        .iter()
        .filter_map(|l| match l.content {
            LayerContent::Adjustment { adjustment } => Some(adjustment),
            _ => None,
        })
        .collect();
    assert_eq!(read.len(), 5);
    assert_eq!(read[4], written[4]);
    // Exact, but for the filter color (Lab in hundredths) and the tint (HSB, rounded).
    assert_eq!(read[1], written[1]);
    assert_eq!(read[3], written[3]);
    assert_eq!(read[0], written[0]);
    let Adjustment::PhotoFilter {
        color,
        density,
        preserve_luminosity,
    } = read[2]
    else {
        panic!("{:?}", read[2]);
    };
    assert!(
        color
            .iter()
            .zip([0.2, 0.5, 0.9])
            .all(|(c, e)| (c - e).abs() < 1e-3),
        "{color:?}"
    );
    assert_eq!((density, preserve_luminosity), (60.0, true));
}

#[test]
fn layered_psd_reports_what_is_outside_the_canvas_and_refuses_psb_sizes() {
    let mut doc = layered_document();
    let moved = doc.layers()[1].id;
    Edit::SetLayerTransform {
        id: moved,
        transform: slopshop_core::Affine::translation(-5.0, 40.0),
    }
    .apply(&mut doc)
    .unwrap();
    let path = temp_path("outside.psd");
    let options = PsdOptions {
        depth: PsdDepth::U8,
        space: ColorSpace::SRGB,
        dither: true,
        large: false,
    };
    let report = export_psd(
        &path,
        &doc,
        &options,
        &mut cpu_render,
        &CancelToken::new(),
        &mut |_| {},
    )
    .unwrap();
    assert!(report.notices.contains(&ExportNotice::PixelsOutsideCanvas));
    std::fs::remove_file(&path).ok();
    let huge = Document::new(Size::new(PSD_MAX_SIDE + 1, 10));
    assert!(matches!(
        export_psd(
            &path,
            &huge,
            &options,
            &mut cpu_render,
            &CancelToken::new(),
            &mut |_| {}
        ),
        Err(ExportError::TooLarge { .. })
    ));
}

#[test]
fn bmp_and_tga_round_trip_bit_exact() {
    let (w, h) = (ODD_SIZE.width, ODD_SIZE.height);
    let rgb = image::RgbImage::from_fn(w, h, |x, y| {
        // Flat runs (for RLE) and detail.
        let v = if x < 60 { 40 } else { (x * 7 + y) as u8 };
        image::Rgb([v, (y * 3) as u8, (x ^ y) as u8])
    });
    let rgba = image::RgbaImage::from_fn(w, h, |x, y| {
        image::Rgba([
            (x * 5 + y) as u8,
            (y * 11) as u8,
            (x + 2 * y) as u8,
            1 + ((x * 13 + y * 7) % 255) as u8,
        ])
    });
    let formats = [
        (ExportFormat::Bmp, "bmp"),
        (
            ExportFormat::Tga {
                compression: TgaCompression::None,
            },
            "tga",
        ),
        (
            ExportFormat::Tga {
                compression: TgaCompression::Rle,
            },
            "tga",
        ),
    ];
    for (i, (format, extension)) in formats.into_iter().enumerate() {
        for keep_alpha in [false, true] {
            let input: image::DynamicImage = if keep_alpha {
                rgba.clone().into()
            } else {
                rgb.clone().into()
            };
            let source = temp_path(&format!("simple-{i}-{keep_alpha}-in.png"));
            input.save(&source).unwrap();
            let doc = raster_document(open_image(&source).unwrap().image);
            std::fs::remove_file(&source).ok();
            let spec = ExportSpec {
                format,
                space: ColorSpace::SRGB,
                keep_alpha,
                matte: WHITE_MATTE,
                dither: true,
                gray: false,
                blend_space: BlendSpace::default(),
            };
            let path = temp_path(&format!("simple-{i}-{keep_alpha}.{extension}"));
            let report = export(&doc, &path, &spec).unwrap();
            assert_eq!(report, ExportReport::default());
            assert!(temp_files(&path).is_empty());
            let what = format!("{format:?} alpha {keep_alpha}");
            let output = image::open(&path).unwrap();
            if keep_alpha {
                assert!(output.to_rgba8() == rgba, "{what} is not bit-exact");
            } else {
                assert!(!output.color().has_alpha(), "{what}");
                assert!(output.to_rgb8() == rgb, "{what} is not bit-exact");
            }
            // Our importer reads it, as sRGB.
            let back = open_image(&path).unwrap();
            assert_eq!(back.image.format().color_space, ColorSpace::SRGB, "{what}");
            std::fs::remove_file(&path).ok();
        }
    }
    // Other spaces cannot be declared.
    assert!(!supports_space(
        ExportFormatKind::Tga,
        &ColorSpace::DISPLAY_P3
    ));
    assert!(!supports_space(
        ExportFormatKind::Bmp,
        &ColorSpace::LINEAR_SRGB
    ));
}

#[test]
fn netpbm_round_trips_are_exact() {
    let (w, h) = (ODD_SIZE.width, ODD_SIZE.height);
    let gray16 = image::ImageBuffer::<image::Luma<u16>, Vec<u16>>::from_fn(w, h, |x, y| {
        image::Luma([(x * 211 + y * 97) as u16])
    });
    let rgba8 = image::RgbaImage::from_fn(w, h, |x, y| {
        image::Rgba([x as u8, (y * 3) as u8, (x ^ y) as u8, 1 + (x % 250) as u8])
    });
    let rgb16 = image::ImageBuffer::<image::Rgb<u16>, Vec<u16>>::from_fn(w, h, |x, y| {
        image::Rgb([
            (x * 200) as u16,
            (y * 120) as u16,
            ((x * y) % 65_536) as u16,
        ])
    });
    let cases: [(image::DynamicImage, PngDepth, bool, bool, &str); 3] = [
        (gray16.into(), PngDepth::U16, true, false, "pgm"),
        (rgba8.into(), PngDepth::U8, false, true, "pam"),
        (rgb16.into(), PngDepth::U16, false, false, "ppm"),
    ];
    for (input, depth, gray, keep_alpha, extension) in cases {
        let source = temp_path(&format!("netpbm-{extension}-in.png"));
        input.save(&source).unwrap();
        let doc = raster_document(open_image(&source).unwrap().image);
        std::fs::remove_file(&source).ok();
        let spec = ExportSpec {
            format: ExportFormat::Pnm { depth },
            space: ColorSpace::SRGB,
            keep_alpha,
            matte: WHITE_MATTE,
            dither: true,
            gray,
            blend_space: BlendSpace::default(),
        };
        let path = temp_path(&format!("netpbm.{extension}"));
        export(&doc, &path, &spec).unwrap();
        let output = image::ImageReader::open(&path)
            .unwrap()
            .with_guessed_format()
            .unwrap()
            .decode()
            .unwrap();
        assert_eq!(output.color(), input.color(), "{extension}");
        assert!(output == input, "{extension} is not exact");
        assert_eq!(
            open_image(&path).unwrap().image.format().color_space,
            ColorSpace::SRGB
        );
        std::fs::remove_file(&path).ok();
    }
}

#[test]
fn pfm_round_trips_float_values_bottom_to_top() {
    let size = Size::new(37, 300);
    let format = float_rgb_linear();
    let values: Vec<f32> = (0..size.pixel_count() as usize * 3)
        .map(|i| i as f32 * 0.37 - 900.0)
        .collect();
    let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_ne_bytes()).collect();
    let image = RasterImage::from_pixels(size, format, &bytes).unwrap();
    let doc = raster_document(image);
    let spec = default_spec(ExportFormatKind::Pfm, &doc);
    assert_eq!(spec.format, ExportFormat::Pfm);
    assert!(!spec.keep_alpha);
    let path = temp_path("float.pfm");
    let report = export(&doc, &path, &spec).unwrap();
    assert_eq!(report, ExportReport::default());
    let back = open_image(&path).unwrap().image;
    std::fs::remove_file(&path).ok();
    assert_eq!(back.format().color_space, ColorSpace::LINEAR_SRGB);
    assert_eq!(back.format().sample, SampleType::F32);
    // The first pixel of the last row: rows come back in order.
    let tile = back.levels()[0]
        .tile(slopshop_core::tile::TileCoord { col: 0, row: 1 })
        .unwrap();
    let bpp = back.stored_format().bytes_per_pixel() as usize;
    let at = (299 - 256) * 256 * bpp;
    let v = f32::from_ne_bytes(tile[at..at + 4].try_into().unwrap());
    // Through the working space and back: float rounding only.
    let expected = values[299 * 37 * 3];
    assert!(
        (v - expected).abs() <= expected.abs() * 1e-5,
        "{v} {expected}"
    );
}

fn float_rgb_linear() -> PixelFormat {
    PixelFormat {
        layout: ChannelLayout::Rgb,
        sample: SampleType::F32,
        color_space: ColorSpace::LINEAR_SRGB,
        alpha: AlphaMode::Straight,
    }
}

/// A raster of `size` whose samples `f(x, y, channel)` gives (8-bit or 16-bit values).
fn pattern_raster(
    size: Size,
    layout: ChannelLayout,
    sample: SampleType,
    space: ColorSpace,
    f: impl Fn(u32, u32, usize) -> u16,
) -> RasterImage {
    let format = PixelFormat {
        layout,
        sample,
        color_space: space,
        alpha: AlphaMode::Straight,
    };
    let channels = layout.channels() as usize;
    let mut bytes = Vec::new();
    for y in 0..size.height {
        for x in 0..size.width {
            for c in 0..channels {
                let v = f(x, y, c);
                if sample == SampleType::U8 {
                    bytes.push(v as u8);
                } else {
                    bytes.extend(v.to_ne_bytes());
                }
            }
        }
    }
    RasterImage::from_pixels(size, format, &bytes).unwrap()
}

/// A document's composite, as 16-bit values in reading order.
fn composite_u16(doc: &Document) -> Vec<u16> {
    let size = doc.size();
    let mut out = vec![0f32; size.pixel_count() as usize * 4];
    composite_region(doc, size.bounds(), &mut out).unwrap();
    out.iter()
        .map(|v| (v.clamp(0.0, 1.0) * 65535.0).round() as u16)
        .collect()
}

fn avif_spec(depth: AvifDepth, quality: u8, space: ColorSpace) -> ExportSpec {
    ExportSpec {
        format: ExportFormat::Avif { depth, quality },
        space,
        keep_alpha: false,
        matte: WHITE_MATTE,
        dither: false,
        gray: false,
        blend_space: BlendSpace::default(),
    }
}

/// Export `doc` to AVIF with `spec`, and read it back with our importer.
fn avif_round_trip(name: &str, doc: &Document, spec: &ExportSpec) -> crate::Imported {
    let path = temp_path(&format!("{name}.avif"));
    let report = export(doc, &path, spec).unwrap();
    assert_eq!(report, ExportReport::default(), "{name}");
    assert!(temp_files(&path).is_empty());
    let back = open_image(&path).unwrap();
    std::fs::remove_file(&path).ok();
    back
}

#[test]
fn best_quality_avif_is_close_with_alpha() {
    let size = Size::new(48, 40);
    let image = pattern_raster(
        size,
        ChannelLayout::Rgba,
        SampleType::U8,
        ColorSpace::SRGB,
        |x, y, c| match c {
            0 => (x * 5) as u16,
            1 => (y * 6) as u16,
            2 => 128,
            _ => 128 + (x * 2) as u16,
        },
    );
    let doc = raster_document(image);
    let mut spec = avif_spec(AvifDepth::U8, 100, ColorSpace::SRGB);
    spec.keep_alpha = true;
    let back = avif_round_trip("best", &doc, &spec);
    let format = back.image.format();
    assert_eq!(
        (format.layout, format.sample, format.color_space),
        (ChannelLayout::Rgba, SampleType::U8, ColorSpace::SRGB)
    );
    let (got, want) = (
        composite_u16(&raster_document(back.image)),
        composite_u16(&doc),
    );
    let worst = got
        .iter()
        .zip(&want)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap_or(0);
    // A few 8-bit steps at most.
    assert!(worst < 4 * 257, "{worst}");
}

#[test]
fn lossy_avif_stays_close_and_keeps_its_space_and_depth() {
    let size = Size::new(64, 48);
    // 16-bit Display P3, smooth: written at 10 bits with alpha.
    let image = pattern_raster(
        size,
        ChannelLayout::Rgba,
        SampleType::U16,
        ColorSpace::DISPLAY_P3,
        |x, y, c| match c {
            0 => (x * 1000) as u16,
            1 => (y * 1300) as u16,
            2 => 30_000,
            _ => 20_000 + (x * 500) as u16,
        },
    );
    let doc = raster_document(image);
    let mut spec = avif_spec(AvifDepth::U10, 90, ColorSpace::DISPLAY_P3);
    spec.keep_alpha = true;
    let back = avif_round_trip("p3-10bit", &doc, &spec);
    let format = back.image.format();
    assert_eq!(
        (format.layout, format.sample, format.color_space),
        (ChannelLayout::Rgba, SampleType::U16, ColorSpace::DISPLAY_P3)
    );
    let (a, b) = (
        composite_u16(&raster_document(back.image)),
        composite_u16(&doc),
    );
    let mean = a
        .iter()
        .zip(&b)
        .map(|(x, y)| f64::from(x.abs_diff(*y)))
        .sum::<f64>()
        / a.len() as f64;
    assert!(mean < 400.0, "mean difference {mean} of 65535");
}

#[test]
fn gray_and_hdr_avif_are_declared() {
    let size = Size::new(40, 24);
    let gray = pattern_raster(
        size,
        ChannelLayout::Gray,
        SampleType::U8,
        ColorSpace::SRGB,
        |x, y, _| ((x * 6 + y) % 256) as u16,
    );
    let mut spec = avif_spec(AvifDepth::U8, 85, ColorSpace::SRGB);
    spec.gray = true;
    let back = avif_round_trip("gray", &raster_document(gray), &spec);
    assert_eq!(back.image.format().layout, ChannelLayout::Gray);

    let hdr = pattern_raster(
        size,
        ChannelLayout::Rgb,
        SampleType::U16,
        ColorSpace::REC2100_PQ,
        |x, _, _| (x * 1500) as u16,
    );
    let spec = avif_spec(AvifDepth::U10, 85, ColorSpace::REC2100_PQ);
    let back = avif_round_trip("pq", &raster_document(hdr), &spec);
    assert_eq!(back.image.format().color_space, ColorSpace::REC2100_PQ);
    // Spaces without code points (Adobe RGB) are refused.
    assert!(!supports_space(
        ExportFormatKind::Avif,
        &ColorSpace::ADOBE_RGB
    ));
    assert!(supports_space(
        ExportFormatKind::Avif,
        &ColorSpace::REC2100_HLG
    ));
}

fn jxl_spec(depth: PngDepth, space: ColorSpace, keep_alpha: bool, gray: bool) -> ExportSpec {
    ExportSpec {
        format: ExportFormat::Jxl { depth },
        space,
        keep_alpha,
        matte: WHITE_MATTE,
        dither: false,
        gray,
        blend_space: BlendSpace::default(),
    }
}

/// Export `doc` to JPEG XL with `spec`, and read it back with our importer.
fn jxl_round_trip(name: &str, doc: &Document, spec: &ExportSpec) -> crate::Imported {
    let path = temp_path(&format!("{name}.jxl"));
    let report = export(doc, &path, spec).unwrap();
    assert_eq!(report, ExportReport::default(), "{name}");
    assert!(temp_files(&path).is_empty());
    let back = open_image(&path).unwrap();
    std::fs::remove_file(&path).ok();
    back
}

#[test]
fn jxl_round_trips_are_lossless_at_8_and_16_bits_with_alpha() {
    let size = Size::new(70, 45);
    let cases = [
        (ChannelLayout::Rgb, SampleType::U8),
        (ChannelLayout::Rgba, SampleType::U16),
        (ChannelLayout::Gray, SampleType::U8),
        (ChannelLayout::GrayAlpha, SampleType::U16),
    ];
    for (layout, sample) in cases {
        let image = pattern_raster(size, layout, sample, ColorSpace::SRGB, |x, y, c| {
            let v = (x * 97 + y * 31 + c as u32 * 7_000) % 65_536;
            let alpha = matches!(layout, ChannelLayout::Rgba | ChannelLayout::GrayAlpha)
                && c + 1 == layout.channels() as usize;
            // Opaque enough everywhere: colors under alpha 0 are not kept, by design.
            let v = if alpha { 30_000 + v % 30_000 } else { v };
            if sample == SampleType::U8 {
                (v % 256) as u16
            } else {
                v as u16
            }
        });
        let doc = raster_document(image);
        let gray = layout.is_gray();
        let alpha = matches!(layout, ChannelLayout::Rgba | ChannelLayout::GrayAlpha);
        let depth = if sample == SampleType::U8 {
            PngDepth::U8
        } else {
            PngDepth::U16
        };
        let back = jxl_round_trip(
            &format!("{layout:?}"),
            &doc,
            &jxl_spec(depth, ColorSpace::SRGB, alpha, gray),
        );
        let format = back.image.format();
        // 16-bit gray with alpha is written in color (an encoder limitation).
        let expected = if layout == ChannelLayout::GrayAlpha && sample == SampleType::U16 {
            ChannelLayout::Rgba
        } else {
            layout
        };
        assert_eq!(
            (format.layout, format.sample),
            (expected, sample),
            "{layout:?}"
        );
        assert_eq!(format.color_space, ColorSpace::SRGB, "{layout:?}");
        assert!(
            composite_u16(&raster_document(back.image)) == composite_u16(&doc),
            "{layout:?} is not lossless"
        );
    }
}

#[test]
fn jxl_declares_wide_custom_and_hdr_spaces() {
    let size = Size::new(24, 16);
    for space in [
        ColorSpace::DISPLAY_P3,
        ColorSpace::REC2100_PQ,
        ColorSpace::ADOBE_RGB,
        ColorSpace::PROPHOTO,
        ColorSpace::LINEAR_REC2020,
    ] {
        let image = pattern_raster(
            size,
            ChannelLayout::Rgb,
            SampleType::U16,
            space,
            |x, y, c| ((x * 2_000 + y * 900 + c as u32 * 10_000) % 65_536) as u16,
        );
        let doc = raster_document(image);
        let back = jxl_round_trip("space", &doc, &jxl_spec(PngDepth::U16, space, false, false));
        let got = back.image.format().color_space;
        let close =
            |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4;
        let (p, q) = (got.primaries, space.primaries);
        assert!(
            close(p.red, q.red)
                && close(p.green, q.green)
                && close(p.blue, q.blue)
                && close(p.white, q.white),
            "{space:?} came back as {got:?}"
        );
        assert!(
            composite_u16(&raster_document(back.image)) == composite_u16(&doc),
            "{space:?} is not lossless"
        );
    }
}

fn simple_spec(format: ExportFormat, space: ColorSpace, keep_alpha: bool) -> ExportSpec {
    ExportSpec {
        format,
        space,
        keep_alpha,
        matte: WHITE_MATTE,
        dither: true,
        gray: false,
        blend_space: BlendSpace::default(),
    }
}

/// Save `input` as PNG, import it, export it with `spec` to `name` and decode the file with the
/// `image` crate; our importer must read it back in `spec.space`.
fn simple_round_trip(
    name: &str,
    input: &image::DynamicImage,
    spec: &ExportSpec,
) -> image::DynamicImage {
    let source = temp_path(&format!("{name}-in.png"));
    input.save(&source).unwrap();
    let doc = raster_document(open_image(&source).unwrap().image);
    std::fs::remove_file(&source).ok();
    let path = temp_path(name);
    let report = export(&doc, &path, spec).unwrap();
    assert_eq!(report, ExportReport::default(), "{name}");
    assert!(temp_files(&path).is_empty());
    let output = image::ImageReader::open(&path)
        .unwrap()
        .with_guessed_format()
        .unwrap()
        .decode()
        .unwrap();
    let back = open_image(&path).unwrap();
    assert_eq!(back.image.format().color_space, spec.space, "{name}");
    std::fs::remove_file(&path).ok();
    output
}

#[test]
fn qoi_and_farbfeld_round_trip_bit_exact() {
    let (w, h) = (ODD_SIZE.width, ODD_SIZE.height);
    // Runs, small differences (QOI's DIFF and LUMA operations), repeats (INDEX) and detail.
    let rgb = image::RgbImage::from_fn(w, h, |x, y| {
        let v = if x < 60 { 40 } else { (x * 7 + y) as u8 };
        image::Rgb([v, (y * 3 + x / 9) as u8, (x ^ y) as u8])
    });
    let rgba = image::RgbaImage::from_fn(w, h, |x, y| {
        let alpha = if x % 50 < 25 {
            255
        } else {
            1 + ((x * 13 + y * 7) % 255) as u8
        };
        image::Rgba([
            (x * 5 + y) as u8,
            (y * 11) as u8,
            (x / 3 + 2 * y) as u8,
            alpha,
        ])
    });
    for keep_alpha in [false, true] {
        let input: image::DynamicImage = if keep_alpha {
            rgba.clone().into()
        } else {
            rgb.clone().into()
        };
        let spec = simple_spec(ExportFormat::Qoi, ColorSpace::SRGB, keep_alpha);
        let output = simple_round_trip(&format!("simple-{keep_alpha}.qoi"), &input, &spec);
        assert_eq!(output.color().has_alpha(), keep_alpha);
        assert!(output == input, "QOI alpha {keep_alpha} is not bit-exact");
    }

    let rgba16 = image::ImageBuffer::<image::Rgba<u16>, Vec<u16>>::from_fn(w, h, |x, y| {
        image::Rgba([
            (x * 200) as u16,
            (y * 120) as u16,
            ((x * y) % 65_536) as u16,
            1 + ((x * 211 + y) % 65_535) as u16,
        ])
    });
    let spec = simple_spec(ExportFormat::Farbfeld, ColorSpace::SRGB, true);
    let output = simple_round_trip("rgba16.ff", &rgba16.clone().into(), &spec);
    assert!(output.to_rgba16() == rgba16, "farbfeld is not bit-exact");
    // Without alpha: opaque RGBA.
    let rgb16 = image::ImageBuffer::<image::Rgb<u16>, Vec<u16>>::from_fn(w, h, |x, y| {
        image::Rgb([
            (x * 200) as u16,
            (y * 120) as u16,
            ((x * y) % 65_536) as u16,
        ])
    });
    let spec = simple_spec(ExportFormat::Farbfeld, ColorSpace::SRGB, false);
    let output = simple_round_trip("rgb16.ff", &rgb16.clone().into(), &spec);
    assert_eq!(output.color(), image::ColorType::Rgba16);
    assert!(
        output.to_rgb16() == rgb16,
        "opaque farbfeld is not bit-exact"
    );
    assert!(output.to_rgba16().pixels().all(|p| p[3] == 65_535));
}

#[test]
fn linear_qoi_is_declared_linear() {
    let image = pattern_raster(
        Size::new(40, 300),
        ChannelLayout::Rgba,
        SampleType::U8,
        ColorSpace::LINEAR_SRGB,
        |x, y, c| (((x * 7 + y * 3 + c as u32 * 50) % 255) + u32::from(c == 3)) as u16,
    );
    let doc = raster_document(image);
    let spec = simple_spec(ExportFormat::Qoi, ColorSpace::LINEAR_SRGB, true);
    let path = temp_path("linear.qoi");
    assert_eq!(export(&doc, &path, &spec).unwrap(), ExportReport::default());
    let back = open_image(&path).unwrap().image;
    std::fs::remove_file(&path).ok();
    assert_eq!(back.format().color_space, ColorSpace::LINEAR_SRGB);
    assert!(composite_u16(&raster_document(back)) == composite_u16(&doc));
}

#[test]
fn radiance_hdr_round_trips_within_rgbe_precision() {
    let size = Size::new(37, 300);
    let format = float_rgb_linear();
    // Never exactly 0: a matrix round trip could make a 0 slightly negative.
    let values: Vec<f32> = (0..size.pixel_count() as usize * 3)
        .map(|i| ((i * 37) % 1000) as f32 * 0.05 - 4.975)
        .collect();
    let negative = values.iter().filter(|v| **v < 0.0).count() as u64;
    let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_ne_bytes()).collect();
    let image = RasterImage::from_pixels(size, format, &bytes).unwrap();
    let doc = raster_document(image);
    let spec = default_spec(ExportFormatKind::Hdr, &doc);
    assert_eq!(spec.format, ExportFormat::Hdr);
    assert_eq!(spec.space, ColorSpace::LINEAR_SRGB);
    assert!(!spec.keep_alpha);
    let path = temp_path("float.hdr");
    let report = export(&doc, &path, &spec).unwrap();
    assert_eq!(report.notices, [ExportNotice::ClippedLow(negative)]);
    let back = open_image(&path).unwrap().image;
    std::fs::remove_file(&path).ok();
    assert_eq!(back.format().color_space, ColorSpace::LINEAR_SRGB);
    assert_eq!(back.format().layout, ChannelLayout::Rgb);
    // Both through the working space: compare in linear sRGB.
    let read = linear_srgb_composite(&raster_document(back));
    for (pixel, expected) in read
        .as_chunks::<3>()
        .0
        .iter()
        .zip(values.as_chunks::<3>().0)
    {
        let clipped: Vec<f32> = expected.iter().map(|v| v.max(0.0)).collect();
        let max = clipped.iter().copied().fold(0f32, f32::max);
        // Half a mantissa step of the brightest channel, and float rounding.
        for (got, want) in pixel.iter().zip(&clipped) {
            assert!(
                (got - want).abs() <= max / 256.0 + max * 1e-4 + 1e-5,
                "{got} {want} in {expected:?}"
            );
        }
    }
}

/// A document's composite as linear sRGB values, RGB.
fn linear_srgb_composite(doc: &Document) -> Vec<f32> {
    let to_srgb = slopshop_core::color::WORKING_SPACE.matrix_to(&ColorSpace::LINEAR_SRGB);
    composite_all(doc)
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|px| {
            let c = LinearRgba::new(px[0], px[1], px[2], px[3]).transform(&to_srgb);
            [c.r, c.g, c.b]
        })
        .collect()
}

#[test]
fn simple_formats_refuse_what_they_cannot_store() {
    let path = temp_path("refused.qoi");
    let small = Size::new(4, 4);
    let cases = [
        (
            simple_spec(ExportFormat::Hdr, ColorSpace::SRGB, false),
            small,
            "unsupportedSpace",
        ),
        (
            simple_spec(ExportFormat::Hdr, ColorSpace::LINEAR_SRGB, true),
            small,
            "invalidSpec",
        ),
        (
            simple_spec(ExportFormat::Farbfeld, ColorSpace::LINEAR_SRGB, true),
            small,
            "unsupportedSpace",
        ),
        (
            simple_spec(ExportFormat::Qoi, ColorSpace::DISPLAY_P3, true),
            small,
            "unsupportedSpace",
        ),
        // QOI readers refuse more than 400 million pixels.
        (
            simple_spec(ExportFormat::Qoi, ColorSpace::SRGB, true),
            Size::new(20_000, 20_001),
            "tooLarge",
        ),
    ];
    for (spec, size, code) in cases {
        let result = export_image(
            &path,
            size,
            &spec,
            |_, _| panic!("the source must not be called"),
            &CancelToken::new(),
            &mut |_| panic!("no progress expected"),
        );
        assert_eq!(result.unwrap_err().code(), code, "{:?}", spec.format);
        assert!(!path.exists() && temp_files(&path).is_empty());
    }
    for kind in [
        ExportFormatKind::Qoi,
        ExportFormatKind::Farbfeld,
        ExportFormatKind::Hdr,
        ExportFormatKind::Ico,
        ExportFormatKind::Gif,
        ExportFormatKind::Dds,
    ] {
        assert!(!has_gray(kind), "{kind:?}");
        assert!(!supports_gray(kind, &ColorSpace::SRGB), "{kind:?}");
    }
    assert!(supports_space(
        ExportFormatKind::Qoi,
        &ColorSpace::LINEAR_SRGB
    ));
    assert!(!supports_alpha(ExportFormatKind::Hdr));
}

#[test]
fn icon_gif_and_dds_limits() {
    assert_eq!(max_side(ExportFormatKind::Ico), Some(256));
    assert_eq!(max_side(ExportFormatKind::Gif), Some(65_535));
    assert_eq!(max_side(ExportFormatKind::Dds), None);
    let path = temp_path("refused.gif");
    let cases = [
        (ExportFormat::Ico, Size::new(257, 16), "tooLarge"),
        (ExportFormat::Gif, Size::new(65_536, 1), "tooLarge"),
        // Within GIF's sides, beyond what is held in memory.
        (ExportFormat::Gif, Size::new(65_535, 65_535), "tooLarge"),
    ];
    for (format, size, code) in cases {
        let result = export_image(
            &path,
            size,
            &simple_spec(format, ColorSpace::SRGB, true),
            |_, _| panic!("the source must not be called"),
            &CancelToken::new(),
            &mut |_| panic!("no progress expected"),
        );
        assert_eq!(result.unwrap_err().code(), code, "{format:?} {size:?}");
        assert!(!path.exists() && temp_files(&path).is_empty());
    }
    for kind in [
        ExportFormatKind::Ico,
        ExportFormatKind::Gif,
        ExportFormatKind::Dds,
    ] {
        assert!(supports_alpha(kind), "{kind:?}");
        assert!(supports_space(kind, &ColorSpace::SRGB), "{kind:?}");
        assert!(!supports_space(kind, &ColorSpace::LINEAR_SRGB), "{kind:?}");
    }
}

#[test]
fn ico_round_trips_bit_exact_up_to_256_pixels() {
    // 256 is stored as 0 in the directory entry.
    let (w, h) = (256, 200);
    let rgba = image::RgbaImage::from_fn(w, h, |x, y| {
        image::Rgba([
            x as u8,
            (y * 3) as u8,
            (x ^ y) as u8,
            1 + ((x * 7 + y) % 255) as u8,
        ])
    });
    let spec = simple_spec(ExportFormat::Ico, ColorSpace::SRGB, true);
    let output = simple_round_trip("icon.ico", &rgba.clone().into(), &spec);
    assert!(output.to_rgba8() == rgba, "ICO is not bit-exact");
    // Without alpha: opaque.
    let rgb = image::RgbImage::from_fn(w, h, |x, y| image::Rgb([x as u8, y as u8, 7]));
    let spec = simple_spec(ExportFormat::Ico, ColorSpace::SRGB, false);
    let output = simple_round_trip("opaque.ico", &rgb.clone().into(), &spec);
    assert!(output.to_rgb8() == rgb, "opaque ICO is not bit-exact");
    assert!(output.to_rgba8().pixels().all(|p| p[3] == 255));
}

#[test]
fn dds_round_trips_bit_exact_through_our_importer() {
    for (layout, keep_alpha) in [(ChannelLayout::Rgb, false), (ChannelLayout::Rgba, true)] {
        let image = pattern_raster(
            ODD_SIZE,
            layout,
            SampleType::U8,
            ColorSpace::SRGB,
            |x, y, c| {
                let v = (x * 5 + y * 3 + c as u32 * 40) % 255;
                // Alpha above 0: the color under alpha 0 is not kept, by design.
                (v + u32::from(c == 3)) as u16
            },
        );
        let doc = raster_document(image);
        let spec = simple_spec(ExportFormat::Dds, ColorSpace::SRGB, keep_alpha);
        let path = temp_path(&format!("texture-{keep_alpha}.dds"));
        assert_eq!(export(&doc, &path, &spec).unwrap(), ExportReport::default());
        let back = open_image(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert!(back.warnings.is_empty());
        let format = back.image.format();
        assert_eq!(
            (format.layout, format.color_space),
            (layout, ColorSpace::SRGB)
        );
        assert!(
            composite_u16(&raster_document(back.image)) == composite_u16(&doc),
            "{layout:?} is not bit-exact"
        );
    }
}

/// Export `input` (imported from a PNG) to GIF; the GIF decoded by the `image` crate, and the
/// report.
fn gif_round_trip(
    name: &str,
    input: &image::RgbaImage,
    keep_alpha: bool,
) -> (image::RgbaImage, ExportReport) {
    let source = temp_path(&format!("{name}-in.png"));
    input.save(&source).unwrap();
    let doc = raster_document(open_image(&source).unwrap().image);
    std::fs::remove_file(&source).ok();
    let path = temp_path(name);
    let spec = simple_spec(ExportFormat::Gif, ColorSpace::SRGB, keep_alpha);
    let report = export(&doc, &path, &spec).unwrap();
    assert!(temp_files(&path).is_empty());
    let output = image::open(&path).unwrap().to_rgba8();
    assert_eq!(
        open_image(&path).unwrap().image.format().color_space,
        ColorSpace::SRGB
    );
    std::fs::remove_file(&path).ok();
    (output, report)
}

#[test]
fn gif_keeps_up_to_256_colors_exactly() {
    let (w, h) = (ODD_SIZE.width, ODD_SIZE.height);
    // 255 colors and transparency (whose index is the 256th).
    let color = |i: u32| [(i * 37) as u8, (i * 11) as u8, i as u8];
    let input = image::RgbaImage::from_fn(w, h, |x, y| {
        if (x + y) % 7 == 0 {
            image::Rgba([9, 9, 9, 0])
        } else {
            let [r, g, b] = color((x / 3 + y * 5) % 255);
            image::Rgba([r, g, b, 255])
        }
    });
    let (output, report) = gif_round_trip("few.gif", &input, true);
    assert_eq!(report, ExportReport::default());
    for (got, want) in output.pixels().zip(input.pixels()) {
        if want[3] == 0 {
            assert_eq!(got[3], 0);
        } else {
            assert_eq!(got, want);
        }
    }
    // 256 colors without transparency.
    let input = image::RgbaImage::from_fn(w, h, |x, y| {
        let [r, g, b] = color((x + y * 3) % 256);
        image::Rgba([r, g, b, 255])
    });
    let (output, report) = gif_round_trip("opaque.gif", &input, false);
    assert_eq!(report, ExportReport::default());
    assert!(output == input, "256 colors are not exact");
}

#[test]
fn gif_quantizes_many_colors_and_reports_it() {
    let (w, h) = (ODD_SIZE.width, ODD_SIZE.height);
    let input = image::RgbaImage::from_fn(w, h, |x, y| {
        // A smooth gradient of thousands of colors; a column half transparent.
        let alpha = if x == 0 { 200 } else { 255 };
        image::Rgba([(x * 255 / w) as u8, (y * 255 / h) as u8, 128, alpha])
    });
    let (output, report) = gif_round_trip("many.gif", &input, true);
    let [ExportNotice::ColorsQuantized(changed)] = report.notices[..] else {
        panic!("{report:?}");
    };
    assert!(changed > 0 && changed <= u64::from(w * h), "{changed}");
    let mut error = 0u64;
    for (got, want) in output.pixels().zip(input.pixels()) {
        // Alpha 200 is opaque in a GIF.
        assert_eq!(got[3], 255);
        error += (0..3)
            .map(|c| u64::from(got[c].abs_diff(want[c])))
            .sum::<u64>();
    }
    let mean = error as f64 / f64::from(w * h * 3);
    assert!(mean < 4.0, "mean error {mean}");
}
