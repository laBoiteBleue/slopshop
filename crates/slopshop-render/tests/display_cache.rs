//! Display cache tests (ADR 0022). Skipped when no adapter is available, unless
//! `SLOPSHOP_REQUIRE_GPU=1`, or when `SLOPSHOP_SKIP_GPU_TESTS=1`.

use std::sync::Arc;

use slopshop_core::adjust::Adjustment;
use slopshop_core::color::{AlphaMode, ChannelLayout, ColorSpace, PixelFormat, SampleType};
use slopshop_core::view::ViewTransform;
use slopshop_core::{
    Affine, BlendMode, Document, Edit, Layer, LayerContent, LayerId, LayerMask, LinearRgba,
    RasterImage, Session, Size,
};
use slopshop_render::{FrameStats, Renderer};

fn renderer(display_cache: bool) -> Option<Renderer> {
    // Windows CI runners render on WARP (software Direct3D 12), far too slow for these tests:
    // Linux CI runs them on lavapipe.
    if std::env::var("SLOPSHOP_SKIP_GPU_TESTS").as_deref() == Ok("1") {
        eprintln!("skipping GPU test: SLOPSHOP_SKIP_GPU_TESTS=1");
        return None;
    }
    match Renderer::new() {
        Ok(r) => Some(r.with_display_cache(display_cache)),
        Err(e) if std::env::var("SLOPSHOP_REQUIRE_GPU").as_deref() == Ok("1") => {
            panic!("GPU required but unavailable: {e}")
        }
        Err(e) => {
            eprintln!("skipping GPU test: {e}");
            None
        }
    }
}

fn image(size: Size, pixel: impl Fn(u32, u32) -> [u8; 4]) -> Arc<RasterImage> {
    let mut px = Vec::with_capacity(size.pixel_count() as usize * 4);
    for y in 0..size.height {
        for x in 0..size.width {
            px.extend(pixel(x, y));
        }
    }
    Arc::new(RasterImage::from_pixels(size, PixelFormat::RGBA8_SRGB, &px).unwrap())
}

fn gray_mask(size: Size, value: impl Fn(u32, u32) -> u8) -> Arc<RasterImage> {
    let format = PixelFormat {
        layout: ChannelLayout::Gray,
        sample: SampleType::U8,
        color_space: ColorSpace::LINEAR_SRGB,
        alpha: AlphaMode::Straight,
    };
    let mut px = Vec::with_capacity(size.pixel_count() as usize);
    for y in 0..size.height {
        for x in 0..size.width {
            px.push(value(x, y));
        }
    }
    Arc::new(RasterImage::from_pixels(size, format, &px).unwrap())
}

fn layer(s: &mut Session, content: LayerContent) -> Layer {
    Layer {
        style: None,
        transform: Affine::IDENTITY.into(),
        clipped: false,
        id: s.allocate_layer_id(),
        name: "layer".into(),
        visible: true,
        opacity: 1.0,
        blend_mode: BlendMode::Normal,
        mask: None,
        content,
    }
}

fn raster(s: &mut Session, image: Arc<RasterImage>) -> Layer {
    layer(s, LayerContent::raster(image))
}

fn push(s: &mut Session, layer: Layer) -> LayerId {
    let id = layer.id;
    let index = s.document().layers().len();
    s.perform(Edit::InsertLayer {
        parent: None,
        index,
        layer,
    })
    .unwrap();
    id
}

fn levels(white: f32) -> Adjustment {
    Adjustment::Levels {
        input_black: 0.1,
        input_white: white,
        gamma: 1.2,
        output_black: 0.0,
        output_white: 1.0,
        channels: [slopshop_core::adjust::LEVELS_IDENTITY; 3],
    }
}

