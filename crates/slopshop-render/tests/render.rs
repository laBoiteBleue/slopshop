//! GPU rendering tests. Skipped when no adapter is available, unless `SLOPSHOP_REQUIRE_GPU=1`.

use slopshop_core::color::{PixelFormat, srgb_encode};
use slopshop_core::view::ViewTransform;
use slopshop_core::{Document, Edit, Layer, LayerContent, LinearRgba, Session, Size};
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
            index,
            layer: Layer {
                id,
                name: "fill".into(),
                visible: true,
                opacity,
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
fn empty_output_is_an_error() {
    let Some(r) = renderer() else { return };
    let doc = Document::new(Size::new(8, 8));
    assert!(matches!(
        r.render_view(&doc, identity(), Size::new(0, 10)),
        Err(RenderError::EmptyOutput)
    ));
}
