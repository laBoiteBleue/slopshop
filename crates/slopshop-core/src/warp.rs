//! Warps (ADR 0044): a deformation placing a layer's content before its transform.
//!
//! A [`Warp`] is a grid of bicubic Bézier patches over a frame of the layer's content (its
//! pixels): Edit > Transform > Warp, its styles ([`WarpStyle`]) giving starting grids. It is drawn
//! as triangles ([`PlacedMesh`]): each patch is cut into small cells, each cell into two
//! triangles mapped from the document to the content by their own affine map, so that the
//! resampling of ADR 0018 runs per triangle with a constant Jacobian. The CPU compositor and the
//! GPU renderer find the triangle under each pixel the same way (bins over the document) and
//! sample through its map.

use crate::transform::{Affine, Projective};

mod planes;

pub use planes::{CELLS_PER_PLANE, MAX_PLANES, Planes};

/// Most patches per side of a warp grid.
pub const MAX_PATCHES: u32 = 16;
/// Cells per patch side the mesh is cut into: fine enough that each triangle is close to its
/// exact map for the styles' bends (a quarter turn of a patch per side at most).
pub const CELLS_PER_PATCH: u32 = 12;
/// Bins per side of the lookup grid at most.
const MAX_BINS: u32 = 64;

/// The styles of Photoshop's Warp, each a starting grid made from a bend, in its menu's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarpStyle {
    Arc,
    ArcLower,
    ArcUpper,
    Arch,
    Bulge,
    ShellLower,
    ShellUpper,
    Flag,
    Wave,
    Fish,
    Rise,
    Fisheye,
    Inflate,
    Squeeze,
    Twist,
}

impl WarpStyle {
    pub const ALL: [WarpStyle; 15] = [
        WarpStyle::Arc,
        WarpStyle::ArcLower,
        WarpStyle::ArcUpper,
        WarpStyle::Arch,
        WarpStyle::Bulge,
        WarpStyle::ShellLower,
        WarpStyle::ShellUpper,
        WarpStyle::Flag,
        WarpStyle::Wave,
        WarpStyle::Fish,
        WarpStyle::Rise,
        WarpStyle::Fisheye,
        WarpStyle::Inflate,
        WarpStyle::Squeeze,
        WarpStyle::Twist,
    ];

    /// Its stable identifier (files, IPC).
    pub fn id(self) -> &'static str {
        match self {
            WarpStyle::Arc => "arc",
            WarpStyle::ArcLower => "arcLower",
            WarpStyle::ArcUpper => "arcUpper",
            WarpStyle::Arch => "arch",
            WarpStyle::Bulge => "bulge",
            WarpStyle::ShellLower => "shellLower",
            WarpStyle::ShellUpper => "shellUpper",
            WarpStyle::Flag => "flag",
            WarpStyle::Wave => "wave",
            WarpStyle::Fish => "fish",
            WarpStyle::Rise => "rise",
            WarpStyle::Fisheye => "fisheye",
            WarpStyle::Inflate => "inflate",
            WarpStyle::Squeeze => "squeeze",
            WarpStyle::Twist => "twist",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.id() == id)
    }
}

/// What a grid was made from: a style, its bend (`-1` to `1`, Photoshop's -100 % to 100 %) and
/// its orientation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WarpPreset {
    pub style: WarpStyle,
    pub bend: f64,
    /// Vertical rather than Photoshop's default horizontal.
    pub vertical: bool,
}

impl WarpPreset {
    pub fn is_valid(&self) -> bool {
        self.bend.is_finite() && (-1.0..=1.0).contains(&self.bend)
    }
}

/// A layer's warp (ADR 0044): Edit > Transform > Warp's grid, or Edit > Perspective Warp's
/// planes. Either is drawn as a [`PlacedMesh`].
#[derive(Debug, Clone, PartialEq)]
pub enum Warp {
    Grid(Grid),
    Planes(Planes),
}

impl From<Grid> for Warp {
    fn from(grid: Grid) -> Self {
        Warp::Grid(grid)
    }
}

impl From<Planes> for Warp {
    fn from(planes: Planes) -> Self {
        Warp::Planes(planes)
    }
}

impl Warp {
    /// The frame of the content it warps (content pixels, `[x0, y0, x1, y1]`).
    pub fn frame(&self) -> [f64; 4] {
        match self {
            Warp::Grid(grid) => grid.frame(),
            Warp::Planes(planes) => planes.frame(),
        }
    }

    /// The grid, when it is Warp's.
    pub fn grid(&self) -> Option<&Grid> {
        match self {
            Warp::Grid(grid) => Some(grid),
            Warp::Planes(_) => None,
        }
    }

    /// The planes, when it is Perspective Warp's.
    pub fn planes(&self) -> Option<&Planes> {
        match self {
            Warp::Planes(planes) => Some(planes),
            Warp::Grid(_) => None,
        }
    }

