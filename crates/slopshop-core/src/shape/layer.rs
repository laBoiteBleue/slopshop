//! What a vector layer draws (ADR 0041), as a layer style draws its effects (ADR 0032): each of
//! its paints (the fill, the stroke) a fill layer masked by its coverage, placed in the
//! document. The compositor and the renderer draw them as they draw effects, so a shape needs
//! no path of its own there; the CPU and the GPU read the same coverage.
//!
//! The coverage is drawn at the document's pixels, where the layer is placed (first version:
//! zoomed beyond 100 % it shows its pixels, as a raster does), over the canvas only.

use std::sync::{Arc, OnceLock};

use kurbo::Shape as _;

use super::draw::{coverage, cut};
use super::model::{FillRule, Geometry, Paint, ShapeSource, StrokeAlign};
use super::path::{outline, placed, stroke_outline, tolerance_for};
use crate::blend::BlendMode;
use crate::document::Layer;
use crate::geom::{Rect, Size};
use crate::raster::TILE_SIZE;
use crate::transform::Projective;

/// A vector layer's paints, placed in the document.
#[derive(Debug)]
pub struct Drawn {
    /// Bottom to top: the fill, then the stroke; each a fill layer masked by its coverage.
    pub paints: Vec<Layer>,
    /// Where the layer was placed, on what canvas, when they were drawn.
    at: Projective,
    canvas: Size,
}

/// A vector layer's drawings: the place it was first drawn at, kept; another place (a style's
/// coverage drawn on a canvas of its own, Rasterize) drawn once each in a chain, so that a
/// drawing handed out is never replaced. A layer moved or changed gets a new one
/// (`Layer::move_styles`, `Layer::redraw_styles`).
#[derive(Debug, Default)]
pub struct Drawing {
    ready: OnceLock<Arc<Drawn>>,
    next: OnceLock<Box<Drawing>>,
}

impl Drawing {
    /// What `source` draws placed by `to_document` on a `canvas`: drawn now if it is not (on
    /// every core), then kept.
    pub fn drawn(&self, source: &ShapeSource, to_document: Projective, canvas: Size) -> &Drawn {
        let ready = self
            .ready
            .get_or_init(|| Arc::new(draw(source, to_document, canvas)));
        if ready.at == to_document && ready.canvas == canvas {
            return ready;
        }
        self.next
            .get_or_init(Box::default)
            .drawn(source, to_document, canvas)
    }
}

/// The area of the canvas `bounds` (document pixels) covers, its origin on the tile grid (so
/// that the coverage's pyramid lines up with the display's), `None` when it shows nowhere.
fn area_of(bounds: kurbo::Rect, canvas: Size) -> Option<Rect> {
    if !(bounds.x0.is_finite()
        && bounds.y0.is_finite()
        && bounds.x1.is_finite()
        && bounds.y1.is_finite())
    {
        return None;
    }
    let t = f64::from(TILE_SIZE);
    let (w, h) = (f64::from(canvas.width), f64::from(canvas.height));
    let x0 = ((bounds.x0 / t).floor() * t).clamp(0.0, w);
    let y0 = ((bounds.y0 / t).floor() * t).clamp(0.0, h);
    let x1 = bounds.x1.ceil().clamp(0.0, w);
    let y1 = bounds.y1.ceil().clamp(0.0, h);
    (x1 > x0 && y1 > y0)
        .then(|| Rect::new(x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32))
}

/// A fill layer of `paint`'s color masked by `coverage`, placed at `area`'s origin.
fn painted(paint: Paint, coverage: Arc<crate::raster::RasterImage>, area: Rect) -> Layer {
    let Paint::Solid(color) = paint;
    crate::style::colored(
        coverage,
        color,
        BlendMode::Normal,
        1.0,
        Projective::translation(f64::from(area.x), f64::from(area.y)),
    )
}

