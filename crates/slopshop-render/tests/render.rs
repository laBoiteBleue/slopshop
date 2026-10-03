//! GPU rendering tests. Skipped when no adapter is available, unless `SLOPSHOP_REQUIRE_GPU=1`.

use slopshop_core::color::{PixelFormat, srgb_encode};
use slopshop_core::view::ViewTransform;
use slopshop_core::{BlendMode, Document, Edit, Layer, LayerContent, LinearRgba, Session, Size};
use slopshop_render::{Frame, RenderError, Renderer};

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

fn add_fill(session: &mut Session, color: LinearRgba, opacity: f32) {
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
                name: "fill".into(),
                visible: true,
                opacity,
                blend_mode: BlendMode::Normal,
                mask: None,
                content: LayerContent::Fill { color },
            },
        })
        .unwrap();
}

fn pixel(frame: &Frame, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * frame.size.width + x) * 4) as usize;
    frame.data[i..i + 4].try_into().unwrap()
}

/// Expected display value for a linear color, as the shader encodes it.
fn to_display(linear: [f32; 3]) -> [u8; 4] {
    let q = |v: f32| (srgb_encode(v.clamp(0.0, 1.0)) * 255.0).round() as u8;
    [q(linear[0]), q(linear[1]), q(linear[2]), 255]
}

fn assert_close(actual: [u8; 4], expected: [u8; 4]) {
    let ok = actual.iter().zip(expected).all(|(a, e)| a.abs_diff(e) <= 1);
    assert!(ok, "pixel {actual:?} != expected {expected:?}");
}

/// Identity view: output pixel == document pixel.
fn identity() -> ViewTransform {
    ViewTransform {
        origin: [0.0, 0.0],
        scale: 1.0,
    }
}

#[test]
fn opaque_fill_covers_document() {
    let Some(r) = renderer() else { return };
    let mut s = Session::new(Document::new(Size::new(32, 16)));
    add_fill(&mut s, LinearRgba::new(1.0, 0.0, 0.0, 1.0), 1.0);

    let frame = r
        .render_view(s.document(), identity(), Size::new(32, 16))
        .unwrap();
    assert_eq!(frame.format, PixelFormat::RGBA8_SRGB);
    assert_eq!(frame.data.len(), 32 * 16 * 4);
    for (x, y) in [(0, 0), (31, 15), (17, 3)] {
        assert_eq!(pixel(&frame, x, y), [255, 0, 0, 255]);
    }
}

#[test]
fn pasteboard_outside_document() {
    let Some(r) = renderer() else { return };
    let mut s = Session::new(Document::new(Size::new(10, 10)));
    add_fill(&mut s, LinearRgba::new(0.0, 1.0, 0.0, 1.0), 1.0);

    // Output twice as wide as the document: the right half is pasteboard.
    let frame = r
        .render_view(s.document(), identity(), Size::new(20, 10))
        .unwrap();
    assert_eq!(pixel(&frame, 5, 5), [0, 255, 0, 255]);
    assert_close(pixel(&frame, 15, 5), to_display([0.0144, 0.0144, 0.0168]));
}

#[test]
fn composites_in_linear_light_over_checkerboard() {
    let Some(r) = renderer() else { return };
    let mut s = Session::new(Document::new(Size::new(16, 16)));
    // White at 50%: the result must be the linear average, not the sRGB-encoded one.
    add_fill(&mut s, LinearRgba::new(1.0, 1.0, 1.0, 1.0), 0.5);

    let frame = r
        .render_view(s.document(), identity(), Size::new(16, 16))
        .unwrap();
    let over = |bg: f32| {
        let v = 0.5 + bg * 0.5;
        [v, v, v]
    };
    // Checker squares are 8 output pixels: (0,0) is light, (8,0) is dark.
    assert_close(pixel(&frame, 0, 0), to_display(over(0.527)));
    assert_close(pixel(&frame, 8, 0), to_display(over(0.314)));
}

#[test]
fn hidden_layers_are_ignored_and_order_matters() {
    let Some(r) = renderer() else { return };
    let mut s = Session::new(Document::new(Size::new(8, 8)));
    add_fill(&mut s, LinearRgba::new(1.0, 0.0, 0.0, 1.0), 1.0);
    add_fill(&mut s, LinearRgba::new(0.0, 0.0, 1.0, 1.0), 1.0);
    let render = |s: &Session| {
        let f = r
            .render_view(s.document(), identity(), Size::new(8, 8))
            .unwrap();
        pixel(&f, 4, 4)
    };
    assert_eq!(render(&s), [0, 0, 255, 255], "top layer wins");

    let top = s.document().layers()[1].id;
    s.perform(Edit::SetLayerVisible {
        id: top,
        visible: false,
    })
    .unwrap();
    assert_eq!(render(&s), [255, 0, 0, 255]);
}