    /// The box of the warped content (content pixels).
    pub fn bounds(&self) -> [f64; 4] {
        match self {
            Warp::Grid(grid) => grid.bounds(),
            Warp::Planes(planes) => planes.bounds(),
        }
    }

    /// [`Self::bounds`] with the reach of the resampling filter around it: where the warped
    /// content's antialiased edges can show.
    pub fn drawn_bounds(&self) -> [f64; 4] {
        let m = crate::resample::RADIUS + 1.0;
        let [x0, y0, x1, y1] = self.bounds();
        [x0 - m, y0 - m, x1 + m, y1 + m]
    }

    /// The same warp of content moved by (`dx`, `dy`) content pixels.
    pub fn translated(&self, dx: f64, dy: f64) -> Result<Warp, InvalidWarp> {
        let [x0, y0, x1, y1] = self.frame();
        let frame = [x0 + dx, y0 + dy, x1 + dx, y1 + dy];
        let moved = |points: &[(f64, f64)]| -> Vec<(f64, f64)> {
            points.iter().map(|&(x, y)| (x + dx, y + dy)).collect()
        };
        Ok(match self {
            Warp::Grid(grid) => Warp::Grid(Grid::new(
                frame,
                grid.columns,
                grid.rows,
                moved(&grid.points),
                grid.preset,
            )?),
            Warp::Planes(planes) => Warp::Planes(Planes::new(
                frame,
                moved(planes.layout()),
                moved(planes.warped()),
                planes.planes().to_vec(),
            )?),
        })
    }
}

/// A warp grid (ADR 0044): `columns × rows` bicubic Bézier patches over `frame` (content pixels,
/// `[x0, y0, x1, y1]`), their control points in content pixels, row-major, `3·columns + 1` per
/// row and `3·rows + 1` rows (neighboring patches share their edges' points).
#[derive(Debug, Clone, PartialEq)]
pub struct Grid {
    frame: [f64; 4],
    columns: u32,
    rows: u32,
    points: Vec<(f64, f64)>,
    /// The style the grid was made from (`None`: edited point by point, Photoshop's Custom).
    preset: Option<WarpPreset>,
}

/// Why a warp is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidWarp;

impl std::fmt::Display for InvalidWarp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("invalid warp grid")
    }
}

impl std::error::Error for InvalidWarp {}

/// Coordinates beyond which a warp is refused (content pixels), as transforms are (ADR 0018).
const MAX_COORDINATE: f64 = 1e7;

impl Grid {
    /// A grid of `columns × rows` patches with these control points; refused when the counts do
    /// not match, the frame is empty or a number is not finite or too large.
    pub fn new(
        frame: [f64; 4],
        columns: u32,
        rows: u32,
        points: Vec<(f64, f64)>,
        preset: Option<WarpPreset>,
    ) -> Result<Self, InvalidWarp> {
        let in_range = |v: f64| v.is_finite() && v.abs() < MAX_COORDINATE;
        let valid = (1..=MAX_PATCHES).contains(&columns)
            && (1..=MAX_PATCHES).contains(&rows)
            && points.len() == ((3 * columns + 1) * (3 * rows + 1)) as usize
            && frame.iter().all(|&v| in_range(v))
            && frame[0] < frame[2]
            && frame[1] < frame[3]
            && points.iter().all(|&(x, y)| in_range(x) && in_range(y))
            && preset.is_none_or(|p| p.is_valid());
        if valid {
            Ok(Self {
                frame,
                columns,
                rows,
                points,
                preset,
            })
        } else {
            Err(InvalidWarp)
        }
    }

    /// The grid of `columns × rows` patches that changes nothing.
    pub fn identity(frame: [f64; 4], columns: u32, rows: u32) -> Result<Self, InvalidWarp> {
        let (nx, ny) = (3 * columns + 1, 3 * rows + 1);
        let points = (0..ny)
            .flat_map(|j| (0..nx).map(move |i| (i, j)))
            .map(|(i, j)| {
                frame_point(
                    frame,
                    f64::from(i) / f64::from(nx - 1),
                    f64::from(j) / f64::from(ny - 1),
                )
            })
            .collect();
        Self::new(frame, columns, rows, points, None)
    }

    /// Photoshop's starting grid for `preset` over `frame`: 3 × 3 patches whose control points
    /// are the style's deformation of the plain grid's.
    pub fn from_preset(frame: [f64; 4], preset: WarpPreset) -> Result<Self, InvalidWarp> {
        let plain = Self::identity(frame, 3, 3)?;
        if !preset.is_valid() {
            return Err(InvalidWarp);
        }
        let points = plain
            .points
            .iter()
            .map(|&(x, y)| deform(frame, preset, x, y))
            .collect();
        Self::new(frame, 3, 3, points, Some(preset))
    }

    pub fn frame(&self) -> [f64; 4] {
        self.frame
    }

    pub fn columns(&self) -> u32 {
        self.columns
    }

    pub fn rows(&self) -> u32 {
        self.rows
    }

