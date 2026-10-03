//! Full-resolution region rendering (export). Skipped when no GPU adapter is available, unless
//! `SLOPSHOP_REQUIRE_GPU=1`.

use std::sync::Arc;

use slopshop_core::color::{
    AlphaMode, ChannelLayout, ColorSpace, Mat3, PixelFormat, SampleType, WORKING_SPACE, mat_vec,
    srgb_decode,
};
use slopshop_core::{Affine, BlendMode, BlendSpace, LayerId, Size};
use slopshop_core::{Document, Edit, Layer, LayerContent, LinearRgba, RasterImage, Rect, Session};
use slopshop_render::{RenderError, Renderer};

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

fn push_layer(session: &mut Session, content: LayerContent, opacity: f32) -> LayerId {
    let id = session.allocate_layer_id();
    let index = session.document().layers().len();
    session
        .perform(Edit::InsertLayer {
            parent: None,
            index,
            layer: Layer {
                transform: slopshop_core::Affine::IDENTITY,
                clipped: false,
                id,
                name: "layer".into(),
                visible: true,
                opacity,
                blend_mode: BlendMode::Normal,
                mask: None,
                content,
            },
        })
        .unwrap();
    id
}

fn image(size: Size, format: PixelFormat, pixel: impl Fn(u32, u32) -> Vec<u8>) -> Arc<RasterImage> {
    let mut bytes = Vec::new();
    for y in 0..size.height {
        for x in 0..size.width {
            bytes.extend(pixel(x, y));
        }
    }
    Arc::new(RasterImage::from_pixels(size, format, &bytes).unwrap())
}

fn raster(image: &Arc<RasterImage>) -> LayerContent {
    LayerContent::Raster {
        stack: None,
        image: slopshop_core::stack::Pixels::ready(image.clone()),
    }
}

fn float_format(space: ColorSpace, alpha: AlphaMode) -> PixelFormat {
    PixelFormat {
        layout: ChannelLayout::Rgba,
        sample: SampleType::F32,
        color_space: space,
        alpha,
    }
}

fn floats(values: [f32; 4]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_ne_bytes()).collect()
}

/// A pattern with distinct values per pixel, to catch any tile or coordinate mix-up.
fn pattern(x: u32, y: u32) -> Vec<u8> {
    vec![(x % 251) as u8, (y % 241) as u8, ((x + y) % 239) as u8, 255]
}

/// A document blending in linear space, where normal mode is premultiplied "over" (and values
/// near the f32 limit stay comparable between the GPU and the f64 CPU reference).
fn linear_document(size: Size) -> Document {
    let mut doc = Document::new(size);
    Edit::SetBlendSpace {
        space: BlendSpace::Linear,
    }
    .apply(&mut doc)
    .unwrap();
    doc
}

fn render(r: &Renderer, doc: &Document, region: Rect) -> Vec<f32> {
    let mut out = vec![0.0; region.size().pixel_count() as usize * 4];
    r.render_region(doc, region, &mut out).unwrap();
    out
}

/// Pixel (x, y) of a region's output, in region coordinates.
fn at(out: &[f32], region: Rect, x: u32, y: u32) -> [f32; 4] {
    let i = ((y * region.width + x) * 4) as usize;
    out[i..i + 4].try_into().unwrap()
}

/// Premultiplied working-space value of an 8-bit sRGB pixel, decoded on the CPU.
fn expected_srgb8(px: &[u8], to_working: &Mat3) -> [f32; 4] {
    let a = f32::from(px[3]) / 255.0;
    let linear = [0, 1, 2].map(|i| f64::from(srgb_decode(f32::from(px[i]) / 255.0) * a));
    let rgb = mat_vec(to_working, linear);
    [rgb[0] as f32, rgb[1] as f32, rgb[2] as f32, a]
}

fn assert_close(actual: [f32; 4], expected: [f32; 4], what: &str) {
    let ok = actual
        .iter()
        .zip(expected)
        .all(|(a, e)| (a - e).abs() <= 2e-6 + 1e-5 * e.abs());
    assert!(ok, "{what}: {actual:?} != expected {expected:?}");
}

#[test]
fn srgb_raster_region_is_linear_working_space_at_full_resolution() {
    let Some(r) = renderer() else { return };
    let size = Size::new(600, 300);
    let img = image(size, PixelFormat::RGBA8_SRGB, pattern);
    let mut s = Session::new(Document::new(size));
    push_layer(&mut s, raster(&img), 1.0);
    let m = img.matrix_to(&WORKING_SPACE);

    // Not aligned to tiles, spanning two tile columns and two tile rows.
    let region = Rect::new(100, 200, 300, 90);
    let out = render(&r, s.document(), region);
    for y in 0..region.height {
        for x in 0..region.width {
            let px = pattern(region.x + x, region.y + y);
            assert_close(at(&out, region, x, y), expected_srgb8(&px, &m), "pixel");
        }
    }

    // Straight alpha comes out premultiplied.
    let size = Size::new(2, 1);
    let img = image(size, PixelFormat::RGBA8_SRGB, |x, _| {
        vec![255, 128, 0, if x == 0 { 128 } else { 0 }]
    });
    let mut s = Session::new(Document::new(size));
    push_layer(&mut s, raster(&img), 1.0);
    let region = size.bounds();
    let out = render(&r, s.document(), region);
    assert_close(
        at(&out, region, 0, 0),
        expected_srgb8(&[255, 128, 0, 128], &m),
        "half transparent",
    );
    assert_eq!(at(&out, region, 1, 0), [0.0; 4]);
}

#[test]
fn float_values_are_not_clamped() {
    let Some(r) = renderer() else { return };
    let size = Size::new(3, 1);
    let straight = float_format(WORKING_SPACE, AlphaMode::Straight);
    let values = [
        [4.0, 1e5, -0.5, 1.0],
        [70_000.0, 0.25, 3e38, 1.0],
        [-1e6, 0.0, 65_505.0, 1.0],
    ];
    let img = image(size, straight, |x, _| floats(values[x as usize]));
    let mut s = Session::new(Document::new(size));
    push_layer(&mut s, raster(&img), 1.0);
    let region = size.bounds();
    let out = render(&r, s.document(), region);
    // Already in the working space: no conversion at all, bit-exact.
    for (x, v) in values.iter().enumerate() {
        assert_eq!(at(&out, region, x as u32, 0), *v, "pixel {x}");
    }

    // Linear sRGB gray well above the half-float range: converted, not clamped.
    let img = image(
        Size::new(1, 1),
        float_format(ColorSpace::LINEAR_SRGB, AlphaMode::Straight),
        |_, _| floats([1e5, 1e5, 1e5, 1.0]),
    );
    let mut s = Session::new(Document::new(Size::new(1, 1)));
    push_layer(&mut s, raster(&img), 1.0);
    let region = Rect::new(0, 0, 1, 1);
    let out = render(&r, s.document(), region);
    assert_close(at(&out, region, 0, 0), [1e5, 1e5, 1e5, 1.0], "gray");
}

