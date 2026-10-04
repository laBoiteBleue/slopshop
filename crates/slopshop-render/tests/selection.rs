//! The selection tools (Magic Wand, Grow, Similar, Color Range) reading their pixels from the
//! GPU (`export_renderer`, rows of tiles at full resolution) rather than the CPU compositor.
//! Skipped when no GPU adapter is available, unless `SLOPSHOP_REQUIRE_GPU=1`, or when
//! `SLOPSHOP_SKIP_GPU_TESTS=1`.
//!
//! The tools compare 8-bit display values, so a GPU value that differs from the CPU's by a
//! rounding at a boundary can flip a pixel. Documents with flat colors and exact blends give
//! identical selections; a document with resampled layers (EWA on the CPU, the GPU's own
//! filter) may differ along edges: the tests count those pixels and bound them.

use std::sync::{Arc, Mutex};

use slopshop_core::blend::BlendMode;
use slopshop_core::color::{AlphaMode, ChannelLayout, ColorSpace, PixelFormat, SampleType};
use slopshop_core::selection::{
    self, ColorRange, Combine, Localized, PixelSource, WandOptions, color_range_from,
    color_range_with, grow, grow_from, magic_wand, magic_wand_from, sample_colors,
    sample_colors_from,
};
use slopshop_core::stack::Pixels;
use slopshop_core::{
    Affine, Document, Edit, Layer, LayerContent, LinearRgba, RasterImage, Rect, Session, Size,
};
use slopshop_render::Renderer;

fn renderer() -> Option<Renderer> {
    // Windows CI runners render on WARP (software Direct3D 12), far too slow for these tests:
    // Linux CI runs them on lavapipe.
    if std::env::var("SLOPSHOP_SKIP_GPU_TESTS").as_deref() == Ok("1") {
        eprintln!("skipping GPU test: SLOPSHOP_SKIP_GPU_TESTS=1");
        return None;
    }
    match Renderer::new() {
        Ok(r) => Some(r),
        Err(e) if std::env::var("SLOPSHOP_REQUIRE_GPU").as_deref() == Ok("1") => {
            panic!("GPU required but unavailable: {e}")
        }
        Err(e) => {
            eprintln!("skipping GPU test: {e}");
            None
        }
    }
}

/// The source the app gives the tools: export renderers, one per concurrent call (pooled).
fn gpu_source(
    renderer: &Renderer,
) -> impl Fn(&Document, Rect, &mut [f32]) -> Result<(), String> + Sync + '_ {
    type Render<'a> =
        Box<dyn FnMut(&Document, Rect, &mut [f32]) -> Result<u64, String> + Send + 'a>;
    let pool: Mutex<Vec<Render<'_>>> = Mutex::new(Vec::new());
    move |document, region, out| {
        let free = pool.lock().unwrap().pop();
        let mut render =
            free.unwrap_or_else(|| Box::new(slopshop_render::export_renderer(Some(renderer))));
        let result = render(document, region, out).map(|_| ());
        pool.lock().unwrap().push(render);
        result
    }
}

fn push_layer(
    session: &mut Session,
    content: LayerContent,
    opacity: f32,
    transform: Affine,
    blend_mode: BlendMode,
) {
    let id = session.allocate_layer_id();
    let index = session.document().layers().len();
    session
        .perform(Edit::InsertLayer {
            parent: None,
            index,
            layer: Layer {
                style: None,
                transform,
                clipped: false,
                id,
                name: "layer".into(),
                visible: true,
                opacity,
                blend_mode,
                mask: None,
                content,
            },
        })
        .unwrap();
}

fn raster(size: Size, format: PixelFormat, pixel: impl Fn(u32, u32) -> Vec<u8>) -> LayerContent {
    let mut bytes = Vec::new();
    for y in 0..size.height {
        for x in 0..size.width {
            bytes.extend(pixel(x, y));
        }
    }
    LayerContent::Raster {
        stack: None,
        image: Pixels::ready(Arc::new(
            RasterImage::from_pixels(size, format, &bytes).unwrap(),
        )),
    }
}

const SIZE: Size = Size::new(700, 420);