    /// Control points, row-major (see [`Warp`]).
    pub fn points(&self) -> &[(f64, f64)] {
        &self.points
    }

    pub fn preset(&self) -> Option<WarpPreset> {
        self.preset
    }

    /// Where the grid's parameter `(s, t)` (`0..=columns`, `0..=rows`, beyond them extrapolated
    /// from the border patches) lands, in content pixels.
    pub fn eval(&self, s: f64, t: f64) -> (f64, f64) {
        let pc = (s.floor().max(0.0) as u32).min(self.columns - 1);
        let pr = (t.floor().max(0.0) as u32).min(self.rows - 1);
        let (u, v) = (s - f64::from(pc), t - f64::from(pr));
        let (bu, bv) = (bernstein(u), bernstein(v));
        let stride = (3 * self.columns + 1) as usize;
        let (mut x, mut y) = (0.0, 0.0);
        for (j, wv) in bv.iter().enumerate() {
            let row = (3 * pr) as usize + j;
            for (i, wu) in bu.iter().enumerate() {
                let (px, py) = self.points[row * stride + (3 * pc) as usize + i];
                x += wu * wv * px;
                y += wu * wv * py;
            }
        }
        (x, y)
    }

    /// Where the grid's parameter `(s, t)` is in the content before the warp.
    pub fn source(&self, s: f64, t: f64) -> (f64, f64) {
        frame_point(
            self.frame,
            s / f64::from(self.columns),
            t / f64::from(self.rows),
        )
    }

    /// The box of the warped content (content pixels): its patches' control points bound them.
    pub fn bounds(&self) -> [f64; 4] {
        self.points.iter().fold(
            [
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ],
            |[x0, y0, x1, y1], &(x, y)| [x0.min(x), y0.min(y), x1.max(x), y1.max(y)],
        )
    }
}

/// A regular grid of mesh vertices, row-major, `stride` per row: each where it lands (content
/// pixels, before the layer's transform) and the content point it shows. Each cell is cut into
/// two triangles.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct VertexGrid {
    pub(crate) stride: usize,
    pub(crate) vertices: Vec<((f64, f64), (f64, f64))>,
}

impl VertexGrid {
    fn triangles(&self) -> usize {
        let rows = self.vertices.len() / self.stride.max(1);
        2 * self.stride.saturating_sub(1) * rows.saturating_sub(1)
    }
}

impl Grid {
    /// The mesh's vertices: each patch cut into [`CELLS_PER_PATCH`] cells a side, the border
    /// extended by `margin` content pixels (at most a quarter of a patch: extrapolating beyond
    /// that folds the border).
    fn vertices(&self, margin: f64) -> VertexGrid {
        let (cols, rows) = (self.columns, self.rows);
        let [fx0, fy0, fx1, fy1] = self.frame;
        let ms = (margin * f64::from(cols) / (fx1 - fx0)).clamp(0.0, 0.25);
        let mt = (margin * f64::from(rows) / (fy1 - fy0)).clamp(0.0, 0.25);
        let params = |patches: u32, m: f64| -> Vec<f64> {
            let n = patches * CELLS_PER_PATCH;
            let mut p = Vec::with_capacity(n as usize + 3);
            if m > 0.0 {
                p.push(-m);
            }
            p.extend((0..=n).map(|k| f64::from(k) / f64::from(CELLS_PER_PATCH)));
            if m > 0.0 {
                p.push(f64::from(patches) + m);
            }
            p
        };
        let (ss, ts) = (params(cols, ms), params(rows, mt));
        let vertices = ts
            .iter()
            .flat_map(|&t| ss.iter().map(move |&s| (s, t)))
            .map(|(s, t)| (self.eval(s, t), self.source(s, t)))
            .collect();
        VertexGrid {
            stride: ss.len(),
            vertices,
        }
    }
}

/// `frame`'s point at fractions `(fx, fy)` of its width and height.
fn frame_point([x0, y0, x1, y1]: [f64; 4], fx: f64, fy: f64) -> (f64, f64) {
    (x0 + (x1 - x0) * fx, y0 + (y1 - y0) * fy)
}

/// The cubic Bernstein polynomials at `t`.
fn bernstein(t: f64) -> [f64; 4] {
    let s = 1.0 - t;
    [s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t]
}

