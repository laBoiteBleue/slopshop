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