/// Flat blocks across several tiles: red left, blue right with a red square in it and a
/// slightly different red strip, a green fill at half opacity over the lower right corner.
/// Everything axis-aligned, so the GPU and the CPU round the same values.
fn blocks() -> Document {
    let mut s = Session::new(Document::new(SIZE));
    let base = raster(SIZE, PixelFormat::RGBA8_SRGB, |x, y| {
        let red = x < 350 || ((520..600).contains(&x) && (100..180).contains(&y));
        let rgb = if x < 350 && y < 30 {
            [245, 8, 0]
        } else if red {
            [255, 0, 0]
        } else {
            [0, 0, 255]
        };
        vec![rgb[0], rgb[1], rgb[2], 255]
    });
    push_layer(&mut s, base, 1.0, Affine::IDENTITY, BlendMode::Normal);
    // A half-transparent layer (a 300 x 200 raster placed at (400, 220)) and a half-opaque fill.
    let spot = raster(Size::new(300, 200), PixelFormat::RGBA8_SRGB, |x, _| {
        vec![20, 200, 60, if x < 150 { 255 } else { 128 }]
    });
    push_layer(
        &mut s,
        spot,
        0.5,
        Affine::translation(400.0, 220.0),
        BlendMode::Normal,
    );
    push_layer(
        &mut s,
        LayerContent::Fill {
            color: LinearRgba::new(0.2, 0.2, 0.2, 0.5),
        },
        0.3,
        Affine::IDENTITY,
        BlendMode::Multiply,
    );
    s.document().clone()
}

/// Smooth gradients under a rotated, scaled layer in a wide gamut: every pixel's value is
/// resampled, so the GPU's filter and the CPU's differ by float rounding, and 8-bit rounding
/// then flips the pixels whose value is at a boundary.
fn gradients() -> Document {
    let mut s = Session::new(Document::new(SIZE));
    let base = raster(SIZE, PixelFormat::RGBA8_SRGB, |x, y| {
        vec![
            (x * 255 / SIZE.width) as u8,
            (y * 255 / SIZE.height) as u8,
            ((x + y) * 255 / (SIZE.width + SIZE.height)) as u8,
            255,
        ]
    });
    push_layer(&mut s, base, 1.0, Affine::IDENTITY, BlendMode::Normal);
    let p3 = PixelFormat {
        layout: ChannelLayout::Rgba,
        sample: SampleType::U16,
        color_space: ColorSpace::DISPLAY_P3,
        alpha: AlphaMode::Straight,
    };
    let wide = raster(Size::new(500, 300), p3, |x, y| {
        [
            x * 131 % 65536,
            y * 217 % 65536,
            (x + y) * 97 % 65536,
            50000,
        ]
        .iter()
        .flat_map(|&v| (v as u16).to_ne_bytes())
        .collect()
    });
    let placed = Affine::rotation(0.4)
        .then(Affine::scale(1.3, 1.3))
        .then(Affine::translation(150.0, 20.0));
    push_layer(&mut s, wide, 0.6, placed, BlendMode::Normal);
    s.document().clone()
}

/// The coverage of a selection of a canvas of `size` at every pixel (nothing selected: zeros).
fn coverage(size: Size, selection: &Option<RasterImage>) -> Vec<f32> {
    let (w, h) = (size.width as usize, size.height as usize);
    match selection {
        Some(image) => {
            selection::sample_grid(image, Rect::new(0, 0, size.width, size.height), w, h)
        }
        None => vec![0.0; w * h],
    }
}

/// The pixels where two selections of a canvas of `size` differ.
fn differences_in(size: Size, a: &Option<RasterImage>, b: &Option<RasterImage>) -> usize {
    coverage(size, a)
        .iter()
        .zip(coverage(size, b))
        .filter(|(a, b)| **a != *b)
        .count()
}

/// The pixels where two selections of [`SIZE`] differ.
fn differences(a: &Option<RasterImage>, b: &Option<RasterImage>) -> usize {
    differences_in(SIZE, a, b)
}