#[test]
fn non_finite_samples_are_mapped_and_counted() {
    let Some(r) = renderer() else { return };
    let size = Size::new(2, 1);
    let straight = float_format(WORKING_SPACE, AlphaMode::Straight);
    let img = image(size, straight, |x, _| {
        floats(if x == 0 {
            [f32::INFINITY, f32::NEG_INFINITY, f32::NAN, 1.0]
        } else {
            [0.5, 0.5, 0.5, f32::NAN]
        })
    });
    let mut s = Session::new(Document::new(size));
    push_layer(&mut s, raster(&img), 1.0);
    let region = size.bounds();
    let mut out = vec![0.0; 8];
    assert_eq!(r.render_region(s.document(), region, &mut out).unwrap(), 4);
    // ±inf reads as ±65504, not ±f32::MAX (see `an_infinity_keeps_the_other_channels`).
    assert_eq!(at(&out, region, 0, 0), [65504.0, -65504.0, 0.0, 1.0]);
    // A NaN alpha is transparent.
    assert_eq!(at(&out, region, 1, 0), [0.0; 4]);

    // Under a layer that hides the whole canvas, nothing shows through: the layer is not
    // composited at all (like a hidden one), so none of its samples reach the result.
    let blue = LinearRgba::new(0.0, 0.0, 1.0, 1.0);
    push_layer(&mut s, LayerContent::Fill { color: blue }, 1.0);
    assert_eq!(r.render_region(s.document(), region, &mut out).unwrap(), 0);
    for x in 0..2 {
        assert_eq!(at(&out, region, x, 0), [0.0, 0.0, 1.0, 1.0], "pixel {x}");
    }
}

/// Render the whole document through both export sources: same values (within float
/// rounding) and the same non-finite count, `expected`. Returns the GPU output.
fn both_sources(r: &Renderer, doc: &Document, expected: u64) -> Vec<f32> {
    let region = doc.size().bounds();
    let mut cpu = vec![0.0; region.size().pixel_count() as usize * 4];
    let mut gpu = cpu.clone();
    let cpu_count = slopshop_render::export_source(None, doc)(region, &mut cpu).unwrap();
    let gpu_count = slopshop_render::export_source(Some(r), doc)(region, &mut gpu).unwrap();
    assert_eq!(
        (cpu_count, gpu_count),
        (expected, expected),
        "CPU, GPU counts"
    );
    for (i, (g, c)) in gpu.iter().zip(&cpu).enumerate() {
        assert!(
            (g - c).abs() <= 1e-4 * c.abs().max(1.0),
            "sample {i}: GPU {g} vs CPU {c}"
        );
    }
    gpu
}

/// Regression: NaN and ±inf samples were replaced without being counted, and ±inf became
/// ±f32::MAX, which the matrices to the working space and back spread to the pixel's other
/// channels (0.5 became about ±1e30).
#[test]
fn an_infinity_keeps_the_other_channels() {
    let Some(r) = renderer() else { return };
    // Linear sRGB (a matrix to the working space), specials in four different tiles.
    let size = Size::new(600, 300);
    let specials = [
        ((0, 0), [f32::NAN, 0.5, 0.5, 1.0]),
        ((300, 10), [f32::INFINITY, 0.5, 0.5, 1.0]),
        ((520, 280), [0.5, f32::NEG_INFINITY, 0.5, 1.0]),
        ((10, 290), [0.5, 0.5, 0.5, f32::NAN]),
    ];
    let pixel = |x, y| {
        let special = specials.iter().find(|(at, _)| *at == (x, y));
        special.map_or([0.25, 0.5, 0.75, 1.0], |(_, v)| *v)
    };
    let img = image(
        size,
        float_format(ColorSpace::LINEAR_SRGB, AlphaMode::Straight),
        |x, y| floats(pixel(x, y)),
    );
    let mut s = Session::new(Document::new(size));
    push_layer(&mut s, raster(&img), 1.0);
    let out = both_sources(&r, s.document(), 4);
    // Chunked (one tile per chunk): the counts of every chunk add up.
    let tiny = Renderer::new().unwrap().with_tile_capacity(1);
    let region = size.bounds();
    let mut chunked = vec![0.0; out.len()];
    assert_eq!(
        tiny.render_region(s.document(), region, &mut chunked)
            .unwrap(),
        4
    );
    assert!(
        chunked
            .iter()
            .zip(&out)
            .all(|(a, b)| a.to_bits() == b.to_bits())
    );

    // Back in linear sRGB, as an export to that space reads them.
    let back = WORKING_SPACE.matrix_to(&ColorSpace::LINEAR_SRGB);
    let expected = [
        [0.0, 0.5, 0.5, 1.0],
        [65504.0, 0.5, 0.5, 1.0],
        [0.5, -65504.0, 0.5, 1.0],
        // NaN alpha: transparent.
        [0.0, 0.0, 0.0, 0.0],
    ];
    for (((x, y), _), expected) in specials.iter().zip(expected) {
        let [red, green, blue, alpha] = at(&out, region, *x, *y);
        let rgb = mat_vec(&back, [red, green, blue].map(f64::from));
        let actual = [rgb[0] as f32, rgb[1] as f32, rgb[2] as f32, alpha];
        let close = actual
            .iter()
            .zip(expected)
            .all(|(a, e)| (a - e).abs() < 0.02);
        assert!(close, "({x}, {y}): {actual:?} != {expected:?}");
    }
}

#[test]
fn non_finite_counts_follow_stored_samples_and_decoded_values() {
    let Some(r) = renderer() else { return };
    // Gray: one stored sample per pixel, even though the GPU sees it three times.
    let gray = PixelFormat {
        layout: ChannelLayout::Gray,
        sample: SampleType::F32,
        color_space: ColorSpace::LINEAR_SRGB,
        alpha: AlphaMode::Straight,
    };
    let values = [f32::INFINITY, f32::NAN, 0.5];
    let img = image(Size::new(3, 1), gray, |x, _| {
        values[x as usize].to_ne_bytes().to_vec()
    });
    let mut s = Session::new(Document::new(Size::new(3, 1)));
    push_layer(&mut s, raster(&img), 1.0);
    both_sources(&r, s.document(), 2);

    // A finite sample whose decoded value overflows (sRGB curve of 1e30): replaced, counted.
    let img = image(
        Size::new(2, 1),
        float_format(ColorSpace::SRGB, AlphaMode::Straight),
        |x, _| {
            floats(if x == 0 {
                [1e30, 0.5, 0.5, 1.0]
            } else {
                [0.5; 4]
            })
        },
    );
    let mut s = Session::new(Document::new(Size::new(2, 1)));
    push_layer(&mut s, raster(&img), 1.0);
    // A layer at opacity 0 changes nothing and is not read: its samples are not counted.
    let hidden = image(
        Size::new(2, 1),
        float_format(WORKING_SPACE, AlphaMode::Straight),
        |_, _| floats([f32::NAN; 4]),
    );
    push_layer(&mut s, raster(&hidden), 0.0);
    both_sources(&r, s.document(), 1);
}