#[test]
fn fit_view_renders_large_document_at_viewport_size() {
    let Some(r) = renderer() else { return };
    // 100 megapixels: only the 64x64 output is ever computed.
    let mut s = Session::new(Document::new(Size::new(12_000, 8_400)));
    add_fill(&mut s, LinearRgba::new(0.0, 0.0, 1.0, 1.0), 1.0);
    let output = Size::new(64, 64);
    let view = ViewTransform::fit(s.document().size(), output, 0);

    let frame = r.render_view(s.document(), view, output).unwrap();
    assert_eq!(frame.data.len(), 64 * 64 * 4);
    assert_eq!(pixel(&frame, 32, 32), [0, 0, 255, 255]);
    // Wide document in a square output: top rows are pasteboard.
    assert_ne!(pixel(&frame, 32, 0), [0, 0, 255, 255]);
}

#[test]
fn oversized_output_is_rejected_before_allocating() {
    let Some(r) = renderer() else { return };
    assert!(matches!(
        r.output_byte_len(Size::new(u32::MAX, u32::MAX)),
        Err(RenderError::OutputTooLarge { .. })
    ));
    assert_eq!(r.output_byte_len(Size::new(10, 10)).unwrap(), 400);
}

#[test]
fn empty_output_is_an_error() {
    let Some(r) = renderer() else { return };
    let doc = Document::new(Size::new(8, 8));
    assert!(matches!(
        r.render_view(&doc, identity(), Size::new(0, 10)),
        Err(RenderError::EmptyOutput)
    ));
}

#[test]
fn render_into_appends_after_existing_bytes() {
    let Some(r) = renderer() else { return };
    let mut s = Session::new(Document::new(Size::new(4, 4)));
    add_fill(&mut s, LinearRgba::new(0.0, 1.0, 0.0, 1.0), 1.0);
    let mut out = vec![7u8; 16];
    r.render_view_into(
        s.document(),
        identity(),
        Default::default(),
        Size::new(4, 4),
        &mut out,
    )
    .unwrap();
    assert_eq!(out.len(), 16 + 4 * 4 * 4);
    assert_eq!(&out[..16], &[7u8; 16], "prefix untouched");
    assert_eq!(&out[16..20], &[0, 255, 0, 255]);

    let before = out.clone();
    assert!(
        r.render_view_into(
            s.document(),
            identity(),
            Default::default(),
            Size::new(0, 4),
            &mut out
        )
        .is_err()
    );
    assert_eq!(out, before, "unchanged on error");
}

fn raster_session(size: Size, pixel: impl Fn(u32, u32) -> [u8; 4]) -> Session {
    use slopshop_core::RasterImage;
    use std::sync::Arc;
    let mut px = Vec::with_capacity(size.pixel_count() as usize * 4);
    for y in 0..size.height {
        for x in 0..size.width {
            px.extend(pixel(x, y));
        }
    }
    let image = RasterImage::from_pixels(size, PixelFormat::RGBA8_SRGB, &px).unwrap();
    let mut s = Session::new(Document::new(size));
    let id = s.allocate_layer_id();
    s.perform(Edit::InsertLayer {
        parent: None,
        index: 0,
        layer: Layer {
            transform: slopshop_core::Affine::IDENTITY,
            clipped: false,
            id,
            name: "image".into(),
            visible: true,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            mask: None,
            content: LayerContent::Raster {
                stack: None,
                image: slopshop_core::stack::Pixels::ready(Arc::new(image)),
            },
        },
    })
    .unwrap();
    s
}

/// A pattern with distinct values per pixel, to catch any tile or coordinate mix-up.
fn pattern(x: u32, y: u32) -> [u8; 4] {
    [(x % 251) as u8, (y % 241) as u8, ((x + y) % 239) as u8, 255]
}

#[test]
fn raster_pixels_are_displayed_exactly_at_100_percent() {
    let Some(r) = renderer() else { return };
    // Two tile columns and two tile rows, with partial edge tiles.
    let size = Size::new(300, 270);
    let s = raster_session(size, pattern);
    let frame = r.render_view(s.document(), identity(), size).unwrap();
    for (x, y) in [
        (0, 0),
        (255, 0),
        (256, 0),
        (299, 269),
        (123, 257),
        (290, 10),
    ] {
        // sRGB 8-bit -> linear on GPU load -> sRGB 8-bit out: must round-trip within 1.
        assert_close(pixel(&frame, x, y), pattern(x, y));
    }
}

