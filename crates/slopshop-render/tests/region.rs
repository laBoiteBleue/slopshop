//! Full-resolution region rendering (export). Skipped when no GPU adapter is available, unless
//! `SLOPSHOP_REQUIRE_GPU=1`.

use std::sync::Arc;

use slopshop_core::color::{
    AlphaMode, ChannelLayout, ColorSpace, Mat3, PixelFormat, SampleType, WORKING_SPACE, mat_vec,
    srgb_decode,
};
use slopshop_core::{Document, Edit, Layer, LayerContent, LinearRgba, RasterImage, Rect, Session};
use slopshop_core::{LayerId, Size};
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
            index,
            layer: Layer {
                id,
                name: "layer".into(),
                visible: true,
                opacity,
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
        image: image.clone(),
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
fn non_finite_samples_are_mapped() {
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
    let out = render(&r, s.document(), region);
    assert_eq!(at(&out, region, 0, 0), [f32::MAX, -f32::MAX, 0.0, 1.0]);
    // A NaN alpha is transparent.
    assert_eq!(at(&out, region, 1, 0), [0.0; 4]);

    // Under an opaque layer, nothing shows through.
    let blue = LinearRgba::new(0.0, 0.0, 1.0, 1.0);
    push_layer(&mut s, LayerContent::Fill { color: blue }, 1.0);
    let out = render(&r, s.document(), region);
    for x in 0..2 {
        assert_eq!(at(&out, region, x, 0), [0.0, 0.0, 1.0, 1.0], "pixel {x}");
    }
}

#[test]
fn overflowing_composites_stay_finite() {
    let Some(r) = renderer() else { return };
    let size = Size::new(1, 1);
    let premultiplied = float_format(WORKING_SPACE, AlphaMode::Premultiplied);
    let opaque = image(size, premultiplied, |_, _| floats([3e38, 3e38, 3e38, 1.0]));
    // Premultiplied color far above its alpha: "over" overflows f32.
    let glow = image(size, premultiplied, |_, _| floats([3e38, 3e38, 3e38, 0.01]));
    let mut s = Session::new(Document::new(size));
    push_layer(&mut s, raster(&opaque), 1.0);
    push_layer(&mut s, raster(&glow), 1.0);
    let region = size.bounds();
    let out = render(&r, s.document(), region);
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
    let mut s = Session::new(Document::new(size));
    let color = LinearRgba::new(0.2, 0.4, 0.6, 1.0);
    push_layer(&mut s, LayerContent::Fill { color }, 0.25);
    let region = Rect::new(5, 3, 10, 4);
    let out = render(&r, s.document(), region);
    for (x, y) in [(0, 0), (9, 3), (4, 2)] {
        assert_close(at(&out, region, x, y), [0.05, 0.1, 0.15, 0.25], "alone");
    }

    // Red at 50 % over opaque blue.
    let mut s = Session::new(Document::new(size));
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