/// Every tool on `doc`, on the CPU and from the GPU: the number of pixels that differ for each.
fn compare(renderer: &Renderer, doc: &Document, label: &str) -> Vec<(String, usize)> {
    let source = gpu_source(renderer);
    let pixels: Option<&PixelSource<'_>> = Some(&source);
    let mut found = Vec::new();
    let mut note = |name: String, count: usize| {
        eprintln!("{label}: {name}: {count} pixels differ");
        found.push((name, count));
    };
    let token = slopshop_core::job::CancelToken::new();
    for seed in [(100, 200), (560, 140), (650, 400), (500, 300)] {
        for (tolerance, contiguous, anti_alias) in [
            (0.0, true, false),
            (12.0, true, false),
            (32.0, true, true),
            (12.0, false, false),
            (60.0, false, false),
        ] {
            let options = WandOptions {
                tolerance,
                contiguous,
                anti_alias,
            };
            let cpu = magic_wand(doc, None, seed, options, Combine::Replace).unwrap();
            let gpu = magic_wand_from(
                doc,
                pixels,
                None,
                seed,
                options,
                Combine::Replace,
                &|_, _| {},
                &slopshop_core::job::CancelToken::new(),
            )
            .unwrap();
            note(
                format!("wand {seed:?} tolerance {tolerance} contiguous {contiguous}"),
                differences(&cpu, &gpu),
            );
        }
    }
    // Grow and Similar from a small selection, on the CPU's own wand selection.
    let start = selection::select_shape(
        SIZE,
        None,
        &selection::Shape::Rectangle {
            left: 90.0,
            top: 190.0,
            right: 110.0,
            bottom: 210.0,
        },
        selection::EdgeOptions::default(),
        Combine::Replace,
    )
    .unwrap()
    .unwrap();
    for (tolerance, contiguous) in [(4.0, true), (20.0, true), (4.0, false), (20.0, false)] {
        let options = WandOptions {
            tolerance,
            contiguous,
            anti_alias: false,
        };
        let cpu = grow(doc, &start, options).unwrap();
        let gpu = grow_from(
            doc,
            pixels,
            &start,
            options,
            &|_, _| {},
            &slopshop_core::job::CancelToken::new(),
        )
        .unwrap();
        note(
            format!("grow tolerance {tolerance} contiguous {contiguous}"),
            differences(&cpu, &gpu),
        );
    }
    // Color Range from the colors at two points, taken from each source.
    for fuzziness in [0.0, 30.0, 100.0] {
        let range = |colors: Vec<[f32; 4]>| ColorRange {
            included: colors,
            excluded: Vec::new(),
            fuzziness,
            invert: false,
            localized: Some(Localized {
                points: vec![(100.5, 200.5), (560.5, 140.5)],
                radius: 250.0,
            }),
        };
        let points = [(100, 200), (560, 140)];
        let cpu_range = range(sample_colors(doc, &points));
        let gpu_range = range(sample_colors_from(doc, pixels, &points));
        let cpu = color_range_with(doc, None, &cpu_range, &|_, _| {}, &token).unwrap();
        let gpu = color_range_from(doc, pixels, None, &gpu_range, &|_, _| {}, &token).unwrap();
        note(
            format!("color range fuzziness {fuzziness}"),
            differences(&cpu, &gpu),
        );
    }
    found
}

#[test]
fn flat_colors_select_the_same_pixels_from_the_gpu() {
    let Some(r) = renderer() else { return };
    let found = compare(&r, &blocks(), "blocks");
    // Flat colors and exact blends: nothing may differ.
    for (name, count) in &found {
        assert_eq!(*count, 0, "{name}");
    }
}

#[test]
fn resampled_layers_select_nearly_the_same_pixels_from_the_gpu() {
    let Some(r) = renderer() else { return };
    let found = compare(&r, &gradients(), "gradients");
    // A resampled gradient is the worst case: the GPU's filter and the CPU's differ by float
    // rounding, which flips a pixel only where its 8-bit value is at a boundary. Measured at
    // a pixel or two of 294000; a wrong source or a shifted tile would differ
    // by many times that.
    let canvas = (SIZE.width * SIZE.height) as usize;
    for (name, count) in &found {
        assert!(
            *count * 1000 <= canvas,
            "{name}: {count} of {canvas} pixels differ"
        );
    }
}

#[test]
fn the_displayed_colors_from_the_gpu_match_the_cpu_ones() {
    let Some(r) = renderer() else { return };
    for (doc, label, most_wrong) in [(blocks(), "blocks", 0), (gradients(), "gradients", 300)] {
        let source = gpu_source(&r);
        let points: Vec<(u32, u32)> = (0..SIZE.height)
            .flat_map(|y| (0..SIZE.width).map(move |x| (x, y)))
            .collect();
        let cpu = sample_colors(&doc, &points);
        let gpu = sample_colors_from(&doc, Some(&source), &points);
        let mut wrong = 0;
        let mut worst = 0f32;
        for (c, g) in cpu.iter().zip(&gpu) {
            let d = (0..4).map(|i| (c[i] - g[i]).abs()).fold(0.0, f32::max);
            wrong += usize::from(d > 0.0);
            worst = worst.max(d);
        }
        eprintln!(
            "{label}: {wrong} of {} colors differ, by {worst} level(s) at most",
            points.len()
        );
        assert!(wrong <= most_wrong, "{label}: {wrong} colors differ");
        assert!(worst <= 1.0, "{label}: by {worst} levels");
    }
}