#[test]
fn raster_zoomed_out_uses_the_pyramid() {
    let Some(r) = renderer() else { return };
    let size = Size::new(2048, 1024);
    let s = raster_session(size, |_, _| [200, 100, 50, 255]);
    let view = ViewTransform {
        origin: [0.0, 0.0],
        scale: 8.0,
    };
    let frame = r
        .render_view(s.document(), view, Size::new(256, 128))
        .unwrap();
    for (x, y) in [(0, 0), (255, 127), (100, 60)] {
        assert_close(pixel(&frame, x, y), [200, 100, 50, 255]);
    }
}

#[test]
fn raster_opacity_and_transparency_composite_over_checkerboard() {
    let Some(r) = renderer() else { return };
    let size = Size::new(16, 16);
    let mut s = raster_session(size, |x, _| {
        if x < 8 {
            [255, 255, 255, 255]
        } else {
            [255, 255, 255, 0]
        }
    });
    let id = s.document().layers()[0].id;
    s.perform(Edit::SetLayerOpacity { id, opacity: 0.5 })
        .unwrap();
    let frame = r.render_view(s.document(), identity(), size).unwrap();
    // Opaque white at 50% over the light checker square.
    let v = 0.5 + 0.527 * 0.5;
    assert_close(pixel(&frame, 0, 0), to_display([v, v, v]));
    // Fully transparent pixels show the dark checker square untouched.
    assert_close(pixel(&frame, 8, 0), to_display([0.314, 0.314, 0.314]));
}

#[test]
fn raster_cache_survives_many_frames_and_views() {
    let Some(r) = renderer() else { return };
    let size = Size::new(1500, 900);
    let s = raster_session(size, pattern);
    // Pan across the image at 100% and zoomed out: tiles get uploaded and reused.
    for step in 0..6 {
        let view = ViewTransform {
            origin: [f64::from(step) * 200.0, 100.0],
            scale: if step % 2 == 0 { 1.0 } else { 3.0 },
        };
        let out = Size::new(320, 240);
        let frame = r.render_view(s.document(), view, out).unwrap();
        if view.scale == 1.0 {
            let x = (view.origin[0] as u32) + 10;
            assert_close(pixel(&frame, 10, 20), pattern(x, 120));
        }
    }
}

// --- Tile budget and cache behavior, with a deliberately tiny cache -------------------------

fn raster_image(
    size: Size,
    pixel: impl Fn(u32, u32) -> [u8; 4],
) -> std::sync::Arc<slopshop_core::RasterImage> {
    let mut px = Vec::with_capacity(size.pixel_count() as usize * 4);
    for y in 0..size.height {
        for x in 0..size.width {
            px.extend(pixel(x, y));
        }
    }
    std::sync::Arc::new(
        slopshop_core::RasterImage::from_pixels(size, PixelFormat::RGBA8_SRGB, &px).unwrap(),
    )
}

/// A document whose layers (bottom to top) show the given images.
fn raster_stack(size: Size, images: &[std::sync::Arc<slopshop_core::RasterImage>]) -> Session {
    let mut s = Session::new(Document::new(size));
    for image in images {
        let id = s.allocate_layer_id();
        let index = s.document().layers().len();
        s.perform(Edit::InsertLayer {
            parent: None,
            index,
            layer: Layer {
                transform: slopshop_core::Affine::IDENTITY,
                clipped: false,
                id,
                name: "image".into(),
                visible: true,
                opacity: 1.0,
                blend_mode: BlendMode::Normal,
                mask: None,
                content: LayerContent::Raster {
                    stack: None,
                    image: slopshop_core::stack::Pixels::ready(image.clone()),
                },
            },
        })
        .unwrap();
    }
    s
}

/// Pixel (x, y) of a pyramid level, as stored in its tiles.
fn level_pixel(image: &slopshop_core::RasterImage, level: usize, x: u32, y: u32) -> [u8; 4] {
    use slopshop_core::raster::TILE_SIZE;
    let tile = image.levels()[level]
        .tile(slopshop_core::tile::TileCoord {
            col: x / TILE_SIZE,
            row: y / TILE_SIZE,
        })
        .unwrap();
    let i = (((y % TILE_SIZE) * TILE_SIZE + x % TILE_SIZE) * 4) as usize;
    tile[i..i + 4].try_into().unwrap()
}

