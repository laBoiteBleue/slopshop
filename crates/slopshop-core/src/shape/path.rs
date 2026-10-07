//! A shape's outlines as Béziers (`kurbo`): its geometry, its stroke's, placed by a layer's
//! transform.

use kurbo::{
    BezPath, Cap, Ellipse, Join, PathEl, Point, RoundedRect, RoundedRectRadii, Shape as _, Stroke,
    StrokeOpts,
};

use super::model::{Geometry, Segment, ShapeStroke, StrokeCap, StrokeJoin};
use crate::transform::Projective;

/// How far a flattened curve strays from the true one, in pixels of where it is drawn.
pub const TOLERANCE: f64 = 0.1;

/// The outline of `geometry` in its layer's space, curves within `tolerance`.
pub fn outline(geometry: &Geometry, tolerance: f64) -> BezPath {
    match geometry {
        &Geometry::Rectangle { rect, radii } => {
            let [l, t, r, b] = rect;
            // No corner rounds past half the side.
            let most = ((r - l) / 2.0).min((b - t) / 2.0).max(0.0);
            let [tl, tr, br, bl] = radii.map(|v| v.min(most));
            RoundedRect::new(l, t, r, b, RoundedRectRadii::new(tl, tr, br, bl)).to_path(tolerance)
        }
        &Geometry::Ellipse { center, radii } => {
            Ellipse::new((center[0], center[1]), (radii[0], radii[1]), 0.0).to_path(tolerance)
        }
        &Geometry::Polygon {
            center,
            radius,
            sides,
            star,
            rotation,
        } => {
            let corners = if star.is_some() { 2 * sides } else { sides };
            let step = 360.0 / f64::from(corners);
            let mut path = BezPath::new();
            for i in 0..corners {
                let r = match star {
                    Some(inner) if i % 2 == 1 => radius * inner,
                    _ => radius,
                };
                let (sin, cos) = (rotation + step * f64::from(i)).to_radians().sin_cos();
                // Up is negative y.
                let p = Point::new(center[0] + r * cos, center[1] - r * sin);
                if i == 0 {
                    path.move_to(p);
                } else {
                    path.line_to(p);
                }
            }
            path.close_path();
            path
        }
        &Geometry::Line { from, to } => {
            let mut path = BezPath::new();
            path.move_to((from[0], from[1]));
            path.line_to((to[0], to[1]));
            path
        }
        Geometry::Path { subpaths, .. } => {
            let mut path = BezPath::new();
            for subpath in subpaths {
                path.move_to((subpath.start[0], subpath.start[1]));
                for segment in &subpath.segments {
                    match *segment {
                        Segment::Line(p) => path.line_to((p[0], p[1])),
                        Segment::Cubic(a, b, p) => {
                            path.curve_to((a[0], a[1]), (b[0], b[1]), (p[0], p[1]))
                        }
                    }
                }
                if subpath.closed {
                    path.close_path();
                }
            }
            path
        }
    }
}

/// The outline of `stroke` along `path`, `widen` times as wide (an inside or outside stroke is
/// drawn twice as wide, then cut by the shape: see `draw`).
pub fn stroke_outline(path: &BezPath, stroke: &ShapeStroke, widen: f64, tolerance: f64) -> BezPath {
    let width = stroke.width * widen;
    let style = Stroke::new(width)
        .with_caps(match stroke.cap {
            StrokeCap::Butt => Cap::Butt,
            StrokeCap::Round => Cap::Round,
            StrokeCap::Square => Cap::Square,
        })
        .with_join(match stroke.join {
            StrokeJoin::Miter => Join::Miter,
            StrokeJoin::Round => Join::Round,
            StrokeJoin::Bevel => Join::Bevel,
        })
        .with_miter_limit(stroke.miter_limit)
        // Dashes in stroke widths, as Photoshop's.
        .with_dashes(
            stroke.dash_offset * stroke.width,
            stroke.dashes.iter().map(|d| d * stroke.width),
        );
    kurbo::stroke(path.iter(), &style, &StrokeOpts::default(), tolerance)
}

/// The largest factor `transform` scales lengths by around the point `at` (a projective map's
/// scale varies): how fine to flatten curves before placing them.
fn scale_at(transform: Projective, at: Point) -> f64 {
    let (x0, y0) = transform.apply(at.x, at.y);
    let (x1, y1) = transform.apply(at.x + 1.0, at.y);
    let (x2, y2) = transform.apply(at.x, at.y + 1.0);
    let dx = (x1 - x0).hypot(y1 - y0);
    let dy = (x2 - x0).hypot(y2 - y0);
    dx.max(dy).max(1e-9)
}

/// The tolerance to flatten `geometry` with in its own space, so that once placed by
/// `transform` it is [`TOLERANCE`] pixels.
pub fn tolerance_for(geometry: &Geometry, transform: Projective) -> f64 {
    let bounds = outline(geometry, 1.0).bounding_box();
    let center = bounds.center();
    TOLERANCE / scale_at(transform, center)
}