/// Rasters placed at whole pixels and resampled, blend modes, a masked fill and an isolated
/// group with an adjustment.
fn varied_document() -> Session {
    let mut s = Session::new(Document::new(Size::new(700, 500)));
    let bg = image(Size::new(700, 500), |x, y| {
        [(x % 251) as u8, (y % 241) as u8, ((x + y) % 239) as u8, 255]
    });
    let bg = raster(&mut s, bg);
    push(&mut s, bg);
    let mut multiply = raster(
        &mut s,
        image(Size::new(300, 200), |x, y| {
            [200, (x * 3) as u8, (y * 5) as u8, (x + y) as u8]
        }),
    );
    multiply.transform = Affine::translation(37.0, 51.0).into();
    multiply.blend_mode = BlendMode::Multiply;
    multiply.opacity = 0.8;
    push(&mut s, multiply);
    let mut scaled = raster(
        &mut s,
        image(Size::new(200, 150), |x, y| [(x ^ y) as u8, 90, 160, 230]),
    );
    scaled.transform = Affine::scale(1.3, 1.3)
        .then(Affine::translation(400.5, 260.25))
        .into();
    push(&mut s, scaled);
    let mut fill = layer(
        &mut s,
        LayerContent::Fill {
            color: LinearRgba::new(0.9, 0.1, 0.2, 1.0),
        },
    );
    fill.opacity = 0.6;
    fill.mask = Some(LayerMask {
        original: None,
        image: gray_mask(Size::new(300, 300), |x, y| ((x + 2 * y) % 256) as u8),
        enabled: true,
        replaces_alpha: false,
    });
    fill.transform = Affine::translation(300.0, 100.0).into();
    push(&mut s, fill);
    let mut slice = raster(
        &mut s,
        image(Size::new(120, 120), |x, y| [x as u8, y as u8, 128, 255]),
    );
    slice.transform = Affine::translation(500.0, 40.0).into();
    let window = layer(
        &mut s,
        LayerContent::Adjustment {
            adjustment: levels(0.7),
        },
    );
    let group = layer(
        &mut s,
        LayerContent::Group {
            children: vec![slice, window],
            pass_through: false,
        },
    );
    push(&mut s, group);
    s
}

fn assert_frames_match(a: &[u8], b: &[u8], what: &str) {
    assert_eq!(a.len(), b.len());
    let worst = a
        .iter()
        .zip(b)
        .map(|(x, y)| x.abs_diff(*y))
        .max()
        .unwrap_or(0);
    assert!(worst <= 1, "{what}: channels differ by up to {worst}");
}

#[test]
fn cached_views_match_direct_ones_at_100_percent_and_power_of_two_zooms() {
    let (Some(cached), Some(direct)) = (renderer(true), renderer(false)) else {
        return;
    };
    let s = varied_document();
    let output = Size::new(320, 240);
    let views = [
        ([0.0, 0.0], 1.0),
        ([320.0, 192.0], 1.0),
        ([64.0, 32.0], 2.0),
        ([0.0, 0.0], 4.0),
        ([480.0, 30.0], 0.25),
        // Beyond the document's edges: the pasteboard.
        ([-40.0, -24.0], 1.0),
    ];
    for (origin, scale) in views {
        let view = ViewTransform { origin, scale };
        let a = cached.render_view(s.document(), view, output).unwrap();
        let b = direct.render_view(s.document(), view, output).unwrap();
        assert_frames_match(&a.data, &b.data, &format!("{origin:?} at {scale}"));
    }
}

fn frame(r: &Renderer, document: &Document) -> FrameStats {
    let view = ViewTransform {
        origin: [0.0, 0.0],
        scale: 1.0,
    };
    r.profile_view(document, view, document.size(), false)
        .unwrap()
}

#[test]
fn edits_recomposite_only_the_tiles_they_reach_and_undo_reuses_them() {
    let Some(r) = renderer(true) else { return };
    // 4 × 2 tiles at 100 %; the small image and the group are in the first one.
    let mut s = Session::new(Document::new(Size::new(1024, 512)));
    let bg = layer(
        &mut s,
        LayerContent::Fill {
            color: LinearRgba::new(0.2, 0.3, 0.4, 1.0),
        },
    );
    push(&mut s, bg);
    let mut small = raster(&mut s, image(Size::new(50, 50), |_, _| [255, 0, 0, 255]));
    small.transform = Affine::translation(10.0, 10.0).into();
    let small = push(&mut s, small);
    let mut slice = raster(&mut s, image(Size::new(40, 40), |_, _| [0, 255, 0, 255]));
    slice.transform = Affine::translation(100.0, 100.0).into();
    let window = layer(
        &mut s,
        LayerContent::Adjustment {
            adjustment: levels(0.7),
        },
    );
    let window_id = window.id;
    let group = layer(
        &mut s,
        LayerContent::Group {
            children: vec![slice, window],
            pass_through: false,
        },
    );
    push(&mut s, group);

    let first = frame(&r, s.document());
    assert_eq!((first.tiles_composited, first.tiles_reused), (8, 0));
    let again = frame(&r, s.document());
    assert_eq!((again.tiles_composited, again.tiles_reused), (0, 8));
    assert_eq!(again.tiles_uploaded, 0);

    s.perform(Edit::SetLayerOpacity {
        id: small,
        opacity: 0.5,
    })
    .unwrap();
    let edited = frame(&r, s.document());
    assert_eq!((edited.tiles_composited, edited.tiles_reused), (1, 7));

    // The adjustment of an isolated group only changes where the group has content.
    s.perform(Edit::SetAdjustment {
        id: window_id,
        adjustment: levels(0.5),
    })
    .unwrap();
    let adjusted = frame(&r, s.document());
    assert_eq!((adjusted.tiles_composited, adjusted.tiles_reused), (1, 7));

    s.undo().unwrap();
    s.undo().unwrap();
    let undone = frame(&r, s.document());
    assert_eq!((undone.tiles_composited, undone.tiles_reused), (0, 8));
}