#[test]
fn zoomed_out_pixels_come_from_the_right_level_and_tile() {
    let Some(r) = renderer() else { return };
    let size = Size::new(1024, 1024);
    let image = raster_image(size, pattern);
    let s = raster_stack(size, std::slice::from_ref(&image));
    // Scale 2: level 1, one level pixel per output pixel.
    let view = ViewTransform {
        origin: [0.0, 0.0],
        scale: 2.0,
    };
    let frame = r
        .render_view(s.document(), view, Size::new(512, 512))
        .unwrap();
    for (x, y) in [(0, 0), (255, 255), (256, 3), (300, 400), (511, 511)] {
        assert_close(pixel(&frame, x, y), level_pixel(&image, 1, x, y));
    }
}

#[test]
fn over_budget_view_falls_back_to_a_coarser_level() {
    let Some(r) = renderer() else { return };
    let r = r.with_tile_capacity(4);
    let size = Size::new(1024, 1024);
    let image = raster_image(size, pattern);
    let s = raster_stack(size, std::slice::from_ref(&image));
    // 100% needs 16 level-0 tiles; only 4 fit, so level 1 (4 tiles) is used instead.
    let frame = r.render_view(s.document(), identity(), size).unwrap();
    for (x, y) in [(0, 0), (513, 7), (1023, 1023), (600, 900)] {
        assert_close(pixel(&frame, x, y), level_pixel(&image, 1, x / 2, y / 2));
    }
}

#[test]
fn cache_evicts_and_reloads_tiles_when_panning() {
    let Some(r) = renderer() else { return };
    let r = r.with_tile_capacity(2);
    let size = Size::new(1024, 256);
    let s = raster_stack(size, &[raster_image(size, pattern)]);
    // Each view shows exactly one tile; four tiles cycle through a two-slot cache, then back.
    for column in [0u32, 1, 2, 3, 0, 2] {
        let view = ViewTransform {
            origin: [f64::from(column * 256), 0.0],
            scale: 1.0,
        };
        let frame = r
            .render_view(s.document(), view, Size::new(256, 256))
            .unwrap();
        for (x, y) in [(0, 0), (255, 255), (100, 37)] {
            assert_close(pixel(&frame, x, y), pattern(column * 256 + x, y));
        }
    }
}

#[test]
fn top_layer_stays_visible_when_layers_exceed_the_budget() {
    let Some(r) = renderer() else { return };
    let r = r.with_tile_capacity(8);
    let size = Size::new(1024, 512);
    // Three opaque full-size layers (8 tiles each at 100%) for an 8-tile cache.
    let blue = raster_image(size, |_, _| [0, 0, 255, 255]);
    let green = raster_image(size, |_, _| [0, 255, 0, 255]);
    let red = raster_image(size, |_, _| [255, 0, 0, 255]);
    let s = raster_stack(size, &[blue, green, red]);
    let frame = r.render_view(s.document(), identity(), size).unwrap();
    for (x, y) in [(0, 0), (1023, 511), (512, 256)] {
        assert_close(pixel(&frame, x, y), [255, 0, 0, 255]);
    }
}

#[test]
fn layers_sharing_an_image_share_its_tiles() {
    let Some(r) = renderer() else { return };
    let r = r.with_tile_capacity(8);
    let size = Size::new(1024, 512);
    let image = raster_image(size, pattern);
    // Same image twice: 8 distinct tiles, so both layers keep full resolution.
    let s = raster_stack(size, &[image.clone(), image]);
    let frame = r.render_view(s.document(), identity(), size).unwrap();
    for (x, y) in [(0, 0), (257, 3), (1023, 511), (700, 300)] {
        assert_close(pixel(&frame, x, y), pattern(x, y));
    }
}

#[test]
fn partially_transparent_top_layer_composites_over_the_bottom_one() {
    let Some(r) = renderer() else { return };
    let size = Size::new(600, 300);
    let bottom = raster_image(size, pattern);
    let top = raster_image(size, |x, _| {
        if x < 300 {
            [255, 0, 0, 255]
        } else {
            [0, 0, 0, 0]
        }
    });
    let s = raster_stack(size, &[bottom, top]);
    let frame = r.render_view(s.document(), identity(), size).unwrap();
    assert_close(pixel(&frame, 10, 10), [255, 0, 0, 255]);
    assert_close(pixel(&frame, 299, 299), [255, 0, 0, 255]);
    for (x, y) in [(300, 0), (599, 299), (450, 123)] {
        assert_close(pixel(&frame, x, y), pattern(x, y));
    }
}

