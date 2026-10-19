//! Make Work Path from a selection (ADR 0041, Photoshop's Paths panel): the selection's outline
//! (where its coverage crosses one half, along the pixels' edges) as a path of lines and cubic
//! Béziers, within a tolerance in pixels (Photoshop's, 0.5 to 10, 2 by default).
//!
//! Each outline is a closed staircase. Its corners are found where its direction turns sharply
//! over a few pixels (a rectangle's, a triangle's), not at each step of a slanted edge; between
//! two corners, the staircase is smoothed (its edges' midpoints) and fitted with as few cubic
//! Béziers as the tolerance allows (`kurbo`'s curve fitting), straight runs as lines.

use kurbo::simplify::SimplifyBezPath;
use kurbo::{BezPath, PathEl, Point};

use super::model::{FillRule, Geometry, Segment, Subpath};
use crate::raster::RasterImage;

/// Photoshop's range of tolerances, in pixels.
pub const MIN_TOLERANCE: f64 = 0.5;
pub const MAX_TOLERANCE: f64 = 10.0;
/// Photoshop's default.
pub const DEFAULT_TOLERANCE: f64 = 2.0;

/// A turn sharper than this over the corner window makes a corner (degrees).
const CORNER_TURN: f64 = 65.0;

/// The path of `selection`'s outline (document pixels), fitted within `tolerance` (clamped to
/// Photoshop's range); filled by the nonzero rule (holes run the other way). `None` when
/// nothing is selected.
pub fn path_of_selection(selection: &RasterImage, tolerance: f64) -> Option<Geometry> {
    let tolerance = if tolerance.is_finite() {
        tolerance.clamp(MIN_TOLERANCE, MAX_TOLERANCE)
    } else {
        DEFAULT_TOLERANCE
    };
    let bounds = crate::selection::bounds(selection)?;
    let loops = crate::selection::outline(selection, 0, bounds, usize::MAX)?;
    let subpaths: Vec<Subpath> = loops
        .iter()
        .filter_map(|line| fitted(line, tolerance))
        .collect();
    (!subpaths.is_empty()).then_some(Geometry::Path {
        subpaths,
        rule: FillRule::NonZero,
    })
}

/// The corners of a closed polygon `points` whose sides lie along the pixels' edges.
fn vertices(line: &[[u32; 2]]) -> Vec<[f64; 2]> {
    let mut points: Vec<[f64; 2]> = line
        .iter()
        .map(|&[x, y]| [f64::from(x), f64::from(y)])
        .collect();
    if points.len() > 1 && points.first() == points.last() {
        points.pop();
    }
    // Points in the middle of a straight run (around the start too) dropped.
    loop {
        let n = points.len();
        if n < 3 {
            return points;
        }
        let straight = (0..n).find(|&i| {
            let [a, b, c] = [points[(i + n - 1) % n], points[i], points[(i + 1) % n]];
            let cross = (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0]);
            cross == 0.0
        });
        match straight {
            Some(i) => {
                points.remove(i);
            }
            None => return points,
        }
    }
}

