//! Liquify (ADR 0037): a displacement field warping a raster layer, edited by brush tools.
//!
//! - The [`Field`] holds, in the layer's own pixels, where each output pixel reads its color
//!   from: `output(x) = input(x + d(x))` (backward mapping, bilinear in premultiplied blend
//!   values), plus a freeze mask of the same grid. Both are stored on a grid one node per
//!   `cell` pixels (1, 2 or 4 by the layer's size, so that the field never takes more than about
//!   120 MB whatever the layer), interpolated between nodes, in tiles aligned with the layer's
//!   own (`TILE_SIZE` pixels a side) and sparse: a tile no stroke touched does not exist.
//! - A [`Stroke`] turns pointer movement and time into dabs of a [`Tool`]; each dab composes its
//!   shift with the field (`d'(x) = s(x) + d(x + s(x))`), so the field stays exact however many
//!   strokes pile up, and freezes protect the nodes they cover.
//! - Evaluation ([`warp_layer`]) is tile by tile with no margin to copy: pixels are read where
//!   the field says, from the input's tiles; tiles whose field neighbourhood is empty are the
//!   input's own tiles, shared. The same sampling draws crops at a pyramid level (looks, ADR
//!   0034) and the workspace's frames ([`frame`]).

use std::collections::HashMap;
use std::f64::consts::PI;
use std::sync::Arc;

use crate::blend::BlendSpace;
use crate::color::{AlphaMode, ChannelLayout, ColorSpace, PixelFormat, SampleType};
use crate::convert::{ConversionReport, ConvertOptions, Converter, WHITE_MATTE};
use crate::geom::Size;
use crate::raster::{RasterError, RasterImage, TILE_SIZE, pad_tile, parallel_for_each};
use crate::stack::PremulPixels;
use crate::tile::TileCoord;

/// The brush sizes, in pixels (Photoshop's range).
pub const MIN_BRUSH_SIZE: f32 = 1.0;
pub const MAX_BRUSH_SIZE: f32 = 15000.0;

/// Dabs are placed every this fraction of the brush's size along a stroke.
const SPACING: f64 = 0.06;
/// What a tool acting while the pointer is held still does per second at a rate of 100.
const HOLD_SPEED: f64 = 2.0;
/// A twirl turns by this angle (radians) at the center for an amount of 1.
const TWIRL_ANGLE: f64 = PI / 2.0;
/// Pucker and Bloat shift by this fraction of the distance to the center for an amount of 1.
const PINCH: f64 = 0.5;
/// The most Pucker and Bloat shift at once (a fraction of the distance), so that the map stays
/// invertible.
const MAX_PINCH: f64 = 0.9;

/// A dab covering at least this many nodes is computed on every core.
const PARALLEL_NODES: usize = 65_536;
/// The largest amount a dab takes (a second held still at the highest rate is 2).
const MAX_AMOUNT: f64 = 10.0;
/// The most dabs one movement of the pointer lays: a jump across a huge layer drops the first
/// part of its path rather than spending minutes on it.
const MAX_DABS_PER_MOVE: f64 = 100_000.0;

/// A brush's settings, Photoshop's: its size, its density (how the edge feathers, 0 to 100), its
/// pressure (how strongly dragging distorts, 1 to 100) and its rate (how fast the tools that act
/// while the pointer is held still do, 0 to 100).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Brush {
    pub size: f32,
    pub density: f32,
    pub pressure: f32,
    pub rate: f32,
}

impl Default for Brush {
    /// Photoshop's defaults.
    fn default() -> Self {
        Self {
            size: 100.0,
            density: 50.0,
            pressure: 100.0,
            rate: 80.0,
        }
    }
}

impl Brush {
    pub fn is_valid(&self) -> bool {
        (MIN_BRUSH_SIZE..=MAX_BRUSH_SIZE).contains(&self.size)
            && (0.0..=100.0).contains(&self.density)
            && (1.0..=100.0).contains(&self.pressure)
            && (0.0..=100.0).contains(&self.rate)
    }

    /// How much of the effect reaches a point at `r` brush radii from the center, 1 to 0: whole
    /// within the plateau a high density leaves, then a cosine down to nothing at the edge.
    pub fn falloff(&self, r: f64) -> f64 {
        if r >= 1.0 {
            return 0.0;
        }
        let plateau = 0.9 * f64::from(self.density) / 100.0;
        if r <= plateau {
            return 1.0;
        }
        0.5 * (1.0 + (PI * (r - plateau) / (1.0 - plateau)).cos())
    }
}

/// What a stroke does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    /// Pixels follow the brush.
    ForwardWarp,
    /// The field goes back toward nothing under the brush.
    Reconstruct,
    /// The field is smoothed under the brush.
    Smooth,
    TwirlClockwise,
    TwirlCounterclockwise,
    /// Pixels move toward the brush's center.
    Pucker,
    /// Pixels move away from the brush's center.
    Bloat,
    /// Pixels move to the left of the direction the brush travels.
    PushLeft,
    /// Protect what is under the brush from the other tools.
    Freeze,
    /// Remove that protection.
    Thaw,
}

impl Tool {
    pub const IDS: [&'static str; 10] = [
        "forwardWarp",
        "reconstruct",
        "smooth",
        "twirlClockwise",
        "twirlCounterclockwise",
        "pucker",
        "bloat",
        "pushLeft",
        "freeze",
        "thaw",
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::ForwardWarp => "forwardWarp",
            Self::Reconstruct => "reconstruct",
            Self::Smooth => "smooth",
            Self::TwirlClockwise => "twirlClockwise",
            Self::TwirlCounterclockwise => "twirlCounterclockwise",
            Self::Pucker => "pucker",
            Self::Bloat => "bloat",
            Self::PushLeft => "pushLeft",
            Self::Freeze => "freeze",
            Self::Thaw => "thaw",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        [
            Self::ForwardWarp,
            Self::Reconstruct,
            Self::Smooth,
            Self::TwirlClockwise,
            Self::TwirlCounterclockwise,
            Self::Pucker,
            Self::Bloat,
            Self::PushLeft,
            Self::Freeze,
            Self::Thaw,
        ]
        .into_iter()
        .find(|tool| tool.id() == id)
    }

    /// Whether it acts while the pointer is held still (at the brush's rate).
    pub fn acts_when_held(self) -> bool {
        matches!(
            self,
            Self::Reconstruct
                | Self::Smooth
                | Self::TwirlClockwise
                | Self::TwirlCounterclockwise
                | Self::Pucker
                | Self::Bloat
        )
    }
}

/// Why a field was refused.
#[derive(Debug, Clone, PartialEq)]
pub enum FieldError {
    /// A cell that is not a power of two up to `TILE_SIZE`.
    InvalidCell(u32),
    /// Images that do not fit the layer and the cell.
    SizeMismatch,
    Raster(RasterError),
}