// --- Pixel formats and color spaces --------------------------------------------------------

use slopshop_core::color::{AlphaMode, ChannelLayout, ColorSpace, SampleType, f32_to_f16};

/// A one-layer document from raw bytes in any format.
fn raw_session(size: Size, format: PixelFormat, bytes: &[u8]) -> Session {
    let image = slopshop_core::RasterImage::from_pixels(size, format, bytes).unwrap();
    raster_stack(size, &[std::sync::Arc::new(image)])
}

fn fmt(layout: ChannelLayout, sample: SampleType, space: ColorSpace) -> PixelFormat {
    PixelFormat {
        layout,
        sample,
        color_space: space,
        alpha: AlphaMode::Straight,
    }
}

#[test]
fn sixteen_bit_gray_displays_like_its_8_bit_equivalent() {
    let Some(r) = renderer() else { return };
    let size = Size::new(4, 1);
    // 16-bit sRGB gray values that are exact multiples of 257 (= 8-bit values × 257).
    let values: [u16; 4] = [0, 128 * 257, 200 * 257, 65535];
    let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_ne_bytes()).collect();
    let s = raw_session(
        size,
        fmt(ChannelLayout::Gray, SampleType::U16, ColorSpace::SRGB),
        &bytes,
    );
    let frame = r.render_view(s.document(), identity(), size).unwrap();
    for (x, v) in [(0u32, 0u8), (1, 128), (2, 200), (3, 255)] {
        assert_close(pixel(&frame, x, 0), [v, v, v, 255]);
    }
}

#[test]
fn hdr_float_values_are_clipped_only_at_display() {
    let Some(r) = renderer() else { return };
    let size = Size::new(2, 1);
    // Linear sRGB 4.0 (HDR, brighter than white) and mid gray.
    let px: Vec<u8> = [
        [4.0f32, 4.0, 4.0, 1.0],
        [0.214_041_14, 0.214_041_14, 0.214_041_14, 1.0],
    ]
    .iter()
    .flatten()
    .flat_map(|v| v.to_ne_bytes())
    .collect();
    let mut s = raw_session(
        size,
        fmt(
            ChannelLayout::Rgba,
            SampleType::F32,
            ColorSpace::LINEAR_SRGB,
        ),
        &px,
    );
    let frame = r.render_view(s.document(), identity(), size).unwrap();
    assert_eq!(pixel(&frame, 0, 0), [255, 255, 255, 255]);
    assert_close(pixel(&frame, 1, 0), [128, 128, 128, 255]);
    // At 25% opacity over the checkerboard, 4.0 is not clipped before compositing:
    // 4.0 × 0.25 = 1.0 → still white, where a clipped 1.0 would give ~0.25 + background.
    let id = s.document().layers()[0].id;
    s.perform(Edit::SetLayerOpacity { id, opacity: 0.25 })
        .unwrap();
    let frame = r.render_view(s.document(), identity(), size).unwrap();
    assert_eq!(pixel(&frame, 0, 0), [255, 255, 255, 255]);
}

#[test]
fn half_float_and_premultiplied_sources() {
    let Some(r) = renderer() else { return };
    let size = Size::new(1, 1);
    // Premultiplied linear red at 50% alpha, as half floats.
    let px: Vec<u8> = [0.5f32, 0.0, 0.0, 0.5]
        .iter()
        .flat_map(|v| f32_to_f16(*v).to_ne_bytes())
        .collect();
    let format = PixelFormat {
        alpha: AlphaMode::Premultiplied,
        ..fmt(
            ChannelLayout::Rgba,
            SampleType::F16,
            ColorSpace::LINEAR_SRGB,
        )
    };
    let s = raw_session(size, format, &px);
    let frame = r.render_view(s.document(), identity(), size).unwrap();
    // 0.5 red + 0.5 × light checker (0.527).
    let bg = 0.527 * 0.5;
    assert_close(pixel(&frame, 0, 0), to_display([0.5 + bg, bg, bg]));
}