/// `path` placed by `transform`: exactly when the map is affine; flattened then mapped point by
/// point when it is projective (straight lines stay straight under a homography, ADR 0038).
pub fn placed(path: &BezPath, transform: Projective, tolerance: f64) -> BezPath {
    if let Some(t) = transform.as_affine() {
        return kurbo::Affine::new([t.a, t.b, t.c, t.d, t.e, t.f]) * path.clone();
    }
    let map = |p: Point| {
        let (x, y) = transform.apply(p.x, p.y);
        Point::new(x, y)
    };
    let mut out = BezPath::new();
    kurbo::flatten(path.iter(), tolerance, |el| match el {
        PathEl::MoveTo(p) => out.move_to(map(p)),
        PathEl::LineTo(p) => out.line_to(map(p)),
        PathEl::ClosePath => out.close_path(),
        // Flattening gives lines only.
        PathEl::QuadTo(_, p) | PathEl::CurveTo(_, _, p) => out.line_to(map(p)),
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shape::model::{Paint, StrokeAlign, Subpath};
    use crate::transform::Affine;

    #[test]
    fn live_shapes_give_their_outlines() {
        let rect = Geometry::Rectangle {
            rect: [10.0, 20.0, 50.0, 40.0],
            radii: [0.0; 4],
        };
        let bounds = outline(&rect, TOLERANCE).bounding_box();
        assert_eq!(
            (bounds.x0, bounds.y0, bounds.x1, bounds.y1),
            (10.0, 20.0, 50.0, 40.0)
        );
        assert!((outline(&rect, TOLERANCE).area().abs() - 800.0).abs() < 1e-9);
        // Corners rounded: less area; radii beyond half a side are clamped.
        let round = Geometry::Rectangle {
            rect: [0.0, 0.0, 20.0, 10.0],
            radii: [100.0; 4],
        };
        let area = outline(&round, 0.01).area().abs();
        assert!(area < 200.0 && area > 150.0, "{area}");
        let ellipse = Geometry::Ellipse {
            center: [0.0, 0.0],
            radii: [10.0, 5.0],
        };
        let area = outline(&ellipse, 0.01).area().abs();
        assert!((area - std::f64::consts::PI * 50.0).abs() < 0.5, "{area}");
        // A square on its corner (45°), and a five-pointed star.
        let square = Geometry::Polygon {
            center: [0.0, 0.0],
            radius: 10.0,
            sides: 4,
            star: None,
            rotation: 0.0,
        };
        assert!((outline(&square, TOLERANCE).area().abs() - 200.0).abs() < 1e-9);
        let star = Geometry::Polygon {
            center: [0.0, 0.0],
            radius: 10.0,
            sides: 5,
            star: Some(0.5),
            rotation: 90.0,
        };
        let path = outline(&star, TOLERANCE);
        assert_eq!(path.elements().len(), 10 + 1);
        // Its first corner straight up.
        assert!(path.bounding_box().y0 < -9.99);
        let open = Geometry::Path {
            subpaths: vec![Subpath {
                start: [0.0, 0.0],
                segments: vec![
                    Segment::Line([10.0, 0.0]),
                    Segment::Cubic([10.0, 10.0], [0.0, 10.0], [0.0, 0.0]),
                ],
                closed: true,
            }],
            rule: crate::shape::model::FillRule::NonZero,
        };
        assert!(outline(&open, TOLERANCE).area().abs() > 0.0);
    }

    #[test]
    fn strokes_widen_dash_and_place() {
        let line = outline(
            &Geometry::Line {
                from: [0.0, 0.0],
                to: [100.0, 0.0],
            },
            TOLERANCE,
        );
        let stroke = ShapeStroke {
            paint: Paint::Solid(crate::color::LinearRgba::new(0.0, 0.0, 0.0, 1.0)),
            width: 4.0,
            align: StrokeAlign::Center,
            cap: StrokeCap::Butt,
            join: StrokeJoin::Miter,
            miter_limit: 4.0,
            dashes: Vec::new(),
            dash_offset: 0.0,
        };
        let solid = stroke_outline(&line, &stroke, 1.0, TOLERANCE);
        assert!((solid.area().abs() - 400.0).abs() < 1e-6);
        // Square caps add half a width at each end; twice as wide for an aligned stroke.
        let square = ShapeStroke {
            cap: StrokeCap::Square,
            ..stroke.clone()
        };
        assert!((stroke_outline(&line, &square, 1.0, TOLERANCE).area().abs() - 416.0).abs() < 1e-6);
        assert!((stroke_outline(&line, &stroke, 2.0, TOLERANCE).area().abs() - 800.0).abs() < 1e-6);
        // Dashes of one width, gaps of one width: half of it.
        let dashed = ShapeStroke {
            dashes: vec![1.0, 1.0],
            ..stroke.clone()
        };
        let area = stroke_outline(&line, &dashed, 1.0, TOLERANCE).area().abs();
        // Thirteen dashes of 4 along 100 (the last one whole at 96).
        assert!((area - 208.0).abs() < 1e-6, "{area}");
        // Placed: an affine map exactly, a projective one point by point.
        let moved = placed(&solid, Affine::translation(5.0, 7.0).into(), TOLERANCE);
        let b = moved.bounding_box();
        assert_eq!((b.x0, b.y0), (5.0, 5.0));
        let keystone = Projective::from_rect_to_quad(
            [0.0, -2.0, 100.0, 2.0],
            [(10.0, 0.0), (90.0, 0.0), (100.0, 4.0), (0.0, 4.0)],
        )
        .unwrap();
        let b = placed(&solid, keystone, TOLERANCE).bounding_box();
        assert!(
            (b.x0 - 0.0).abs() < 1e-6 && (b.x1 - 100.0).abs() < 1e-6,
            "{b:?}"
        );
    }
}