#[test]
fn the_active_layer_alone_selects_the_same_from_the_gpu() {
    let Some(r) = renderer() else { return };
    // A document holding one layer, as the app samples "the active layer only".
    let doc = blocks();
    let layer = doc.layers()[1].clone();
    let mut layer = layer;
    layer.clipped = false;
    let alone = Document::restore(
        doc.size(),
        doc.working_space(),
        doc.blend_space(),
        vec![layer],
        doc.next_layer_id(),
    )
    .unwrap();
    let source = gpu_source(&r);
    let options = WandOptions {
        tolerance: 10.0,
        contiguous: true,
        anti_alias: false,
    };
    for seed in [(450, 300), (650, 300), (10, 10)] {
        let cpu = magic_wand(&alone, None, seed, options, Combine::Replace).unwrap();
        let gpu = magic_wand_from(
            &alone,
            Some(&source),
            None,
            seed,
            options,
            Combine::Replace,
            &|_, _| {},
            &slopshop_core::job::CancelToken::new(),
        )
        .unwrap();
        assert_eq!(differences(&cpu, &gpu), 0, "{seed:?}");
    }
}

/// A large document for timing: flat quadrants with a small patch, under `overlays` half-opaque
/// textures (a mix of blend modes, one rotated and resampled every third), each layer built row
/// by row on every core.
fn large(width: u32, height: u32, overlays: u32) -> Document {
    let size = Size::new(width, height);
    let mut s = Session::new(Document::new(size));
    let make = |pixel: &(dyn Fn(u32, u32) -> [u8; 4] + Sync)| {
        let mut bytes = vec![0u8; width as usize * height as usize * 4];
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
        let rows = (height as usize).div_ceil(threads);
        std::thread::scope(|scope| {
            for (band, chunk) in bytes.chunks_mut(rows * width as usize * 4).enumerate() {
                scope.spawn(move || {
                    for (i, px) in chunk.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                        let (x, y) = (
                            (i % width as usize) as u32,
                            (band * rows + i / width as usize) as u32,
                        );
                        px.copy_from_slice(&pixel(x, y));
                    }
                });
            }
        });
        LayerContent::Raster {
            stack: None,
            image: Pixels::ready(Arc::new(
                RasterImage::from_pixels(size, PixelFormat::RGBA8_SRGB, &bytes).unwrap(),
            )),
        }
    };
    let base = make(&|x, y| {
        if (300..900).contains(&x) && (300..900).contains(&y) {
            return [10, 200, 200, 255];
        }
        match (x < width / 2, y < height / 2) {
            (true, true) => [200, 40, 40, 255],
            (false, true) => [40, 40, 200, 255],
            (true, false) => [40, 160, 40, 255],
            (false, false) => [220, 220, 60, 255],
        }
    });
    push_layer(&mut s, base, 1.0, Affine::IDENTITY, BlendMode::Normal);
    let modes = [BlendMode::Multiply, BlendMode::Normal, BlendMode::Screen];
    for i in 0..overlays {
        let texture = make(&|x, y| {
            let on = (x / 64 + y / 64 + i) % 2 == 0;
            [128, 128, 128, if on { 40 } else { 0 }]
        });
        let placed = if i % 3 == 2 {
            Affine::rotation(0.01).then(Affine::translation(5.0, 5.0))
        } else {
            Affine::IDENTITY
        };
        push_layer(&mut s, texture, 0.5, placed, modes[i as usize % 3]);
    }
    s.document().clone()
}

