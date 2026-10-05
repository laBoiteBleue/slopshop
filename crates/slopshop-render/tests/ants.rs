//! The marching ants drawn by the GPU over the frame (ADR 0024). Skipped when no adapter is
//! available, unless `SLOPSHOP_REQUIRE_GPU=1`, or when `SLOPSHOP_SKIP_GPU_TESTS=1`.

use std::collections::HashSet;
use std::sync::Arc;

use slopshop_core::selection::{self, Combine, EdgeOptions, Selection, Shape};
use slopshop_core::view::ViewTransform;
use slopshop_core::{
    Affine, BlendMode, Document, Edit, Layer, LayerContent, LinearRgba, Session, Size,
};
use slopshop_render::{Ants, Renderer, SelectionView, ViewOverlays};

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

/// The canvas of most tests: several tiles wide.
const CANVAS: Size = Size::new(600, 300);

/// What the frame shows without ants: a mid-gray fill (neither black nor white).
const FILL: [u8; 4] = [188, 188, 188, 255];

fn document(size: Size) -> Session {
    let mut session = Session::new(Document::new(size));
    let id = session.allocate_layer_id();
    session
        .perform(Edit::InsertLayer {
            parent: None,
            index: 0,
            layer: Layer {
                style: None,
                transform: Affine::IDENTITY.into(),
                clipped: false,
                id,
                name: "fill".into(),
                visible: true,
                opacity: 1.0,
                blend_mode: BlendMode::Normal,
                mask: None,
                content: LayerContent::Fill {
                    color: LinearRgba::new(0.5, 0.5, 0.5, 1.0),
                },
            },
        })
        .unwrap();
    session
}

fn select(session: &mut Session, shape: Shape) {
    let image = selection::select_shape(
        session.document().size(),
        None,
        &shape,
        EdgeOptions::default(),
        Combine::Replace,
    )
    .unwrap()
    .unwrap();
    session
        .perform(Edit::SetSelection {
            selection: Selection::new(Arc::new(image)),
        })
        .unwrap();
}

fn rectangle(left: f64, top: f64, right: f64, bottom: f64) -> Shape {
    Shape::Rectangle {
        left,
        top,
        right,
        bottom,
    }
}

/// A frame as rows of RGBA pixels.
struct Pixels {
    width: u32,
    data: Vec<u8>,
}

impl Pixels {
    fn at(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        self.data[i..i + 4].try_into().unwrap()
    }
}

fn render(
    r: &Renderer,
    doc: &Document,
    view: ViewTransform,
    output: Size,
    ants: Option<Ants>,
) -> Pixels {
    render_with(
        r,
        doc,
        view,
        output,
        ViewOverlays {
            ants,
            ..ViewOverlays::default()
        },
    )
}

fn render_with(
    r: &Renderer,
    doc: &Document,
    view: ViewTransform,
    output: Size,
    overlays: ViewOverlays,
) -> Pixels {
    let mut data = Vec::new();
    r.render_view_into(doc, view, overlays, output, &mut data)
        .unwrap();
    Pixels {
        width: output.width,
        data,
    }
}

fn at_scale(scale: f64, origin: [f64; 2]) -> ViewTransform {
    ViewTransform { origin, scale }
}

fn marching(phase: u32) -> Option<Ants> {
    Some(Ants {
        phase,
        transform: Affine::IDENTITY,
    })
}

/// The pixels where `with` differs from `without`: the ants. Each is black or white.
fn drawn(with: &Pixels, without: &Pixels) -> Vec<(u32, u32, bool)> {
    assert_eq!(with.data.len(), without.data.len());
    let mut out = Vec::new();
    for (i, (a, b)) in with
        .data
        .as_chunks::<4>()
        .0
        .iter()
        .zip(without.data.as_chunks::<4>().0)
        .enumerate()
    {
        if a != b {
            let (x, y) = (i as u32 % with.width, i as u32 / with.width);
            assert!(
                *a == [0, 0, 0, 255] || *a == [255, 255, 255, 255],
                "an ant at ({x}, {y}) is black or white, not {a:?}"
            );
            out.push((x, y, a[0] == 0));
        }
    }
    out
}

/// Whether `(x, y)` of the frame is dark at `phase`.
fn dark(x: u32, y: u32, phase: u32) -> bool {
    ((x + y + phase) / 4).is_multiple_of(2)
}

/// Both display paths: through the display cache and compositing every layer for every pixel.
fn each_path(test: impl Fn(&Renderer)) {
    for cached in [true, false] {
        let Some(r) = renderer() else { return };
        test(&r.with_display_cache(cached));
    }
}