#[test]
fn overflowing_composites_stay_finite() {
    let Some(r) = renderer() else { return };
    let size = Size::new(1, 1);
    let premultiplied = float_format(WORKING_SPACE, AlphaMode::Premultiplied);
    let opaque = image(size, premultiplied, |_, _| floats([3e38, 3e38, 3e38, 1.0]));
    // Premultiplied color far above its alpha: "over" overflows f32.
    let glow = image(size, premultiplied, |_, _| floats([3e38, 3e38, 3e38, 0.01]));
    let mut s = Session::new(linear_document(size));
    push_layer(&mut s, raster(&opaque), 1.0);
    push_layer(&mut s, raster(&glow), 1.0);
    let region = size.bounds();
    // Saturated, and counted (the CPU saturates the same values).
    let out = both_sources(&r, s.document(), 3);
    let [red, green, blue, alpha] = at(&out, region, 0, 0);
    assert_eq!([red, green, blue], [f32::MAX; 3]);
    assert!((alpha - 1.0).abs() < 1e-6, "{alpha}");

    // An opaque layer above hides it (no inf × 0 = NaN).
    let red_fill = LinearRgba::new(1.0, 0.0, 0.0, 1.0);
    push_layer(&mut s, LayerContent::Fill { color: red_fill }, 1.0);
    let out = render(&r, s.document(), region);
    assert_eq!(at(&out, region, 0, 0), [1.0, 0.0, 0.0, 1.0]);
}

#[test]
fn fill_layers_composite_with_opacity() {
    let Some(r) = renderer() else { return };
    let size = Size::new(20, 10);
    let mut s = Session::new(linear_document(size));
    let color = LinearRgba::new(0.2, 0.4, 0.6, 1.0);
    push_layer(&mut s, LayerContent::Fill { color }, 0.25);
    let region = Rect::new(5, 3, 10, 4);
    let out = render(&r, s.document(), region);
    for (x, y) in [(0, 0), (9, 3), (4, 2)] {
        assert_close(at(&out, region, x, y), [0.05, 0.1, 0.15, 0.25], "alone");
    }

    // Red at 50 % over opaque blue.
    let mut s = Session::new(linear_document(size));
    let blue = LinearRgba::new(0.0, 0.0, 1.0, 1.0);
    let red = LinearRgba::new(1.0, 0.0, 0.0, 1.0);
    push_layer(&mut s, LayerContent::Fill { color: blue }, 1.0);
    let top = push_layer(&mut s, LayerContent::Fill { color: red }, 0.5);
    let out = render(&r, s.document(), region);
    assert_close(at(&out, region, 3, 1), [0.5, 0.0, 0.5, 1.0], "over");

    // Hidden layers are ignored.
    s.perform(Edit::SetLayerVisible {
        id: top,
        visible: false,
    })
    .unwrap();
    let out = render(&r, s.document(), region);
    assert_eq!(at(&out, region, 3, 1), [0.0, 0.0, 1.0, 1.0]);
}

#[test]
fn a_raster_smaller_than_the_document_is_transparent_outside() {
    let Some(r) = renderer() else { return };
    let img = image(Size::new(300, 200), PixelFormat::RGBA8_SRGB, pattern);
    let mut s = Session::new(Document::new(Size::new(400, 300)));
    push_layer(&mut s, raster(&img), 1.0);
    let m = img.matrix_to(&WORKING_SPACE);
    let region = Rect::new(250, 150, 150, 150);
    let out = render(&r, s.document(), region);
    assert_close(
        at(&out, region, 49, 49),
        expected_srgb8(&pattern(299, 199), &m),
        "in",
    );
    for (x, y) in [(50, 0), (0, 50), (149, 149), (100, 20)] {
        assert_eq!(at(&out, region, x, y), [0.0; 4], "pixel ({x}, {y})");
    }
}

/// Bottom to top: an opaque 8-bit pattern, a smaller half-transparent 8-bit image at 70 %, and
/// a float image with HDR values.
fn mixed_stack(size: Size) -> Session {
    let base = image(size, PixelFormat::RGBA8_SRGB, pattern);
    let over = image(Size::new(700, 500), PixelFormat::RGBA8_SRGB, |x, y| {
        vec![
            (x * 3 % 256) as u8,
            200,
            (y % 256) as u8,
            ((x * y) % 256) as u8,
        ]
    });
    let hdr = image(
        Size::new(900, 400),
        float_format(ColorSpace::LINEAR_SRGB, AlphaMode::Straight),
        |x, y| {
            let v = (x + y) as f32 / 100.0;
            floats([v, -v / 4.0, 70_000.0, if x % 3 == 0 { 0.0 } else { 0.4 }])
        },
    );
    let mut s = Session::new(Document::new(size));
    push_layer(&mut s, raster(&base), 1.0);
    push_layer(&mut s, raster(&over), 0.7);
    push_layer(&mut s, raster(&hdr), 1.0);
    s
}

#[test]
fn chunked_rendering_matches_a_single_pass() {
    let Some(r) = renderer() else { return };
    let size = Size::new(1000, 600);
    let s = mixed_stack(size);
    for region in [
        Rect::new(37, 50, 900, 500),
        size.bounds(),
        Rect::new(255, 255, 2, 2),
    ] {
        let whole = render(&r, s.document(), region);
        assert!(whole.iter().all(|v| v.is_finite()));
        let small = Renderer::new().unwrap().with_tile_capacity(3);
        // Two 8-bit images for three slots: one tile per chunk, so chunk edges are everywhere.
        let chunked = render(&small, s.document(), region);
        let same = whole
            .iter()
            .zip(&chunked)
            .all(|(a, b)| a.to_bits() == b.to_bits());
        assert!(same, "region {region:?} differs when chunked");
    }
}

#[test]
fn too_many_images_for_the_tile_cache_is_an_error_not_a_coarser_level() {
    let Some(r) = renderer() else { return };
    let r = r.with_tile_capacity(1);
    let s = mixed_stack(Size::new(1000, 600));
    // Two 8-bit images visible for one slot.
    let region = Rect::new(0, 0, 300, 100);
    let mut out = vec![0.0; region.size().pixel_count() as usize * 4];
    assert!(matches!(
        r.render_region(s.document(), region, &mut out),
        Err(RenderError::TooManyLayers {
            images: 2,
            capacity: 1
        })
    ));
    // Only the bottom image reaches this corner: rendered one tile at a time.
    let region = Rect::new(800, 500, 200, 100);
    let mut out = vec![0.0; region.size().pixel_count() as usize * 4];
    r.render_region(s.document(), region, &mut out).unwrap();
}