#[test]
fn wide_gamut_sources_are_converted_to_the_display() {
    let Some(r) = renderer() else { return };
    let size = Size::new(2, 1);
    // Display P3 pixels: white stays white; P3 (0.5, 0.5, 0.5) encoded is neutral too, and
    // a P3 color inside sRGB converts to its sRGB equivalent.
    let srgb_red_in_p3 = slopshop_core::LinearRgba::new(1.0, 0.0, 0.0, 1.0)
        .transform(&ColorSpace::LINEAR_SRGB.matrix_to(&ColorSpace::DISPLAY_P3))
        .to_srgb_encoded();
    let to_u8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let px = [
        255,
        255,
        255,
        255,
        to_u8(srgb_red_in_p3[0]),
        to_u8(srgb_red_in_p3[1]),
        to_u8(srgb_red_in_p3[2]),
        255,
    ];
    let s = raw_session(
        size,
        fmt(ChannelLayout::Rgba, SampleType::U8, ColorSpace::DISPLAY_P3),
        &px,
    );
    let frame = r.render_view(s.document(), identity(), size).unwrap();
    assert_close(pixel(&frame, 0, 0), [255, 255, 255, 255]);
    let red = pixel(&frame, 1, 0);
    assert!(red[0] >= 250 && red[1] <= 12 && red[2] <= 12, "{red:?}");
}

#[test]
fn pq_reference_white_displays_as_white() {
    let Some(r) = renderer() else { return };
    let size = Size::new(2, 1);
    // PQ-encoded 203 cd/m² (reference white) and 10 000 cd/m² (peak): both display as white.
    let white = slopshop_core::color::TransferFunction::Pq.encode(1.0);
    let px: Vec<u8> = [white, white, white, 1.0, 1.0, 1.0, 1.0, 1.0]
        .iter()
        .flat_map(|v| v.to_ne_bytes())
        .collect();
    let s = raw_session(
        size,
        fmt(ChannelLayout::Rgba, SampleType::F32, ColorSpace::REC2100_PQ),
        &px,
    );
    let frame = r.render_view(s.document(), identity(), size).unwrap();
    assert_close(pixel(&frame, 0, 0), [255, 255, 255, 255]);
    assert_eq!(pixel(&frame, 1, 0), [255, 255, 255, 255]);
}

#[test]
fn premultiplied_encoded_sources_are_unpremultiplied_before_decoding() {
    let Some(r) = renderer() else { return };
    let size = Size::new(1, 1);
    // 8-bit sRGB with associated alpha: (128, 128, 128, 128) is white at 50 %.
    let format = PixelFormat {
        alpha: AlphaMode::Premultiplied,
        ..PixelFormat::RGBA8_SRGB
    };
    let s = raw_session(size, format, &[128, 128, 128, 128]);
    let frame = r.render_view(s.document(), identity(), size).unwrap();
    let a = 128.0 / 255.0;
    let v = a + 0.527 * (1.0 - a);
    assert_close(pixel(&frame, 0, 0), to_display([v, v, v]));
}

#[test]
fn non_finite_samples_do_not_show_through_opaque_layers() {
    let Some(r) = renderer() else { return };
    let size = Size::new(3, 1);
    let px: Vec<u8> = [
        [f32::INFINITY, f32::INFINITY, f32::INFINITY, 1.0],
        [f32::NAN, 0.0, f32::NEG_INFINITY, 1.0],
        [0.5, 0.5, 0.5, f32::NAN],
    ]
    .iter()
    .flatten()
    .flat_map(|v| v.to_ne_bytes())
    .collect();
    let hdr = std::sync::Arc::new(
        slopshop_core::RasterImage::from_pixels(
            size,
            fmt(
                ChannelLayout::Rgba,
                SampleType::F32,
                ColorSpace::LINEAR_SRGB,
            ),
            &px,
        )
        .unwrap(),
    );
    // Alone: +inf is the brightest white, NaN is 0, a NaN alpha is transparent.
    let s = raster_stack(size, std::slice::from_ref(&hdr));
    let frame = r.render_view(s.document(), identity(), size).unwrap();
    assert_eq!(pixel(&frame, 0, 0), [255, 255, 255, 255]);
    assert_eq!(pixel(&frame, 1, 0), [0, 0, 0, 255]);
    assert_close(pixel(&frame, 2, 0), to_display([0.527; 3]));
    // Under an opaque blue layer: only blue.
    let blue = raster_image(size, |_, _| [0, 0, 255, 255]);
    let s = raster_stack(size, &[hdr, blue]);
    let frame = r.render_view(s.document(), identity(), size).unwrap();
    for x in 0..3 {
        assert_eq!(pixel(&frame, x, 0), [0, 0, 255, 255], "pixel {x}");
    }
}