#[test]
fn a_rectangle_is_outlined_inside_its_edge_in_dashes() {
    each_path(|r| {
        let mut s = document(CANVAS);
        select(&mut s, rectangle(100.0, 50.0, 300.0, 200.0));
        let bare = render(r, s.document(), at_scale(1.0, [0.0; 2]), CANVAS, None);
        assert_eq!(bare.at(100, 50), FILL, "no ants asked, none drawn");

        let ants = render(
            r,
            s.document(),
            at_scale(1.0, [0.0; 2]),
            CANVAS,
            marching(0),
        );
        let pixels = drawn(&ants, &bare);
        // The ring of the selected pixels along the boundary: nothing inside, nothing outside.
        let ring: HashSet<(u32, u32)> = (100..300)
            .flat_map(|x| [(x, 50), (x, 199)])
            .chain((50..200).flat_map(|y| [(100, y), (299, y)]))
            .collect();
        let found: HashSet<(u32, u32)> = pixels.iter().map(|&(x, y, _)| (x, y)).collect();
        assert_eq!(found, ring);
        // Dashes of four, dark then light, along the diagonal.
        for &(x, y, is_dark) in &pixels {
            assert_eq!(is_dark, dark(x, y, 0), "dash at ({x}, {y})");
        }
        assert!(pixels.iter().any(|p| p.2) && pixels.iter().any(|p| !p.2));
        // Along the top edge: four dark, four light (the sum x + y = 152 starts a dash).
        let top: Vec<bool> = (102..118).map(|x| ants.at(x, 50)[0] == 0).collect();
        assert_eq!(
            top,
            [vec![true; 4], vec![false; 4], vec![true; 4], vec![false; 4]].concat()
        );
    });
}

#[test]
fn the_canvas_edge_is_not_an_outline() {
    each_path(|r| {
        let mut s = document(CANVAS);
        // Touching the left, top and bottom edges; the right edge is inside the canvas.
        select(&mut s, rectangle(0.0, 0.0, 250.0, 300.0));
        let view = at_scale(1.0, [0.0; 2]);
        let bare = render(r, s.document(), view, CANVAS, None);
        let found: HashSet<(u32, u32)> =
            drawn(&render(r, s.document(), view, CANVAS, marching(0)), &bare)
                .iter()
                .map(|&(x, y, _)| (x, y))
                .collect();
        assert_eq!(found, (0..300).map(|y| (249, y)).collect());

        // Everything selected: no outline at all, even though the pasteboard is beyond.
        let all = selection::select_all(CANVAS).unwrap();
        s.perform(Edit::SetSelection {
            selection: Selection::new(Arc::new(all)),
        })
        .unwrap();
        let shifted = at_scale(1.0, [-20.0, -10.0]);
        let bare = render(r, s.document(), shifted, CANVAS, None);
        let ants = render(r, s.document(), shifted, CANVAS, marching(0));
        assert!(drawn(&ants, &bare).is_empty());
    });
}

#[test]
fn an_ellipse_is_outlined_along_its_curve() {
    each_path(|r| {
        let mut s = document(CANVAS);
        select(
            &mut s,
            Shape::Ellipse {
                left: 100.0,
                top: 50.0,
                right: 500.0,
                bottom: 250.0,
            },
        );
        let view = at_scale(1.0, [0.0; 2]);
        let bare = render(r, s.document(), view, CANVAS, None);
        let pixels = drawn(&render(r, s.document(), view, CANVAS, marching(0)), &bare);
        // About the curve's length: Ramanujan's perimeter of 200 × 100 semi-axes is about 970;
        // staircases of one pixel steps come to a little more or less.
        assert!((800..1400).contains(&pixels.len()), "{} ants", pixels.len());
        for &(x, y, is_dark) in &pixels {
            // On the ellipse: its normalized radius is 1 within the pixel's width.
            let (dx, dy) = (
                (f64::from(x) + 0.5 - 300.0) / 200.0,
                (f64::from(y) + 0.5 - 150.0) / 100.0,
            );
            let radius = (dx * dx + dy * dy).sqrt();
            assert!(
                (0.97..=1.0).contains(&radius),
                "an ant at ({x}, {y}) is on the curve, not at {radius}"
            );
            assert_eq!(is_dark, dark(x, y, 0));
        }
        // Nothing at its center nor far outside, and the canvas's edge is no outline.
        assert_eq!(bare.at(300, 150), FILL);
        assert!(pixels.iter().all(|&(x, y, _)| x != 0 && y != 0));
    });
}

