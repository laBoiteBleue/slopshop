//! Perspective Warp's planes (ADR 0044): quads laid out over the layer's content (Layout mode),
//! then their corners moved (Warp mode). Each plane maps its quad as laid out to its quad as
//! moved by a homography; planes joined at an edge share its two corners.
//!
//! Two homographies agreeing on an edge's two ends send it onto the same segment, but spread its
//! points differently along it. So that joined planes stay joined, each plane's map is its
//! homography plus a correction on its joined edges: the mean of the joined planes' images of
//! each edge point (on the same segment: edges stay straight) minus its own, blended inward as a
//! Coons patch blends its edges, so that it vanishes at the corners and on the free edges.
//!
//! The content outside the planes follows them: each plane extended beyond its edges by its
//! map's first order there (never crossing a horizon line), the planes weighted by the inverse
//! of their distance to the fourth power (each plane leads near it). The mesh is a grid over the frame (that background), then each
//! plane's own cells on top, exact along the planes' edges.

use super::{InvalidWarp, MAX_COORDINATE, VertexGrid};
use crate::transform::Projective;

/// Most planes of a perspective warp.
pub const MAX_PLANES: usize = 32;
/// Cells per side each plane is cut into: each close to its exact (projective) map.
pub const CELLS_PER_PLANE: u32 = 16;
/// Cells along the longer side of the frame for the content outside the planes.
const BACKGROUND_CELLS: u32 = 64;
/// The smallest area of a plane as laid out (content pixels²).
const MIN_AREA: f64 = 1.0;

type Point = (f64, f64);

/// Perspective Warp's planes over `frame` (content pixels, `[x0, y0, x1, y1]`): corners where
/// they were laid out and where they were moved (content pixels, before the layer's transform),
/// and each plane's four corners (indices, in order around it). Planes listing the same two
/// corners one after the other are joined along that edge.
#[derive(Debug, Clone, PartialEq)]
pub struct Planes {
    frame: [f64; 4],
    layout: Vec<Point>,
    warped: Vec<Point>,
    planes: Vec<[u32; 4]>,
    /// Derived from the above: each plane's map, and the box of the warped content.
    maps: Vec<PlaneMap>,
    bounds: [f64; 4],
}

/// A plane's map: its homography from its corners as laid out, and the homographies of every
/// plane with each of its edges (corner `k` to corner `k + 1`), its own included.
#[derive(Debug, Clone, PartialEq)]
struct PlaneMap {
    corners: [Point; 4],
    homography: Projective,
    edges: [Vec<Projective>; 4],
}

impl Planes {
    /// Planes with these corners; refused when an index is out of range or repeated in a plane,
    /// a corner is in no plane, a plane as laid out is not convex (or smaller than a pixel), a
    /// plane as moved is not a quad its homography can reach (folded through its horizon line),
    /// or a number is not finite or too large.
    pub fn new(
        frame: [f64; 4],
        layout: Vec<Point>,
        warped: Vec<Point>,
        planes: Vec<[u32; 4]>,
    ) -> Result<Self, InvalidWarp> {
        let in_range = |v: f64| v.is_finite() && v.abs() < MAX_COORDINATE;
        let n = layout.len();
        let mut used = vec![false; n];
        let indices_ok = planes.iter().all(|plane| {
            let distinct = (0..4).all(|i| (i + 1..4).all(|j| plane[i] != plane[j]));
            plane.iter().all(|&k| {
                let ok = (k as usize) < n;
                if ok {
                    used[k as usize] = true;
                }
                ok
            }) && distinct
        });
        let valid = (1..=MAX_PLANES).contains(&planes.len())
            && warped.len() == n
            && n <= 4 * planes.len()
            && frame.iter().all(|&v| in_range(v))
            && frame[0] < frame[2]
            && frame[1] < frame[3]
            && layout
                .iter()
                .chain(&warped)
                .all(|&(x, y)| in_range(x) && in_range(y))
            && indices_ok
            && used.iter().all(|&u| u);
        if !valid {
            return Err(InvalidWarp);
        }
        let quad = |points: &[Point], plane: &[u32; 4]| plane.map(|k| points[k as usize]);
        let mut homographies = Vec::with_capacity(planes.len());
        for plane in &planes {
            let (from, to) = (quad(&layout, plane), quad(&warped, plane));
            if !convex(from) {
                return Err(InvalidWarp);
            }
            homographies.push(homography(from, to).ok_or(InvalidWarp)?);
        }
        // Each edge's planes: an edge is its two corners, in either order.
        let edge = |plane: &[u32; 4], k: usize| {
            let (a, b) = (plane[k], plane[(k + 1) % 4]);
            (a.min(b), a.max(b))
        };
        let maps = planes
            .iter()
            .zip(&homographies)
            .map(|(plane, &h)| PlaneMap {
                corners: quad(&layout, plane),
                homography: h,
                edges: std::array::from_fn(|k| {
                    let e = edge(plane, k);
                    planes
                        .iter()
                        .zip(&homographies)
                        .filter(|(other, _)| (0..4).any(|j| edge(other, j) == e))
                        .map(|(_, &h)| h)
                        .collect()
                }),
            })
            .collect();
        let mut planes = Self {
            frame,
            layout,
            warped,
            planes,
            maps,
            bounds: [0.0; 4],
        };
        planes.bounds = planes
            .vertices(0.0)
            .iter()
            .flat_map(|g| g.vertices.iter().map(|&(w, _)| w))
            .fold(
                [
                    f64::INFINITY,
                    f64::INFINITY,
                    f64::NEG_INFINITY,
                    f64::NEG_INFINITY,
                ],
                |[x0, y0, x1, y1], (x, y)| [x0.min(x), y0.min(y), x1.max(x), y1.max(y)],
            );
        if !planes.bounds.iter().all(|v| v.is_finite()) {
            return Err(InvalidWarp);
        }
        Ok(planes)
    }

