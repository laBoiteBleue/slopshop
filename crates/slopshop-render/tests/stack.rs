//! A layer's stack evaluated by the GPU while its pixels are not evaluated yet (ADR 0029) shows
//! what the CPU evaluates. Skipped when no adapter is available, unless
//! `SLOPSHOP_REQUIRE_GPU=1`.

use std::sync::Arc;

use slopshop_core::adjust::Adjustment;
use slopshop_core::color::PixelFormat;
use slopshop_core::selection::{SELECTION_FORMAT, Selection};
use slopshop_core::stack::{Effect, LayerStack, PaintEntry, PaintOp};
use slopshop_core::tile::TileCoord;
use slopshop_core::view::ViewTransform;
use slopshop_core::{
    Affine, BlendMode, BlendSpace, Document, Edit, Layer, LayerContent, LinearRgba, RasterImage,
    Size,
};
use slopshop_render::{Frame, Renderer};

const W: u32 = 600;
const H: u32 = 300;

fn renderer() -> Option<Renderer> {
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

fn image(format: PixelFormat, pixel: impl Fn(u32, u32) -> Vec<u8>) -> Arc<RasterImage> {
    let mut bytes = Vec::new();
    for y in 0..H {
        for x in 0..W {
            bytes.extend(pixel(x, y));
        }
    }
    Arc::new(RasterImage::from_pixels(Size::new(W, H), format, &bytes).unwrap())
}

/// Paint `op` at `amount(x, y)` (layer pixels) over `paint`, on the tiles of `coords`.
fn painted(
    paint: &PaintEntry,
    coords: &[(u32, u32)],
    op: PaintOp,
    amount: impl Fn(u32, u32) -> f32,
) -> Arc<PaintEntry> {
    let tiles = coords
        .iter()
        .map(|&(col, row)| {
            let coord = TileCoord { col, row };
            let tile = paint.painted_tile(coord, op, |x, y| {
                amount(col * 256 + x as u32, row * 256 + y as u32)
            });
            (coord, tile)
        })
        .collect();
    Arc::new(paint.with_tiles(tiles).unwrap())
}

/// A stack of soft paint, an Invert within a selection, a Brightness/Contrast everywhere and an
/// erasing paint on top, over an opaque gradient.
fn stack() -> LayerStack {
    let original = image(PixelFormat::RGBA8_SRGB, |x, y| {
        vec![
            (x * 255 / W) as u8,
            (y * 255 / H) as u8,
            ((x + y) * 255 / (W + H)) as u8,
            255,
        ]
    });
    let empty = PaintEntry::empty(original.format(), original.size(), BlendSpace::Perceptual);
    let paint = painted(
        &empty,
        &[(0, 0), (1, 0)],
        PaintOp::Color(LinearRgba::new(0.8, 0.1, 0.05, 1.0)),
        |x, y| ((x * 3 + y) % 200) as f32 / 200.0,
    );
    let selection = image(SELECTION_FORMAT, |x, y| {
        let v: u16 = if (100..400).contains(&x) && (50..250).contains(&y) {
            u16::MAX
        } else {
            0
        };
        v.to_ne_bytes().to_vec()
    });
    let invert = Effect {
        adjustment: Adjustment::Invert,
        selection: Selection::new(selection),
        to_document: Affine::IDENTITY,
        space: BlendSpace::Perceptual,
    };
    let brighter = Effect {
        adjustment: Adjustment::BrightnessContrast {
            brightness: 40.0,
            contrast: 20.0,
        },
        selection: None,
        to_document: Affine::IDENTITY,
        space: BlendSpace::Perceptual,
    };
    let erased = painted(&empty, &[(2, 1)], PaintOp::Erase, |x, _| {
        if x > 520 { 0.7 } else { 0.0 }
    });
    LayerStack::new(original)
        .with_top_paint(paint)
        .unwrap()
        .with_effect(invert)
        .unwrap()
        .with_effect(brighter)
        .unwrap()
        .with_top_paint(erased)
        .unwrap()
}

/// A document with one raster layer holding `stack`, its pixels evaluated (`ready`) or not.
fn document(stack: &LayerStack, ready: bool) -> Document {
    let mut doc = Document::new(Size::new(W, H));
    let id = doc.allocate_layer_id();
    Edit::InsertLayer {
        parent: None,
        index: 0,
        layer: Layer {
            transform: Affine::IDENTITY,
            clipped: false,
            id,
            name: "stacked".into(),
            visible: true,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            mask: None,
            content: LayerContent::raster(Arc::clone(stack.original())),
        },
    }
    .apply(&mut doc)
    .unwrap();
    Edit::SetLayerStack {
        id,
        stack: stack.clone(),
        shown: ready.then(|| stack.evaluate().unwrap()),
    }
    .apply(&mut doc)
    .unwrap();
    doc
}

fn pending(doc: &Document) -> bool {
    match &doc.layers()[0].content {
        LayerContent::Raster { image, .. } => image.ready_image().is_none(),
        _ => false,
    }
}

/// The largest and the mean difference of the samples of two frames.
fn differences(a: &Frame, b: &Frame) -> (u8, f64) {
    let worst = a
        .data
        .iter()
        .zip(&b.data)
        .map(|(x, y)| x.abs_diff(*y))
        .max();
    let sum: u64 = a
        .data
        .iter()
        .zip(&b.data)
        .map(|(x, y)| u64::from(x.abs_diff(*y)))
        .sum();
    (worst.unwrap_or(0), sum as f64 / a.data.len().max(1) as f64)
}

#[test]
fn the_gpu_shows_a_stack_as_the_cpu_evaluates_it() {
    if renderer().is_none() {
        return;
    }
    let stack = stack();
    let gpu = document(&stack, false);
    let cpu = document(&stack, true);
    assert!(pending(&gpu) && !pending(&cpu));
    for cached in [true, false] {
        let r = Renderer::new()
            .unwrap()
            .with_display_cache(cached)
            .with_stack_evaluation(false);
        // At 100 %: the same pixels but for rounding (the CPU rounds after each entry).
        let view = ViewTransform {
            origin: [0.0, 0.0],
            scale: 1.0,
        };
        let a = r.render_view(&gpu, view, Size::new(W, H)).unwrap();
        let b = r.render_view(&cpu, view, Size::new(W, H)).unwrap();
        let (worst, _) = differences(&a, &b);
        assert!(worst <= 3, "cached {cached}: {worst}");
        // Zoomed out, the stack is evaluated on the coarser level of each image: close, but
        // for the edges of paint with nonlinear effects above it (until the pixels are there).
        let view = ViewTransform {
            origin: [0.0, 0.0],
            scale: 2.0,
        };
        let a = r.render_view(&gpu, view, Size::new(W / 2, H / 2)).unwrap();
        let b = r.render_view(&cpu, view, Size::new(W / 2, H / 2)).unwrap();
        let (_, mean) = differences(&a, &b);
        assert!(mean <= 1.5, "cached {cached}, zoomed out: {mean}");
    }
    // The shader showed the stack: the pixels were not evaluated.
    assert!(pending(&gpu));
}

#[test]
fn frames_showing_a_stack_ask_again_until_its_pixels_are_there() {
    let Some(r) = renderer() else { return };
    let gpu = document(&stack(), false);
    let view = ViewTransform {
        origin: [0.0, 0.0],
        scale: 1.0,
    };
    let stats = r.profile_view(&gpu, view, Size::new(W, H), false).unwrap();
    assert!(stats.incomplete);
    // Evaluated on a thread of its own: waiting for it gives the pixels.
    let LayerContent::Raster { image, .. } = &gpu.layers()[0].content else {
        panic!("a raster layer");
    };
    image.get();
    let stats = r.profile_view(&gpu, view, Size::new(W, H), false).unwrap();
    assert!(!stats.incomplete);
}