/// The point `distance` along the closed polygon from vertex `from` (backwards when negative),
/// `lengths[i]` being side `i`'s (from vertex `i` to the next).
fn along(points: &[[f64; 2]], lengths: &[f64], from: usize, distance: f64) -> [f64; 2] {
    let n = points.len();
    let mut left = distance.abs();
    let mut at = from;
    for _ in 0..n {
        let (side, next) = if distance >= 0.0 {
            (at, (at + 1) % n)
        } else {
            let previous = (at + n - 1) % n;
            (previous, previous)
        };
        let length = lengths[side];
        if left <= length {
            let (a, b) = (points[at], points[next]);
            let t = if length > 0.0 { left / length } else { 0.0 };
            return [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
        }
        left -= length;
        at = next;
    }
    points[at]
}

/// The vertices of `points` that are corners: where the direction turns by more than
/// [`CORNER_TURN`] between `window` pixels before and after, the sharpest of its neighbors.
fn corners(points: &[[f64; 2]], window: f64) -> Vec<usize> {
    let n = points.len();
    let lengths: Vec<f64> = (0..n)
        .map(|i| {
            let (a, b) = (points[i], points[(i + 1) % n]);
            (b[0] - a[0]).hypot(b[1] - a[1])
        })
        .collect();
    let perimeter: f64 = lengths.iter().sum();
    let window = window.min(perimeter / 4.0);
    let turns: Vec<f64> = (0..n)
        .map(|i| {
            let p = points[i];
            let before = along(points, &lengths, i, -window);
            let after = along(points, &lengths, i, window);
            let (ux, uy) = (p[0] - before[0], p[1] - before[1]);
            let (vx, vy) = (after[0] - p[0], after[1] - p[1]);
            (ux * vy - uy * vx)
                .atan2(ux * vx + uy * vy)
                .abs()
                .to_degrees()
        })
        .collect();
    // Within `window` of each other, only the sharpest turn (the first of equal ones).
    let beats = |i: usize, j: usize| turns[j] < turns[i] || (turns[j] == turns[i] && j > i);
    let sharpest = |i: usize| {
        // The vertices `window` pixels on either side.
        let (mut ahead, mut behind) = (0.0, 0.0);
        for k in 1..n {
            let (j, back) = ((i + k) % n, (i + n - k) % n);
            ahead += lengths[(j + n - 1) % n];
            behind += lengths[back];
            if ahead > window && behind > window {
                break;
            }
            if (ahead <= window && !beats(i, j)) || (behind <= window && !beats(i, back)) {
                return false;
            }
        }
        true
    };
    (0..n)
        .filter(|&i| turns[i] > CORNER_TURN && sharpest(i))
        .collect()
}

/// One outline as a closed subpath, `None` when it is too small to be one.
fn fitted(line: &[[u32; 2]], tolerance: f64) -> Option<Subpath> {
    let points = vertices(line);
    let n = points.len();
    if n < 3 {
        return None;
    }
    let mid = |i: usize| {
        let (a, b) = (points[i], points[(i + 1) % n]);
        [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0]
    };
    let found = corners(&points, (2.0 * tolerance).clamp(3.0, 12.0));
    let mut segments = Vec::new();
    let start;
    if found.is_empty() {
        // Smooth all round: from the first side's middle back to it.
        start = mid(0);
        let mut run: Vec<[f64; 2]> = (0..n).map(mid).collect();
        run.push(start);
        fit_run(&run, tolerance, &mut segments);
    } else {
        start = points[found[0]];
        for (k, &from) in found.iter().enumerate() {
            let to = found[(k + 1) % found.len()];
            let count = (to + n - from - 1) % n + 1;
            let mut run = vec![points[from]];
            run.extend((0..count).map(|j| mid((from + j) % n)));
            run.push(points[to]);
            fit_run(&run, tolerance, &mut segments);
        }
    }
    Some(Subpath {
        start,
        segments,
        closed: true,
    })
}

/// Whether every point of `run` lies within a hundredth of a pixel of the line through its
/// ends.
fn is_straight(run: &[[f64; 2]]) -> bool {
    let (Some(&a), Some(&b)) = (run.first(), run.last()) else {
        return true;
    };
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length = dx.hypot(dy);
    if length == 0.0 {
        return false;
    }
    run.iter()
        .all(|p| ((p[0] - a[0]) * dy - (p[1] - a[1]) * dx).abs() / length <= 0.01)
}

/// `run` (a polyline from one corner to the next) as lines and cubics within `tolerance`.
fn fit_run(run: &[[f64; 2]], tolerance: f64, out: &mut Vec<Segment>) {
    let Some(&end) = run.last() else {
        return;
    };
    if run.len() <= 2 || is_straight(run) {
        out.push(Segment::Line(end));
        return;
    }
    let mut polyline = BezPath::new();
    polyline.move_to(Point::new(run[0][0], run[0][1]));
    for p in &run[1..] {
        polyline.line_to(Point::new(p[0], p[1]));
    }
    let source = SimplifyBezPath::new(&polyline);
    let fit = kurbo::fit_to_bezpath_opt(&source, tolerance);
    let mut from = run[0];
    for element in fit.elements() {
        let segment = match *element {
            PathEl::LineTo(p) => Segment::Line([p.x, p.y]),
            PathEl::QuadTo(c, p) => {
                // A quadratic as the cubic it is.
                let q = |a: f64, b: f64| a + (b - a) * 2.0 / 3.0;
                Segment::Cubic(
                    [q(from[0], c.x), q(from[1], c.y)],
                    [q(p.x, c.x), q(p.y, c.y)],
                    [p.x, p.y],
                )
            }
            PathEl::CurveTo(a, b, p) => {
                if is_straight(&[from, [a.x, a.y], [b.x, b.y], [p.x, p.y]]) {
                    Segment::Line([p.x, p.y])
                } else {
                    Segment::Cubic([a.x, a.y], [b.x, b.y], [p.x, p.y])
                }
            }
            PathEl::MoveTo(_) | PathEl::ClosePath => continue,
        };
        from = match segment {
            Segment::Line(p) | Segment::Cubic(_, _, p) => p,
        };
        out.push(segment);
    }
    // The run ends exactly on its corner.
    match out.last_mut() {
        Some(Segment::Line(p) | Segment::Cubic(_, _, p)) => *p = end,
        None => out.push(Segment::Line(end)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Size;
    use crate::selection::{Combine, EdgeOptions, Shape, select_shape};

    fn selected(shape: &Shape) -> RasterImage {
        select_shape(
            Size::new(120, 100),
            None,
            shape,
            EdgeOptions::default(),
            Combine::Replace,
        )
        .unwrap()
        .unwrap()
    }

    /// The share of pixels whose selection differs (more than half covered or not) between
    /// `a` and `b`, over the pixels either selects.
    fn mismatch(a: &RasterImage, b: &RasterImage) -> f64 {
        let area = crate::geom::Rect::new(0, 0, 120, 100);
        let (va, vb) = (
            crate::selection::sample_grid(a, area, 120, 100),
            crate::selection::sample_grid(b, area, 120, 100),
        );
        let (mut differ, mut either) = (0, 0);
        for (x, y) in va.iter().zip(&vb) {
            let (x, y) = (*x >= 0.5, *y >= 0.5);
            differ += usize::from(x != y);
            either += usize::from(x || y);
        }
        differ as f64 / either as f64
    }

    fn reselected(geometry: &Geometry) -> RasterImage {
        let shape = crate::selection::path_shape(geometry, crate::transform::Projective::IDENTITY);
        selected(&shape)
    }

    #[test]
    fn a_rectangle_gives_its_four_corners_and_straight_sides() {
        let rect = selected(&Shape::Rectangle {
            left: 10.0,
            top: 20.0,
            right: 70.0,
            bottom: 60.0,
        });
        let Some(Geometry::Path { subpaths, .. }) = path_of_selection(&rect, 2.0) else {
            panic!("a selection gives a path");
        };
        assert_eq!(subpaths.len(), 1);
        let subpath = &subpaths[0];
        assert!(subpath.closed);
        let mut corners: Vec<[f64; 2]> = subpath
            .segments
            .iter()
            .map(|s| match *s {
                Segment::Line(p) => p,
                Segment::Cubic(..) => panic!("a rectangle's sides are lines: {s:?}"),
            })
            .collect();
        corners.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(
            corners,
            [[10.0, 20.0], [10.0, 60.0], [70.0, 20.0], [70.0, 60.0]]
        );
        assert!(corners.contains(&subpath.start));
    }

    #[test]
    fn an_ellipse_gives_a_few_curves_enclosing_what_it_selected() {
        let ellipse = selected(&Shape::Ellipse {
            left: 10.0,
            top: 10.0,
            right: 110.0,
            bottom: 80.0,
        });
        let geometry = path_of_selection(&ellipse, 2.0).unwrap();
        let Geometry::Path { subpaths, .. } = &geometry else {
            unreachable!("a path");
        };
        assert_eq!(subpaths.len(), 1);
        let segments = &subpaths[0].segments;
        assert!(segments.iter().all(|s| matches!(s, Segment::Cubic(..))));
        assert!(
            (2..=16).contains(&segments.len()),
            "{} segments",
            segments.len()
        );
        assert!(mismatch(&ellipse, &reselected(&geometry)) < 0.02);
        // A tighter tolerance takes more curves, and follows the outline as closely.
        let tight = path_of_selection(&ellipse, 0.5).unwrap();
        let Geometry::Path {
            subpaths: tight_paths,
            ..
        } = &tight
        else {
            unreachable!("a path");
        };
        assert!(tight_paths[0].segments.len() >= segments.len());
        assert!(mismatch(&ellipse, &reselected(&tight)) < 0.02);
    }

    #[test]
    fn holes_and_separate_parts_are_subpaths_of_one_path() {
        let ring = select_shape(
            Size::new(120, 100),
            None,
            &Shape::Rings {
                rings: vec![
                    vec![[10.0, 10.0], [60.0, 10.0], [60.0, 60.0], [10.0, 60.0]],
                    vec![[25.0, 25.0], [45.0, 25.0], [45.0, 45.0], [25.0, 45.0]],
                    vec![[80.0, 10.0], [110.0, 10.0], [110.0, 40.0], [80.0, 40.0]],
                ],
                even_odd: true,
            },
            EdgeOptions::default(),
            Combine::Replace,
        )
        .unwrap()
        .unwrap();
        let geometry = path_of_selection(&ring, 2.0).unwrap();
        let Geometry::Path { subpaths, rule } = &geometry else {
            unreachable!("a path");
        };
        assert_eq!(subpaths.len(), 3);
        assert_eq!(*rule, FillRule::NonZero);
        // The hole stays a hole under the nonzero rule: it runs the other way.
        assert!(mismatch(&ring, &reselected(&geometry)) < 0.001);
    }

    #[test]
    fn nothing_selected_gives_no_path_and_tolerances_are_clamped() {
        let empty = crate::selection::uniform_mask(Size::new(20, 20), false).unwrap();
        assert_eq!(path_of_selection(&empty, 2.0), None);
        let rect = selected(&Shape::Rectangle {
            left: 10.0,
            top: 10.0,
            right: 30.0,
            bottom: 30.0,
        });
        assert_eq!(
            path_of_selection(&rect, f64::NAN),
            path_of_selection(&rect, DEFAULT_TOLERANCE)
        );
        assert_eq!(
            path_of_selection(&rect, 100.0),
            path_of_selection(&rect, MAX_TOLERANCE)
        );
    }
}