#[test]
fn gpu_transfer_functions_match_the_cpu() {
    use slopshop_core::color::TransferFunction;
    let Some(r) = renderer() else { return };
    let offset = TransferFunction::Parametric {
        g: 2.2,
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 0.0,
        e: 0.02,
        f: 0.02,
    };
    let values = [0.0805f32, 0.0812, 0.0815, 0.3, 0.8];
    for transfer in [TransferFunction::Rec709, offset, TransferFunction::Hlg] {
        let space = ColorSpace {
            transfer,
            ..ColorSpace::LINEAR_SRGB
        };
        let size = Size::new(values.len() as u32, 1);
        let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_ne_bytes()).collect();
        let s = raw_session(
            size,
            fmt(ChannelLayout::Gray, SampleType::F32, space),
            &bytes,
        );
        let frame = r.render_view(s.document(), identity(), size).unwrap();
        for (x, v) in values.iter().enumerate() {
            let linear = transfer.decode(*v);
            assert_close(pixel(&frame, x as u32, 0), to_display([linear; 3]));
        }
    }
}

#[test]
fn zoomed_out_rasters_are_area_filtered_not_aliased() {
    let Some(r) = renderer() else { return };
    // One-pixel black and white columns seen at 66.7%: every output pixel covers 1.5 columns.
    // Nearest sampling would show pure black or white; the area filter gives 1/3 or 2/3 of the
    // light (sRGB ~156 or ~213).
    let size = Size::new(300, 2);
    let image = raster_image(size, |x, _| {
        let v = if x % 2 == 0 { 0 } else { 255 };
        [v, v, v, 255]
    });
    let s = raster_stack(size, &[image]);
    let view = ViewTransform {
        origin: [0.0, 0.0],
        scale: 1.5,
    };
    let frame = r
        .render_view(s.document(), view, Size::new(100, 1))
        .unwrap();
    for x in 0..100 {
        let [v, ..] = pixel(&frame, x, 0);
        // Pixel k covers [1.5k, 1.5k + 1.5]: the pattern repeats every 4 pixels (6 columns).
        let expected = if x % 4 < 2 { 1.0 / 3.0 } else { 2.0 / 3.0 };
        assert_close(pixel(&frame, x, 0), to_display([expected; 3]));
        assert!((100..240).contains(&v), "pixel {x}: {v}");
    }
}

#[test]
fn document_edges_blend_with_the_pasteboard_not_the_checkerboard() {
    let Some(r) = renderer() else { return };
    // An opaque black image; the first output pixel covers [-0.75, 0.75]: half pasteboard, half
    // image. The checkerboard (light there) must not show through the antialiased edge, or
    // edges would shimmer while navigating.
    let size = Size::new(100, 100);
    let s = raster_stack(size, &[raster_image(size, |_, _| [0, 0, 0, 255])]);
    let view = ViewTransform {
        origin: [-0.75, 10.0],
        scale: 1.5,
    };
    let frame = r.render_view(s.document(), view, Size::new(4, 1)).unwrap();
    let pasteboard = [0.0144, 0.0144, 0.0168];
    assert_close(pixel(&frame, 0, 0), to_display(pasteboard.map(|v| v * 0.5)));
    // Fully inside: the image.
    assert_close(pixel(&frame, 2, 0), [0, 0, 0, 255]);
}

#[test]
fn a_moved_raster_is_displayed_moved() {
    let Some(r) = renderer() else { return };
    let size = Size::new(300, 270);
    let mut s = raster_session(size, pattern);
    let id = s.document().layers()[0].id;
    s.perform(Edit::SetLayerTransform {
        id,
        transform: slopshop_core::Affine::translation(40.0, -30.0),
    })
    .unwrap();
    let frame = r.render_view(s.document(), identity(), size).unwrap();
    for (x, y) in [(40, 0), (299, 239), (296, 10), (100, 226), (256, 100)] {
        assert_close(pixel(&frame, x, y), pattern(x - 40, y + 30));
    }
}