#[test]
fn invalid_regions_are_rejected() {
    let Some(r) = renderer() else { return };
    let doc = Document::new(Size::new(100, 50));
    let mut out = vec![0.0; 4 * 10 * 10];
    assert!(matches!(
        r.render_region(&doc, Rect::new(0, 0, 0, 10), &mut out),
        Err(RenderError::EmptyOutput)
    ));
    assert!(matches!(
        r.render_region(&doc, Rect::new(95, 0, 10, 10), &mut out),
        Err(RenderError::RegionOutsideDocument { .. })
    ));
    assert!(matches!(
        r.render_region(&doc, Rect::new(0, 0, 10, 9), &mut out),
        Err(RenderError::RegionBufferLength {
            expected: 360,
            actual: 400
        })
    ));
    // An empty document is transparent.
    r.render_region(&doc, Rect::new(90, 40, 10, 10), &mut out)
        .unwrap();
    assert!(out.iter().all(|&v| v == 0.0));
}

#[test]
fn gpu_regions_match_the_cpu_reference_compositor() {
    let Some(r) = renderer() else { return };
    // A mixed stack: an opaque 8-bit sRGB background, a 16-bit Display P3 layer with alpha, a
    // premultiplied linear f32 layer smaller than the document, and a translucent fill.
    let size = Size::new(600, 300);
    let mut s = Session::new(Document::new(size));
    let background = image(size, PixelFormat::RGBA8_SRGB, pattern);
    push_layer(&mut s, raster(&background), 1.0);
    let p3 = PixelFormat {
        layout: ChannelLayout::Rgba,
        sample: SampleType::U16,
        color_space: ColorSpace::DISPLAY_P3,
        alpha: AlphaMode::Straight,
    };
    let wide = image(size, p3, |x, y| {
        [
            x * 97 % 65536,
            y * 211 % 65536,
            (x ^ y) * 13 % 65536,
            (x + 2 * y) * 71 % 65536,
        ]
        .iter()
        .flat_map(|&v| (v as u16).to_ne_bytes())
        .collect()
    });
    push_layer(&mut s, raster(&wide), 0.7);
    let hdr = image(
        Size::new(300, 200),
        float_format(ColorSpace::LINEAR_SRGB, AlphaMode::Premultiplied),
        |x, y| {
            let a = ((x + y) % 5) as f32 / 4.0;
            floats([3.0 * a, 0.25 * a, (x as f32 / 300.0) * a, a])
        },
    );
    push_layer(&mut s, raster(&hdr), 1.0);
    push_layer(
        &mut s,
        LayerContent::Fill {
            color: LinearRgba::new(0.1, 0.4, 0.8, 0.5),
        },
        0.6,
    );

    // Regions across tile edges, including the edge of the smaller layer.
    for region in [
        Rect::new(0, 0, 600, 300),
        Rect::new(250, 150, 100, 100),
        Rect::new(511, 3, 89, 297),
    ] {
        let gpu = render(&r, s.document(), region);
        let mut cpu = vec![0.0; gpu.len()];
        slopshop_core::composite::composite_region(s.document(), region, &mut cpu).unwrap();
        for (i, (g, c)) in gpu.iter().zip(&cpu).enumerate() {
            let tolerance = 1e-4 * c.abs().max(1.0);
            assert!(
                (g - c).abs() <= tolerance,
                "{region:?} sample {i}: GPU {g} vs CPU {c}"
            );
        }
    }
}

#[test]
fn export_source_uses_the_gpu_or_the_cpu_compositor_alike() {
    let size = Size::new(300, 280);
    let mut s = Session::new(Document::new(size));
    push_layer(
        &mut s,
        raster(&image(size, PixelFormat::RGBA8_SRGB, pattern)),
        0.8,
    );
    let region = Rect::new(20, 250, 280, 30);
    let mut cpu = vec![0.0; region.size().pixel_count() as usize * 4];
    slopshop_render::export_source(None, s.document())(region, &mut cpu).unwrap();
    let Some(r) = renderer() else { return };
    let mut gpu = vec![0.0; cpu.len()];
    slopshop_render::export_source(Some(&r), s.document())(region, &mut gpu).unwrap();
    for (g, c) in gpu.iter().zip(&cpu) {
        assert!(
            (g - c).abs() <= 1e-4 * c.abs().max(1.0),
            "GPU {g} vs CPU {c}"
        );
    }
    // The fallback: more images than tile slots still renders, on the CPU.
    let tiny = Renderer::new().unwrap().with_tile_capacity(1);
    let mut stack = Session::new(Document::new(size));
    for _ in 0..3 {
        push_layer(
            &mut stack,
            raster(&image(size, PixelFormat::RGBA8_SRGB, pattern)),
            0.5,
        );
    }
    let mut out = vec![0.0; cpu.len()];
    slopshop_render::export_source(Some(&tiny), stack.document())(region, &mut out).unwrap();
}

#[test]
fn every_blend_mode_matches_the_cpu_reference_in_both_spaces() {
    let Some(r) = renderer() else { return };
    let size = Size::new(260, 130);
    let background = image(size, PixelFormat::RGBA8_SRGB, pattern);
    let p3 = PixelFormat {
        layout: ChannelLayout::Rgba,
        sample: SampleType::U16,
        color_space: ColorSpace::DISPLAY_P3,
        alpha: AlphaMode::Straight,
    };
    let top = image(size, p3, |x, y| {
        [
            x * 251 % 65536,
            y * 499 % 65536,
            (x * y) * 37 % 65536,
            20_000 + (x + 3 * y) * 97 % 45_536,
        ]
        .iter()
        .flat_map(|&v| (v as u16).to_ne_bytes())
        .collect()
    });
    for space in [BlendSpace::Perceptual, BlendSpace::Linear] {
        for mode in BlendMode::ALL {
            let mut doc = Document::new(size);
            Edit::SetBlendSpace { space }.apply(&mut doc).unwrap();
            let mut s = Session::new(doc);
            push_layer(&mut s, raster(&background), 1.0);
            let id = push_layer(&mut s, raster(&top), 0.8);
            let fill = push_layer(
                &mut s,
                LayerContent::Fill {
                    color: LinearRgba::new(0.2, 0.5, 0.9, 0.7),
                },
                0.5,
            );
            for layer in [id, fill] {
                s.perform(Edit::SetLayerBlendMode { id: layer, mode })
                    .unwrap();
            }
            let region = size.bounds();
            let mut cpu = vec![0.0; region.size().pixel_count() as usize * 4];
            let mut gpu = cpu.clone();
            slopshop_render::export_source(None, s.document())(region, &mut cpu).unwrap();
            slopshop_render::export_source(Some(&r), s.document())(region, &mut gpu).unwrap();
            // f32 on the GPU, f64 on the CPU: a value right at a mode's threshold (hard mix,
            // darker color…) may fall on either side, and the dodges amplify f32 rounding near
            // their pole (`b / (1 − s)`); everything else must agree.
            let off = gpu
                .iter()
                .zip(&cpu)
                .filter(|(g, c)| (*g - *c).abs() > 1e-3 * c.abs().max(1.0))
                .count();
            assert!(
                off * 200 <= gpu.len(),
                "{mode} {space:?}: {off} of {} samples differ",
                gpu.len()
            );
            assert!(gpu.iter().all(|v| v.is_finite()), "{mode} {space:?}");
        }
    }
}