/// Where a style moves content point `(x, y)` of `frame`.
fn deform(frame: [f64; 4], preset: WarpPreset, x: f64, y: f64) -> (f64, f64) {
    let (cx, cy) = ((frame[0] + frame[2]) / 2.0, (frame[1] + frame[3]) / 2.0);
    let (hw, hh) = ((frame[2] - frame[0]) / 2.0, (frame[3] - frame[1]) / 2.0);
    // Vertical: the same deformation with the axes swapped.
    let (dx, dy, w, h) = if preset.vertical {
        (y - cy, x - cx, hh, hw)
    } else {
        (x - cx, y - cy, hw, hh)
    };
    let (u, v) = (dx / w, dy / h);
    let b = preset.bend;
    let (mx, my) = match preset.style {
        // The center line bent into an arc of `b`·180°, its length kept; the other lines
        // concentric arcs around the same center (Photoshop's Arc).
        WarpStyle::Arc => {
            let angle = b * std::f64::consts::PI;
            if angle.abs() < 1e-9 {
                (dx, dy)
            } else {
                let radius = 2.0 * w / angle;
                let theta = u * angle / 2.0;
                let r = radius - dy;
                (r * theta.sin(), radius - r * theta.cos())
            }
        }
        // The bottom edge bent down (or up) in the middle, the top edge straight.
        WarpStyle::ArcLower => (dx, dy + b * h * (1.0 - u * u) * (1.0 + v) / 2.0),
        // The top edge bent up (or down) in the middle, the bottom edge straight.
        WarpStyle::ArcUpper => (dx, dy - b * h * (1.0 - u * u) * (1.0 - v) / 2.0),
        // Every line raised alike in the middle, the sides upright: an arch.
        WarpStyle::Arch => (dx, dy - b * h * 0.5 * (1.0 - u * u)),
        // The top and bottom edges pushed out (or in) in the middle, the sides straight.
        WarpStyle::Bulge => (dx, dy * (1.0 + b * 0.5 * (1.0 - u * u))),
        // Arc Lower with the top drawn in: a shell opening downward.
        WarpStyle::ShellLower => (
            dx * (1.0 - b * 0.3 * (1.0 - v) / 2.0),
            dy + b * h * (1.0 - u * u) * (1.0 + v) / 2.0,
        ),
        // Arc Upper with the bottom drawn in: a shell opening upward.
        WarpStyle::ShellUpper => (
            dx * (1.0 - b * 0.3 * (1.0 + v) / 2.0),
            dy - b * h * (1.0 - u * u) * (1.0 - v) / 2.0,
        ),
        // Every line waving by one period of a sine, the sides straight.
        WarpStyle::Flag => (dx, dy - b * h * 0.5 * (std::f64::consts::PI * u).sin()),
        // A wave whose phase shifts from top to bottom.
        WarpStyle::Wave => (
            dx,
            dy - b * h * 0.35 * (std::f64::consts::PI * (u + 0.5 * v)).sin(),
        ),
        // The top and bottom edges waving in opposite ways: tall on one side, narrow on the
        // other.
        WarpStyle::Fish => (dx, dy - v * b * h * 0.4 * (std::f64::consts::PI * u).sin()),
        // Every line rising from one side to the other along a quarter of a sine.
        WarpStyle::Rise => (
            dx,
            dy - b * h * 0.5 * (std::f64::consts::FRAC_PI_2 * u).sin(),
        ),
        // Pushed out from the center, the most halfway, the corners staying.
        WarpStyle::Fisheye => {
            let k = 1.0 + b * 0.5 * (1.0 - (u * u + v * v) / 2.0).max(0.0);
            (dx * k, dy * k)
        }
        // Every edge pushed out in its middle, as a pillow.
        WarpStyle::Inflate => (
            dx * (1.0 + b * 0.3 * (1.0 - v * v)),
            dy * (1.0 + b * 0.3 * (1.0 - u * u)),
        ),
        // Drawn in across the middle and stretched along it.
        WarpStyle::Squeeze => (
            dx * (1.0 - b * 0.3 * (1.0 - v * v)),
            dy * (1.0 + b * 0.3 * (1.0 - u * u)),
        ),
        // Turned around the center, the more the closer to it.
        WarpStyle::Twist => {
            let d = (u * u + v * v).sqrt() / std::f64::consts::SQRT_2;
            let angle = b * std::f64::consts::FRAC_PI_2 * (1.0 - d).max(0.0);
            let (sin, cos) = angle.sin_cos();
            // Turned in normalized units, so that a non-square frame turns evenly.
            let (ru, rv) = (u * cos - v * sin, u * sin + v * cos);
            (ru * w, rv * h)
        }
    };
    if preset.vertical {
        (cx + my, cy + mx)
    } else {
        (cx + mx, cy + my)
    }
}

/// One triangle of a placed warp: its corners in the document and the affine map from the
/// document to the content's pixels inside it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Triangle {
    pub corners: [(f64, f64); 3],
    pub to_content: Affine,
}

impl Triangle {
    /// Whether document point `(x, y)` is inside (edges included).
    pub fn contains(&self, x: f64, y: f64) -> bool {
        let [a, b, c] = self.corners;
        let edge = |p: (f64, f64), q: (f64, f64)| (q.0 - p.0) * (y - p.1) - (q.1 - p.1) * (x - p.0);
        let (e0, e1, e2) = (edge(a, b), edge(b, c), edge(c, a));
        (e0 >= 0.0 && e1 >= 0.0 && e2 >= 0.0) || (e0 <= 0.0 && e1 <= 0.0 && e2 <= 0.0)
    }
}