impl std::fmt::Display for FieldError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidCell(cell) => write!(f, "a liquify field cannot have {cell}-pixel cells"),
            Self::SizeMismatch => write!(f, "a liquify field of another size than its layer"),
            Self::Raster(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for FieldError {}

impl From<RasterError> for FieldError {
    fn from(e: RasterError) -> Self {
        Self::Raster(e)
    }
}

/// The displacements of one tile's nodes (`tile_nodes²`, row-major): where the output reads
/// from, relative to itself, in the layer's pixels.
#[derive(Debug, Clone)]
struct Nodes(Vec<[f32; 2]>);

/// The freeze mask of one tile's nodes: 0 (free) to 255 (frozen).
#[derive(Debug, Clone)]
struct Frozen(Vec<u8>);

/// A displacement field and freeze mask over a layer (see the module's documentation). Cloning
/// shares every tile; a stroke copies the tiles it changes.
#[derive(Debug, Clone)]
pub struct Field {
    size: Size,
    cell: u32,
    /// Tiles of the layer, by column and row (the field's own tiles are the same).
    columns: u32,
    rows: u32,
    displacement: Vec<Option<Arc<Nodes>>>,
    frozen: Vec<Option<Arc<Frozen>>>,
}

impl Field {
    /// The cell of a layer of `size`: one node per pixel up to 4 million pixels, one per 2×2
    /// up to 32 million, one per 4×4 beyond, so that a field holds at most about 120 MB (a
    /// 233-megapixel layer: 14.6 million nodes of 8 bytes) and a brush of a few dozen pixels
    /// still spans a few nodes.
    pub fn cell_for(size: Size) -> u32 {
        let pixels = size.pixel_count();
        if pixels <= 4_000_000 {
            1
        } else if pixels <= 32_000_000 {
            2
        } else {
            4
        }
    }

    /// An empty field over a layer of `size`, with the cell [`Self::cell_for`] gives.
    pub fn new(size: Size) -> Self {
        Self::empty(size, Self::cell_for(size))
    }

    /// An empty field with `cell` pixels per node (a power of two up to `TILE_SIZE`).
    pub fn with_cell(size: Size, cell: u32) -> Result<Self, FieldError> {
        if cell == 0 || !cell.is_power_of_two() || cell > TILE_SIZE {
            return Err(FieldError::InvalidCell(cell));
        }
        Ok(Self::empty(size, cell))
    }

    fn empty(size: Size, cell: u32) -> Self {
        let columns = size.width.div_ceil(TILE_SIZE);
        let rows = size.height.div_ceil(TILE_SIZE);
        let count = columns as usize * rows as usize;
        Self {
            size,
            cell,
            columns,
            rows,
            displacement: vec![None; count],
            frozen: vec![None; count],
        }
    }

    /// The layer's size in pixels.
    pub fn size(&self) -> Size {
        self.size
    }

    /// Pixels per node, a side.
    pub fn cell(&self) -> u32 {
        self.cell
    }

    /// Nodes in a tile, a side.
    fn tile_nodes(&self) -> usize {
        (TILE_SIZE / self.cell) as usize
    }

    /// The nodes of the whole grid, columns and rows.
    pub fn nodes(&self) -> (u32, u32) {
        (
            self.size.width.div_ceil(self.cell),
            self.size.height.div_ceil(self.cell),
        )
    }

    fn slot(&self, tile_col: usize, tile_row: usize) -> usize {
        tile_row * self.columns as usize + tile_col
    }

    /// The displacement at node (`i`, `j`), the grid's edges repeating outward.
    fn node(&self, i: i64, j: i64) -> [f32; 2] {
        let (nw, nh) = self.nodes();
        let i = i.clamp(0, i64::from(nw) - 1) as usize;
        let j = j.clamp(0, i64::from(nh) - 1) as usize;
        let t = self.tile_nodes();
        match &self.displacement[self.slot(i / t, j / t)] {
            Some(tile) => tile.0[(j % t) * t + i % t],
            None => [0.0; 2],
        }
    }

    /// The freeze at node (`i`, `j`), 0 to 255.
    fn frozen_node(&self, i: i64, j: i64) -> u8 {
        let (nw, nh) = self.nodes();
        let i = i.clamp(0, i64::from(nw) - 1) as usize;
        let j = j.clamp(0, i64::from(nh) - 1) as usize;
        let t = self.tile_nodes();
        match &self.frozen[self.slot(i / t, j / t)] {
            Some(tile) => tile.0[(j % t) * t + i % t],
            None => 0,
        }
    }

    /// Where the output at layer point `p` reads from, relative to `p`: the nodes around it
    /// interpolated.
    pub fn displacement_at(&self, p: [f64; 2]) -> [f64; 2] {
        let cell = f64::from(self.cell);
        let (u, v) = (p[0] / cell - 0.5, p[1] / cell - 0.5);
        let (i, j) = (u.floor(), v.floor());
        let (a, b) = (u - i, v - j);
        let (i, j) = (i as i64, j as i64);
        let (n00, n10, n01, n11) = (
            self.node(i, j),
            self.node(i + 1, j),
            self.node(i, j + 1),
            self.node(i + 1, j + 1),
        );
        std::array::from_fn(|c| {
            let top = f64::from(n00[c]) * (1.0 - a) + f64::from(n10[c]) * a;
            let bottom = f64::from(n01[c]) * (1.0 - a) + f64::from(n11[c]) * a;
            top * (1.0 - b) + bottom * b
        })
    }

    /// How frozen layer point `p` is, 0 to 1.
    pub fn frozen_at(&self, p: [f64; 2]) -> f64 {
        let cell = f64::from(self.cell);
        let (u, v) = (p[0] / cell - 0.5, p[1] / cell - 0.5);
        let (i, j) = (u.floor(), v.floor());
        let (a, b) = (u - i, v - j);
        let (i, j) = (i as i64, j as i64);
        let at = |i, j| f64::from(self.frozen_node(i, j)) / 255.0;
        let top = at(i, j) * (1.0 - a) + at(i + 1, j) * a;
        let bottom = at(i, j + 1) * (1.0 - a) + at(i + 1, j + 1) * a;
        top * (1.0 - b) + bottom * b
    }

    /// Whether no node displaces anything.
    pub fn is_identity(&self) -> bool {
        self.displacement
            .iter()
            .flatten()
            .all(|tile| tile.0.iter().all(|d| *d == [0.0; 2]))
    }

    /// Whether some node is frozen.
    pub fn has_frozen(&self) -> bool {
        self.frozen
            .iter()
            .flatten()
            .any(|tile| tile.0.iter().any(|f| *f != 0))
    }

    /// Whether `other` has the very tiles this field has (clones of one another, nothing
    /// stroked since): cheap, and exact for what a workspace needs to know, whether it changed.
    pub fn shares_tiles_with(&self, other: &Field) -> bool {
        fn same<T>(a: &[Option<Arc<T>>], b: &[Option<Arc<T>>]) -> bool {
            a.len() == b.len()
                && a.iter().zip(b).all(|(x, y)| match (x, y) {
                    (None, None) => true,
                    (Some(x), Some(y)) => Arc::ptr_eq(x, y),
                    _ => false,
                })
        }
        self.size == other.size
            && same(&self.displacement, &other.displacement)
            && same(&self.frozen, &other.frozen)
    }

    /// The farthest any pixel reads from, in the layer's pixels: how much margin a crop of the
    /// input needs. A bilinear interpolation never exceeds the largest node.
    pub fn reach(&self) -> f64 {
        self.displacement
            .iter()
            .flatten()
            .flat_map(|tile| tile.0.iter())
            .map(|d| f64::from(d[0]).hypot(f64::from(d[1])))
            .fold(0.0, f64::max)
    }

    /// Bytes its tiles take (each shared tile counted once per field).
    pub fn memory_bytes(&self) -> u64 {
        let nodes = self.displacement.iter().flatten().count() * self.tile_nodes().pow(2) * 8;
        let frozen = self.frozen.iter().flatten().count() * self.tile_nodes().pow(2);
        (nodes + frozen) as u64
    }

    /// The same field without the tiles that hold nothing (all zero): what a stroke that went
    /// back leaves, so that evaluating it again shares the input's tiles.
    pub fn pruned(mut self) -> Self {
        for tile in &mut self.displacement {
            if tile
                .as_ref()
                .is_some_and(|t| t.0.iter().all(|d| *d == [0.0; 2]))
            {
                *tile = None;
            }
        }
        for tile in &mut self.frozen {
            if tile.as_ref().is_some_and(|t| t.0.iter().all(|f| *f == 0)) {
                *tile = None;
            }
        }
        self
    }

    /// The field with every displacement taken back (Restore All); the freeze mask stays.
    pub fn restored(&self) -> Self {
        Self {
            displacement: vec![None; self.displacement.len()],
            ..self.clone()
        }
    }

    /// The same layer grown by `offset` whole tiles (columns, rows) before its pixels, to `size`
    /// (ADR 0027): what was displaced moves with its pixels.
    pub fn grown(&self, offset: (u32, u32), size: Size) -> Self {
        let mut grown = Self::empty(size, self.cell);
        for row in 0..self.rows {
            for col in 0..self.columns {
                let (c, r) = (col + offset.0, row + offset.1);
                if c >= grown.columns || r >= grown.rows {
                    continue;
                }
                let (from, to) = (
                    self.slot(col as usize, row as usize),
                    grown.slot(c as usize, r as usize),
                );
                grown.displacement[to] = self.displacement[from].clone();
                grown.frozen[to] = self.frozen[from].clone();
            }
        }
        grown
    }

    // --- Tools -------------------------------------------------------------------------------

    /// One dab of `tool` with `brush` centered on layer point `center`: `motion` is how far the
    /// brush moved (the tools that follow it), `amount` how much the tools that turn, pinch,
    /// restore or smooth do. Nodes beyond the brush's radius are untouched, frozen ones are
    /// protected (partly, where the freeze is).
    pub fn stamp(
        &mut self,
        tool: Tool,
        brush: &Brush,
        center: [f64; 2],
        motion: [f64; 2],
        amount: f64,
    ) {
        if !(center.iter().chain(&motion).all(|v| v.is_finite()) && amount.is_finite()) {
            return;
        }
        let radius = f64::from(brush.size) / 2.0;
        // A dab moves the pixels by at most the brush's size, and does what it does a few times
        // over at most: whatever the pointer or the clock said.
        let length = motion[0].hypot(motion[1]);
        let motion = if length > f64::from(brush.size) {
            let scale = f64::from(brush.size) / length;
            [motion[0] * scale, motion[1] * scale]
        } else {
            motion
        };
        let amount = amount.clamp(0.0, MAX_AMOUNT);
        let cell = f64::from(self.cell);
        let (nw, nh) = self.nodes();
        // The nodes within the brush's square.
        let first = |v: f64| (v / cell - 0.5).ceil();
        let last = |v: f64| (v / cell - 0.5).floor();
        let i0 = first(center[0] - radius).max(0.0) as i64;
        let j0 = first(center[1] - radius).max(0.0) as i64;
        let i1 = last(center[0] + radius).min(f64::from(nw) - 1.0) as i64;
        let j1 = last(center[1] + radius).min(f64::from(nh) - 1.0) as i64;
        if i0 > i1 || j0 > j1 {
            return;
        }
        let at = |i: i64, j: i64| [(i as f64 + 0.5) * cell, (j as f64 + 0.5) * cell];
        let weight = |i: i64, j: i64| {
            let p = at(i, j);
            let r = (p[0] - center[0]).hypot(p[1] - center[1]) / radius;
            brush.falloff(r)
        };

        if matches!(tool, Tool::Freeze | Tool::Thaw) {
            for j in j0..=j1 {
                for i in i0..=i1 {
                    let w = weight(i, j);
                    if w <= 0.0 {
                        continue;
                    }
                    let level = (w * 255.0).round() as u8;
                    let old = self.frozen_node(i, j);
                    let new = if tool == Tool::Freeze {
                        old.max(level)
                    } else {
                        old.min(255 - level)
                    };
                    self.set_frozen(i, j, new);
                }
            }
            return;
        }

        // What the field was around the brush: the shifts read it where the pixels come from.
        let pressure = f64::from(brush.pressure) / 100.0;
        let reading = match tool {
            Tool::ForwardWarp | Tool::PushLeft => motion[0].hypot(motion[1]) * pressure,
            Tool::Pucker | Tool::Bloat => radius * MAX_PINCH,
            _ => 0.0,
        };
        let pad = (reading / cell).ceil() as i64 + 2;
        // Only what is on the grid: beyond it the edge repeats, which reading clamps to.
        let patch = Patch::of(
            self,
            (i0 - pad).max(0),
            (j0 - pad).max(0),
            (i1 + pad).min(i64::from(nw) - 1),
            (j1 + pad).min(i64::from(nh) - 1),
        );
        let (width, height) = ((i1 - i0 + 1) as usize, (j1 - j0 + 1) as usize);
        let mut changed = vec![[0.0f32; 2]; width * height];
        {
            let field = &*self;
            // What node (`i`, `j`) becomes.
            let compute = |i: i64, j: i64| -> [f32; 2] {
                let old = patch.read(i, j);
                let w = weight(i, j);
                if w <= 0.0 {
                    return old;
                }
                let free = 1.0 - f64::from(field.frozen_node(i, j)) / 255.0;
                let k = w * free;
                let p = at(i, j);
                let (rx, ry) = (p[0] - center[0], p[1] - center[1]);
                let shift = match tool {
                    Tool::ForwardWarp => [-motion[0] * pressure * k, -motion[1] * pressure * k],
                    // To the left of the travel (up, on screen, for a drag to the right).
                    Tool::PushLeft => [-motion[1] * pressure * k, motion[0] * pressure * k],
                    Tool::TwirlClockwise | Tool::TwirlCounterclockwise => {
                        let sign = if tool == Tool::TwirlClockwise {
                            1.0
                        } else {
                            -1.0
                        };
                        let angle = -sign * amount * TWIRL_ANGLE * k;
                        let (sin, cos) = angle.sin_cos();
                        [rx * cos - ry * sin - rx, rx * sin + ry * cos - ry]
                    }
                    Tool::Pucker => {
                        let pinch = (amount * PINCH * k).clamp(0.0, MAX_PINCH);
                        [rx * pinch, ry * pinch]
                    }
                    Tool::Bloat => {
                        let pinch = (amount * PINCH * k).clamp(0.0, MAX_PINCH);
                        [-rx * pinch, -ry * pinch]
                    }
                    Tool::Reconstruct | Tool::Smooth | Tool::Freeze | Tool::Thaw => [0.0; 2],
                };
                match tool {
                    Tool::Reconstruct => {
                        let keep = 1.0 - (amount * k).clamp(0.0, 1.0);
                        [
                            (f64::from(old[0]) * keep) as f32,
                            (f64::from(old[1]) * keep) as f32,
                        ]
                    }
                    Tool::Smooth => {
                        let mut sum = [0.0f64; 2];
                        for dj in -1..=1 {
                            for di in -1..=1 {
                                let n = patch.read(i + di, j + dj);
                                sum[0] += f64::from(n[0]);
                                sum[1] += f64::from(n[1]);
                            }
                        }
                        let t = (amount * k).clamp(0.0, 1.0);
                        std::array::from_fn(|c| {
                            let mean = sum[c] / 9.0;
                            (f64::from(old[c]) + (mean - f64::from(old[c])) * t) as f32
                        })
                    }
                    _ => {
                        // The pixel now shown here comes from where this shift reads, which
                        // showed what the old field says there.
                        let (u, v) = (
                            (p[0] + shift[0]) / cell - 0.5,
                            (p[1] + shift[1]) / cell - 0.5,
                        );
                        let there = patch.sample(u, v);
                        [(shift[0] + there[0]) as f32, (shift[1] + there[1]) as f32]
                    }
                }
            };
            let mut rows: Vec<(i64, &mut [[f32; 2]])> = changed
                .chunks_mut(width)
                .enumerate()
                .map(|(n, row)| (j0 + n as i64, row))
                .collect();
            let fill = |(j, row): &mut (i64, &mut [[f32; 2]])| {
                for (n, out) in row.iter_mut().enumerate() {
                    *out = compute(i0 + n as i64, *j);
                }
            };
            // A large brush on every core; a small one is quicker alone.
            if width * height >= PARALLEL_NODES {
                parallel_for_each(&mut rows, fill);
            } else {
                rows.iter_mut().for_each(fill);
            }
        }
        // Written a tile at a time: a tile only the dab's zeros would reach is not made.
        let t = self.tile_nodes() as i64;
        for tj in j0 / t..=j1 / t {
            for ti in i0 / t..=i1 / t {
                let (x0, x1) = (i0.max(ti * t), i1.min(ti * t + t - 1));
                let (y0, y1) = (j0.max(tj * t), j1.min(tj * t + t - 1));
                let span = |j: i64| {
                    let start = (j - j0) as usize * width + (x0 - i0) as usize;
                    start..start + (x1 - x0 + 1) as usize
                };
                let slot = self.slot(ti as usize, tj as usize);
                let tile = &mut self.displacement[slot];
                if tile.is_none()
                    && !(y0..=y1).any(|j| changed[span(j)].iter().any(|d| *d != [0.0; 2]))
                {
                    continue;
                }
                let nodes = Arc::make_mut(
                    tile.get_or_insert_with(|| Arc::new(Nodes(vec![[0.0; 2]; (t * t) as usize]))),
                );
                for j in y0..=y1 {
                    let to = ((j - tj * t) * t + (x0 - ti * t)) as usize;
                    let from = &changed[span(j)];
                    nodes.0[to..to + from.len()].copy_from_slice(from);
                }
            }
        }
    }

    #[cfg(test)]
    fn set_node(&mut self, i: i64, j: i64, value: [f32; 2]) {
        let t = self.tile_nodes();
        let (i, j) = (i as usize, j as usize);
        let slot = self.slot(i / t, j / t);
        let tile = &mut self.displacement[slot];
        if tile.is_none() && value == [0.0; 2] {
            return;
        }
        let nodes = tile.get_or_insert_with(|| Arc::new(Nodes(vec![[0.0; 2]; t * t])));
        let nodes = Arc::make_mut(nodes);
        nodes.0[(j % t) * t + i % t] = value;
    }

    fn set_frozen(&mut self, i: i64, j: i64, value: u8) {
        let t = self.tile_nodes();
        let (i, j) = (i as usize, j as usize);
        let slot = self.slot(i / t, j / t);
        let tile = &mut self.frozen[slot];
        if tile.is_none() && value == 0 {
            return;
        }
        let mask = tile.get_or_insert_with(|| Arc::new(Frozen(vec![0; t * t])));
        Arc::make_mut(mask).0[(j % t) * t + i % t] = value;
    }

    // --- Files -------------------------------------------------------------------------------

    /// The format of the displacement image: `dx` as gray and `dy` as alpha, `f32`, never
    /// converted (the tiles are stored as they are).
    pub fn displacement_format() -> PixelFormat {
        PixelFormat {
            layout: ChannelLayout::GrayAlpha,
            sample: SampleType::F32,
            color_space: ColorSpace::LINEAR_SRGB,
            alpha: AlphaMode::Straight,
        }
    }

    /// The format of the freeze image.
    pub fn frozen_format() -> PixelFormat {
        PixelFormat {
            layout: ChannelLayout::Gray,
            sample: SampleType::U8,
            color_space: ColorSpace::LINEAR_SRGB,
            alpha: AlphaMode::Straight,
        }
    }

    /// The displacements and the freeze as whole images of one pixel per node (what files
    /// store): every tile the field lacks is one shared zero tile.
    pub fn to_images(&self) -> Result<(RasterImage, RasterImage), FieldError> {
        let (nw, nh) = self.nodes();
        let size = Size::new(nw, nh);
        let displacement = self.image_of(size, 8, |slot| {
            self.displacement[slot].as_ref().map(|tile| {
                tile.0
                    .iter()
                    .flat_map(|d| d.iter().flat_map(|v| v.to_ne_bytes()))
                    .collect::<Vec<u8>>()
            })
        });
        let frozen = self.image_of(size, 1, |slot| {
            self.frozen[slot].as_ref().map(|tile| tile.0.clone())
        });
        Ok((
            RasterImage::from_level0_tiles(size, Self::displacement_format(), displacement)?,
            RasterImage::from_level0_tiles(size, Self::frozen_format(), frozen)?,
        ))
    }

    /// The tiles of an image of `size` nodes made of the field's tiles (`bytes` bytes a node):
    /// each image tile holds `cell²` field tiles.
    fn image_of(
        &self,
        size: Size,
        bytes: usize,
        tile: impl Fn(usize) -> Option<Vec<u8>>,
    ) -> Vec<Arc<[u8]>> {
        let t = self.tile_nodes();
        let per = self.cell as usize;
        let side = TILE_SIZE as usize;
        let (image_columns, image_rows) = (
            size.width.div_ceil(TILE_SIZE) as usize,
            size.height.div_ceil(TILE_SIZE) as usize,
        );
        let mut tiles: HashMap<usize, Vec<u8>> = HashMap::new();
        for row in 0..self.rows as usize {
            for col in 0..self.columns as usize {
                let Some(source) = tile(self.slot(col, row)) else {
                    continue;
                };
                let target = tiles
                    .entry((row / per) * image_columns + col / per)
                    .or_insert_with(|| vec![0; side * side * bytes]);
                let (x0, y0) = ((col % per) * t, (row % per) * t);
                for y in 0..t {
                    let from = y * t * bytes;
                    let to = ((y0 + y) * side + x0) * bytes;
                    target[to..to + t * bytes].copy_from_slice(&source[from..from + t * bytes]);
                }
            }
        }
        let zero: Arc<[u8]> = Arc::from(vec![0u8; side * side * bytes]);
        (0..image_columns * image_rows)
            .map(|index| match tiles.remove(&index) {
                Some(mut bytes_of) => {
                    let (col, row) = (index % image_columns, index / image_columns);
                    let valid = |c: usize, count: u32| (count as usize - c * side).min(side);
                    pad_tile(
                        &mut bytes_of,
                        valid(col, size.width),
                        valid(row, size.height),
                        bytes,
                    );
                    Arc::from(bytes_of)
                }
                None => Arc::clone(&zero),
            })
            .collect()
    }

    /// A field read back from its images ([`Self::to_images`]) over a layer of `size`, `cell`
    /// pixels a node.
    pub fn from_images(
        size: Size,
        cell: u32,
        displacement: &RasterImage,
        frozen: &RasterImage,
    ) -> Result<Self, FieldError> {
        let mut field = Self::with_cell(size, cell)?;
        let (nw, nh) = field.nodes();
        let nodes = Size::new(nw, nh);
        if displacement.size() != nodes
            || frozen.size() != nodes
            || displacement.format() != Self::displacement_format()
            || frozen.format() != Self::frozen_format()
        {
            return Err(FieldError::SizeMismatch);
        }
        let t = field.tile_nodes();
        let per = field.cell as usize;
        let side = TILE_SIZE as usize;
        for (image, bytes) in [(displacement, 8usize), (frozen, 1)] {
            let level = &image.levels()[0];
            for row in 0..field.rows as usize {
                for col in 0..field.columns as usize {
                    let coord = TileCoord {
                        col: (col / per) as u32,
                        row: (row / per) as u32,
                    };
                    let Some(source) = level.tile(coord) else {
                        return Err(FieldError::SizeMismatch);
                    };
                    let (x0, y0) = ((col % per) * t, (row % per) * t);
                    let mut part = Vec::with_capacity(t * t * bytes);
                    for y in 0..t {
                        let from = ((y0 + y) * side + x0) * bytes;
                        part.extend_from_slice(&source[from..from + t * bytes]);
                    }
                    if part.iter().all(|b| *b == 0) {
                        continue;
                    }
                    let slot = field.slot(col, row);
                    if bytes == 8 {
                        let nodes = part
                            .as_chunks::<8>()
                            .0
                            .iter()
                            .map(|n| {
                                [
                                    f32::from_ne_bytes([n[0], n[1], n[2], n[3]]),
                                    f32::from_ne_bytes([n[4], n[5], n[6], n[7]]),
                                ]
                            })
                            // A damaged file must not put NaN into the warp.
                            .map(|d| d.map(|v| if v.is_finite() { v } else { 0.0 }))
                            .collect();
                        field.displacement[slot] = Some(Arc::new(Nodes(nodes)));
                    } else {
                        field.frozen[slot] = Some(Arc::new(Frozen(part)));
                    }
                }
            }
        }
        Ok(field)
    }
}

/// The nodes of a region of a field, copied before a dab changes them, read with the grid's
/// edges repeating outward.
struct Patch {
    i0: i64,
    j0: i64,
    width: usize,
    height: usize,
    data: Vec<[f32; 2]>,
}

impl Patch {
    fn of(field: &Field, i0: i64, j0: i64, i1: i64, j1: i64) -> Self {
        let (width, height) = ((i1 - i0 + 1) as usize, (j1 - j0 + 1) as usize);
        let (nw, nh) = field.nodes();
        let t = field.tile_nodes() as i64;
        let mut data = vec![[0.0f32; 2]; width * height];
        for (n, row) in data.chunks_mut(width).enumerate() {
            let j = (j0 + n as i64).clamp(0, i64::from(nh) - 1);
            // The columns on the grid, a run of one tile at a time (an absent tile is zeros).
            let (lo, hi) = (i0.max(0), i1.min(i64::from(nw) - 1));
            let mut i = lo;
            while i <= hi {
                let run = (t - i % t).min(hi - i + 1);
                let at = (i - i0) as usize;
                if let Some(tile) =
                    &field.displacement[field.slot((i / t) as usize, (j / t) as usize)]
                {
                    let from = ((j % t) * t + i % t) as usize;
                    row[at..at + run as usize].copy_from_slice(&tile.0[from..from + run as usize]);
                }
                i += run;
            }
            // Beyond the grid the edge repeats.
            if lo <= hi {
                let (first, last) = (row[(lo - i0) as usize], row[(hi - i0) as usize]);
                row[..(lo - i0) as usize].fill(first);
                row[(hi - i0) as usize + 1..].fill(last);
            }
        }
        Self {
            i0,
            j0,
            width,
            height,
            data,
        }
    }

    fn read(&self, i: i64, j: i64) -> [f32; 2] {
        let x = (i - self.i0).clamp(0, self.width as i64 - 1) as usize;
        let y = (j - self.j0).clamp(0, self.height as i64 - 1) as usize;
        self.data[y * self.width + x]
    }

    /// Bilinear at node coordinates (`u`, `v`).
    fn sample(&self, u: f64, v: f64) -> [f64; 2] {
        let (i, j) = (u.floor(), v.floor());
        let (a, b) = (u - i, v - j);
        let (i, j) = (i as i64, j as i64);
        let (n00, n10, n01, n11) = (
            self.read(i, j),
            self.read(i + 1, j),
            self.read(i, j + 1),
            self.read(i + 1, j + 1),
        );
        std::array::from_fn(|c| {
            let top = f64::from(n00[c]) * (1.0 - a) + f64::from(n10[c]) * a;
            let bottom = f64::from(n01[c]) * (1.0 - a) + f64::from(n11[c]) * a;
            top * (1.0 - b) + bottom * b
        })
    }
}

/// A stroke being drawn: the tool and brush, where the pointer was and what the dabs placed so
/// far left of its path to cover.
#[derive(Debug, Clone)]
pub struct Stroke {
    tool: Tool,
    brush: Brush,
    /// The pointer's last position.
    last: Option<[f64; 2]>,
    /// The position of the last dab, and what the pointer has moved since.
    dabbed: Option<[f64; 2]>,
}

impl Stroke {
    pub fn new(tool: Tool, brush: Brush) -> Self {
        Self {
            tool,
            brush,
            last: None,
            dabbed: None,
        }
    }

    pub fn tool(&self) -> Tool {
        self.tool
    }

    fn spacing(&self) -> f64 {
        (f64::from(self.brush.size) * SPACING).max(1.0)
    }

    /// What a dab covering `distance` pixels of the path does, for the tools that turn, pinch,
    /// restore or smooth while moving: the pressure, in brush radii.
    fn moving_amount(&self, distance: f64) -> f64 {
        f64::from(self.brush.pressure) / 100.0 * distance / (f64::from(self.brush.size) / 2.0)
    }

    /// The pointer went to layer point `to` (a stroke starts at its first call): dabs are laid
    /// along the way, each following the part of the path it covers.
    pub fn move_to(&mut self, field: &mut Field, to: [f64; 2]) {
        if !to.iter().all(|v| v.is_finite()) {
            return;
        }
        let Some(from) = self.last.replace(to) else {
            self.dabbed = Some(to);
            // Freezing and thawing mark where the pointer goes down.
            if matches!(self.tool, Tool::Freeze | Tool::Thaw) {
                field.stamp(self.tool, &self.brush, to, [0.0; 2], 0.0);
            }
            return;
        };
        let spacing = self.spacing();
        let mut at = self.dabbed.unwrap_or(from);
        let (dx, dy) = (to[0] - at[0], to[1] - at[1]);
        let distance = dx.hypot(dy);
        if distance > MAX_DABS_PER_MOVE * spacing {
            let skip = (distance - MAX_DABS_PER_MOVE * spacing) / distance;
            at = [at[0] + dx * skip, at[1] + dy * skip];
        }
        loop {
            let (dx, dy) = (to[0] - at[0], to[1] - at[1]);
            let distance = dx.hypot(dy);
            if distance < spacing {
                break;
            }
            let next = [
                at[0] + dx / distance * spacing,
                at[1] + dy / distance * spacing,
            ];
            self.dab(field, at, next);
            at = next;
        }
        self.dabbed = Some(at);
    }

    /// The pointer's path ends: what it moved since the last dab is laid as a last one.
    pub fn finish(&mut self, field: &mut Field) {
        if let (Some(last), Some(at)) = (self.last, self.dabbed)
            && last != at
            && !matches!(self.tool, Tool::Freeze | Tool::Thaw)
        {
            self.dab(field, at, last);
            self.dabbed = Some(last);
        }
    }

    /// A dab for the pointer's move from `from` to `to`, centered between them.
    fn dab(&self, field: &mut Field, from: [f64; 2], to: [f64; 2]) {
        let motion = [to[0] - from[0], to[1] - from[1]];
        let center = [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0];
        let distance = motion[0].hypot(motion[1]);
        field.stamp(
            self.tool,
            &self.brush,
            center,
            motion,
            self.moving_amount(distance),
        );
    }

    /// The pointer has stayed where it was for `seconds`: the tools that act while held do their
    /// part, at the brush's rate.
    pub fn hold(&mut self, field: &mut Field, seconds: f64) {
        let (Some(at), true) = (self.last, self.tool.acts_when_held()) else {
            return;
        };
        if !seconds.is_finite() || seconds <= 0.0 {
            return;
        }
        let amount = f64::from(self.brush.rate) / 100.0 * seconds * HOLD_SPEED;
        field.stamp(self.tool, &self.brush, at, [0.0; 2], amount);
    }
}

// --- Evaluation ------------------------------------------------------------------------------

/// A raster read where a warp asks: pixels of one pyramid level, which sit `factor` layer pixels
/// apart from `origin` (the layer point of the level's first texel's corner), over a layer of
/// `layer` pixels.
#[derive(Debug)]
pub struct Source<'a> {
    pixels: PremulPixels<'a>,
    origin: [f64; 2],
    factor: f64,
    layer: [f64; 2],
}