#[test]
fn the_phase_moves_the_dashes() {
    each_path(|r| {
        let mut s = document(CANVAS);
        select(&mut s, rectangle(100.0, 50.0, 300.0, 200.0));
        let view = at_scale(1.0, [0.0; 2]);
        let bare = render(r, s.document(), view, CANVAS, None);
        let at = |phase| {
            drawn(
                &render(r, s.document(), view, CANVAS, marching(phase)),
                &bare,
            )
        };
        let first = at(0);
        // Four on: dark and light swap; eight on (a whole period): the same; the same pixels
        // are lit all along.
        let swapped = at(4);
        assert_eq!(at(8), first);
        assert_eq!(first.len(), swapped.len());
        for (a, b) in first.iter().zip(&swapped) {
            assert_eq!((a.0, a.1), (b.0, b.1));
            assert_ne!(a.2, b.2);
        }
        // One step: the pattern moved by one pixel.
        let one = at(1);
        for &(x, y, is_dark) in &one {
            assert_eq!(is_dark, dark(x, y, 1));
        }
        assert_ne!(one, first);
    });
}

#[test]
fn a_shift_moves_the_outline_without_changing_the_selection() {
    each_path(|r| {
        let mut s = document(CANVAS);
        select(&mut s, rectangle(100.0, 50.0, 300.0, 200.0));
        let view = at_scale(1.0, [0.0; 2]);
        let bare = render(r, s.document(), view, CANVAS, None);
        let moved = Some(Ants {
            phase: 0,
            transform: Affine::translation(30.0, -20.0),
        });
        let found: HashSet<(u32, u32)> =
            drawn(&render(r, s.document(), view, CANVAS, moved), &bare)
                .iter()
                .map(|&(x, y, _)| (x, y))
                .collect();
        let ring: HashSet<(u32, u32)> = (130..330)
            .flat_map(|x| [(x, 30), (x, 179)])
            .chain((30..180).flat_map(|y| [(130, y), (329, y)]))
            .collect();
        assert_eq!(found, ring);
        // A shift off the canvas: nothing.
        let away = Some(Ants {
            phase: 0,
            transform: Affine::translation(1000.0, 0.0),
        });
        let nothing = render(r, s.document(), view, CANVAS, away);
        assert!(drawn(&nothing, &bare).is_empty());
    });
}

#[test]
fn a_transform_in_progress_outlines_the_transformed_selection() {
    each_path(|r| {
        let mut s = document(CANVAS);
        select(&mut s, rectangle(100.0, 50.0, 300.0, 200.0));
        let view = at_scale(1.0, [0.0; 2]);
        let bare = render(r, s.document(), view, CANVAS, None);
        // Half the size about the origin: the rectangle is (50, 25) to (150, 100).
        let scaled = Some(Ants {
            phase: 0,
            transform: Affine::scale(0.5, 0.5),
        });
        let pixels = drawn(&render(r, s.document(), view, CANVAS, scaled), &bare);
        assert!(pixels.len() > 300, "{} ants", pixels.len());
        for &(x, y, _) in &pixels {
            let (x, y) = (i64::from(x), i64::from(y));
            let on_edge = ((49..=51).contains(&x) || (148..=150).contains(&x))
                && (24..=101).contains(&y)
                || ((24..=26).contains(&y) || (98..=101).contains(&y)) && (49..=150).contains(&x);
            assert!(
                on_edge,
                "an ant at ({x}, {y}) is on the half-size rectangle"
            );
        }
        // Moved by half a pixel: still an outline of the same extent.
        let half = Some(Ants {
            phase: 0,
            transform: Affine::translation(0.5, 0.5),
        });
        let pixels = drawn(&render(r, s.document(), view, CANVAS, half), &bare);
        assert!(pixels.len() > 600, "{} ants", pixels.len());
        assert!(
            pixels
                .iter()
                .all(|&(x, y, _)| (99..=301).contains(&x) && (49..=201).contains(&y))
        );
    });
}

#[test]
fn zoomed_out_the_outline_is_one_pixel_of_the_frame() {
    each_path(|r| {
        let mut s = document(CANVAS);
        select(&mut s, rectangle(100.0, 50.0, 300.0, 200.0));
        // 50 %: the rectangle is (50, 25) to (150, 100) in frame pixels.
        let view = at_scale(2.0, [0.0; 2]);
        let output = Size::new(300, 150);
        let bare = render(r, s.document(), view, output, None);
        let found: HashSet<(u32, u32)> =
            drawn(&render(r, s.document(), view, output, marching(0)), &bare)
                .iter()
                .map(|&(x, y, _)| (x, y))
                .collect();
        let ring: HashSet<(u32, u32)> = (50..150)
            .flat_map(|x| [(x, 25), (x, 99)])
            .chain((25..100).flat_map(|y| [(50, y), (149, y)]))
            .collect();
        assert_eq!(found, ring);
        // 25 %, from the pyramid's level 2 (its texels are 4 pixels wide, so the edge is aligned).
        select(&mut s, rectangle(100.0, 48.0, 300.0, 200.0));
        let view = at_scale(4.0, [0.0; 2]);
        let output = Size::new(150, 75);
        let bare = render(r, s.document(), view, output, None);
        let found: HashSet<(u32, u32)> =
            drawn(&render(r, s.document(), view, output, marching(0)), &bare)
                .iter()
                .map(|&(x, y, _)| (x, y))
                .collect();
        let ring: HashSet<(u32, u32)> = (25..75)
            .flat_map(|x| [(x, 12), (x, 49)])
            .chain((12..50).flat_map(|y| [(25, y), (74, y)]))
            .collect();
        assert_eq!(found, ring);
    });
}