#[test]
fn masks_match_the_cpu_reference() {
    let Some(r) = renderer() else { return };
    let size = Size::new(300, 280);
    let background = image(size, PixelFormat::RGBA8_SRGB, pattern);
    // Translucent pixels of every alpha, and a gray float mask with extreme values.
    let top = image(Size::new(270, 260), PixelFormat::RGBA8_SRGB, |x, y| {
        vec![
            (x % 256) as u8,
            (y % 256) as u8,
            200,
            ((x * 7 + y * 13) % 256) as u8,
        ]
    });
    let gray = PixelFormat {
        layout: ChannelLayout::Gray,
        sample: SampleType::F32,
        color_space: ColorSpace::LINEAR_SRGB,
        alpha: AlphaMode::Straight,
    };
    let custom = image(Size::new(200, 290), gray, |x, y| {
        let v = match (x + y) % 7 {
            0 => f32::NAN,
            1 => f32::INFINITY,
            2 => -3.0,
            3 => 1.5,
            _ => (x * y % 100) as f32 / 99.0,
        };
        v.to_ne_bytes().to_vec()
    });
    for space in [BlendSpace::Perceptual, BlendSpace::Linear] {
        for (enabled, from_transparency) in [(true, true), (false, true), (true, false)] {
            let mut doc = Document::new(size);
            Edit::SetBlendSpace { space }.apply(&mut doc).unwrap();
            let mut s = Session::new(doc);
            push_layer(&mut s, raster(&background), 1.0);
            let id = push_layer(&mut s, raster(&top), 0.9);
            let fill = push_layer(
                &mut s,
                LayerContent::Fill {
                    color: LinearRgba::new(0.1, 0.6, 0.3, 0.8),
                },
                0.7,
            );
            let mask = if from_transparency {
                let mut mask = slopshop_core::LayerMask::from_transparency(&top).unwrap();
                mask.enabled = enabled;
                mask
            } else {
                slopshop_core::LayerMask {
                    original: None,
                    image: custom.clone(),
                    enabled,
                    replaces_alpha: false,
                }
            };
            s.perform(Edit::SetLayerMask {
                id,
                mask: Some(mask.clone()),
            })
            .unwrap();
            s.perform(Edit::SetLayerMask {
                id: fill,
                mask: Some(slopshop_core::LayerMask {
                    original: None,
                    image: custom.clone(),
                    enabled: true,
                    replaces_alpha: false,
                }),
            })
            .unwrap();
            let region = size.bounds();
            let mut cpu = vec![0.0; region.size().pixel_count() as usize * 4];
            let mut gpu = cpu.clone();
            slopshop_render::export_source(None, s.document())(region, &mut cpu).unwrap();
            slopshop_render::export_source(Some(&r), s.document())(region, &mut gpu).unwrap();
            let case = format!("{space:?} enabled {enabled} from transparency {from_transparency}");
            for (i, (g, c)) in gpu.iter().zip(&cpu).enumerate() {
                assert!(
                    (g - c).abs() <= 1e-4 * c.abs().max(1.0),
                    "{case}: sample {i}: GPU {g} vs CPU {c}"
                );
            }
        }
    }
}

/// Insert `content` into `parent` (on top of its layers) with a blend mode and opacity.
fn push_into(
    session: &mut Session,
    parent: Option<LayerId>,
    content: LayerContent,
    mode: BlendMode,
    opacity: f32,
) -> LayerId {
    let id = session.allocate_layer_id();
    let index = session.document().children_of(parent).unwrap().len();
    session
        .perform(Edit::InsertLayer {
            parent,
            index,
            layer: Layer {
                transform: slopshop_core::Affine::IDENTITY,
                clipped: false,
                id,
                name: "layer".into(),
                visible: true,
                opacity,
                blend_mode: mode,
                mask: None,
                content,
            },
        })
        .unwrap();
    id
}

fn group(pass_through: bool) -> LayerContent {
    LayerContent::Group {
        children: Vec::new(),
        pass_through,
    }
}

#[test]
fn gpu_groups_match_the_cpu_reference_compositor() {
    let Some(r) = renderer() else { return };
    let size = Size::new(300, 280);
    for space in [BlendSpace::Perceptual, BlendSpace::Linear] {
        let mut s = Session::new(Document::new(size));
        s.perform(Edit::SetBlendSpace { space }).unwrap();
        let background = image(size, PixelFormat::RGBA8_SRGB, pattern);
        push_into(&mut s, None, raster(&background), BlendMode::Normal, 1.0);

        // A pass-through group at 60 % with a gradient mask, holding a multiplied layer and an
        // isolated screen group of a fill and a float raster.
        let faded = push_into(&mut s, None, group(true), BlendMode::Normal, 0.6);
        let gray = PixelFormat {
            layout: ChannelLayout::Gray,
            sample: SampleType::U8,
            color_space: ColorSpace::LINEAR_SRGB,
            alpha: AlphaMode::Straight,
        };
        let mask = image(Size::new(260, 280), gray, |x, _| vec![(x % 256) as u8]);
        s.perform(Edit::SetLayerMask {
            id: faded,
            mask: Some(slopshop_core::LayerMask {
                original: None,
                image: mask,
                enabled: true,
                replaces_alpha: false,
            }),
        })
        .unwrap();
        let wide = image(size, PixelFormat::RGBA8_SRGB, |x, y| {
            vec![
                (x * 7) as u8,
                (y * 3) as u8,
                ((x + y) * 5) as u8,
                (x ^ y) as u8,
            ]
        });
        push_into(&mut s, Some(faded), raster(&wide), BlendMode::Multiply, 0.8);
        let isolated = push_into(&mut s, Some(faded), group(false), BlendMode::Screen, 0.7);
        push_into(
            &mut s,
            Some(isolated),
            LayerContent::Fill {
                color: LinearRgba::new(0.1, 0.4, 0.8, 0.5),
            },
            BlendMode::Normal,
            1.0,
        );
        let hdr = image(
            Size::new(200, 150),
            float_format(ColorSpace::LINEAR_SRGB, AlphaMode::Straight),
            |x, y| {
                floats([
                    x as f32 / 100.0,
                    0.3,
                    y as f32 / 150.0,
                    ((x + y) % 7) as f32 / 6.0,
                ])
            },
        );
        push_into(
            &mut s,
            Some(isolated),
            raster(&hdr),
            BlendMode::Overlay,
            0.9,
        );
        // A neutral pass-through group (inlined) holding a difference layer.
        let neutral = push_into(&mut s, None, group(true), BlendMode::Normal, 1.0);
        push_into(
            &mut s,
            Some(neutral),
            raster(&wide),
            BlendMode::Difference,
            0.5,
        );

        for region in [Rect::new(0, 0, 300, 280), Rect::new(250, 100, 50, 180)] {
            let gpu = render(&r, s.document(), region);
            let mut cpu = vec![0.0; gpu.len()];
            slopshop_core::composite::composite_region(s.document(), region, &mut cpu).unwrap();
            for (i, (g, c)) in gpu.iter().zip(&cpu).enumerate() {
                let tolerance = 1e-4 * c.abs().max(1.0);
                assert!(
                    (g - c).abs() <= tolerance,
                    "{space:?} {region:?} sample {i}: GPU {g} vs CPU {c}"
                );
            }
        }
    }
}