/// Timings of every tool on the CPU and from the GPU on large documents; run by hand:
/// `cargo test -p slopshop-render --release --test selection -- --ignored --nocapture`
/// (`SLOPSHOP_BENCH_SIZE=8000x6250`, the default, 50 MP). The GPU is run twice, the best kept (its first
/// call builds its pipelines and uploads), the CPU once.
#[test]
#[ignore = "timing, by hand"]
fn timings_on_a_large_document() {
    let Some(r) = renderer() else { return };
    let (width, height) = std::env::var("SLOPSHOP_BENCH_SIZE")
        .ok()
        .and_then(|v| {
            let (w, h) = v.split_once('x')?;
            Some((w.parse().ok()?, h.parse().ok()?))
        })
        .unwrap_or((8000u32, 6250u32));
    let token = slopshop_core::job::CancelToken::new();
    eprintln!(
        "{width} x {height} ({:.0} MP), adapter {:?}",
        f64::from(width) * f64::from(height) / 1e6,
        r.adapter_summary()
    );
    let best = |runs: usize, tool: &dyn Fn() -> Option<RasterImage>| {
        let mut best = std::time::Duration::MAX;
        let mut last = None;
        for _ in 0..runs {
            let t = std::time::Instant::now();
            last = tool();
            best = best.min(t.elapsed());
        }
        (best, last)
    };
    // Overlay layers over the base, `SLOPSHOP_BENCH_OVERLAYS=1,8` by default.
    let overlays: Vec<u32> = std::env::var("SLOPSHOP_BENCH_OVERLAYS")
        .unwrap_or_else(|_| "1,8".into())
        .split(',')
        .filter_map(|n| n.parse().ok())
        .collect();
    for overlays in overlays {
        let doc = large(width, height, overlays);
        let source = gpu_source(&r);
        let pixels: Option<&PixelSource<'_>> = Some(&source);
        eprintln!("-- {} layers", overlays + 1);
        let time = |name: &str,
                    cpu: &dyn Fn() -> Option<RasterImage>,
                    gpu: &dyn Fn() -> Option<RasterImage>| {
            let (gpu_time, from_gpu) = best(2, gpu);
            let (cpu_time, from_cpu) = best(1, cpu);
            eprintln!(
                "{name}: CPU {} ms, GPU {} ms (x{:.1}), {} pixels differ",
                cpu_time.as_millis(),
                gpu_time.as_millis(),
                cpu_time.as_secs_f64() / gpu_time.as_secs_f64(),
                differences_in(Size::new(width, height), &from_cpu, &from_gpu)
            );
        };
        let options = |tolerance: f32, contiguous: bool| WandOptions {
            tolerance,
            contiguous,
            anti_alias: false,
        };
        // The large quadrant (all the tiles of the canvas), then the small patch (about 12 tiles).
        for (what, seed) in [
            ("quadrant", (width / 4, height / 4 + 2000)),
            ("patch", (500, 500)),
        ] {
            for contiguous in [true, false] {
                let o = options(20.0, contiguous);
                time(
                    &format!("wand {what}, contiguous {contiguous}"),
                    &|| magic_wand(&doc, None, seed, o, Combine::Replace).unwrap(),
                    &|| {
                        magic_wand_from(
                            &doc,
                            pixels,
                            None,
                            seed,
                            o,
                            Combine::Replace,
                            &|_, _| {},
                            &slopshop_core::job::CancelToken::new(),
                        )
                        .unwrap()
                    },
                );
            }
        }
        let start = selection::select_shape(
            Size::new(width, height),
            None,
            &selection::Shape::Rectangle {
                left: 400.0,
                top: 400.0,
                right: 500.0,
                bottom: 500.0,
            },
            selection::EdgeOptions::default(),
            Combine::Replace,
        )
        .unwrap()
        .unwrap();
        for contiguous in [true, false] {
            let o = options(20.0, contiguous);
            time(
                &format!("grow patch, contiguous {contiguous}"),
                &|| grow(&doc, &start, o).unwrap(),
                &|| {
                    grow_from(
                        &doc,
                        pixels,
                        &start,
                        o,
                        &|_, _| {},
                        &slopshop_core::job::CancelToken::new(),
                    )
                    .unwrap()
                },
            );
        }
        let range = |colors| ColorRange {
            included: colors,
            excluded: Vec::new(),
            fuzziness: 40.0,
            invert: false,
            localized: None,
        };
        let (cpu_range, gpu_range) = (
            range(sample_colors(&doc, &[(500, 500)])),
            range(sample_colors_from(&doc, pixels, &[(500, 500)])),
        );
        time(
            "color range",
            &|| color_range_with(&doc, None, &cpu_range, &|_, _| {}, &token).unwrap(),
            &|| color_range_from(&doc, pixels, None, &gpu_range, &|_, _| {}, &token).unwrap(),
        );
    }
}