#[test]
fn zoomed_in_the_outline_stays_one_pixel_wide() {
    each_path(|r| {
        let mut s = document(CANVAS);
        select(&mut s, rectangle(100.0, 50.0, 300.0, 200.0));
        // 200 %, from (90, 40): the rectangle's left edge is at frame x = 20, its top at y = 20.
        let view = at_scale(0.5, [90.0, 40.0]);
        let output = Size::new(200, 100);
        let bare = render(r, s.document(), view, output, None);
        let ants = render(r, s.document(), view, output, marching(0));
        let found: HashSet<(u32, u32)> = drawn(&ants, &bare)
            .iter()
            .map(|&(x, y, _)| (x, y))
            .collect();
        let ring: HashSet<(u32, u32)> = (20..200)
            .map(|x| (x, 20))
            .chain((20..100).map(|y| (20, y)))
            .collect();
        assert_eq!(found, ring);
    });
}

#[test]
fn nothing_is_drawn_without_a_selection_or_where_another_view_shows_it() {
    each_path(|r| {
        let mut s = document(CANVAS);
        let view = at_scale(1.0, [0.0; 2]);
        let ants = ViewOverlays {
            ants: marching(0),
            ..ViewOverlays::default()
        };
        let bare = render(r, s.document(), view, CANVAS, None);
        let none = render_with(r, s.document(), view, CANVAS, ants);
        assert!(drawn(&none, &bare).is_empty(), "nothing selected");

        select(&mut s, rectangle(100.0, 50.0, 300.0, 200.0));
        let shown = render_with(r, s.document(), view, CANVAS, ants);
        assert!(!drawn(&shown, &bare).is_empty());
        // Select and Mask's views show the selection themselves.
        for selection_view in [SelectionView::Overlay, SelectionView::OnBlack] {
            let overlays = ViewOverlays {
                selection_view,
                ..ants
            };
            let other = render_with(r, s.document(), view, CANVAS, overlays);
            let without = render_with(
                r,
                s.document(),
                view,
                CANVAS,
                ViewOverlays {
                    ants: None,
                    ..overlays
                },
            );
            assert_eq!(other.data, without.data, "{selection_view:?}");
        }
        // So does Quick Mask.
        let enter = Edit::enter_quick_mask(s.document()).unwrap().unwrap();
        s.perform(enter).unwrap();
        let masked = render_with(r, s.document(), view, CANVAS, ants);
        let plain = render(r, s.document(), view, CANVAS, None);
        assert_eq!(masked.data, plain.data);
    });
}

#[test]
fn a_selection_moved_by_the_document_is_outlined_where_it_is() {
    each_path(|r| {
        let mut s = document(CANVAS);
        select(&mut s, rectangle(100.0, 50.0, 300.0, 200.0));
        select(&mut s, rectangle(400.0, 100.0, 450.0, 120.0));
        let view = at_scale(1.0, [0.0; 2]);
        let bare = render(r, s.document(), view, CANVAS, None);
        let found: HashSet<(u32, u32)> =
            drawn(&render(r, s.document(), view, CANVAS, marching(0)), &bare)
                .iter()
                .map(|&(x, y, _)| (x, y))
                .collect();
        assert!(
            found
                .iter()
                .all(|&(x, y)| (400..450).contains(&x) && (100..120).contains(&y))
        );
        assert_eq!(found.len(), 2 * 50 + 2 * 20 - 4);
    });
}

#[test]
fn ants_phase_follows_the_clock() {
    use std::time::Duration;
    let identity = Affine::IDENTITY;
    assert_eq!(Ants::at(Duration::ZERO, identity).phase, 0);
    assert_eq!(Ants::at(Ants::STEP, identity).phase, 1);
    assert_eq!(
        Ants::at(Ants::STEP * 3 + Duration::from_millis(10), identity).phase,
        3
    );
    // A whole period (8 steps, 0.6 s) is back at the start.
    assert_eq!(Ants::at(Ants::STEP * 8, identity).phase, 0);
    assert_eq!(
        Ants::at(Ants::STEP * 8 * 1000 + Ants::STEP * 2, identity).phase,
        2
    );
    assert_eq!(Ants::still(identity).phase, 0);
}