/// Mark layer `id` clipped.
fn clip(session: &mut Session, id: LayerId) {
    session
        .perform(Edit::SetLayerClipped { id, clipped: true })
        .unwrap();
}

#[test]
fn gpu_clipping_masks_match_the_cpu_reference_compositor() {
    let Some(r) = renderer() else { return };
    let size = Size::new(300, 280);
    for space in [BlendSpace::Perceptual, BlendSpace::Linear] {
        let mut s = Session::new(Document::new(size));
        s.perform(Edit::SetBlendSpace { space }).unwrap();
        let background = image(size, PixelFormat::RGBA8_SRGB, pattern);
        push_into(&mut s, None, raster(&background), BlendMode::Normal, 1.0);

        // A semi-transparent base in multiply at 70 %, two clipped layers.
        let base = image(size, PixelFormat::RGBA8_SRGB, |x, y| {
            vec![
                200,
                (x % 256) as u8,
                (y % 256) as u8,
                ((x + 2 * y) % 256) as u8,
            ]
        });
        push_into(&mut s, None, raster(&base), BlendMode::Multiply, 0.7);
        let wide = image(size, PixelFormat::RGBA8_SRGB, |x, y| {
            vec![
                (x * 7) as u8,
                (y * 3) as u8,
                ((x + y) * 5) as u8,
                (x ^ y) as u8,
            ]
        });
        let a = push_into(&mut s, None, raster(&wide), BlendMode::Screen, 0.8);
        clip(&mut s, a);
        let b = push_into(
            &mut s,
            None,
            LayerContent::Fill {
                color: LinearRgba::new(0.9, 0.2, 0.1, 0.6),
            },
            BlendMode::Normal,
            1.0,
        );
        clip(&mut s, b);

        // A group as a base, with a clipped isolated group on it.
        let folder = push_into(&mut s, None, group(true), BlendMode::Normal, 0.9);
        push_into(&mut s, Some(folder), raster(&base), BlendMode::Normal, 1.0);
        let clipped_group = push_into(&mut s, None, group(false), BlendMode::Overlay, 0.8);
        push_into(
            &mut s,
            Some(clipped_group),
            raster(&wide),
            BlendMode::Normal,
            1.0,
        );
        clip(&mut s, clipped_group);

        for region in [Rect::new(0, 0, 300, 280), Rect::new(250, 100, 50, 180)] {
            let gpu = render(&r, s.document(), region);
            let mut cpu = vec![0.0; gpu.len()];
            slopshop_core::composite::composite_region(s.document(), region, &mut cpu).unwrap();
            for (i, (g, c)) in gpu.iter().zip(&cpu).enumerate() {
                let tolerance = 1e-4 * c.abs().max(1.0);
                assert!(
                    (g - c).abs() <= tolerance,
                    "{space:?} {region:?} sample {i}: GPU {g} vs CPU {c}"
                );
            }
        }
    }
}

/// Move layer `id` by whole pixels.
fn translate(session: &mut Session, id: LayerId, x: f64, y: f64) {
    session
        .perform(Edit::SetLayerTransform {
            id,
            transform: slopshop_core::Affine::translation(x, y),
        })
        .unwrap();
}

#[test]
fn gpu_moved_layers_match_the_cpu_reference_compositor() {
    let Some(r) = renderer() else { return };
    let size = Size::new(300, 280);
    for space in [BlendSpace::Perceptual, BlendSpace::Linear] {
        let mut s = Session::new(Document::new(size));
        s.perform(Edit::SetBlendSpace { space }).unwrap();
        let background = image(size, PixelFormat::RGBA8_SRGB, pattern);
        let bg = push_into(&mut s, None, raster(&background), BlendMode::Normal, 1.0);
        translate(&mut s, bg, -37.0, 21.0);
        let small = image(Size::new(180, 150), PixelFormat::RGBA8_SRGB, |x, y| {
            vec![(x * 3) as u8, (y * 5) as u8, 90, ((x + y) % 256) as u8]
        });
        let moved = push_into(&mut s, None, raster(&small), BlendMode::Multiply, 0.8);
        translate(&mut s, moved, 200.0, -40.0);
        // A moved, masked group holding a moved layer and a clipped one.
        let folder = push_into(&mut s, None, group(false), BlendMode::Screen, 0.9);
        let gray = PixelFormat {
            layout: ChannelLayout::Gray,
            sample: SampleType::U8,
            color_space: ColorSpace::LINEAR_SRGB,
            alpha: AlphaMode::Straight,
        };
        let mask = image(Size::new(200, 200), gray, |x, _| vec![(x % 256) as u8]);
        s.perform(Edit::SetLayerMask {
            id: folder,
            mask: Some(slopshop_core::LayerMask {
                original: None,
                image: mask,
                enabled: true,
                replaces_alpha: false,
            }),
        })
        .unwrap();
        translate(&mut s, folder, 60.0, 50.0);
        let inside = push_into(&mut s, Some(folder), raster(&small), BlendMode::Normal, 1.0);
        translate(&mut s, inside, -20.0, 10.0);
        let clipped = push_into(
            &mut s,
            Some(folder),
            raster(&background),
            BlendMode::Overlay,
            1.0,
        );
        s.perform(Edit::SetLayerClipped {
            id: clipped,
            clipped: true,
        })
        .unwrap();

        for region in [Rect::new(0, 0, 300, 280), Rect::new(250, 100, 50, 180)] {
            let gpu = render(&r, s.document(), region);
            let mut cpu = vec![0.0; gpu.len()];
            slopshop_core::composite::composite_region(s.document(), region, &mut cpu).unwrap();
            for (i, (g, c)) in gpu.iter().zip(&cpu).enumerate() {
                let tolerance = 1e-4 * c.abs().max(1.0);
                assert!(
                    (g - c).abs() <= tolerance,
                    "{space:?} {region:?} sample {i}: GPU {g} vs CPU {c}"
                );
            }
        }
    }
}

/// Give layer `id` a transform.
fn transform(session: &mut Session, id: LayerId, transform: Affine) {
    session
        .perform(Edit::SetLayerTransform { id, transform })
        .unwrap();
}

/// GPU and CPU renders of `region` agree. f32 texel coordinates on the GPU make the weights
/// differ slightly from the CPU's f64; and a texel right at the anti-ringing boundary
/// (`resample::ANTIRING_R2`) can bound one side and not the other, which moves that sample by
/// at most the kernel's overshoot: allowed for 1 sample in 100 000.
fn assert_matches_cpu(r: &Renderer, doc: &Document, region: Rect, what: &str) {
    let gpu = render(r, doc, region);
    let mut cpu = vec![0.0; gpu.len()];
    slopshop_core::composite::composite_region(doc, region, &mut cpu).unwrap();
    let mut outliers = 0;
    for (i, (g, c)) in gpu.iter().zip(&cpu).enumerate() {
        let scale = c.abs().max(1.0);
        let d = (g - c).abs();
        assert!(
            d <= 0.05 * scale,
            "{what} {region:?} sample {i}: GPU {g} vs CPU {c}"
        );
        if d > 1e-3 * scale {
            outliers += 1;
        }
    }
    assert!(
        outliers <= gpu.len() / 100_000,
        "{what} {region:?}: {outliers} of {} samples beyond 1e-3",
        gpu.len()
    );
}