/// A warp placed in the document by a transform: its triangles and the bins that find the one
/// under a point.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedMesh {
    triangles: Vec<Triangle>,
    /// The triangles' box in the document.
    bounds: [f64; 4],
    /// The triangles' box in the content.
    source_bounds: [f64; 4],
    bins: Bins,
}

/// A grid of bins over the mesh's box, each listing the triangles that overlap it.
#[derive(Debug, Clone, PartialEq)]
pub struct Bins {
    pub origin: (f64, f64),
    pub cell: (f64, f64),
    pub columns: u32,
    pub rows: u32,
    /// Where each bin's triangles start in `items` (one more entry than bins).
    pub start: Vec<u32>,
    pub items: Vec<u32>,
}

impl PlacedMesh {
    /// `warp` placed by `transform`, its border extended by `margin` content pixels (the
    /// resampling filter's reach, so that the layer's edges are antialiased as unwarped ones
    /// are). `None` where the transform sends a point beyond its horizon line.
    pub fn new(warp: &Warp, transform: Projective, margin: f64) -> Option<Self> {
        match warp {
            Warp::Grid(grid) => Self::from_grids(&[grid.vertices(margin)], transform),
            Warp::Planes(planes) => Self::from_grids(&planes.vertices(margin), transform),
        }
    }

    /// The triangles of regular vertex grids, in order (where they overlap, the last one's
    /// show), placed by `transform`.
    fn from_grids(grids: &[VertexGrid], transform: Projective) -> Option<Self> {
        let mut triangles = Vec::with_capacity(grids.iter().map(VertexGrid::triangles).sum());
        let mut bounds = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        let mut source_bounds = bounds;
        for grid in grids {
            // Each vertex: in the document, and in the content before the warp.
            let mut vertices = Vec::with_capacity(grid.vertices.len());
            for &((wx, wy), source) in &grid.vertices {
                if transform.w(wx, wy) <= 0.0 {
                    return None;
                }
                vertices.push((transform.apply(wx, wy), source));
            }
            let stride = grid.stride;
            let rows = vertices.len() / stride;
            for j in 0..rows.saturating_sub(1) {
                for i in 0..stride - 1 {
                    let v = |di: usize, dj: usize| vertices[(j + dj) * stride + i + di];
                    let (v00, v10, v01, v11) = (v(0, 0), v(1, 0), v(0, 1), v(1, 1));
                    triangles.extend(triangle(v00, v10, v11));
                    triangles.extend(triangle(v00, v11, v01));
                }
            }
            for ((x, y), (sx, sy)) in &vertices {
                bounds = [
                    bounds[0].min(*x),
                    bounds[1].min(*y),
                    bounds[2].max(*x),
                    bounds[3].max(*y),
                ];
                source_bounds = [
                    source_bounds[0].min(*sx),
                    source_bounds[1].min(*sy),
                    source_bounds[2].max(*sx),
                    source_bounds[3].max(*sy),
                ];
            }
        }
        if !bounds.iter().all(|v| v.is_finite()) {
            return None;
        }
        let bins = Bins::new(&triangles, bounds);
        Some(Self {
            triangles,
            bounds,
            source_bounds,
            bins,
        })
    }

    pub fn triangles(&self) -> &[Triangle] {
        &self.triangles
    }

    pub fn bins(&self) -> &Bins {
        &self.bins
    }

    /// The box of the triangles in the document, `[x0, y0, x1, y1]`.
    pub fn bounds(&self) -> [f64; 4] {
        self.bounds
    }

    /// The box of the triangles in the content.
    pub fn source_bounds(&self) -> [f64; 4] {
        self.source_bounds
    }

    /// The triangle under document point `(x, y)`: the last one listed where the warp folds
    /// over itself (the GPU does the same).
    pub fn find(&self, x: f64, y: f64) -> Option<&Triangle> {
        let bin = self.bins.at(x, y)?;
        let (from, to) = (
            self.bins.start[bin] as usize,
            self.bins.start[bin + 1] as usize,
        );
        self.bins.items[from..to]
            .iter()
            .rev()
            .map(|&k| &self.triangles[k as usize])
            .find(|t| t.contains(x, y))
    }

    /// The content area (pixels) that the document `area` shows: the triangles overlapping it.
    pub fn source_area(&self, area: [f64; 4]) -> [f64; 4] {
        let mut out = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for t in &self.triangles {
            let b = corners_box(&t.corners);
            if b[0] > area[2] || b[2] < area[0] || b[1] > area[3] || b[3] < area[1] {
                continue;
            }
            let content = t.corners.map(|(x, y)| t.to_content.apply(x, y));
            let c = corners_box(&content);
            out = [
                out[0].min(c[0]),
                out[1].min(c[1]),
                out[2].max(c[2]),
                out[3].max(c[3]),
            ];
        }
        out
    }
}