#[test]
fn views_needing_more_tiles_than_the_cache_holds_are_composited_directly() {
    let (Some(direct), Some(r)) = (renderer(false), renderer(true)) else {
        return;
    };
    // The 700 × 500 document at 100 %: 3 × 2 tiles, for a cache of 4.
    let r = r.with_display_cache_capacity(4);
    let s = varied_document();
    let stats = frame(&r, s.document());
    assert_eq!((stats.tiles_composited, stats.tiles_reused), (0, 0));
    assert!(stats.layers > 0);
    let view = ViewTransform {
        origin: [0.0, 0.0],
        scale: 1.0,
    };
    let output = s.document().size();
    let a = r.render_view(s.document(), view, output).unwrap();
    let b = direct.render_view(s.document(), view, output).unwrap();
    assert_eq!(a.data, b.data);
    // Zoomed out, 1 tile: through the cache.
    let view = ViewTransform {
        origin: [0.0, 0.0],
        scale: 4.0,
    };
    let stats = r
        .profile_view(s.document(), view, Size::new(175, 125), false)
        .unwrap();
    assert_eq!(stats.tiles_composited, 1);
}

#[test]
fn progressive_frames_composite_a_budget_of_tiles_and_fill_in_from_a_coarser_level() {
    let (Some(r), Some(direct)) = (renderer(true), renderer(false)) else {
        return;
    };
    // Many resampled layers covering the whole canvas: costly tiles (3 fit a frame's budget),
    // and a uniform result, so that the coarser level shows exactly what the view's level will.
    // Small enough for software adapters (CI).
    let size = Size::new(1024, 512);
    let mut s = Session::new(Document::new(size));
    let bg = layer(
        &mut s,
        LayerContent::Fill {
            color: LinearRgba::new(0.2, 0.3, 0.4, 1.0),
        },
    );
    push(&mut s, bg);
    for _ in 0..40 {
        let mut veil = raster(
            &mut s,
            image(Size::new(300, 160), |_, _| [200, 120, 40, 20]),
        );
        veil.transform = Affine::scale(4.0, 4.0)
            .then(Affine::translation(-100.5, -100.25))
            .into();
        push(&mut s, veil);
    }
    let view = ViewTransform {
        origin: [0.0, 0.0],
        scale: 1.0,
    };
    let expected = direct.render_view(s.document(), view, size).unwrap();

    let (first, stats) = r.render_view_progressive(s.document(), view, size).unwrap();
    assert!(stats.incomplete);
    // A few of the 8 tiles, plus the coarser level's.
    assert!(stats.tiles_composited < 8, "{stats:?}");
    assert_frames_match(&first.data, &expected.data, "partial frame");

    let mut frames = 1;
    loop {
        let (frame, stats) = r.render_view_progressive(s.document(), view, size).unwrap();
        frames += 1;
        assert!(frames < 64, "never complete");
        if !stats.incomplete {
            assert_frames_match(&frame.data, &expected.data, "complete frame");
            break;
        }
    }
    assert!(frames > 2, "{frames} frames");
    // Everything cached: a frame composites nothing.
    let stats = r.profile_view(s.document(), view, size, true).unwrap();
    assert_eq!((stats.tiles_composited, stats.incomplete), (0, false));
}