/// What a sample is: a pixel as stored (the field there is nothing), or blended values.
enum Sampled<'a> {
    Raw(&'a [u8]),
    Values([f64; 4]),
}

impl<'a> Source<'a> {
    /// `image`'s level `level`, the texels `factor` layer pixels apart from `origin`.
    pub fn new(
        image: &'a RasterImage,
        space: BlendSpace,
        level: usize,
        origin: [f64; 2],
        factor: f64,
        layer: Size,
    ) -> Self {
        Self {
            pixels: PremulPixels::at_level(image, space, level),
            origin,
            factor,
            layer: [f64::from(layer.width), f64::from(layer.height)],
        }
    }

    /// Whether layer point `p` is on the layer.
    fn inside(&self, p: [f64; 2]) -> bool {
        p[0] >= 0.0 && p[1] >= 0.0 && p[0] < self.layer[0] && p[1] < self.layer[1]
    }

    /// The color read at layer point `q`: bilinear between the four texels around it, in
    /// premultiplied blend values, the edges repeating outward.
    fn sample(&self, q: [f64; 2]) -> Sampled<'_> {
        let tx = (q[0] - self.origin[0]) / self.factor - 0.5;
        let ty = (q[1] - self.origin[1]) / self.factor - 0.5;
        let (x0, y0) = (tx.floor(), ty.floor());
        let (fx, fy) = (tx - x0, ty - y0);
        let (x0, y0) = (x0 as i64, y0 as i64);
        if fx == 0.0 && fy == 0.0 {
            return match self.pixels.raw(x0, y0) {
                Some(px) => Sampled::Raw(px),
                None => Sampled::Values([0.0; 4]),
            };
        }
        let (p00, p10, p01, p11) = (
            self.pixels.tap(x0, y0),
            self.pixels.tap(x0 + 1, y0),
            self.pixels.tap(x0, y0 + 1),
            self.pixels.tap(x0 + 1, y0 + 1),
        );
        Sampled::Values(std::array::from_fn(|c| {
            (p00[c] * (1.0 - fx) + p10[c] * fx) * (1.0 - fy)
                + (p01[c] * (1.0 - fx) + p11[c] * fx) * fy
        }))
    }

    /// What is seen at layer point `p` through `field`.
    fn through(&self, field: &Field, p: [f64; 2]) -> Sampled<'_> {
        let d = field.displacement_at(p);
        self.sample([p[0] + d[0], p[1] + d[1]])
    }
}