#[test]
fn zoomed_in_a_transformed_raster_shows_its_document_pixels() {
    let Some(r) = renderer() else { return };
    let size = Size::new(200, 180);
    let mut s = raster_session(size, pattern);
    let id = s.document().layers()[0].id;
    s.perform(Edit::SetLayerTransform {
        id,
        transform: slopshop_core::Affine::rotation(0.4)
            .then(slopshop_core::Affine::translation(60.0, -30.0)),
    })
    .unwrap();
    let full = r.render_view(s.document(), identity(), size).unwrap();
    // At 400 %, each document pixel is a 4 × 4 block showing what 100 % shows (inside the
    // rotated image: no checkerboard, which is laid out in output pixels).
    let view = ViewTransform {
        origin: [50.0, 60.0],
        scale: 0.25,
    };
    let zoomed = r
        .render_view(s.document(), view, Size::new(64, 64))
        .unwrap();
    for oy in 0..64 {
        for ox in 0..64 {
            assert_eq!(
                pixel(&zoomed, ox, oy),
                pixel(&full, 50 + ox / 4, 60 + oy / 4),
                "output ({ox}, {oy})"
            );
        }
    }
}

#[test]
fn zoomed_out_a_reduced_raster_keeps_its_color() {
    let Some(r) = renderer() else { return };
    let size = Size::new(400, 400);
    let mut s = raster_session(size, |_, _| [200, 100, 50, 255]);
    let id = s.document().layers()[0].id;
    s.perform(Edit::SetLayerTransform {
        id,
        transform: slopshop_core::Affine::scale(0.5, 0.5)
            .then(slopshop_core::Affine::rotation(0.3))
            .then(slopshop_core::Affine::translation(100.0, 100.0)),
    })
    .unwrap();
    // Zoomed out 3×: the image's center, (200, 200), is at document (166, 225).
    let view = ViewTransform {
        origin: [0.0, 0.0],
        scale: 3.0,
    };
    let frame = r
        .render_view(s.document(), view, Size::new(134, 134))
        .unwrap();
    assert_close(pixel(&frame, 55, 75), [200, 100, 50, 255]);
}

/// A frame of `doc` at identity with `overlays`.
fn render_overlays(
    r: &Renderer,
    doc: &Document,
    overlays: slopshop_render::ViewOverlays,
    output: Size,
) -> Frame {
    let mut data = Vec::new();
    r.render_view_into(doc, identity(), overlays, output, &mut data)
        .unwrap();
    Frame {
        size: output,
        format: PixelFormat::RGBA8_SRGB,
        data,
    }
}

#[test]
fn quick_mask_tints_what_the_selection_leaves_out() {
    use slopshop_core::selection::{self, Combine, EdgeOptions, Selection, Shape};
    use std::sync::Arc;
    let quick_mask = slopshop_render::ViewOverlays {
        quick_mask: true,
        ..Default::default()
    };
    for cached in [true, false] {
        let Some(r) = renderer() else { return };
        let r = r.with_display_cache(cached);
        // Several tiles, the selection's edge half covering column 300.
        let size = Size::new(600, 300);
        let mut s = Session::new(Document::new(size));
        add_fill(&mut s, LinearRgba::new(1.0, 1.0, 1.0, 1.0), 1.0);
        let without = render_overlays(&r, s.document(), quick_mask, size);
        assert_eq!(
            pixel(&without, 450, 10),
            [255, 255, 255, 255],
            "no selection, no tint"
        );

        let shape = Shape::Rectangle {
            left: 0.0,
            top: 0.0,
            right: 300.5,
            bottom: 300.0,
        };
        let image =
            selection::select_shape(size, None, &shape, EdgeOptions::default(), Combine::Replace)
                .unwrap()
                .unwrap();
        s.perform(Edit::SetSelection {
            selection: Selection::new(Arc::new(image)),
        })
        .unwrap();
        let plain = render_overlays(&r, s.document(), Default::default(), size);
        let masked = render_overlays(&r, s.document(), quick_mask, size);
        assert_eq!(
            pixel(&plain, 450, 10),
            [255, 255, 255, 255],
            "off by default"
        );
        assert_eq!(
            pixel(&masked, 10, 10),
            [255, 255, 255, 255],
            "selected: untouched"
        );
        assert_close(pixel(&masked, 450, 290), [255, 128, 128, 255]);
        // Half selected: half the tint.
        assert_close(pixel(&masked, 300, 150), [255, 191, 191, 255]);
        // The overlay's opacity: a quarter, or fully opaque.
        let light = slopshop_render::ViewOverlays {
            quick_mask_opacity: 25,
            ..quick_mask
        };
        let opaque = slopshop_render::ViewOverlays {
            quick_mask_opacity: 100,
            ..quick_mask
        };
        let light = render_overlays(&r, s.document(), light, size);
        let opaque = render_overlays(&r, s.document(), opaque, size);
        assert_close(pixel(&light, 450, 290), [255, 191, 191, 255]);
        assert_close(pixel(&opaque, 450, 290), [255, 0, 0, 255]);
    }
}