/// The frame a pixel layer's warp is made over: its current warp's, else the box of its pixels
/// that are not transparent (Photoshop warps what shows), else its whole image. `None` for the
/// other layers.
pub fn frame_of(layer: &crate::document::Layer) -> Option<[f64; 4]> {
    let crate::document::LayerContent::Raster { image, .. } = &layer.content else {
        return None;
    };
    if let Some(warp) = &layer.warp {
        return Some(warp.frame());
    }
    let image = image.get();
    let r = image
        .content_bounds()
        .unwrap_or_else(|| image.size().bounds());
    let (x, y) = (f64::from(r.x), f64::from(r.y));
    Some([x, y, x + f64::from(r.width), y + f64::from(r.height)])
}

/// The grid Edit > Transform > Warp shows for a pixel layer: its warp, or a plain 3 × 3 grid over
/// [`frame_of`] when it has none. `None` for the other layers, and for a layer warped by
/// Perspective Warp's planes.
pub fn grid_of(layer: &crate::document::Layer) -> Option<Grid> {
    match layer.warp.as_deref() {
        Some(Warp::Grid(grid)) => Some(grid.clone()),
        Some(Warp::Planes(_)) => None,
        None => Grid::identity(frame_of(layer)?, 3, 3).ok(),
    }
}

/// The content point (pixels) under document point `(x, y)` of a layer warped by `warp` then
/// placed by `transform`: `None` where the layer is not.
pub fn content_point(warp: &Warp, transform: Projective, x: f64, y: f64) -> Option<(f64, f64)> {
    let mesh = PlacedMesh::new(warp, transform, 0.0)?;
    mesh.find(x, y).map(|t| t.to_content.apply(x, y))
}

fn corners_box(points: &[(f64, f64); 3]) -> [f64; 4] {
    points.iter().fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |[x0, y0, x1, y1], &(x, y)| [x0.min(x), y0.min(y), x1.max(x), y1.max(y)],
    )
}

type Vertex = ((f64, f64), (f64, f64));

/// The triangle of three vertices (document, content), `None` when it is degenerate.
fn triangle(a: Vertex, b: Vertex, c: Vertex) -> Option<Triangle> {
    let ((ax, ay), (au, av)) = a;
    let ((bx, by), (bu, bv)) = b;
    let ((cx, cy), (cu, cv)) = c;
    let (d1x, d1y, d2x, d2y) = (bx - ax, by - ay, cx - ax, cy - ay);
    let det = d1x * d2y - d2x * d1y;
    if det.is_nan() || det.abs() <= 1e-12 {
        return None;
    }
    // L·D = C, D = [d1 d2], C = [c1 c2] (columns): L = C·D⁻¹.
    let (inv_a, inv_b, inv_c, inv_d) = (d2y / det, -d1y / det, -d2x / det, d1x / det);
    let (c1u, c1v, c2u, c2v) = (bu - au, bv - av, cu - au, cv - av);
    let la = c1u * inv_a + c2u * inv_b;
    let lc = c1u * inv_c + c2u * inv_d;
    let lb = c1v * inv_a + c2v * inv_b;
    let ld = c1v * inv_c + c2v * inv_d;
    let to_content = Affine {
        a: la,
        b: lb,
        c: lc,
        d: ld,
        e: au - la * ax - lc * ay,
        f: av - lb * ax - ld * ay,
    };
    Some(Triangle {
        corners: [(ax, ay), (bx, by), (cx, cy)],
        to_content,
    })
}

impl Bins {
    fn new(triangles: &[Triangle], bounds: [f64; 4]) -> Self {
        let side = ((triangles.len() as f64).sqrt().ceil() as u32).clamp(1, MAX_BINS);
        let (columns, rows) = (side, side);
        let cell = (
            ((bounds[2] - bounds[0]) / f64::from(columns)).max(1e-6),
            ((bounds[3] - bounds[1]) / f64::from(rows)).max(1e-6),
        );
        let origin = (bounds[0], bounds[1]);
        let range = |lo: f64, hi: f64, o: f64, c: f64, n: u32| {
            let a = (((lo - o) / c).floor().max(0.0) as u32).min(n - 1);
            let b = (((hi - o) / c).floor().max(0.0) as u32).min(n - 1);
            a..=b
        };
        let mut lists = vec![Vec::new(); (columns * rows) as usize];
        for (k, t) in triangles.iter().enumerate() {
            let b = corners_box(&t.corners);
            for row in range(b[1], b[3], origin.1, cell.1, rows) {
                for col in range(b[0], b[2], origin.0, cell.0, columns) {
                    lists[(row * columns + col) as usize].push(k as u32);
                }
            }
        }
        let mut start = Vec::with_capacity(lists.len() + 1);
        let mut items = Vec::new();
        for list in lists {
            start.push(items.len() as u32);
            items.extend(list);
        }
        start.push(items.len() as u32);
        Self {
            origin,
            cell,
            columns,
            rows,
            start,
            items,
        }
    }