    pub fn frame(&self) -> [f64; 4] {
        self.frame
    }

    /// The corners where they were laid out (content pixels).
    pub fn layout(&self) -> &[Point] {
        &self.layout
    }

    /// The corners where they were moved (content pixels, before the layer's transform).
    pub fn warped(&self) -> &[Point] {
        &self.warped
    }

    /// Each plane's corners (indices into [`Self::layout`] and [`Self::warped`]).
    pub fn planes(&self) -> &[[u32; 4]] {
        &self.planes
    }

    /// The box of the warped content (content pixels): the mesh's vertices'.
    pub fn bounds(&self) -> [f64; 4] {
        self.bounds
    }

    /// Where content point (`x`, `y`) goes: inside a plane as laid out (the last one listed
    /// where they overlap), by its map; elsewhere, by the planes' maps extended (see the
    /// module's documentation).
    pub fn apply(&self, x: f64, y: f64) -> Point {
        let p = (x, y);
        if let Some(map) = self.maps.iter().rev().find(|m| inside(m.corners, p)) {
            let (u, v) = inverse_bilinear(map.corners, p);
            return map.at(u, v);
        }
        let (mut sx, mut sy, mut total) = (0.0, 0.0, 0.0);
        for map in &self.maps {
            let ((cx, cy), (u, v)) = nearest(map.corners, p);
            let (dx, dy) = (x - cx, y - cy);
            let d2 = dx * dx + dy * dy;
            let (mx, my) = map.at(u, v);
            if d2 == 0.0 {
                return (mx, my);
            }
            // The map's first order at the nearest point of the plane: the extension leaves
            // the plane without a bend.
            let [a, b, c, d] = map.jacobian(u, v);
            let w = 1.0 / (d2 * d2);
            sx += w * (mx + a * dx + c * dy);
            sy += w * (my + b * dx + d * dy);
            total += w;
        }
        (sx / total, sy / total)
    }

    /// The mesh's vertices: a grid over the frame (grown by `margin` content pixels, for the
    /// edges' antialiasing), then each plane's cells (drawn over it).
    pub(super) fn vertices(&self, margin: f64) -> Vec<VertexGrid> {
        let [x0, y0, x1, y1] = self.frame;
        let (x0, y0, x1, y1) = (x0 - margin, y0 - margin, x1 + margin, y1 + margin);
        let (w, h) = (x1 - x0, y1 - y0);
        let cells = |side: f64| {
            ((f64::from(BACKGROUND_CELLS) * side / w.max(h)).round() as u32)
                .clamp(1, BACKGROUND_CELLS)
        };
        let (nx, ny) = (cells(w), cells(h));
        let mut grids = Vec::with_capacity(self.maps.len() + 1);
        let background = (0..=ny)
            .flat_map(|j| (0..=nx).map(move |i| (i, j)))
            .map(|(i, j)| {
                let p = (
                    x0 + w * f64::from(i) / f64::from(nx),
                    y0 + h * f64::from(j) / f64::from(ny),
                );
                (self.apply(p.0, p.1), p)
            })
            .collect();
        grids.push(VertexGrid {
            stride: nx as usize + 1,
            vertices: background,
        });
        let n = CELLS_PER_PLANE;
        for map in &self.maps {
            let vertices = (0..=n)
                .flat_map(|j| (0..=n).map(move |i| (i, j)))
                .map(|(i, j)| {
                    let (u, v) = (f64::from(i) / f64::from(n), f64::from(j) / f64::from(n));
                    (map.at(u, v), bilinear(map.corners, u, v))
                })
                .collect();
            grids.push(VertexGrid {
                stride: n as usize + 1,
                vertices,
            });
        }
        grids
    }
}