#[test]
fn a_styled_layer_shows_at_once_and_its_effects_once_drawn() {
    use slopshop_core::style::{DropShadow, LayerStyle, Stroke};
    let (Some(r), Some(direct)) = (renderer(true), renderer(false)) else {
        return;
    };
    let size = Size::new(512, 384);
    let mut s = Session::new(Document::new(size));
    let bg = layer(
        &mut s,
        LayerContent::Fill {
            color: LinearRgba::new(1.0, 1.0, 1.0, 1.0),
        },
    );
    push(&mut s, bg);
    let mut square = raster(
        &mut s,
        image(Size::new(100, 100), |_, _| [200, 60, 30, 255]),
    );
    square.transform = Affine::translation(150.0, 90.0).into();
    let id = push(&mut s, square);
    s.perform(Edit::SetLayerStyle {
        id,
        style: Some(Box::new(LayerStyle {
            drop_shadow: Some(DropShadow {
                size: 20.0,
                distance: 15.0,
                ..DropShadow::default()
            }),
            stroke: Some(Stroke {
                size: 6.0,
                ..Stroke::default()
            }),
            ..LayerStyle::default()
        })),
    })
    .unwrap();
    let view = ViewTransform {
        origin: [0.0, 0.0],
        scale: 1.0,
    };
    // In the stroke, 3 pixels left of the square.
    let stroke_px = |data: &[u8]| {
        let i = (140 * size.width as usize + 147) * 4;
        [data[i], data[i + 1], data[i + 2]]
    };
    // The first frame does not wait for the effects: computed meanwhile.
    let (first, stats) = r.render_view_progressive(s.document(), view, size).unwrap();
    assert!(stats.incomplete);
    assert_eq!(stroke_px(&first.data), [255, 255, 255]);
    let start = std::time::Instant::now();
    let last = loop {
        let (frame, stats) = r.render_view_progressive(s.document(), view, size).unwrap();
        if !stats.incomplete {
            break frame;
        }
        assert!(start.elapsed().as_secs() < 30, "effects never shown");
        std::thread::sleep(std::time::Duration::from_millis(2));
    };
    assert_eq!(stroke_px(&last.data), [0, 0, 0]);
    let expected = direct.render_view(s.document(), view, size).unwrap();
    assert_frames_match(&last.data, &expected.data, "effects drawn");
}

/// A document of the background `bg` and a half-transparent layer showing `painted`.
fn painted_document(bg: &Arc<RasterImage>, painted: &Arc<RasterImage>) -> Document {
    let size = painted.size();
    let mut s = Session::new(Document::new(size));
    let bg = raster(&mut s, Arc::clone(bg));
    push(&mut s, bg);
    let mut top = raster(&mut s, Arc::clone(painted));
    top.opacity = 0.5;
    push(&mut s, top);
    s.document().clone()
}

/// `image` with its level-0 tile (`col`, `row`) replaced by one filled with `value`: the next
/// frame of a stroke, sharing every other tile (ADR 0027).
fn stroked(image: &RasterImage, col: u32, row: u32, value: u8) -> Arc<RasterImage> {
    let tile = vec![value; RasterImage::tile_bytes(image.format())];
    let replaced = vec![(slopshop_core::tile::TileCoord { col, row }, Arc::from(tile))];
    Arc::new(image.with_tiles(replaced).unwrap())
}

#[test]
fn a_stroke_s_frames_recomposite_only_the_tiles_over_the_raster_tiles_they_change() {
    let (Some(r), Some(direct)) = (renderer(true), renderer(false)) else {
        return;
    };
    // A raster tile cache of 3 tiles forgets the tiles of dropped frames at once: only the display
    // cache keeps them from being mistaken for others.
    let r = r.with_tile_capacity(3);
    // 4 × 2 display tiles at 100 %, as many raster tiles in each layer.
    let size = Size::new(1024, 512);
    let view = ViewTransform {
        origin: [0.0, 0.0],
        scale: 1.0,
    };
    let bg = image(size, |x, y| [(x % 251) as u8, (y % 241) as u8, 90, 255]);
    let mut painted = image(size, |x, y| [(x ^ y) as u8, 40, 200, 255]);
    let first = painted_document(&bg, &painted);
    let stats = frame(&r, &first);
    assert_eq!((stats.tiles_composited, stats.tiles_reused), (8, 0));

    // Frames of a stroke: a new image per frame, the one before it dropped (its tiles, which
    // the display cache must keep from being taken for others, are freed otherwise).
    let mut previous = Vec::new();
    for step in 0..24u32 {
        // Twice the same tile in a row, then the next: freed tiles and new ones meet.
        let (col, row) = ((step / 2) % 4, (step / 8) % 2);
        let next = stroked(&painted, col, row, 10 + step as u8 * 9);
        previous.push(painted);
        painted = next;
        let document = painted_document(&bg, &painted);
        let stats = frame(&r, &document);
        assert_eq!(
            (stats.tiles_composited, stats.tiles_reused),
            (1, 7),
            "frame {step}"
        );
        let a = r.render_view(&document, view, size).unwrap();
        let b = direct.render_view(&document, view, size).unwrap();
        assert_frames_match(&a.data, &b.data, &format!("frame {step}"));
        // Only the first image stays: the stroke's other frames are gone.
        if previous.len() > 1 {
            previous.pop();
        }
    }

    // Undo: the first image again, its tiles still cached (their keys are the tiles').
    let again = painted_document(&bg, &previous.remove(0));
    let stats = frame(&r, &again);
    assert_eq!((stats.tiles_composited, stats.tiles_reused), (0, 8));
}