/// Where the pixels of a warped grid are: pixel (`i`, `j`) is at layer point
/// `origin + ((i + ½) × step, (j + ½) × step)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub origin: [f64; 2],
    pub step: f64,
}

impl Placement {
    fn at(&self, i: usize, j: usize) -> [f64; 2] {
        [
            self.origin[0] + (i as f64 + 0.5) * self.step,
            self.origin[1] + (j as f64 + 0.5) * self.step,
        ]
    }
}

/// One tile of the warped grid `placement` of `size` pixels, in `source`'s stored format.
fn warp_tile(
    source: &Source<'_>,
    field: &Field,
    placement: Placement,
    size: Size,
    coord: TileCoord,
) -> Arc<[u8]> {
    let t = TILE_SIZE as usize;
    let bpp = source.pixels.bytes_per_pixel();
    let (x0, y0) = (coord.col as usize * t, coord.row as usize * t);
    let w = (size.width as usize - x0).min(t);
    let h = (size.height as usize - y0).min(t);
    let mut bytes = vec![0u8; t * t * bpp];
    for y in 0..h {
        for x in 0..w {
            let p = placement.at(x0 + x, y0 + y);
            if !source.inside(p) {
                continue;
            }
            let out = &mut bytes[(y * t + x) * bpp..][..bpp];
            match source.through(field, p) {
                Sampled::Raw(px) => out.copy_from_slice(px),
                Sampled::Values(values) => source.pixels.write(values, out),
            }
        }
    }
    pad_tile(&mut bytes, w, h, bpp);
    Arc::from(bytes)
}

fn grid_of(size: Size) -> Vec<TileCoord> {
    let (columns, rows) = (
        size.width.div_ceil(TILE_SIZE),
        size.height.div_ceil(TILE_SIZE),
    );
    (0..rows)
        .flat_map(|row| (0..columns).map(move |col| TileCoord { col, row }))
        .collect()
}

/// A grid of `size` pixels at `placement` seen through `field` from `source`: a whole image of
/// the source's format, without a pyramid unless `pyramid` (what is shown at the level it was made for).
pub fn warp_image(
    source: &Source<'_>,
    field: &Field,
    placement: Placement,
    size: Size,
    format: PixelFormat,
    pyramid: bool,
) -> Result<RasterImage, RasterError> {
    let mut work: Vec<(TileCoord, Option<Arc<[u8]>>)> =
        grid_of(size).into_iter().map(|c| (c, None)).collect();
    parallel_for_each(&mut work, |(coord, out)| {
        *out = Some(warp_tile(source, field, placement, size, *coord));
    });
    let tiles = work.into_iter().filter_map(|(_, tile)| tile).collect();
    if pyramid {
        RasterImage::from_level0_tiles(size, format, tiles)
    } else {
        RasterImage::from_level0_tiles_only(size, format, tiles)
    }
}

/// `input` warped by `field` (the layer's whole result), in `space`: the tiles whose
/// neighbourhood the field leaves alone are `input`'s own, shared. With a pyramid, as what a
/// stack evaluates to.
pub fn warp_layer(
    input: &RasterImage,
    field: &Field,
    space: BlendSpace,
) -> Result<RasterImage, RasterError> {
    let size = input.size();
    let source = Source::new(input, space, 0, [0.0, 0.0], 1.0, size);
    let placement = Placement {
        origin: [0.0, 0.0],
        step: 1.0,
    };
    let level = &input.levels()[0];
    let mut work: Vec<(TileCoord, Option<Arc<[u8]>>)> =
        grid_of(size).into_iter().map(|c| (c, None)).collect();
    parallel_for_each(&mut work, |(coord, out)| {
        *out = Some(match level.tile(*coord) {
            Some(tile) if !field.touches_around(*coord) => Arc::clone(tile),
            _ => warp_tile(&source, field, placement, size, *coord),
        });
    });
    let tiles = work.into_iter().filter_map(|(_, tile)| tile).collect();
    RasterImage::from_level0_tiles(size, input.format(), tiles)
}

impl Field {
    /// Whether the field has nodes in the tile `coord` or the eight around it: the tiles whose
    /// pixels the interpolation can reach.
    fn touches_around(&self, coord: TileCoord) -> bool {
        let (c0, r0) = (coord.col.saturating_sub(1), coord.row.saturating_sub(1));
        let (c1, r1) = (
            (coord.col + 1).min(self.columns - 1),
            (coord.row + 1).min(self.rows - 1),
        );
        (r0..=r1).any(|row| {
            (c0..=c1).any(|col| self.displacement[self.slot(col as usize, row as usize)].is_some())
        })
    }
}

/// What the workspace shows: layer point `origin` at the top left, `zoom` output pixels per
/// layer pixel, `width` × `height` output pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub origin: [f64; 2],
    pub zoom: f64,
    pub width: u32,
    pub height: u32,
}

/// The color of the freeze mask's overlay.
const FROZEN_OVERLAY: [f64; 3] = [255.0, 0.0, 0.0];
/// The overlay's opacity where the mask is fully frozen.
const FROZEN_OPACITY: f64 = 0.5;