#[test]
fn gpu_resampled_layers_match_the_cpu_reference_compositor() {
    let Some(r) = renderer() else { return };
    let size = Size::new(300, 280);
    for space in [BlendSpace::Perceptual, BlendSpace::Linear] {
        let mut s = Session::new(Document::new(size));
        s.perform(Edit::SetBlendSpace { space }).unwrap();
        // A rotated background, larger than the canvas.
        let background = image(Size::new(420, 400), PixelFormat::RGBA8_SRGB, pattern);
        let bg = push_into(&mut s, None, raster(&background), BlendMode::Normal, 1.0);
        transform(
            &mut s,
            bg,
            Affine::rotation(0.3).then(Affine::translation(40.0, -90.0)),
        );
        // An enlarged small layer whose mask replaces its alpha, in multiply.
        let small = image(Size::new(40, 30), PixelFormat::RGBA8_SRGB, |x, y| {
            vec![
                (x * 6) as u8,
                (y * 8) as u8,
                90,
                ((x * 7 + y * 13) % 256) as u8,
            ]
        });
        let big = push_into(&mut s, None, raster(&small), BlendMode::Multiply, 0.8);
        let mask = slopshop_core::LayerMask::from_transparency(&small).unwrap();
        s.perform(Edit::SetLayerMask {
            id: big,
            mask: Some(mask),
        })
        .unwrap();
        transform(
            &mut s,
            big,
            Affine::scale(2.7, 3.1).then(Affine::translation(12.25, 30.5)),
        );
        // A strongly reduced, rotated layer (a coarser level) in a scaled, masked group.
        let folder = push_into(&mut s, None, group(false), BlendMode::Screen, 0.9);
        let gray = PixelFormat {
            layout: ChannelLayout::Gray,
            sample: SampleType::U8,
            color_space: ColorSpace::LINEAR_SRGB,
            alpha: AlphaMode::Straight,
        };
        let ramp = image(Size::new(200, 200), gray, |x, y| {
            vec![((x + y) % 256) as u8]
        });
        s.perform(Edit::SetLayerMask {
            id: folder,
            mask: Some(slopshop_core::LayerMask {
                original: None,
                image: ramp,
                enabled: true,
                replaces_alpha: false,
            }),
        })
        .unwrap();
        transform(
            &mut s,
            folder,
            Affine::scale(0.9, 1.2).then(Affine::translation(100.5, 20.0)),
        );
        let wide = image(Size::new(600, 500), PixelFormat::RGBA8_SRGB, |x, y| {
            vec![
                (x * 7) as u8,
                (y * 3) as u8,
                ((x + y) * 5) as u8,
                (x ^ y) as u8,
            ]
        });
        let reduced = push_into(&mut s, Some(folder), raster(&wide), BlendMode::Normal, 1.0);
        transform(
            &mut s,
            reduced,
            Affine::scale(0.3, 0.2)
                .then(Affine::rotation(-0.7))
                .then(Affine::translation(10.0, 80.0)),
        );
        // A quarter turn with a flip at a whole-pixel place (copied texels), and a layer
        // clipped to the rotated background.
        let turned = push_into(&mut s, None, raster(&small), BlendMode::Overlay, 1.0);
        transform(
            &mut s,
            turned,
            Affine {
                a: 0.0,
                b: -1.0,
                c: -1.0,
                d: 0.0,
                e: 250.0,
                f: 200.0,
            },
        );
        let hdr = image(
            Size::new(90, 70),
            float_format(ColorSpace::LINEAR_SRGB, AlphaMode::Straight),
            |x, y| {
                floats([
                    x as f32 / 30.0,
                    0.3,
                    y as f32 / 70.0,
                    ((x + y) % 7) as f32 / 6.0,
                ])
            },
        );
        let clipped = push_into(&mut s, None, raster(&hdr), BlendMode::Normal, 1.0);
        clip(&mut s, clipped);
        transform(
            &mut s,
            clipped,
            Affine::rotation(1.1).then(Affine::translation(150.0, 60.0)),
        );

        for region in [Rect::new(0, 0, 300, 280), Rect::new(250, 100, 50, 180)] {
            assert_matches_cpu(&r, s.document(), region, &format!("{space:?}"));
        }
    }
}

#[test]
fn transformed_layers_needing_more_tiles_than_their_chunk_are_split_to_fit() {
    let Some(r) = renderer() else { return };
    // A rotated 1000 × 900 image reads tiles well beyond each chunk's own: with 4 slots (a
    // sample reads at most 2 × 2 tiles),
    // chunks must shrink until their tiles fit.
    let r = r.with_tile_capacity(4);
    let size = Size::new(700, 600);
    let mut s = Session::new(Document::new(size));
    let wide = image(Size::new(1000, 900), PixelFormat::RGBA8_SRGB, pattern);
    let id = push_into(&mut s, None, raster(&wide), BlendMode::Normal, 1.0);
    transform(
        &mut s,
        id,
        Affine::rotation(0.785).then(Affine::translation(300.0, -300.0)),
    );
    assert_matches_cpu(&r, s.document(), size.bounds(), "small cache");
}