impl PlaneMap {
    /// The map's Jacobian at bilinear parameters (`u`, `v`), in content pixels:
    /// `[∂x'/∂x, ∂y'/∂x, ∂x'/∂y, ∂y'/∂y]`, from its derivatives along `u` and `v` (central
    /// differences) and the bilinear map's.
    fn jacobian(&self, u: f64, v: f64) -> [f64; 4] {
        const H: f64 = 1e-5;
        let (a, b) = (self.at(u + H, v), self.at(u - H, v));
        let (c, d) = (self.at(u, v + H), self.at(u, v - H));
        let (mux, muy) = ((a.0 - b.0) / (2.0 * H), (a.1 - b.1) / (2.0 * H));
        let (mvx, mvy) = ((c.0 - d.0) / (2.0 * H), (c.1 - d.1) / (2.0 * H));
        let ((pux, puy), (pvx, pvy)) = bilinear_derivatives(self.corners, u, v);
        let det = pux * pvy - pvx * puy;
        [
            (mux * pvy - mvx * puy) / det,
            (muy * pvy - mvy * puy) / det,
            (mvx * pux - mux * pvx) / det,
            (mvy * pux - muy * pvx) / det,
        ]
    }

    /// Where the plane's point at bilinear parameters (`u`, `v`) of its quad as laid out goes:
    /// its homography's image, corrected along its joined edges.
    fn at(&self, u: f64, v: f64) -> Point {
        let c = self.corners;
        let (hx, hy) = image(self.homography, bilinear(c, u, v));
        // Edge k runs from corner k to corner k + 1: (u, 0), (1, v), (1 − u, 1), (0, 1 − v).
        let terms = [
            (1.0 - v, bilinear(c, u, 0.0)),
            (u, bilinear(c, 1.0, v)),
            (v, bilinear(c, u, 1.0)),
            (1.0 - u, bilinear(c, 0.0, v)),
        ];
        let (mut x, mut y) = (hx, hy);
        for (k, (weight, point)) in terms.into_iter().enumerate() {
            let maps = &self.edges[k];
            if maps.len() < 2 || weight == 0.0 {
                continue;
            }
            let (ox, oy) = image(self.homography, point);
            let count = maps.len() as f64;
            let (mx, my) = maps.iter().fold((0.0, 0.0), |(sx, sy), m| {
                let (px, py) = image(*m, point);
                (sx + px / count, sy + py / count)
            });
            x += weight * (mx - ox);
            y += weight * (my - oy);
        }
        (x, y)
    }
}

/// The image of point `p` by `h`.
fn image(h: Projective, (x, y): Point) -> Point {
    h.apply(x, y)
}

/// The quad's point at bilinear parameters (`u`, `v`): corner 0 at (0, 0), 1 at (1, 0), 2 at
/// (1, 1), 3 at (0, 1).
fn bilinear(c: [Point; 4], u: f64, v: f64) -> Point {
    let (a, b, cc, d) = ((1.0 - u) * (1.0 - v), u * (1.0 - v), u * v, (1.0 - u) * v);
    (
        a * c[0].0 + b * c[1].0 + cc * c[2].0 + d * c[3].0,
        a * c[0].1 + b * c[1].1 + cc * c[2].1 + d * c[3].1,
    )
}

/// The derivatives of the quad's bilinear map along `u` and along `v` at (`u`, `v`).
fn bilinear_derivatives(c: [Point; 4], u: f64, v: f64) -> (Point, Point) {
    (
        (
            (1.0 - v) * (c[1].0 - c[0].0) + v * (c[2].0 - c[3].0),
            (1.0 - v) * (c[1].1 - c[0].1) + v * (c[2].1 - c[3].1),
        ),
        (
            (1.0 - u) * (c[3].0 - c[0].0) + u * (c[2].0 - c[1].0),
            (1.0 - u) * (c[3].1 - c[0].1) + u * (c[2].1 - c[1].1),
        ),
    )
}