/// Timing of an adjustment's slider dragged over a document of `rasters` small layers above a
/// large one, at 4K and 100 % (run with `--ignored --nocapture`): the CPU time to prepare each
/// frame, the GPU's, and how much of the view each frame composites.
#[test]
#[ignore = "benchmark"]
fn bench_adjustment_drag() {
    let Some(r) = renderer(true) else { return };
    let curves = |tick: u8| Adjustment::Curves {
        rgb: slopshop_core::curve::Curve::new(&[[0, 0], [64, 70 + tick], [192, 200], [255, 255]])
            .unwrap(),
        red: slopshop_core::curve::Curve::IDENTITY,
        green: slopshop_core::curve::Curve::new(&[[0, 10], [255, 240]]).unwrap(),
        blue: slopshop_core::curve::Curve::IDENTITY,
    };
    let levels = |tick: u8| levels(0.6 + f32::from(tick) / 100.0);
    for (name, adjustment) in [
        ("Levels", &levels as &dyn Fn(u8) -> Adjustment),
        ("Curves", &curves),
    ] {
        for rasters in [1, 24] {
            let size = Size::new(6000, 4000);
            let mut s = Session::new(Document::new(size));
            let bg = image(size, |x, y| [(x % 251) as u8, (y % 241) as u8, 90, 255]);
            let bg = raster(&mut s, bg);
            push(&mut s, bg);
            for n in 1..rasters {
                let small = image(Size::new(600, 400), |x, y| {
                    [(x % 7 * 30) as u8, (y % 5 * 40) as u8, n as u8, 200]
                });
                let mut layer = raster(&mut s, small);
                layer.transform = slopshop_core::Projective::from(Affine::translation(
                    f64::from(n % 6 * 900),
                    f64::from(n / 6 * 900),
                ));
                push(&mut s, layer);
            }
            let top = layer(
                &mut s,
                LayerContent::Adjustment {
                    adjustment: adjustment(0),
                },
            );
            let id = push(&mut s, top);
            let view = ViewTransform {
                origin: [1000.0, 900.0],
                scale: 1.0,
            };
            let output = Size::new(3840, 2160);
            r.profile_view(s.document(), view, output, true).unwrap();
            let (mut prepare, mut gpu, mut composited, mut incomplete) =
                (Vec::new(), Vec::new(), 0, 0);
            let ticks = 20u8;
            for tick in 1..=ticks {
                s.perform_in_gesture(Edit::SetAdjustment {
                    id,
                    adjustment: adjustment(tick),
                })
                .unwrap();
                let stats = r.profile_view(s.document(), view, output, true).unwrap();
                prepare.push(stats.prepare.as_secs_f64() * 1000.0);
                gpu.push(stats.gpu.map_or(0.0, |d| d.as_secs_f64() * 1000.0));
                composited += stats.tiles_composited;
                incomplete += u32::from(stats.incomplete);
            }
            let median = |v: &mut Vec<f64>| {
                v.sort_by(f64::total_cmp);
                v[v.len() / 2]
            };
            println!(
                "{name}, {rasters} rasters, 4K: prepare {:.2} ms, GPU {:.2} ms (medians), {} tiles a frame, {incomplete}/{ticks} incomplete",
                median(&mut prepare),
                median(&mut gpu),
                composited / u32::from(ticks),
            );
        }
    }
}