#[test]
fn gpu_adjustment_layers_match_the_cpu_reference_compositor() {
    use slopshop_core::adjust::Adjustment;
    let Some(r) = renderer() else { return };
    let size = Size::new(300, 280);
    for space in [BlendSpace::Perceptual, BlendSpace::Linear] {
        let mut s = Session::new(Document::new(size));
        s.perform(Edit::SetBlendSpace { space }).unwrap();
        let background = image(size, PixelFormat::RGBA8_SRGB, pattern);
        push_into(&mut s, None, raster(&background), BlendMode::Normal, 1.0);
        let adjust = |adjustment| LayerContent::Adjustment { adjustment };
        // Exposure over everything, masked by a ramp.
        let exposure = push_into(
            &mut s,
            None,
            adjust(Adjustment::Exposure {
                exposure: 0.8,
                offset: -0.02,
                gamma: 1.2,
            }),
            BlendMode::Normal,
            0.9,
        );
        let gray = PixelFormat {
            layout: ChannelLayout::Gray,
            sample: SampleType::U8,
            color_space: ColorSpace::LINEAR_SRGB,
            alpha: AlphaMode::Straight,
        };
        let ramp = image(Size::new(260, 280), gray, |x, _| vec![(x % 256) as u8]);
        s.perform(Edit::SetLayerMask {
            id: exposure,
            mask: Some(slopshop_core::LayerMask {
                original: None,
                image: ramp,
                enabled: true,
                replaces_alpha: false,
            }),
        })
        .unwrap();
        // A translucent shape with a Hue/Saturation clipped to it.
        let shape = image(size, PixelFormat::RGBA8_SRGB, |x, y| {
            vec![(x * 3) as u8, (y * 5) as u8, 90, ((x + y) % 256) as u8]
        });
        push_into(&mut s, None, raster(&shape), BlendMode::Normal, 1.0);
        let hue = push_into(
            &mut s,
            None,
            adjust(Adjustment::HueSaturation {
                hue: -70.0,
                saturation: 45.0,
                lightness: 12.0,
            }),
            BlendMode::Normal,
            1.0,
        );
        clip(&mut s, hue);
        // Levels inside an isolated group, over a float layer.
        let folder = push_into(&mut s, None, group(false), BlendMode::Normal, 0.8);
        let hdr = image(
            Size::new(200, 150),
            float_format(ColorSpace::LINEAR_SRGB, AlphaMode::Straight),
            |x, y| {
                floats([
                    x as f32 / 100.0,
                    0.3,
                    y as f32 / 150.0,
                    ((x + y) % 7) as f32 / 6.0,
                ])
            },
        );
        push_into(&mut s, Some(folder), raster(&hdr), BlendMode::Normal, 1.0);
        push_into(
            &mut s,
            Some(folder),
            adjust(Adjustment::Levels {
                input_black: 0.1,
                input_white: 0.85,
                gamma: 0.8,
                output_black: 0.05,
                output_white: 0.9,
            }),
            BlendMode::Normal,
            0.7,
        );
        for region in [Rect::new(0, 0, 300, 280), Rect::new(250, 100, 50, 180)] {
            assert_matches_cpu(&r, s.document(), region, &format!("{space:?}"));
        }
    }
}

#[test]
fn gpu_new_adjustments_match_the_cpu_reference_compositor() {
    use slopshop_core::adjust::Adjustment;
    let Some(r) = renderer() else { return };
    let size = Size::new(200, 160);
    for space in [BlendSpace::Perceptual, BlendSpace::Linear] {
        for adjustment in [
            Adjustment::BrightnessContrast {
                brightness: 60.0,
                contrast: 40.0,
            },
            Adjustment::BrightnessContrast {
                brightness: -120.0,
                contrast: -30.0,
            },
            Adjustment::Vibrance {
                vibrance: 70.0,
                saturation: -20.0,
            },
            Adjustment::Invert,
            // 7 levels: no 8-bit value (k / 255) falls on a step (255 is not a multiple of 7).
            Adjustment::Posterize { levels: 7.0 },
            Adjustment::Threshold { level: 0.45 },
        ] {
            let mut s = Session::new(Document::new(size));
            s.perform(Edit::SetBlendSpace { space }).unwrap();
            // A smooth gradient: posterize and threshold steps fall between samples, not on them.
            let base = image(size, PixelFormat::RGBA8_SRGB, |x, y| {
                vec![(x * 255 / 199) as u8, (y * 255 / 159) as u8, 128, 255]
            });
            push_into(&mut s, None, raster(&base), BlendMode::Normal, 1.0);
            push_into(
                &mut s,
                None,
                LayerContent::Adjustment { adjustment },
                BlendMode::Normal,
                0.85,
            );
            let what = format!("{space:?} {adjustment:?}");
            if matches!(
                adjustment,
                Adjustment::Posterize { .. } | Adjustment::Threshold { .. }
            ) {
                // Steps: a sample right at one can fall on either side in f32 (GPUs differ, e.g.
                // Metal's power function). Rare samples on a step are allowed.
                let region = size.bounds();
                let gpu = render(&r, s.document(), region);
                let mut cpu = vec![0.0; gpu.len()];
                slopshop_core::composite::composite_region(s.document(), region, &mut cpu).unwrap();
                let off = gpu
                    .iter()
                    .zip(&cpu)
                    .filter(|(g, c)| (*g - *c).abs() > 1e-3)
                    .count();
                assert!(
                    off <= gpu.len() / 100,
                    "{what}: {off} of {} samples differ",
                    gpu.len()
                );
            } else {
                assert_matches_cpu(&r, s.document(), size.bounds(), &what);
            }
        }
    }
}

#[test]
fn gpu_adjustments_of_many_parameters_match_the_cpu_reference_compositor() {
    use slopshop_core::adjust::Adjustment;
    use slopshop_core::curve::Curve;
    let Some(r) = renderer() else { return };
    let size = Size::new(200, 160);
    for space in [BlendSpace::Perceptual, BlendSpace::Linear] {
        for adjustment in [
            Adjustment::defaults("blackWhite").unwrap(),
            Adjustment::BlackWhite {
                weights: [-50.0, 120.0, 250.0, 10.0, -180.0, 300.0],
                tint: true,
                tint_hue: 200.0,
                tint_saturation: 60.0,
            },
            Adjustment::ColorBalance {
                shadows: [40.0, -20.0, 10.0],
                midtones: [-30.0, 60.0, -80.0],
                highlights: [100.0, 0.0, -45.0],
                preserve_luminosity: false,
            },
            Adjustment::ColorBalance {
                shadows: [-70.0, 20.0, 0.0],
                midtones: [30.0, 30.0, 90.0],
                highlights: [-10.0, -60.0, 45.0],
                preserve_luminosity: true,
            },
            Adjustment::defaults("photoFilter").unwrap(),
            Adjustment::PhotoFilter {
                color: [0.1, 0.4, 0.9],
                density: 80.0,
                preserve_luminosity: false,
            },
            Adjustment::ChannelMixer {
                red: [60.0, 50.0, -20.0, 5.0],
                green: [-30.0, 150.0, 0.0, -10.0],
                blue: [10.0, 20.0, 70.0, 0.0],
                monochrome: false,
            },
            Adjustment::ChannelMixer {
                red: [40.0, 40.0, 20.0, 0.0],
                green: [0.0, 100.0, 0.0, 0.0],
                blue: [0.0, 0.0, 100.0, 0.0],
                monochrome: true,
            },
            Adjustment::Curves {
                rgb: Curve::new(&[[0, 20], [70, 50], [180, 220], [255, 250]]).unwrap(),
                red: Curve::new(&[[0, 0], [128, 170], [255, 255]]).unwrap(),
                green: Curve::IDENTITY,
                blue: Curve::new(&[[40, 0], [200, 255]]).unwrap(),
            },
        ] {
            let mut s = Session::new(Document::new(size));
            s.perform(Edit::SetBlendSpace { space }).unwrap();
            let base = image(size, PixelFormat::RGBA8_SRGB, pattern);
            push_into(&mut s, None, raster(&base), BlendMode::Normal, 1.0);
            push_into(
                &mut s,
                None,
                LayerContent::Adjustment { adjustment },
                BlendMode::Normal,
                0.85,
            );
            let what = format!("{space:?} {adjustment:?}");
            assert_matches_cpu(&r, s.document(), size.bounds(), &what);
        }
    }
}