    /// The bin of document point `(x, y)`; `None` outside the grid.
    fn at(&self, x: f64, y: f64) -> Option<usize> {
        let (fx, fy) = (
            (x - self.origin.0) / self.cell.0,
            (y - self.origin.1) / self.cell.1,
        );
        if !(fx >= 0.0 && fy >= 0.0) {
            return None;
        }
        let (col, row) = (fx.floor() as u64, fy.floor() as u64);
        // The far edge belongs to the last bin.
        let col = if col == u64::from(self.columns) && fx <= f64::from(self.columns) {
            col - 1
        } else {
            col
        };
        let row = if row == u64::from(self.rows) && fy <= f64::from(self.rows) {
            row - 1
        } else {
            row
        };
        (col < u64::from(self.columns) && row < u64::from(self.rows))
            .then(|| (row * u64::from(self.columns) + col) as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: [f64; 4] = [10.0, 20.0, 110.0, 70.0];

    fn close(a: (f64, f64), b: (f64, f64)) -> bool {
        (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9
    }

    fn boxes_close(a: [f64; 4], b: [f64; 4]) -> bool {
        a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-9)
    }

    #[test]
    fn the_identity_grid_changes_nothing() {
        let warp = Grid::identity(FRAME, 3, 3).unwrap();
        for (s, t) in [(0.0, 0.0), (1.5, 0.25), (3.0, 3.0), (2.2, 1.7)] {
            assert!(close(warp.eval(s, t), warp.source(s, t)), "{s} {t}");
        }
        let mesh = PlacedMesh::new(&warp.clone().into(), Projective::IDENTITY, 0.0).unwrap();
        let t = mesh.find(50.5, 40.5).unwrap();
        assert!(close(t.to_content.apply(50.5, 40.5), (50.5, 40.5)));
        assert!(boxes_close(mesh.bounds(), FRAME));
        assert!(mesh.find(5.0, 40.0).is_none());
    }

    #[test]
    fn a_placed_mesh_maps_through_the_transform() {
        let warp = Grid::identity(FRAME, 2, 1).unwrap();
        let transform: Projective = Affine::translation(100.0, -5.0).into();
        let mesh = PlacedMesh::new(&warp.clone().into(), transform, 0.0).unwrap();
        let t = mesh.find(150.0, 30.0).unwrap();
        assert!(close(t.to_content.apply(150.0, 30.0), (50.0, 35.0)));
        assert!(boxes_close(mesh.bounds(), [110.0, 15.0, 210.0, 65.0]));
    }

    #[test]
    fn the_margin_extends_the_border() {
        let warp = Grid::identity(FRAME, 3, 3).unwrap();
        let mesh = PlacedMesh::new(&warp.clone().into(), Projective::IDENTITY, 2.0).unwrap();
        let [x0, y0, x1, y1] = mesh.bounds();
        assert!((x0 - 8.0).abs() < 1e-9 && (y0 - 18.0).abs() < 1e-9);
        assert!((x1 - 112.0).abs() < 1e-9 && (y1 - 72.0).abs() < 1e-9);
        let t = mesh.find(9.0, 19.0).unwrap();
        assert!(close(t.to_content.apply(9.0, 19.0), (9.0, 19.0)));
    }

    #[test]
    fn every_style_keeps_a_zero_bend_plain_and_bends_otherwise() {
        for style in WarpStyle::ALL {
            assert_eq!(WarpStyle::from_id(style.id()), Some(style));
            for vertical in [false, true] {
                let plain = WarpPreset {
                    style,
                    bend: 0.0,
                    vertical,
                };
                let warp = Grid::from_preset(FRAME, plain).unwrap();
                let identity = Grid::identity(FRAME, 3, 3).unwrap();
                for (a, b) in warp.points().iter().zip(identity.points()) {
                    assert!(close(*a, *b), "{style:?} {a:?} {b:?}");
                }
                let bent = Grid::from_preset(FRAME, WarpPreset { bend: 0.5, ..plain }).unwrap();
                assert_ne!(bent.points(), identity.points(), "{style:?}");
                assert!(PlacedMesh::new(&bent.clone().into(), Projective::IDENTITY, 2.0).is_some());
            }
        }
    }

    #[test]
    fn the_other_styles_move_the_edges_they_name() {
        let grid = |style| {
            let preset = WarpPreset {
                style,
                bend: 0.5,
                vertical: false,
            };
            Grid::from_preset(FRAME, preset).unwrap()
        };
        let plain = Grid::identity(FRAME, 3, 3).unwrap();
        let y = |w: &Grid, s, t| w.eval(s, t).1;
        let x = |w: &Grid, s, t| w.eval(s, t).0;
        // Arc Lower: the bottom bends down, the top stays; Arc Upper the other way round.
        let lower = grid(WarpStyle::ArcLower);
        assert!(y(&lower, 1.5, 3.0) > y(&plain, 1.5, 3.0) + 1.0);
        assert!(close(lower.eval(1.5, 0.0), plain.eval(1.5, 0.0)));
        let upper = grid(WarpStyle::ArcUpper);
        assert!(y(&upper, 1.5, 0.0) < y(&plain, 1.5, 0.0) - 1.0);
        assert!(close(upper.eval(1.5, 3.0), plain.eval(1.5, 3.0)));
        // Arch: the middle raised alike from top to bottom, the sides upright.
        let arch = grid(WarpStyle::Arch);
        let raised = y(&plain, 1.5, 0.0) - y(&arch, 1.5, 0.0);
        assert!(raised > 1.0);
        assert!((y(&plain, 1.5, 3.0) - y(&arch, 1.5, 3.0) - raised).abs() < 1e-9);
        assert!(close(arch.eval(0.0, 1.5), plain.eval(0.0, 1.5)));
        // Shells: the far edge drawn in.
        let shell = grid(WarpStyle::ShellLower);
        assert!(x(&shell, 0.0, 0.0) > x(&plain, 0.0, 0.0));
        assert!(close(shell.eval(0.0, 3.0), plain.eval(0.0, 3.0)));
        let shell = grid(WarpStyle::ShellUpper);
        assert!(x(&shell, 0.0, 3.0) > x(&plain, 0.0, 3.0));
        // Fish: taller on the left than on the right.
        let fish = grid(WarpStyle::Fish);
        let height = |w: &Grid, s| y(w, s, 3.0) - y(w, s, 0.0);
        assert!(height(&fish, 0.75) > height(&fish, 2.25) + 1.0);
        // Rise: the right side higher than the left.
        let rise = grid(WarpStyle::Rise);
        assert!(y(&rise, 3.0, 1.5) < y(&rise, 0.0, 1.5) - 1.0);
        // Fisheye: the center and the corners stay, the edges' middles go out.
        let fisheye = grid(WarpStyle::Fisheye);
        assert!(close(fisheye.eval(1.5, 1.5), plain.eval(1.5, 1.5)));
        assert!(close(fisheye.eval(0.0, 0.0), plain.eval(0.0, 0.0)));
        assert!(y(&fisheye, 1.5, 0.0) < y(&plain, 1.5, 0.0) - 1.0);
        // Inflate pushes the sides out, Squeeze draws them in.
        assert!(x(&grid(WarpStyle::Inflate), 0.0, 1.5) < x(&plain, 0.0, 1.5) - 1.0);
        assert!(x(&grid(WarpStyle::Squeeze), 0.0, 1.5) > x(&plain, 0.0, 1.5) + 1.0);
    }

    #[test]
    fn arc_bends_the_top_edge_up_in_the_middle() {
        let preset = WarpPreset {
            style: WarpStyle::Arc,
            bend: 0.5,
            vertical: false,
        };
        let warp = Grid::from_preset(FRAME, preset).unwrap();
        let (_, top_middle) = warp.eval(1.5, 0.0);
        let (_, top_corner) = warp.eval(0.0, 0.0);
        assert!(top_middle < top_corner);
        // Symmetric about the vertical center line.
        let (lx, ly) = warp.eval(0.0, 0.0);
        let (rx, ry) = warp.eval(3.0, 0.0);
        assert!((lx - 10.0 - (110.0 - rx)).abs() < 1e-9 && (ly - ry).abs() < 1e-9);
    }

    #[test]
    fn invalid_grids_are_refused() {
        assert!(Grid::new(FRAME, 1, 1, vec![(0.0, 0.0); 15], None).is_err());
        assert!(Grid::new([0.0, 0.0, 0.0, 1.0], 1, 1, vec![(0.0, 0.0); 16], None).is_err());
        assert!(Grid::new(FRAME, 0, 1, vec![], None).is_err());
        let mut points = Grid::identity(FRAME, 1, 1).unwrap().points().to_vec();
        points[3].0 = f64::NAN;
        assert!(Grid::new(FRAME, 1, 1, points, None).is_err());
        let preset = WarpPreset {
            style: WarpStyle::Flag,
            bend: 1.5,
            vertical: false,
        };
        assert!(Grid::from_preset(FRAME, preset).is_err());
    }

    #[test]
    fn folds_show_the_last_triangle() {
        // Every point of the right column pulled onto the left one's: the right patch folds
        // back over the left.
        let mut points = Grid::identity(FRAME, 1, 1).unwrap().points().to_vec();
        for row in 0..4 {
            points[row * 4 + 3].0 = 10.0;
            points[row * 4 + 2].0 = 10.0;
        }
        let warp = Grid::new(FRAME, 1, 1, points, None).unwrap();
        let mesh = PlacedMesh::new(&warp.clone().into(), Projective::IDENTITY, 0.0).unwrap();
        let found = mesh.find(12.0, 45.0).unwrap();
        let last = mesh
            .triangles()
            .iter()
            .rposition(|t| t.contains(12.0, 45.0))
            .unwrap();
        assert_eq!(found, &mesh.triangles()[last]);
    }
}