/// The workspace's frame: `input` (a layer's pixels, with their pyramid) seen through `field` as
/// 8-bit sRGB RGBA with straight alpha, `view.width × view.height` pixels, transparent beyond the
/// layer. Read from the pyramid level the zoom asks for, converted for display (the document's
/// pixels are not changed); `overlay`: the frozen area tinted red.
pub fn frame(
    input: &RasterImage,
    space: BlendSpace,
    field: &Field,
    view: View,
    overlay: bool,
) -> Result<Vec<u8>, FieldError> {
    if !(view.zoom.is_finite() && view.zoom > 0.0 && view.origin.iter().all(|v| v.is_finite())) {
        return Err(FieldError::SizeMismatch);
    }
    let (width, height) = (view.width as usize, view.height as usize);
    if width == 0 || height == 0 {
        return Ok(Vec::new());
    }
    let level = ((-view.zoom.log2()).floor().max(0.0) as usize).min(input.levels().len() - 1);
    let factor = f64::from(1u32 << level);
    let source = Source::new(input, space, level, [0.0, 0.0], factor, input.size());
    let placement = Placement {
        origin: view.origin,
        step: 1.0 / view.zoom,
    };
    let converter = Converter::new(
        PixelFormat::RGBA8_SRGB,
        ConvertOptions {
            dither: false,
            big_endian: false,
            matte: WHITE_MATTE,
            blend_space: BlendSpace::Linear,
        },
    )
    .map_err(|_| FieldError::SizeMismatch)?;
    let mut pixels = vec![0u8; width * height * 4];
    const BAND: usize = 8;
    let mut bands: Vec<(usize, &mut [u8])> = pixels
        .chunks_mut(BAND * width * 4)
        .enumerate()
        .map(|(n, band)| (n * BAND, band))
        .collect();
    // 8-bit sRGB pixels blended as stored are the display's own: no conversion to make.
    let as_displayed = source.pixels.is_displayed();
    parallel_for_each(&mut bands, |(first, band)| {
        let mut row = vec![0.0f32; width * 4];
        let mut report = ConversionReport::default();
        for (n, out) in band.chunks_exact_mut(width * 4).enumerate() {
            let y = *first + n;
            if as_displayed {
                for (x, px) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                    let p = placement.at(x, y);
                    if !source.inside(p) {
                        continue;
                    }
                    match source.through(field, p) {
                        Sampled::Raw(raw) => px.copy_from_slice(raw),
                        Sampled::Values(values) => source.pixels.write(values, &mut px[..]),
                    }
                }
            } else {
                for (x, px) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                    let p = placement.at(x, y);
                    let linear = if source.inside(p) {
                        let values = match source.through(field, p) {
                            Sampled::Raw(raw) => source.pixels.read(raw),
                            Sampled::Values(values) => values,
                        };
                        source.pixels.to_linear(values)
                    } else {
                        [0.0; 4]
                    };
                    *px = linear;
                }
                // Invariant: a row of the right length, so conversion cannot fail.
                let _ = converter.convert_row(&row, 0, y as u32, out, &mut report);
            }
            if overlay {
                for (x, px) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                    let p = placement.at(x, y);
                    if source.inside(p) {
                        tint(px, field.frozen_at(p) * FROZEN_OPACITY);
                    }
                }
            }
        }
    });
    Ok(pixels)
}