fn draw(source: &ShapeSource, to_document: Projective, canvas: Size) -> Drawn {
    let shape = source.shape();
    let mut drawn = Drawn {
        paints: Vec::new(),
        at: to_document,
        canvas,
    };
    let tolerance = tolerance_for(&shape.geometry, to_document);
    let own = outline(&shape.geometry, tolerance);
    let rule = match &shape.geometry {
        Geometry::Path { rule, .. } => *rule,
        _ => FillRule::NonZero,
    };
    let closed = shape.geometry.is_closed();
    let fill_path = placed(&own, to_document, tolerance);
    // The stroke's outline, twice as wide when it is cut to one side of the outline.
    let stroke = shape.stroke.as_ref().map(|s| {
        let widen = if s.align == StrokeAlign::Center || !closed {
            1.0
        } else {
            2.0
        };
        (
            s,
            placed(
                &stroke_outline(&own, s, widen, tolerance),
                to_document,
                tolerance,
            ),
        )
    });
    let mut bounds = fill_path.bounding_box();
    if let Some((_, path)) = &stroke {
        bounds = bounds.union(path.bounding_box());
    }
    let Some(area) = area_of(bounds, canvas) else {
        return drawn;
    };
    let needs_fill = closed
        && (shape.fill.is_some()
            || stroke
                .as_ref()
                .is_some_and(|(s, _)| s.align != StrokeAlign::Center));
    let fill = needs_fill
        .then(|| coverage(&fill_path, rule, area).ok())
        .flatten()
        .map(Arc::new);
    if let (Some(paint), Some(cover)) = (shape.fill, &fill) {
        drawn.paints.push(painted(paint, Arc::clone(cover), area));
    }
    if let Some((s, path)) = stroke
        && let Ok(band) = coverage(&path, FillRule::NonZero, area)
    {
        let band = match (s.align, &fill) {
            (StrokeAlign::Inside, Some(fill)) => cut(&band, fill, true).ok(),
            (StrokeAlign::Outside, Some(fill)) => cut(&band, fill, false).ok(),
            _ => Some(band),
        };
        if let Some(band) = band {
            drawn.paints.push(painted(s.paint, Arc::new(band), area));
        }
    }
    drawn
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::LinearRgba;
    use crate::document::LayerContent;
    use crate::shape::model::{Shape, ShapeStroke, StrokeCap, StrokeJoin};

    fn square(stroke: Option<ShapeStroke>) -> Arc<ShapeSource> {
        ShapeSource::new(
            Shape {
                geometry: Geometry::Rectangle {
                    rect: [10.0, 10.0, 110.0, 60.0],
                    radii: [0.0; 4],
                },
                fill: Some(Paint::Solid(LinearRgba::new(1.0, 0.0, 0.0, 1.0))),
                stroke,
            },
            "Rectangle 1",
        )
    }

    #[test]
    fn a_shape_draws_its_fill_and_stroke_as_masked_fills_where_it_is_placed() {
        let stroke = ShapeStroke {
            paint: Paint::Solid(LinearRgba::new(0.0, 0.0, 1.0, 1.0)),
            width: 4.0,
            align: StrokeAlign::Outside,
            cap: StrokeCap::Butt,
            join: StrokeJoin::Miter,
            miter_limit: 4.0,
            dashes: Vec::new(),
            dash_offset: 0.0,
        };
        let source = square(Some(stroke));
        let drawing = Drawing::default();
        let at: Projective = crate::transform::Affine::translation(300.0, 20.0).into();
        let canvas = Size::new(1000, 500);
        let drawn = drawing.drawn(&source, at, canvas);
        assert_eq!(drawn.paints.len(), 2);
        // Fills with their coverage as masks, on the tile grid (the shape spans 306 to 414).
        for paint in &drawn.paints {
            assert!(matches!(paint.content, LayerContent::Fill { .. }));
            assert_eq!(paint.transform, Projective::translation(256.0, 0.0));
            assert!(paint.mask.is_some());
        }
        // The same place: the same drawing; another canvas: drawn anew, the first kept.
        assert!(std::ptr::eq(drawn, drawing.drawn(&source, at, canvas)));
        let other = drawing.drawn(&source, at, Size::new(400, 400));
        assert!(!std::ptr::eq(drawn, other));
        assert!(std::ptr::eq(drawn, drawing.drawn(&source, at, canvas)));
        // Off the canvas: nothing to draw.
        let away: Projective = crate::transform::Affine::translation(5000.0, 0.0).into();
        assert!(
            Drawing::default()
                .drawn(&source, away, canvas)
                .paints
                .is_empty()
        );
    }

    use crate::blend::BlendSpace;
    use crate::composite::composite_region;
    use crate::document::{Document, LayerId};
    use crate::edit::Edit;

    const RED: Paint = Paint::Solid(LinearRgba::new(1.0, 0.0, 0.0, 1.0));
    const BLUE: Paint = Paint::Solid(LinearRgba::new(0.0, 0.0, 1.0, 1.0));

    fn rectangle(rect: [f64; 4], fill: Option<Paint>, stroke: Option<ShapeStroke>) -> Shape {
        Shape {
            geometry: Geometry::Rectangle {
                rect,
                radii: [0.0; 4],
            },
            fill,
            stroke,
        }
    }

    fn centered(paint: Paint, width: f64) -> ShapeStroke {
        ShapeStroke {
            paint,
            width,
            align: StrokeAlign::Center,
            cap: StrokeCap::Butt,
            join: StrokeJoin::Miter,
            miter_limit: 4.0,
            dashes: Vec::new(),
            dash_offset: 0.0,
        }
    }

    /// A 64 × 64 document blending in linear light (normal mode is plain "over").
    fn document() -> Document {
        let mut doc = Document::new(Size::new(64, 64));
        Edit::SetBlendSpace {
            space: BlendSpace::Linear,
        }
        .apply(&mut doc)
        .unwrap();
        doc
    }

    fn add_vector(doc: &mut Document, source: Arc<ShapeSource>, transform: Projective) -> LayerId {
        let id = doc.allocate_layer_id();
        let index = doc.layers().len();
        Edit::InsertLayer {
            parent: None,
            index,
            layer: Layer {
                id,
                name: source.name().to_owned(),
                visible: true,
                opacity: 1.0,
                blend_mode: BlendMode::Normal,
                content: LayerContent::vector(source),
                mask: None,
                clipped: false,
                transform,
                style: None,
            },
        }
        .apply(doc)
        .unwrap();
        id
    }

    /// The composite at (`x`, `y`): premultiplied working-space RGBA.
    fn at(doc: &Document, x: u32, y: u32) -> [f32; 4] {
        let mut px = [0.0f32; 4];
        composite_region(doc, Rect::new(x, y, 1, 1), &mut px).unwrap();
        px
    }

    fn close(actual: [f32; 4], expected: [f32; 4]) -> bool {
        // Coverage is stored in 8 bits.
        actual
            .iter()
            .zip(expected)
            .all(|(a, e)| (a - e).abs() < 0.01)
    }

    #[test]
    fn a_vector_layer_composites_as_its_shape_where_it_is_placed() {
        let mut doc = document();
        let source = ShapeSource::new(rectangle([8.0, 8.0, 40.0, 24.0], Some(RED), None), "");
        let at_half: Projective = crate::transform::Affine::translation(4.5, 4.0).into();
        add_vector(&mut doc, source, at_half);
        // Placed at 12.5 to 44.5 across, 12 to 28 down.
        assert!(close(at(&doc, 20, 20), [1.0, 0.0, 0.0, 1.0]));
        assert!(close(at(&doc, 2, 2), [0.0; 4]));
        assert!(close(at(&doc, 20, 28), [0.0; 4]));
        // Half pixels at its sides: anti-aliased.
        assert!(
            close(at(&doc, 12, 20), [0.5, 0.0, 0.0, 0.5]),
            "{:?}",
            at(&doc, 12, 20)
        );
        assert!(
            close(at(&doc, 44, 20), [0.5, 0.0, 0.0, 0.5]),
            "{:?}",
            at(&doc, 44, 20)
        );
    }

    #[test]
    fn a_shapes_fill_and_stroke_blend_as_one_layer() {
        let mut doc = document();
        let source = ShapeSource::new(
            rectangle([0.0, 0.0, 32.0, 32.0], Some(RED), Some(centered(BLUE, 4.0))),
            "",
        );
        let id = add_vector(&mut doc, source, Projective::IDENTITY);
        Edit::SetLayerOpacity { id, opacity: 0.5 }
            .apply(&mut doc)
            .unwrap();
        // The stroke covers the fill, then the whole is at half opacity: drawn one after the
        // other, the fill would show through the stroke (alpha 0.75).
        assert!(
            close(at(&doc, 0, 10), [0.0, 0.0, 0.5, 0.5]),
            "{:?}",
            at(&doc, 0, 10)
        );
        assert!(close(at(&doc, 16, 16), [0.5, 0.0, 0.0, 0.5]));
        // Outside the shape, half of the stroke.
        assert!(close(at(&doc, 33, 10), [0.0, 0.0, 0.5, 0.5]));
        assert!(close(at(&doc, 34, 10), [0.0; 4]));
    }

    #[test]
    fn a_vector_layer_is_drawn_again_when_its_shape_changes_or_it_moves() {
        let mut doc = document();
        let source = ShapeSource::new(rectangle([0.0, 0.0, 16.0, 16.0], Some(RED), None), "");
        let id = add_vector(&mut doc, source, Projective::IDENTITY);
        assert!(close(at(&doc, 8, 8), [1.0, 0.0, 0.0, 1.0]));
        let reshaped = ShapeSource::new(rectangle([0.0, 0.0, 16.0, 16.0], Some(BLUE), None), "");
        let undo = Edit::SetShape {
            id,
            source: reshaped,
        }
        .apply(&mut doc)
        .unwrap();
        assert!(close(at(&doc, 8, 8), [0.0, 0.0, 1.0, 1.0]));
        undo.apply(&mut doc).unwrap();
        assert!(close(at(&doc, 8, 8), [1.0, 0.0, 0.0, 1.0]));
        // Moved: drawn where it now is.
        Edit::SetLayerTransform {
            id,
            transform: crate::transform::Affine::translation(30.0, 30.0).into(),
        }
        .apply(&mut doc)
        .unwrap();
        assert!(close(at(&doc, 8, 8), [0.0; 4]));
        assert!(close(at(&doc, 38, 38), [1.0, 0.0, 0.0, 1.0]));
        // A shape that is not valid: refused.
        let bad = ShapeSource::new(rectangle([10.0, 0.0, 0.0, 16.0], Some(RED), None), "");
        assert_eq!(
            Edit::SetShape { id, source: bad }.apply(&mut doc).err(),
            Some(crate::edit::EditError::InvalidShape)
        );
    }

    #[test]
    fn layers_sharing_a_shape_are_made_unique_undoably() {
        let mut doc = document();
        let source = ShapeSource::new(rectangle([0.0, 0.0, 8.0, 8.0], Some(RED), None), "Box");
        let a = add_vector(&mut doc, Arc::clone(&source), Projective::IDENTITY);
        let b = add_vector(&mut doc, source, Projective::IDENTITY);
        let shape_of = |doc: &Document, id| match &doc.layer(id).unwrap().content {
            LayerContent::Vector { source, .. } => Arc::clone(source),
            _ => unreachable!("a vector layer"),
        };
        let undo = Edit::make_unique(&doc, &[b])
            .unwrap()
            .apply(&mut doc)
            .unwrap();
        assert_ne!(shape_of(&doc, a).id(), shape_of(&doc, b).id());
        assert_eq!(shape_of(&doc, a).shape(), shape_of(&doc, b).shape());
        assert_eq!(shape_of(&doc, b).name(), "Box");
        // Alone with its shape: nothing to make unique.
        assert_eq!(
            Edit::make_unique(&doc, &[b]).err(),
            Some(crate::edit::EditError::NoLayers)
        );
        undo.apply(&mut doc).unwrap();
        assert_eq!(shape_of(&doc, a).id(), shape_of(&doc, b).id());
    }

    #[test]
    fn a_vector_layer_is_picked_and_snapped_to_by_its_outline() {
        let mut doc = document();
        let source = ShapeSource::new(
            rectangle([8.0, 8.0, 24.0, 24.0], None, Some(centered(BLUE, 2.0))),
            "",
        );
        let id = add_vector(&mut doc, source, Projective::IDENTITY);
        // On its stroke, not inside it (it has no fill).
        assert_eq!(crate::pick::layer_at(&doc, 8, 16), Some(id));
        assert_eq!(crate::pick::layer_at(&doc, 16, 16), None);
        let bounds = crate::pick::visible_layer_bounds(&doc);
        assert_eq!(bounds.len(), 1);
        let b = bounds[0].1;
        assert_eq!((b.left, b.top, b.right, b.bottom), (7, 7, 25, 25));
    }

    #[test]
    fn a_styled_vector_layer_has_its_effects_around_its_shape() {
        let mut doc = document();
        let source = ShapeSource::new(rectangle([8.0, 8.0, 40.0, 24.0], Some(RED), None), "");
        let id = add_vector(&mut doc, source, Projective::IDENTITY);
        let style = crate::style::LayerStyle {
            stroke: Some(crate::style::Stroke {
                size: 2.0,
                color: LinearRgba::new(0.0, 1.0, 0.0, 1.0),
                ..Default::default()
            }),
            ..Default::default()
        };
        Edit::SetLayerStyle {
            id,
            style: Some(Box::new(style)),
        }
        .apply(&mut doc)
        .unwrap();
        assert!(
            close(at(&doc, 7, 16), [0.0, 1.0, 0.0, 1.0]),
            "{:?}",
            at(&doc, 7, 16)
        );
        assert!(close(at(&doc, 20, 16), [1.0, 0.0, 0.0, 1.0]));
        assert!(close(at(&doc, 4, 16), [0.0; 4]));
    }

    #[test]
    fn a_layer_with_an_invalid_shape_is_not_inserted_nor_restored() {
        let mut doc = document();
        let bad = ShapeSource::new(rectangle([10.0, 0.0, 0.0, 16.0], Some(RED), None), "");
        let id = doc.allocate_layer_id();
        let layer = Layer {
            id,
            name: "bad".into(),
            visible: true,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            content: LayerContent::vector(bad),
            mask: None,
            clipped: false,
            transform: Projective::IDENTITY,
            style: None,
        };
        assert_eq!(
            Edit::InsertLayer {
                parent: None,
                index: 0,
                layer: layer.clone(),
            }
            .apply(&mut doc)
            .err(),
            Some(crate::edit::EditError::InvalidShape)
        );
        assert!(matches!(
            Document::restore(
                Size::new(64, 64),
                crate::color::WORKING_SPACE,
                BlendSpace::Linear,
                vec![layer],
                id.get() + 1,
            ),
            Err(crate::document::RestoreError::InvalidShape(_))
        ));
    }
}