/// The bilinear parameters of point `p` inside convex quad `c` (Newton's method from its
/// center, which converges inside a convex quad), clamped to the quad.
fn inverse_bilinear(c: [Point; 4], p: Point) -> Point {
    let (mut u, mut v) = (0.5, 0.5);
    for _ in 0..12 {
        let (x, y) = bilinear(c, u, v);
        let (fx, fy) = (x - p.0, y - p.1);
        let (du, dv) = bilinear_derivatives(c, u, v);
        let det = du.0 * dv.1 - dv.0 * du.1;
        if det == 0.0 || !det.is_finite() {
            break;
        }
        let (su, sv) = ((fx * dv.1 - dv.0 * fy) / det, (du.0 * fy - fx * du.1) / det);
        u -= su;
        v -= sv;
        if su.abs() < 1e-12 && sv.abs() < 1e-12 {
            break;
        }
    }
    (u.clamp(0.0, 1.0), v.clamp(0.0, 1.0))
}

/// The point of quad `c`'s edges nearest to `p`, and its bilinear parameters.
fn nearest(c: [Point; 4], p: Point) -> (Point, Point) {
    let mut best = (f64::INFINITY, c[0], (0.0, 0.0));
    for k in 0..4 {
        let (a, b) = (c[k], c[(k + 1) % 4]);
        let (ex, ey) = (b.0 - a.0, b.1 - a.1);
        let len2 = ex * ex + ey * ey;
        let s = if len2 > 0.0 {
            (((p.0 - a.0) * ex + (p.1 - a.1) * ey) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let q = (a.0 + s * ex, a.1 + s * ey);
        let d = (q.0 - p.0).powi(2) + (q.1 - p.1).powi(2);
        if d < best.0 {
            let uv = match k {
                0 => (s, 0.0),
                1 => (1.0, s),
                2 => (1.0 - s, 1.0),
                _ => (0.0, 1.0 - s),
            };
            best = (d, q, uv);
        }
    }
    (best.1, best.2)
}

/// The cross products of the quad's consecutive edges at each corner.
fn turns(c: [Point; 4]) -> [f64; 4] {
    std::array::from_fn(|k| {
        let (a, b, d) = (c[k], c[(k + 1) % 4], c[(k + 2) % 4]);
        (b.0 - a.0) * (d.1 - b.1) - (b.1 - a.1) * (d.0 - b.0)
    })
}

/// Whether the quad is strictly convex (either way round) and at least [`MIN_AREA`].
fn convex(c: [Point; 4]) -> bool {
    let t = turns(c);
    let area = 0.5
        * (0..4)
            .map(|k| {
                let (a, b) = (c[k], c[(k + 1) % 4]);
                a.0 * b.1 - b.0 * a.1
            })
            .sum::<f64>();
    (t.iter().all(|&v| v > 0.0) || t.iter().all(|&v| v < 0.0)) && area.abs() >= MIN_AREA
}

/// Whether `p` is inside convex quad `c` (its edges included).
fn inside(c: [Point; 4], p: Point) -> bool {
    let side = |k: usize| {
        let (a, b) = (c[k], c[(k + 1) % 4]);
        (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0)
    };
    let s = [side(0), side(1), side(2), side(3)];
    s.iter().all(|&v| v >= 0.0) || s.iter().all(|&v| v <= 0.0)
}

/// The homography sending quad `from`'s corners to `to`'s, in order; `None` when there is none,
/// or when it sends part of `from` through its horizon line (`to` folded, or not convex).
fn homography(from: [Point; 4], to: [Point; 4]) -> Option<Projective> {
    let unit = [0.0, 0.0, 1.0, 1.0];
    let a = Projective::from_rect_to_quad(unit, from)?;
    let b = Projective::from_rect_to_quad(unit, to)?;
    let h = a.inverse()?.then(b);
    if !h.is_finite() {
        return None;
    }
    // w is linear: of one sign at the four corners, it is of that sign over the whole quad.
    let w = from.map(|(x, y)| h.w(x, y));
    let scale = w.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    let tiny = 1e-9 * scale;
    let same = w.iter().all(|&v| v > tiny) || w.iter().all(|&v| v < -tiny);
    (same && scale.is_finite() && scale > 0.0).then_some(h)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::warp::{PlacedMesh, Warp};

    const FRAME: [f64; 4] = [0.0, 0.0, 200.0, 100.0];

    fn close(a: Point, b: Point, tolerance: f64) -> bool {
        (a.0 - b.0).abs() < tolerance && (a.1 - b.1).abs() < tolerance
    }

    /// Two planes side by side, joined along x = 100: corners 0 1 4 5 on top, 3 2 … below.
    fn two_planes(warped: Vec<Point>) -> Result<Planes, InvalidWarp> {
        let layout = vec![
            (20.0, 20.0),
            (100.0, 20.0),
            (100.0, 80.0),
            (20.0, 80.0),
            (180.0, 20.0),
            (180.0, 80.0),
        ];
        Planes::new(FRAME, layout, warped, vec![[0, 1, 2, 3], [1, 4, 5, 2]])
    }

    fn unmoved() -> Vec<Point> {
        vec![
            (20.0, 20.0),
            (100.0, 20.0),
            (100.0, 80.0),
            (20.0, 80.0),
            (180.0, 20.0),
            (180.0, 80.0),
        ]
    }

    #[test]
    fn unmoved_planes_change_nothing() {
        let planes = two_planes(unmoved()).unwrap();
        for p in [
            (50.0, 50.0),
            (150.0, 30.0),
            (5.0, 5.0),
            (199.0, 99.0),
            (100.0, 50.0),
        ] {
            assert!(close(planes.apply(p.0, p.1), p, 1e-9), "{p:?}");
        }
        let mesh = PlacedMesh::new(&planes.into(), Projective::IDENTITY, 0.0).unwrap();
        let t = mesh.find(120.5, 40.5).unwrap();
        assert!(close(t.to_content.apply(120.5, 40.5), (120.5, 40.5), 1e-6));
    }

    #[test]
    fn a_plane_maps_its_quad_by_a_homography() {
        let layout = vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)];
        let warped = vec![(10.0, 0.0), (90.0, 20.0), (100.0, 80.0), (0.0, 100.0)];
        let planes = Planes::new(
            [0.0, 0.0, 100.0, 100.0],
            layout.clone(),
            warped.clone(),
            vec![[0, 1, 2, 3]],
        )
        .unwrap();
        let h = Projective::from_rect_to_quad(
            [0.0, 0.0, 100.0, 100.0],
            [warped[0], warped[1], warped[2], warped[3]],
        )
        .unwrap();
        // The corners go where they were moved, every inner point as the homography sends it
        // (the projective map, not a bilinear one: lines stay lines).
        for (l, w) in layout.iter().zip(&warped) {
            assert!(close(planes.apply(l.0, l.1), *w, 1e-9));
        }
        for p in [(25.0, 25.0), (50.0, 50.0), (80.0, 10.0), (33.0, 90.0)] {
            assert!(
                close(planes.apply(p.0, p.1), h.apply(p.0, p.1), 1e-6),
                "{p:?}"
            );
        }
    }

    #[test]
    fn joined_planes_agree_along_their_edge_and_keep_it_straight() {
        let mut warped = unmoved();
        // Each plane in a strong perspective of its own: their homographies spread the shared
        // edge differently.
        warped[0] = (40.0, 35.0);
        warped[3] = (40.0, 65.0);
        warped[4] = (190.0, 0.0);
        warped[1] = (110.0, 15.0);
        let planes = two_planes(warped).unwrap();
        let (a, b) = (planes.warped()[1], planes.warped()[2]);
        for k in 0..=10 {
            let t = f64::from(k) / 10.0;
            let p = (100.0, 20.0 + 60.0 * t);
            // Each plane's map at the edge, from either side.
            let left = planes.maps[0].at(1.0, t);
            let right = planes.maps[1].at(0.0, t);
            assert!(close(left, right, 1e-9), "{t}: {left:?} {right:?}");
            assert!(close(planes.apply(p.0, p.1), left, 1e-9));
            // On the segment between the edge's corners as moved.
            let cross = (b.0 - a.0) * (left.1 - a.1) - (b.1 - a.1) * (left.0 - a.0);
            assert!(cross.abs() < 1e-6, "{t}: {cross}");
        }
        // The homographies alone would not agree there.
        let mid = (100.0, 50.0);
        let (h0, h1) = (planes.maps[0].homography, planes.maps[1].homography);
        assert!(!close(h0.apply(mid.0, mid.1), h1.apply(mid.0, mid.1), 0.1));
        // Away from the joined edge, each plane is its homography: the free edges are kept.
        let free = (20.0, 50.0);
        assert!(close(
            planes.apply(free.0, free.1),
            h0.apply(free.0, free.1),
            1e-9
        ));
    }

    #[test]
    fn the_content_outside_follows_the_planes_continuously() {
        let mut warped = unmoved();
        warped[4] = (190.0, 0.0);
        warped[5] = (170.0, 95.0);
        let planes = two_planes(warped).unwrap();
        // Just outside a plane, its map extended; far away, still finite.
        let on = planes.apply(180.0, 50.0);
        let near = planes.apply(180.001, 50.0);
        assert!(close(on, near, 0.01), "{on:?} {near:?}");
        let on = planes.apply(60.0, 20.0);
        let near = planes.apply(60.0, 19.999);
        assert!(close(on, near, 0.01));
        let far = planes.apply(0.0, 0.0);
        assert!(far.0.is_finite() && far.1.is_finite());
        // The mesh covers the frame and the planes.
        let mesh = PlacedMesh::new(&planes.clone().into(), Projective::IDENTITY, 2.0).unwrap();
        let (x, y) = planes.apply(1.0, 99.0);
        let t = mesh.find(x, y).unwrap();
        assert!(close(t.to_content.apply(x, y), (1.0, 99.0), 0.05));
        let [x0, y0, x1, y1] = mesh.bounds();
        assert!(
            x0 < 0.0 && y0 < 0.0 && x1 >= 190.0 && y1 >= 100.0,
            "{:?}",
            mesh.bounds()
        );
        let b = planes.bounds();
        assert!(b[1] <= 0.0 && b[2] >= 190.0);
    }

    #[test]
    fn the_mesh_maps_a_plane_as_its_map_does() {
        let mut warped = unmoved();
        warped[0] = (30.0, 30.0);
        let planes = two_planes(warped).unwrap();
        let mesh = PlacedMesh::new(&planes.clone().into(), Projective::IDENTITY, 0.0).unwrap();
        // A content point inside the first plane: where it goes, the mesh shows it back.
        for p in [(60.0, 50.0), (40.0, 70.0), (150.0, 40.0)] {
            let (x, y) = planes.apply(p.0, p.1);
            let t = mesh.find(x, y).unwrap();
            assert!(close(t.to_content.apply(x, y), p, 0.05), "{p:?}");
        }
    }

    #[test]
    fn invalid_planes_are_refused() {
        let ok = |w: Vec<Point>| two_planes(w).is_ok();
        assert!(ok(unmoved()));
        // A corner pulled across its plane: folded through the horizon line.
        let mut folded = unmoved();
        folded[0] = (150.0, 70.0);
        assert!(!ok(folded));
        let mut nan = unmoved();
        nan[2].0 = f64::NAN;
        assert!(!ok(nan));
        let square = vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
        let one = |layout: Vec<Point>, planes: Vec<[u32; 4]>| {
            Planes::new(FRAME, layout.clone(), layout, planes).is_ok()
        };
        assert!(one(square.clone(), vec![[0, 1, 2, 3]]));
        // Out of range, repeated, unused corners; no plane.
        assert!(!one(square.clone(), vec![[0, 1, 2, 4]]));
        assert!(!one(square.clone(), vec![[0, 1, 2, 2]]));
        let mut extra = square.clone();
        extra.push((5.0, 5.0));
        assert!(!one(extra, vec![[0, 1, 2, 3]]));
        assert!(!one(square.clone(), vec![]));
        // Not convex as laid out (a bow tie), too small.
        assert!(!one(square.clone(), vec![[0, 2, 1, 3]]));
        let tiny = vec![(0.0, 0.0), (0.5, 0.0), (0.5, 0.5), (0.0, 0.5)];
        assert!(!one(tiny, vec![[0, 1, 2, 3]]));
        // Different counts of corners.
        assert!(
            Planes::new(
                FRAME,
                square.clone(),
                square[..3].to_vec(),
                vec![[0, 1, 2, 3]]
            )
            .is_err()
        );
    }

    #[test]
    fn a_warp_of_planes_moves_with_its_content() {
        let planes = two_planes(unmoved()).unwrap();
        let moved = Warp::from(planes.clone()).translated(5.0, -2.0).unwrap();
        let moved = moved.planes().unwrap();
        assert_eq!(moved.frame(), [5.0, -2.0, 205.0, 98.0]);
        assert_eq!(moved.layout()[0], (25.0, 18.0));
        assert_eq!(moved.planes(), planes.planes());
    }
}