/// `px` (8-bit straight RGBA) with the overlay's color over it at opacity `amount`.
fn tint(px: &mut [u8; 4], amount: f64) {
    if amount <= 0.0 {
        return;
    }
    let alpha = f64::from(px[3]) / 255.0;
    let out = amount + alpha * (1.0 - amount);
    for c in 0..3 {
        let over = FROZEN_OVERLAY[c] * amount + f64::from(px[c]) * alpha * (1.0 - amount);
        px[c] = (over / out).round().clamp(0.0, 255.0) as u8;
    }
    px[3] = (out * 255.0).round().clamp(0.0, 255.0) as u8;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(size: Size) -> RasterImage {
        let mut pixels = Vec::new();
        for y in 0..size.height {
            for x in 0..size.width {
                pixels.extend([(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8, 255]);
            }
        }
        RasterImage::from_pixels(size, PixelFormat::RGBA8_SRGB, &pixels).unwrap()
    }

    fn pixel(image: &RasterImage, x: u32, y: u32) -> [u8; 4] {
        let tile = image.levels()[0]
            .tile(TileCoord {
                col: x / TILE_SIZE,
                row: y / TILE_SIZE,
            })
            .unwrap();
        let at = ((y % TILE_SIZE) * TILE_SIZE + x % TILE_SIZE) as usize * 4;
        [tile[at], tile[at + 1], tile[at + 2], tile[at + 3]]
    }

    fn drag(field: &mut Field, tool: Tool, brush: Brush, from: [f64; 2], to: [f64; 2]) {
        let mut stroke = Stroke::new(tool, brush);
        stroke.move_to(field, from);
        stroke.move_to(field, to);
        stroke.finish(field);
    }

    fn brush(size: f32) -> Brush {
        Brush {
            size,
            density: 100.0,
            pressure: 100.0,
            rate: 100.0,
        }
    }

    #[test]
    fn the_cell_follows_the_layers_size() {
        assert_eq!(Field::cell_for(Size::new(1000, 1000)), 1);
        assert_eq!(Field::cell_for(Size::new(4000, 4000)), 2);
        assert_eq!(Field::cell_for(Size::new(18000, 13000)), 4);
        assert!(Field::with_cell(Size::new(10, 10), 3).is_err());
        assert!(Field::with_cell(Size::new(10, 10), 512).is_err());
        assert!(Field::with_cell(Size::new(10, 10), 0).is_err());
    }

    #[test]
    fn a_new_field_is_the_identity() {
        let field = Field::new(Size::new(300, 200));
        assert!(field.is_identity());
        assert!(!field.has_frozen());
        assert_eq!(field.reach(), 0.0);
        assert_eq!(field.displacement_at([150.0, 100.0]), [0.0, 0.0]);
        assert_eq!(field.memory_bytes(), 0);
    }

    #[test]
    fn forward_warp_follows_the_brush() {
        let mut field = Field::new(Size::new(400, 400));
        drag(
            &mut field,
            Tool::ForwardWarp,
            brush(100.0),
            [150.0, 200.0],
            [250.0, 200.0],
        );
        // The pixel under the brush's start reads from further left: the content moved right.
        let d = field.displacement_at([200.0, 200.0]);
        assert!(d[0] < -10.0, "content moves right: {d:?}");
        assert!(d[1].abs() < 1e-3, "and not sideways: {d:?}");
        // Nothing outside the brush's path.
        assert_eq!(field.displacement_at([20.0, 20.0]), [0.0, 0.0]);
        assert_eq!(field.displacement_at([200.0, 380.0]), [0.0, 0.0]);
        // A shorter drag moves less.
        let mut short = Field::new(Size::new(400, 400));
        drag(
            &mut short,
            Tool::ForwardWarp,
            brush(100.0),
            [150.0, 200.0],
            [170.0, 200.0],
        );
        assert!(short.displacement_at([160.0, 200.0])[0].abs() < d[0].abs());
    }

    #[test]
    fn a_still_pointer_forward_warps_nothing() {
        let mut field = Field::new(Size::new(100, 100));
        let mut stroke = Stroke::new(Tool::ForwardWarp, brush(40.0));
        stroke.move_to(&mut field, [50.0, 50.0]);
        stroke.hold(&mut field, 1.0);
        stroke.finish(&mut field);
        assert!(field.is_identity());
    }

    #[test]
    fn push_left_moves_pixels_to_the_left_of_the_travel() {
        let mut up = Field::new(Size::new(300, 300));
        drag(
            &mut up,
            Tool::PushLeft,
            brush(80.0),
            [150.0, 200.0],
            [150.0, 100.0],
        );
        // Dragging up pushes the content left: it reads from the right.
        assert!(up.displacement_at([150.0, 150.0])[0] > 5.0);
        let mut down = Field::new(Size::new(300, 300));
        drag(
            &mut down,
            Tool::PushLeft,
            brush(80.0),
            [150.0, 100.0],
            [150.0, 200.0],
        );
        assert!(down.displacement_at([150.0, 150.0])[0] < -5.0);
    }

    #[test]
    fn twirl_turns_clockwise_or_not() {
        let mut clockwise = Field::new(Size::new(300, 300));
        let mut stroke = Stroke::new(Tool::TwirlClockwise, brush(160.0));
        stroke.move_to(&mut clockwise, [150.0, 150.0]);
        stroke.hold(&mut clockwise, 0.2);
        // Content right of the center moves down (clockwise on screen): it reads from above.
        let right = clockwise.displacement_at([200.0, 150.0]);
        assert!(right[1] < -1.0, "{right:?}");
        // Left of the center it moves up: it reads from below.
        let left = clockwise.displacement_at([100.0, 150.0]);
        assert!(left[1] > 1.0, "{left:?}");
        // The center itself does not move.
        let center = clockwise.displacement_at([150.0, 150.0]);
        assert!(center[0].hypot(center[1]) < 0.5, "{center:?}");

        let mut counter = Field::new(Size::new(300, 300));
        let mut stroke = Stroke::new(Tool::TwirlCounterclockwise, brush(160.0));
        stroke.move_to(&mut counter, [150.0, 150.0]);
        stroke.hold(&mut counter, 0.2);
        assert!(counter.displacement_at([200.0, 150.0])[1] > 1.0);
    }

    #[test]
    fn pucker_pulls_in_and_bloat_pushes_out() {
        let mut pucker = Field::new(Size::new(300, 300));
        let mut stroke = Stroke::new(Tool::Pucker, brush(160.0));
        stroke.move_to(&mut pucker, [150.0, 150.0]);
        stroke.hold(&mut pucker, 0.2);
        // Content right of the center moves toward it: it reads from further right.
        assert!(pucker.displacement_at([200.0, 150.0])[0] > 1.0);
        assert!(pucker.displacement_at([100.0, 150.0])[0] < -1.0);
        let mut bloat = Field::new(Size::new(300, 300));
        let mut stroke = Stroke::new(Tool::Bloat, brush(160.0));
        stroke.move_to(&mut bloat, [150.0, 150.0]);
        stroke.hold(&mut bloat, 0.2);
        assert!(bloat.displacement_at([200.0, 150.0])[0] < -1.0);
        assert!(bloat.displacement_at([100.0, 150.0])[0] > 1.0);
    }

    #[test]
    fn held_tools_follow_the_rate() {
        let slow = Brush {
            rate: 10.0,
            ..brush(160.0)
        };
        let fast = Brush {
            rate: 100.0,
            ..brush(160.0)
        };
        let moved = |brush| {
            let mut field = Field::new(Size::new(300, 300));
            let mut stroke = Stroke::new(Tool::Pucker, brush);
            stroke.move_to(&mut field, [150.0, 150.0]);
            stroke.hold(&mut field, 0.1);
            field.displacement_at([200.0, 150.0])[0]
        };
        assert!(moved(fast) > 5.0 * moved(slow));
        // Time passing is what counts, not the number of ticks.
        let mut once = Field::new(Size::new(300, 300));
        let mut twice = Field::new(Size::new(300, 300));
        let (mut a, mut b) = (
            Stroke::new(Tool::Pucker, fast),
            Stroke::new(Tool::Pucker, fast),
        );
        a.move_to(&mut once, [150.0, 150.0]);
        b.move_to(&mut twice, [150.0, 150.0]);
        a.hold(&mut once, 0.1);
        b.hold(&mut twice, 0.05);
        b.hold(&mut twice, 0.05);
        let (x, y) = (
            once.displacement_at([200.0, 150.0])[0],
            twice.displacement_at([200.0, 150.0])[0],
        );
        assert!((x - y).abs() < 0.05 * x.abs(), "{x} vs {y}");
    }

    #[test]
    fn a_freeze_protects_what_it_covers() {
        let mut field = Field::new(Size::new(400, 400));
        drag(
            &mut field,
            Tool::Freeze,
            brush(100.0),
            [200.0, 200.0],
            [200.0, 200.0],
        );
        assert!(field.has_frozen());
        assert!(field.frozen_at([200.0, 200.0]) > 0.99);
        assert_eq!(field.frozen_at([20.0, 20.0]), 0.0);
        // Warping across the frozen area leaves it alone, the rest moves.
        drag(
            &mut field,
            Tool::ForwardWarp,
            brush(300.0),
            [100.0, 200.0],
            [300.0, 200.0],
        );
        assert_eq!(field.displacement_at([200.0, 200.0]), [0.0, 0.0]);
        assert!(field.displacement_at([120.0, 200.0])[0].abs() > 1.0);
        // Thawing gives it back.
        drag(
            &mut field,
            Tool::Thaw,
            brush(100.0),
            [200.0, 200.0],
            [200.0, 200.0],
        );
        assert_eq!(field.frozen_at([200.0, 200.0]), 0.0);
        drag(
            &mut field,
            Tool::ForwardWarp,
            brush(300.0),
            [100.0, 200.0],
            [300.0, 200.0],
        );
        assert!(field.displacement_at([200.0, 200.0])[0].abs() > 1.0);
    }

    #[test]
    fn a_feathered_freeze_protects_partly() {
        let mut field = Field::new(Size::new(400, 400));
        let soft = Brush {
            density: 0.0,
            ..brush(200.0)
        };
        drag(
            &mut field,
            Tool::Freeze,
            soft,
            [200.0, 200.0],
            [200.0, 200.0],
        );
        let (center, edge) = (
            field.frozen_at([200.0, 200.0]),
            field.frozen_at([270.0, 200.0]),
        );
        assert!(
            center > 0.99 && edge > 0.0 && edge < center,
            "{center} {edge}"
        );
    }

    #[test]
    fn reconstruct_takes_the_field_back() {
        let mut field = Field::new(Size::new(400, 400));
        drag(
            &mut field,
            Tool::ForwardWarp,
            brush(100.0),
            [150.0, 200.0],
            [250.0, 200.0],
        );
        let before = field.displacement_at([200.0, 200.0])[0].abs();
        let mut stroke = Stroke::new(Tool::Reconstruct, brush(200.0));
        stroke.move_to(&mut field, [200.0, 200.0]);
        stroke.hold(&mut field, 0.1);
        let after = field.displacement_at([200.0, 200.0])[0].abs();
        assert!(after < before * 0.9, "{after} < {before}");
        for _ in 0..200 {
            stroke.hold(&mut field, 0.1);
        }
        assert!(field.displacement_at([200.0, 200.0])[0].abs() < 1e-3);
    }

    #[test]
    fn reconstruct_leaves_the_frozen() {
        let mut field = Field::new(Size::new(400, 400));
        drag(
            &mut field,
            Tool::ForwardWarp,
            brush(100.0),
            [150.0, 200.0],
            [250.0, 200.0],
        );
        let kept = field.displacement_at([200.0, 200.0]);
        drag(
            &mut field,
            Tool::Freeze,
            brush(60.0),
            [200.0, 200.0],
            [200.0, 200.0],
        );
        let mut stroke = Stroke::new(Tool::Reconstruct, brush(200.0));
        stroke.move_to(&mut field, [200.0, 200.0]);
        for _ in 0..40 {
            stroke.hold(&mut field, 0.1);
        }
        assert_eq!(field.displacement_at([200.0, 200.0]), kept);
    }

    #[test]
    fn smooth_evens_the_field_out() {
        let mut field = Field::new(Size::new(400, 400));
        // A sharp step in the field: a hard small brush dragged.
        drag(
            &mut field,
            Tool::ForwardWarp,
            brush(30.0),
            [150.0, 200.0],
            [190.0, 200.0],
        );
        let variation = |f: &Field| {
            let samples: Vec<f64> = (150..250)
                .step_by(2)
                .map(|x| f.displacement_at([f64::from(x), 200.0])[0])
                .collect();
            samples
                .windows(2)
                .map(|w| (w[1] - w[0]).abs())
                .fold(0.0, f64::max)
        };
        let before = variation(&field);
        let mut stroke = Stroke::new(Tool::Smooth, brush(200.0));
        stroke.move_to(&mut field, [180.0, 200.0]);
        for _ in 0..10 {
            stroke.hold(&mut field, 0.1);
        }
        let reach = |f: &Field| f.reach();
        assert!(
            variation(&field) < before,
            "{} < {before}",
            variation(&field)
        );
        assert!(reach(&field) > 0.0);
    }

    #[test]
    fn restore_all_clears_the_displacement_and_keeps_the_freeze() {
        let mut field = Field::new(Size::new(300, 300));
        drag(
            &mut field,
            Tool::ForwardWarp,
            brush(100.0),
            [100.0, 150.0],
            [200.0, 150.0],
        );
        drag(
            &mut field,
            Tool::Freeze,
            brush(60.0),
            [50.0, 50.0],
            [50.0, 50.0],
        );
        assert!(!field.is_identity());
        let restored = field.restored();
        assert!(restored.is_identity());
        assert!(restored.has_frozen());
        // The original is untouched (tiles are shared, never changed).
        assert!(!field.is_identity());
    }

    #[test]
    fn a_stroke_never_changes_a_field_it_was_cloned_from() {
        let mut field = Field::new(Size::new(300, 300));
        drag(
            &mut field,
            Tool::ForwardWarp,
            brush(100.0),
            [100.0, 150.0],
            [200.0, 150.0],
        );
        let snapshot = field.clone();
        let before = snapshot.displacement_at([150.0, 150.0]);
        drag(
            &mut field,
            Tool::ForwardWarp,
            brush(100.0),
            [100.0, 150.0],
            [200.0, 150.0],
        );
        assert_eq!(snapshot.displacement_at([150.0, 150.0]), before);
        assert_ne!(field.displacement_at([150.0, 150.0]), before);
    }

    #[test]
    fn the_brush_density_feathers_the_edge() {
        let hard = Brush {
            density: 100.0,
            ..Brush::default()
        };
        let soft = Brush {
            density: 0.0,
            ..Brush::default()
        };
        assert_eq!(hard.falloff(0.5), 1.0);
        assert_eq!(hard.falloff(1.0), 0.0);
        assert!((soft.falloff(0.0) - 1.0).abs() < 1e-9);
        assert!(soft.falloff(0.5) < 0.6);
        assert!(soft.falloff(0.5) > soft.falloff(0.9));
        assert!(Brush::default().is_valid());
        assert!(
            !Brush {
                size: 0.0,
                ..Brush::default()
            }
            .is_valid()
        );
        assert!(
            !Brush {
                pressure: 0.0,
                ..Brush::default()
            }
            .is_valid()
        );
    }

    #[test]
    fn tools_have_stable_identifiers() {
        for id in Tool::IDS {
            assert_eq!(Tool::from_id(id).map(Tool::id), Some(id));
        }
        assert_eq!(Tool::from_id("nope"), None);
        assert!(Tool::Pucker.acts_when_held());
        assert!(!Tool::ForwardWarp.acts_when_held());
        assert!(!Tool::Freeze.acts_when_held());
    }

    #[test]
    fn bad_input_changes_nothing() {
        let mut field = Field::new(Size::new(100, 100));
        field.stamp(
            Tool::ForwardWarp,
            &brush(40.0),
            [f64::NAN, 5.0],
            [1.0, 1.0],
            1.0,
        );
        field.stamp(
            Tool::ForwardWarp,
            &brush(40.0),
            [1e12, 5.0],
            [1.0, 1.0],
            1.0,
        );
        field.stamp(
            Tool::ForwardWarp,
            &brush(40.0),
            [-1e12, 5.0],
            [1.0, 1.0],
            1.0,
        );
        let mut stroke = Stroke::new(Tool::ForwardWarp, brush(40.0));
        stroke.move_to(&mut field, [f64::INFINITY, 0.0]);
        stroke.move_to(&mut field, [10.0, 10.0]);
        stroke.hold(&mut field, f64::NAN);
        assert!(field.is_identity());
    }

    #[test]
    fn an_identity_field_leaves_the_image_as_it_is() {
        let image = gradient(Size::new(300, 200));
        let field = Field::new(image.size());
        let warped = warp_layer(&image, &field, BlendSpace::Perceptual).unwrap();
        // The same tiles, shared.
        for (a, b) in warped.levels()[0]
            .tiles()
            .iter()
            .zip(image.levels()[0].tiles())
        {
            assert!(Arc::ptr_eq(a, b));
        }
    }

    #[test]
    fn a_whole_pixel_displacement_moves_the_pattern() {
        let image = gradient(Size::new(100, 80));
        let mut field = Field::with_cell(image.size(), 1).unwrap();
        // Every node reads 5 pixels to the right and 3 down.
        for j in 0..80 {
            for i in 0..100 {
                field.set_node(i, j, [5.0, 3.0]);
            }
        }
        let warped = warp_layer(&image, &field, BlendSpace::Perceptual).unwrap();
        for (x, y) in [(0u32, 0u32), (10, 10), (50, 40), (90, 70)] {
            assert_eq!(pixel(&warped, x, y), pixel(&image, x + 5, y + 3));
        }
        // The edges repeat outward.
        assert_eq!(pixel(&warped, 99, 79), pixel(&image, 99, 79));
    }

    #[test]
    fn a_half_pixel_displacement_averages_neighbours() {
        let mut pixels = Vec::new();
        for x in 0..8u32 {
            pixels.extend([if x % 2 == 0 { 0u8 } else { 200 }, 0, 0, 255]);
        }
        let image =
            RasterImage::from_pixels(Size::new(8, 1), PixelFormat::RGBA8_SRGB, &pixels).unwrap();
        let mut field = Field::with_cell(image.size(), 1).unwrap();
        for i in 0..8 {
            field.set_node(i, 0, [0.5, 0.0]);
        }
        let warped = warp_layer(&image, &field, BlendSpace::Perceptual).unwrap();
        assert_eq!(pixel(&warped, 3, 0)[0], 100);
        assert_eq!(pixel(&warped, 3, 0)[3], 255);
    }

    #[test]
    fn transparency_is_blended_premultiplied() {
        // An opaque red pixel beside a transparent one: halfway is half-opaque red, not dark.
        let pixels = [255u8, 0, 0, 255, 0, 0, 0, 0];
        let image =
            RasterImage::from_pixels(Size::new(2, 1), PixelFormat::RGBA8_SRGB, &pixels).unwrap();
        let mut field = Field::with_cell(image.size(), 1).unwrap();
        field.set_node(0, 0, [0.5, 0.0]);
        field.set_node(1, 0, [0.5, 0.0]);
        let warped = warp_layer(&image, &field, BlendSpace::Perceptual).unwrap();
        let out = pixel(&warped, 0, 0);
        assert_eq!(out[0], 255, "straight color stays red: {out:?}");
        assert!((i32::from(out[3]) - 128).abs() <= 1, "half opaque: {out:?}");
    }

    #[test]
    fn a_stroke_over_a_large_layer_agrees_with_a_whole_evaluation_by_tiles() {
        // A field crossing tile borders: every tile evaluated alone equals the same pixels of an
        // evaluation of the whole layer by one thread.
        let image = gradient(Size::new(700, 600));
        let mut field = Field::with_cell(image.size(), 2).unwrap();
        drag(
            &mut field,
            Tool::ForwardWarp,
            brush(300.0),
            [200.0, 300.0],
            [520.0, 280.0],
        );
        let warped = warp_layer(&image, &field, BlendSpace::Perceptual).unwrap();
        let source = Source::new(
            &image,
            BlendSpace::Perceptual,
            0,
            [0.0; 2],
            1.0,
            image.size(),
        );
        for (x, y) in [
            (255u32, 255u32),
            (256, 256),
            (300, 300),
            (511, 299),
            (512, 300),
            (400, 590),
        ] {
            let p = [f64::from(x) + 0.5, f64::from(y) + 0.5];
            let mut expected = [0u8; 4];
            match source.through(&field, p) {
                Sampled::Raw(px) => expected.copy_from_slice(px),
                Sampled::Values(v) => source.pixels.write(v, &mut expected),
            }
            assert_eq!(pixel(&warped, x, y), expected, "at {x},{y}");
        }
        // Away from the field, the input's own pixels.
        assert_eq!(pixel(&warped, 5, 5), pixel(&image, 5, 5));
        assert_eq!(pixel(&warped, 690, 590), pixel(&image, 690, 590));
    }

    #[test]
    fn a_crop_at_a_pyramid_level_shows_what_the_layer_shows_there() {
        // A uniform ramp keeps its mean through averaging: the crop at level 1 of a field's
        // warp equals the level-0 warp sampled at the same points, within a level of 8 bits.
        let image = gradient(Size::new(600, 600));
        let mut field = Field::with_cell(image.size(), 2).unwrap();
        for j in 0..300 {
            for i in 0..300 {
                field.set_node(i, j, [6.0, -4.0]);
            }
        }
        let whole = warp_layer(&image, &field, BlendSpace::Perceptual).unwrap();
        let source = Source::new(
            &image,
            BlendSpace::Perceptual,
            1,
            [0.0; 2],
            2.0,
            image.size(),
        );
        let placement = Placement {
            origin: [0.0; 2],
            step: 2.0,
        };
        let crop = warp_image(
            &source,
            &field,
            placement,
            Size::new(300, 300),
            image.format(),
            false,
        )
        .unwrap();
        for (x, y) in [(50u32, 50u32), (120, 200), (250, 100)] {
            let (a, b) = (pixel(&crop, x, y), pixel(&whole, x * 2, y * 2));
            for c in 0..3 {
                assert!(
                    (i32::from(a[c]) - i32::from(b[c])).abs() <= 4,
                    "{a:?} vs {b:?}"
                );
            }
        }
    }

    #[test]
    fn grown_fields_move_with_their_pixels() {
        let mut field = Field::with_cell(Size::new(300, 300), 2).unwrap();
        drag(
            &mut field,
            Tool::ForwardWarp,
            brush(100.0),
            [100.0, 100.0],
            [160.0, 100.0],
        );
        let grown = field.grown((1, 2), Size::new(600, 900));
        let d = field.displacement_at([130.0, 100.0]);
        assert_eq!(grown.displacement_at([130.0 + 256.0, 100.0 + 512.0]), d);
        assert_eq!(grown.displacement_at([130.0, 100.0]), [0.0, 0.0]);
        assert_eq!(grown.cell(), 2);
    }

    #[test]
    fn a_field_round_trips_through_its_images() {
        for cell in [1, 2, 4] {
            let size = Size::new(700, 530);
            let mut field = Field::with_cell(size, cell).unwrap();
            drag(
                &mut field,
                Tool::ForwardWarp,
                brush(250.0),
                [200.0, 300.0],
                [480.0, 260.0],
            );
            drag(
                &mut field,
                Tool::Freeze,
                brush(80.0),
                [600.0, 100.0],
                [600.0, 100.0],
            );
            let (displacement, frozen) = field.to_images().unwrap();
            assert_eq!(
                displacement.size(),
                Size::new(size.width.div_ceil(cell), size.height.div_ceil(cell))
            );
            let read = Field::from_images(size, cell, &displacement, &frozen).unwrap();
            for p in [
                [250.0, 290.0],
                [480.0, 260.0],
                [600.0, 100.0],
                [10.0, 10.0],
                [699.0, 529.0],
            ] {
                assert_eq!(
                    read.displacement_at(p),
                    field.displacement_at(p),
                    "cell {cell} at {p:?}"
                );
                assert_eq!(read.frozen_at(p), field.frozen_at(p));
            }
            assert_eq!(read.reach(), field.reach());
            // Wrong cell or size: refused.
            assert!(Field::from_images(size, cell * 2, &displacement, &frozen).is_err());
            assert!(Field::from_images(Size::new(900, 900), cell, &displacement, &frozen).is_err());
        }
    }

    #[test]
    fn an_empty_field_stores_one_shared_zero_tile() {
        let field = Field::new(Size::new(2000, 1500));
        let (displacement, _) = field.to_images().unwrap();
        let tiles = displacement.levels()[0].tiles();
        assert!(tiles.iter().all(|t| Arc::ptr_eq(t, &tiles[0])));
    }

    #[test]
    fn the_frame_shows_the_layer_at_any_zoom_and_transparent_beyond_it() {
        let image = gradient(Size::new(64, 64));
        let field = Field::new(image.size());
        let view = View {
            origin: [-8.0, -8.0],
            zoom: 1.0,
            width: 80,
            height: 80,
        };
        let rgba = frame(&image, BlendSpace::Perceptual, &field, view, false).unwrap();
        assert_eq!(rgba.len(), 80 * 80 * 4);
        // Beyond the layer: nothing. On it: the pixel (8-bit sRGB in, 8-bit sRGB out).
        assert_eq!(rgba[3], 0);
        let at = ((8 + 5) * 80 + 8 + 7) * 4;
        assert_eq!(rgba[at..at + 4], pixel(&image, 7, 5));
        // Zoomed in by 4, the pixel fills a block.
        let view = View {
            origin: [0.0, 0.0],
            zoom: 4.0,
            width: 40,
            height: 40,
        };
        let rgba = frame(&image, BlendSpace::Perceptual, &field, view, false).unwrap();
        let block = |x: usize, y: usize| rgba[(y * 40 + x) * 4..(y * 40 + x) * 4 + 4].to_vec();
        assert_eq!(block(9, 9), block(10, 10));
        assert!(
            frame(
                &image,
                BlendSpace::Perceptual,
                &field,
                View { zoom: 0.0, ..view },
                false
            )
            .is_err()
        );
        assert!(
            frame(
                &image,
                BlendSpace::Perceptual,
                &field,
                View {
                    zoom: f64::NAN,
                    ..view
                },
                false
            )
            .is_err()
        );
    }

    #[test]
    fn the_frame_follows_the_field_and_tints_the_frozen() {
        let image = gradient(Size::new(64, 64));
        let mut field = Field::with_cell(image.size(), 1).unwrap();
        for j in 0..64 {
            for i in 0..64 {
                field.set_node(i, j, [4.0, 0.0]);
            }
        }
        drag(
            &mut field,
            Tool::Freeze,
            brush(20.0),
            [32.0, 32.0],
            [32.0, 32.0],
        );
        let view = View {
            origin: [0.0, 0.0],
            zoom: 1.0,
            width: 64,
            height: 64,
        };
        let plain = frame(&image, BlendSpace::Perceptual, &field, view, false).unwrap();
        let at = (10 * 64 + 10) * 4;
        assert_eq!(plain[at..at + 4], pixel(&image, 14, 10));
        let tinted = frame(&image, BlendSpace::Perceptual, &field, view, true).unwrap();
        assert_eq!(
            tinted[at..at + 4],
            plain[at..at + 4],
            "unfrozen is not tinted"
        );
        let center = (32 * 64 + 32) * 4;
        assert!(
            tinted[center] > plain[center],
            "the frozen area is tinted red"
        );
        assert!(tinted[center + 1] < plain[center + 1]);
        assert_eq!(tinted[center + 3], 255);
    }

    #[test]
    fn a_large_view_of_a_large_layer_reads_a_coarser_level() {
        // 1024² layer at 25 %: level 2 is read, and an identity field shows the layer's colors.
        let image = gradient(Size::new(1024, 1024));
        let field = Field::new(image.size());
        let view = View {
            origin: [0.0, 0.0],
            zoom: 0.25,
            width: 256,
            height: 256,
        };
        let rgba = frame(&image, BlendSpace::Perceptual, &field, view, false).unwrap();
        // The pixel at (128, 128) shows the layer around (512, 512): its color there ±.
        let at = (128 * 256 + 128) * 4;
        let want = pixel(&image, 512, 512);
        for c in 0..3 {
            assert!(
                (i32::from(rgba[at + c]) - i32::from(want[c])).abs() <= 8,
                "{:?} {want:?}",
                &rgba[at..at + 4]
            );
        }
    }

    /// A layer of `size` made of a few distinct tiles repeated (cheap in memory, whatever the size).
    fn big_layer(size: Size) -> RasterImage {
        let t = TILE_SIZE as usize;
        let patterns: Vec<Arc<[u8]>> = (0..16u32)
            .map(|k| {
                let mut tile = Vec::with_capacity(t * t * 4);
                for y in 0..t {
                    for x in 0..t {
                        tile.extend([
                            ((x as u32 * 3 + k * 17) % 256) as u8,
                            ((y as u32 * 5 + k * 31) % 256) as u8,
                            ((x + y) as u32 % 256) as u8,
                            255,
                        ]);
                    }
                }
                Arc::from(tile)
            })
            .collect();
        let (columns, rows) = (
            size.width.div_ceil(TILE_SIZE),
            size.height.div_ceil(TILE_SIZE),
        );
        let tiles = (0..columns * rows)
            .map(|i| Arc::clone(&patterns[(i % 16) as usize]))
            .collect();
        RasterImage::from_level0_tiles(size, PixelFormat::RGBA8_SRGB, tiles).unwrap()
    }

    /// Timings on this machine: `cargo test --release -p slopshop-core liquify_timings -- --ignored
    /// --nocapture` (SLOPSHOP_LIQUIFY_SIZE=WxH for another layer; default 6000x4000).
    #[test]
    #[ignore = "a measurement, not a test"]
    fn liquify_timings() {
        use std::time::Instant;
        let size = std::env::var("SLOPSHOP_LIQUIFY_SIZE")
            .ok()
            .and_then(|s| {
                let (w, h) = s.split_once('x')?;
                Some(Size::new(w.parse().ok()?, h.parse().ok()?))
            })
            .unwrap_or(Size::new(6000, 4000));
        let t = Instant::now();
        let image = big_layer(size);
        println!(
            "layer {}x{}: built with its pyramid in {:?}",
            size.width,
            size.height,
            t.elapsed()
        );
        let mut field = Field::new(size);
        println!("cell {} px", field.cell());
        for brush_size in [50.0, 300.0, 1000.0, 3000.0] {
            let b = brush(brush_size);
            let t = Instant::now();
            let mut stroke = Stroke::new(Tool::ForwardWarp, b);
            let mut dabs = 0;
            stroke.move_to(&mut field, [1000.0, 1000.0]);
            for i in 1..=20 {
                stroke.move_to(
                    &mut field,
                    [1000.0 + f64::from(i) * f64::from(brush_size) * 0.1, 1000.0],
                );
                dabs += 1;
            }
            println!(
                "forward warp, brush {brush_size}: {dabs} moves in {:?} ({:?} each)",
                t.elapsed(),
                t.elapsed() / dabs
            );
        }
        println!(
            "field memory {} MB, reach {:.0} px",
            field.memory_bytes() / 1_000_000,
            field.reach()
        );
        let t = Instant::now();
        let mut held = Stroke::new(Tool::Pucker, brush(500.0));
        held.move_to(&mut field, [1500.0, 1500.0]);
        for _ in 0..10 {
            held.hold(&mut field, 0.033);
        }
        println!("pucker held, brush 500: 10 ticks in {:?}", t.elapsed());
        for (w, h, zoom) in [
            (1920u32, 1080u32, 1.0),
            (1920, 1080, 0.25),
            (3840, 2160, 0.5),
        ] {
            let view = View {
                origin: [800.0, 600.0],
                zoom,
                width: w,
                height: h,
            };
            let t = Instant::now();
            let _ = frame(&image, BlendSpace::Perceptual, &field, view, true).unwrap();
            println!("frame {w}x{h} at {:.0}%: {:?}", zoom * 100.0, t.elapsed());
        }
        let t = Instant::now();
        let warped = warp_layer(&image, &field, BlendSpace::Perceptual).unwrap();
        println!(
            "whole layer evaluated (touched tiles only): {:?}",
            t.elapsed()
        );
        drop(warped);
        let t = Instant::now();
        let (d, f) = field.to_images().unwrap();
        println!(
            "field to images for .slop: {:?} ({}x{})",
            t.elapsed(),
            d.size().width,
            f.size().height
        );
    }

    /// A PNG of `rgba` (8-bit, `width` × `height`), stored without compression.
    fn png(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
        fn crc(bytes: &[u8]) -> u32 {
            let mut c = 0xFFFF_FFFFu32;
            for &b in bytes {
                c ^= u32::from(b);
                for _ in 0..8 {
                    c = if c & 1 == 1 {
                        0xEDB8_8320 ^ (c >> 1)
                    } else {
                        c >> 1
                    };
                }
            }
            !c
        }
        fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
            out.extend((data.len() as u32).to_be_bytes());
            let mut body = kind.to_vec();
            body.extend(data);
            out.extend(&body);
            out.extend(crc(&body).to_be_bytes());
        }
        let mut raw = Vec::new();
        for row in rgba.chunks(width as usize * 4) {
            raw.push(0);
            raw.extend(row);
        }
        let mut z = vec![0x78, 0x01];
        let blocks: Vec<&[u8]> = raw.chunks(65535).collect();
        for (n, block) in blocks.iter().enumerate() {
            z.push(u8::from(n + 1 == blocks.len()));
            z.extend((block.len() as u16).to_le_bytes());
            z.extend((!(block.len() as u16)).to_le_bytes());
            z.extend(*block);
        }
        let (mut a, mut b) = (1u32, 0u32);
        for &v in &raw {
            a = (a + u32::from(v)) % 65521;
            b = (b + a) % 65521;
        }
        z.extend(((b << 16) | a).to_be_bytes());
        let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        let mut header = Vec::new();
        header.extend(width.to_be_bytes());
        header.extend(height.to_be_bytes());
        header.extend([8, 6, 0, 0, 0]);
        chunk(&mut out, b"IHDR", &header);
        chunk(&mut out, b"IDAT", &z);
        chunk(&mut out, b"IEND", &[]);
        out
    }

    /// A contact sheet of each tool's effect on a test picture, written to `SLOPSHOP_GALLERY`
    /// (a PNG path) to look at: `cargo test -p slopshop-core liquify_gallery -- --ignored`.
    #[test]
    #[ignore = "writes a picture to look at"]
    fn liquify_gallery() {
        let Ok(path) = std::env::var("SLOPSHOP_GALLERY") else {
            return;
        };
        let size = Size::new(360, 270);
        let mut pixels = Vec::new();
        for y in 0..size.height {
            for x in 0..size.width {
                let grid = x % 30 < 2 || y % 30 < 2;
                let (dx, dy) = (f64::from(x) - 180.0, f64::from(y) - 135.0);
                let disc = dx.hypot(dy) < 70.0;
                let ring = (dx.hypot(dy) - 100.0).abs() < 3.0;
                let px = if grid {
                    [40, 40, 40, 255]
                } else if ring {
                    [220, 30, 30, 255]
                } else if disc {
                    [240, 200, 60, 255]
                } else {
                    [
                        (x * 255 / size.width) as u8,
                        120,
                        (y * 255 / size.height) as u8,
                        255,
                    ]
                };
                pixels.extend(px);
            }
        }
        let image = RasterImage::from_pixels(size, PixelFormat::RGBA8_SRGB, &pixels).unwrap();
        let b = |size| Brush {
            size,
            density: 50.0,
            pressure: 100.0,
            rate: 80.0,
        };
        let mut results: Vec<Field> = Vec::new();
        let mut run = |setup: &dyn Fn(&mut Field)| {
            let mut field = Field::new(size);
            setup(&mut field);
            results.push(field);
        };
        run(&|_| {});
        run(&|f| {
            drag(
                f,
                Tool::ForwardWarp,
                b(120.0),
                [100.0, 135.0],
                [220.0, 135.0],
            )
        });
        run(&|f| {
            let mut s = Stroke::new(Tool::TwirlClockwise, b(200.0));
            s.move_to(f, [180.0, 135.0]);
            for _ in 0..20 {
                s.hold(f, 0.033);
            }
        });
        run(&|f| {
            let mut s = Stroke::new(Tool::Pucker, b(200.0));
            s.move_to(f, [180.0, 135.0]);
            for _ in 0..20 {
                s.hold(f, 0.033);
            }
        });
        run(&|f| {
            let mut s = Stroke::new(Tool::Bloat, b(200.0));
            s.move_to(f, [180.0, 135.0]);
            for _ in 0..20 {
                s.hold(f, 0.033);
            }
        });
        run(&|f| drag(f, Tool::PushLeft, b(120.0), [180.0, 200.0], [180.0, 70.0]));
        run(&|f| {
            drag(f, Tool::Freeze, b(100.0), [180.0, 135.0], [180.0, 135.0]);
            drag(
                f,
                Tool::ForwardWarp,
                b(220.0),
                [60.0, 135.0],
                [300.0, 135.0],
            );
        });
        run(&|f| {
            drag(
                f,
                Tool::ForwardWarp,
                b(120.0),
                [100.0, 135.0],
                [260.0, 135.0],
            );
            let mut s = Stroke::new(Tool::Reconstruct, b(260.0));
            s.move_to(f, [180.0, 135.0]);
            for _ in 0..10 {
                s.hold(f, 0.033);
            }
        });
        let columns = 4usize;
        let (w, h) = (size.width as usize, size.height as usize);
        let rows = results.len().div_ceil(columns);
        let mut sheet = vec![30u8; columns * w * rows * h * 4];
        for (n, field) in results.iter().enumerate() {
            let view = View {
                origin: [0.0, 0.0],
                zoom: 1.0,
                width: size.width,
                height: size.height,
            };
            let rgba = frame(&image, BlendSpace::Perceptual, field, view, true).unwrap();
            let (cx, cy) = ((n % columns) * w, (n / columns) * h);
            for y in 0..h {
                let to = ((cy + y) * columns * w + cx) * 4;
                sheet[to..to + w * 4].copy_from_slice(&rgba[y * w * 4..(y + 1) * w * 4]);
            }
        }
        std::fs::write(path, png((columns * w) as u32, (rows * h) as u32, &sheet)).unwrap();
    }

    #[test]
    fn extreme_input_is_bounded_not_a_hang_or_a_huge_allocation() {
        // A pointer sample 10^9 pixels away: the stroke drops the first part of the path.
        let mut field = Field::new(Size::new(500, 500));
        let started = std::time::Instant::now();
        let mut stroke = Stroke::new(Tool::ForwardWarp, brush(1.0));
        stroke.move_to(&mut field, [0.0, 0.0]);
        stroke.move_to(&mut field, [1e9, 0.0]);
        stroke.finish(&mut field);
        assert!(started.elapsed().as_secs() < 30, "{:?}", started.elapsed());
        // A brush far larger than the layer reads only the grid.
        let huge = Brush {
            size: MAX_BRUSH_SIZE,
            ..brush(100.0)
        };
        let mut pucker = Stroke::new(Tool::Pucker, huge);
        pucker.move_to(&mut field, [250.0, 250.0]);
        pucker.hold(&mut field, 1e9);
        field.stamp(Tool::ForwardWarp, &huge, [10.0, 10.0], [1e12, 1e12], 1e12);
        assert!(field.memory_bytes() < 64 * 1024 * 1024);
        assert!(field.reach().is_finite());
    }

    #[test]
    fn a_frame_of_no_pixels_is_empty() {
        let image = gradient(Size::new(16, 16));
        let field = Field::new(image.size());
        let view = View {
            origin: [0.0, 0.0],
            zoom: 1.0,
            width: 0,
            height: 10,
        };
        assert!(
            frame(&image, BlendSpace::Perceptual, &field, view, true)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn every_pixel_format_and_blend_space_shows_the_same_picture_through_no_field() {
        let size = Size::new(40, 30);
        let field = Field::new(size);
        let reference = {
            let image = gradient(size);
            let view = View {
                origin: [0.0, 0.0],
                zoom: 1.0,
                width: 40,
                height: 30,
            };
            (image, view)
        };
        let (image, view) = reference;
        let expected = frame(&image, BlendSpace::Perceptual, &field, view, false).unwrap();
        // The same picture in linear light: converted for display, the same within a level.
        let linear = frame(&image, BlendSpace::Linear, &field, view, false).unwrap();
        for (a, b) in expected.iter().zip(&linear) {
            assert!(a.abs_diff(*b) <= 1, "{a} vs {b}");
        }
        // The same picture in 16 bits.
        let mut samples = Vec::new();
        for y in 0..size.height {
            for x in 0..size.width {
                let [r, g, b, a] = pixel(&image, x, y);
                for v in [r, g, b, a] {
                    samples.extend((u16::from(v) * 257).to_ne_bytes());
                }
            }
        }
        let format16 = PixelFormat {
            sample: SampleType::U16,
            ..PixelFormat::RGBA8_SRGB
        };
        let sixteen = RasterImage::from_pixels(size, format16, &samples).unwrap();
        let shown = frame(&sixteen, BlendSpace::Perceptual, &field, view, false).unwrap();
        for (a, b) in expected.iter().zip(&shown) {
            assert!(a.abs_diff(*b) <= 1, "{a} vs {b}");
        }
        // And warped, the formats agree with each other where the field moves things.
        let mut moved = Field::new(size);
        drag(
            &mut moved,
            Tool::ForwardWarp,
            brush(30.0),
            [10.0, 15.0],
            [30.0, 15.0],
        );
        let a = frame(&image, BlendSpace::Perceptual, &moved, view, false).unwrap();
        let b = frame(&sixteen, BlendSpace::Perceptual, &moved, view, false).unwrap();
        assert!(a.iter().zip(&b).all(|(a, b)| a.abs_diff(*b) <= 1));
    }

    #[test]
    fn a_gray_layer_warps_and_keeps_its_format() {
        let size = Size::new(8, 1);
        let format = PixelFormat {
            layout: ChannelLayout::Gray,
            ..PixelFormat::RGBA8_SRGB
        };
        let pixels: Vec<u8> = (0..8).map(|x| if x % 2 == 0 { 0 } else { 200 }).collect();
        let image = RasterImage::from_pixels(size, format, &pixels).unwrap();
        let mut field = Field::with_cell(size, 1).unwrap();
        for i in 0..8 {
            field.set_node(i, 0, [0.5, 0.0]);
        }
        let warped = warp_layer(&image, &field, BlendSpace::Perceptual).unwrap();
        assert_eq!(warped.format(), format);
        let tile = &warped.levels()[0].tiles()[0];
        // Between 0 and 200 (in sRGB values): the mean's gray, within a level.
        assert!((i32::from(tile[3]) - 100).abs() <= 1, "{}", tile[3]);
    }
}
