//! Filter layers on the display (ADR 0037): what is below one, filtered. Skipped when no adapter
//! is available, unless `SLOPSHOP_REQUIRE_GPU=1`, or when `SLOPSHOP_SKIP_GPU_TESTS=1`.

use std::sync::Arc;

use slopshop_core::color::PixelFormat;
use slopshop_core::filter::Filter;
use slopshop_core::view::ViewTransform;
use slopshop_core::{
    Affine, BlendMode, BlendSpace, Document, Layer, LayerContent, LayerId, RasterImage, Size,
    stack::Pixels,
};
use slopshop_render::{Frame, Renderer};

fn renderer() -> Option<Renderer> {
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

fn layer(id: u64, content: LayerContent) -> Layer {
    Layer {
        id: LayerId::from_raw(id),
        name: format!("layer {id}"),
        visible: true,
        opacity: 1.0,
        blend_mode: BlendMode::Normal,
        mask: None,
        clipped: false,
        transform: Affine::IDENTITY,
        style: None,
        content,
    }
}

/// Opaque black left of the middle, white from there, `width` × `height`, built row by row.
fn halves(width: u32, height: u32) -> LayerContent {
    let row: Vec<u8> = (0..width)
        .flat_map(|x| {
            let v = if x < width / 2 { 0 } else { 255 };
            [v, v, v, 255]
        })
        .collect();
    let bytes: Vec<u8> = (0..height).flat_map(|_| row.iter().copied()).collect();
    let image = RasterImage::from_pixels(Size::new(width, height), PixelFormat::RGBA8_SRGB, &bytes)
        .unwrap();
    LayerContent::Raster {
        image: Pixels::ready(Arc::new(image)),
        stack: None,
    }
}

fn document(width: u32, height: u32, layers: Vec<Layer>) -> Document {
    Document::restore(
        Size::new(width, height),
        slopshop_core::color::WORKING_SPACE,
        BlendSpace::Perceptual,
        layers,
        100,
    )
    .unwrap()
}

fn pixel(frame: &Frame, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * frame.size.width + x) * 4) as usize;
    frame.data[i..i + 4].try_into().unwrap()
}

/// Render until nothing is pending.
fn settled(r: &Renderer, doc: &Document, view: ViewTransform, output: Size) -> Frame {
    for _ in 0..200 {
        let (frame, stats) = r.render_view_progressive(doc, view, output).unwrap();
        if !stats.incomplete {
            return frame;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the frame never completed");
}

#[test]
fn a_filter_layer_blurs_what_is_below_it() {
    let Some(r) = renderer() else { return };
    let (w, h) = (300, 120);
    let blur = Filter::GaussianBlur { radius: 4.0 };
    let doc = document(
        w,
        h,
        vec![
            layer(1, halves(w, h)),
            layer(2, LayerContent::Filter { filter: blur }),
        ],
    );
    let view = ViewTransform {
        origin: [0.0, 0.0],
        scale: 1.0,
    };
    let output = Size::new(w, h);
    let frame = settled(&r, &doc, view, output);
    // Far from the edge (and at the canvas' edges, which repeat), unchanged.
    for x in [0, 100, 200, 299] {
        let expected = if x < 150 { 0 } else { 255 };
        assert_eq!(pixel(&frame, x, 60)[0], expected, "x {x}");
    }
    // Across it, a ramp.
    let ramp: Vec<u8> = (140..160).map(|x| pixel(&frame, x, 60)[0]).collect();
    assert!(ramp.windows(2).all(|p| p[0] <= p[1]), "{ramp:?}");
    assert!(ramp[2] > 0 && ramp[17] < 255, "{ramp:?}");
    // Half its opacity: half way between the sharp edge and the blurred one.
    let mut fading = layer(2, LayerContent::Filter { filter: blur });
    fading.opacity = 0.5;
    let half = document(w, h, vec![layer(1, halves(w, h)), fading]);
    let faded = settled(&r, &half, view, output);
    let (sharp, blurred, mixed) = (0u8, pixel(&frame, 148, 60)[0], pixel(&faded, 148, 60)[0]);
    assert!(
        mixed > sharp && mixed < blurred,
        "{sharp} {mixed} {blurred}"
    );
}

/// The time a filter layer's image takes on a large document, at 100 % and zoomed out, for a
/// small and a large radius. `cargo test --release -p slopshop-render --test filter_layers --
/// --ignored --nocapture`, with `SLOPSHOP_BENCH_SIZE=<width>x<height>` (default 8000x6000).
#[test]
#[ignore]
fn filter_layer_timing() {
    let Some(r) = renderer() else { return };
    let (w, h) = std::env::var("SLOPSHOP_BENCH_SIZE")
        .ok()
        .and_then(|s| {
            let (a, b) = s.split_once('x')?;
            Some((a.parse().ok()?, b.parse().ok()?))
        })
        .unwrap_or((8000u32, 6000u32));
    let output = Size::new(2560, 1440);
    eprintln!("{}", r.adapter_summary().name);
    for radius in [10.0, 200.0] {
        for scale in [1.0, 4.0] {
            let doc = document(
                w,
                h,
                vec![
                    layer(1, halves(w, h)),
                    layer(
                        2,
                        LayerContent::Filter {
                            filter: Filter::GaussianBlur { radius },
                        },
                    ),
                ],
            );
            let view = ViewTransform {
                origin: [
                    f64::from(w) / 2.0 - 1280.0 * scale,
                    f64::from(h) / 2.0 - 720.0 * scale,
                ],
                scale,
            };
            let start = std::time::Instant::now();
            r.render_view(&doc, view, output).unwrap();
            let first = start.elapsed();
            let start = std::time::Instant::now();
            r.render_view(&doc, view, output).unwrap();
            let again = start.elapsed();
            eprintln!(
                "{w}x{h}, radius {radius}, scale {scale}: first frame {first:?}, unchanged {again:?}"
            );
        }
    }
}
