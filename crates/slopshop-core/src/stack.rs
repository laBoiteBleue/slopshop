//! A raster layer's own stack (ADR 0029): its original pixels, then the paint and the effects
//! applied to it, bottom to top. The layer shows the stack's result, evaluated tile by tile.
//!
//! - **Paint** is a delta: it turns what is below it, `B`, into `P + k·B` per pixel, `P` a
//!   color and `k` a factor in `[0, 1]`, in the premultiplied values of the blend space recorded
//!   with it. It stores the tiles it touched only; elsewhere it is the identity (`P = 0`,
//!   `k = 1`). `P` is a pixel of the layer's own format (with alpha); `k` has the same sample
//!   type (floats: `f32`), so that an opaque pixel painted over stays exactly opaque.
//! - **Effects** are parameters: an adjustment applied to the layer's own color, limited by the
//!   selection it was applied with (kept by reference, with the layer's placement then).
//! - **Filters** are parameters too (ADR 0034), but read neighbouring pixels: the result of what
//!   is below a filter is computed whole, the filter applied to it, and the entries above
//!   evaluated from there. Both are kept in a cache, never saved.
//!
//! The result is quantized to the layer's format after every paint and every effect, as each
//! one wrote the layer's pixels in Photoshop. So evaluating from the original and adding to a
//! result already evaluated give the same pixels, and only the tiles that an entry reaches are
//! ever recomputed. Entries are not edited; deleting one merges the neighbours that become alike.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use crate::adjust::{Adjustment, Prepared};
use crate::blend::{BlendSpace, Blender};
use crate::color::{
    ChannelLayout, IDENTITY, LinearRgba, Mat3, PixelFormat, SampleType, WORKING_SPACE, f16_to_f32,
    f32_to_f16, mat_vec,
};
use crate::filter::Filter;
use crate::geom::Size;
use crate::liquify::{Field, FieldError};
use crate::paint::MaskReader;
use crate::raster::{
    Codec, RasterError, RasterImage, TILE_SIZE, pad_tile, parallel_for_each, stored_format,
};
use crate::selection::Selection;
use crate::tile::TileCoord;
use crate::transform::{Affine, Projective};

/// Pixels per tile.
const TILE_PIXELS: usize = (TILE_SIZE * TILE_SIZE) as usize;
/// Rows a thread computes at a time when one tile is shared between threads.
const BAND_ROWS: usize = 8;
/// The largest pixel: RGBA of `f32`.
const MAX_PIXEL_BYTES: usize = 16;

/// A raster layer's original pixels and the entries applied to them, bottom to top. Cloning
/// shares everything.
#[derive(Debug, Clone)]
pub struct LayerStack {
    original: Arc<RasterImage>,
    entries: Vec<Entry>,
}

/// One entry of a [`LayerStack`]. Entries are immutable and shared: two entries are the same
/// when they are the same allocation.
#[derive(Debug, Clone)]
pub enum Entry {
    Paint(Arc<PaintEntry>),
    Effect(Arc<EffectEntry>),
    Filter(Arc<FilterEntry>),
    Liquify(Arc<LiquifyEntry>),
}

impl PartialEq for Entry {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Paint(a), Self::Paint(b)) => Arc::ptr_eq(a, b),
            (Self::Effect(a), Self::Effect(b)) => Arc::ptr_eq(a, b),
            (Self::Filter(a), Self::Filter(b)) => Arc::ptr_eq(a, b),
            (Self::Liquify(a), Self::Liquify(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl Entry {
    /// Hidden by its eye (ADR 0034): kept in the stack, not evaluated.
    pub fn hidden(&self) -> bool {
        match self {
            Self::Paint(p) => p.hidden,
            Self::Effect(e) => e.hidden,
            Self::Filter(f) => f.hidden,
            Self::Liquify(l) => l.hidden,
        }
    }
}

/// What an entry's steps are set to when it is edited again (ADR 0034): an adjustment for each
/// step of an effect entry, a filter for each step of a filter entry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Operation {
    Adjustment(Adjustment),
    Filter(Filter),
}

/// Paint: `P + k·B` over what is below it, on the tiles it touched.
#[derive(Debug)]
pub struct PaintEntry {
    size: Size,
    /// The format of `P`: the layer's, with an alpha channel.
    format: PixelFormat,
    space: BlendSpace,
    tiles: BTreeMap<TileCoord, PaintTile>,
    /// `P` and `k` as whole images, made once when asked (files store them so).
    images: OnceLock<(Arc<RasterImage>, Arc<RasterImage>)>,
    /// What was below it when it was laid, the tiles it reached: the next stroke continuing it
    /// reuses them while the stack below is the same (a cache, never saved).
    below: Mutex<Option<BelowTiles>>,
    /// Hidden by its eye (ADR 0034): kept, not evaluated.
    hidden: bool,
}

/// Tiles of the result of a stack below a paint (see [`PaintEntry`]).
#[derive(Debug, Clone)]
struct BelowTiles {
    stack: LayerStack,
    format: PixelFormat,
    tiles: HashMap<TileCoord, Arc<[u8]>>,
}

/// The paint of one tile: `TILE_SIZE²` pixels of `P` (the entry's stored format) and of `k`,
/// padded as image tiles are.
#[derive(Debug, Clone)]
pub struct PaintTile {
    color: Arc<[u8]>,
    keep: Arc<[u8]>,
    /// Some pixel has `P`'s alpha + `k` below 1: it lowers the alpha of an opaque pixel.
    lowers_alpha: bool,
}

impl PaintTile {
    /// `P`'s pixels.
    pub fn color(&self) -> &Arc<[u8]> {
        &self.color
    }

    /// `k`'s samples.
    pub fn keep(&self) -> &Arc<[u8]> {
        &self.keep
    }
}

/// A paint tile as stored: its position, `P`'s pixels and `k`'s samples.
pub type RawPaintTile = (TileCoord, Arc<[u8]>, Arc<[u8]>);

/// What a painting tool does to the paint where it reaches (by an amount in `[0, 1]`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PaintOp {
    /// Lay a color (working space, straight; its alpha is ignored) over what is there: the
    /// Brush, Fill, Stroke.
    Color(LinearRgba),
    /// Lower the alpha of the result: the Eraser, Delete.
    Erase,
    /// Bring back what is below the paint: the Restore Eraser.
    Restore,
    /// Lay a gradient's colors, each pixel the one at its place in the document (the layer's
    /// pixels placed by `to_document`), its opacity scaling the amount: the Gradient tool.
    Gradient {
        field: crate::gradient::GradientField,
        to_document: Projective,
    },
    /// Lay the colors of a [`crate::clone::CloneSource`] (given to [`TopPaint::lay`]), each
    /// pixel the one `offset` away from its place in the document, its alpha scaling the
    /// amount: the Clone Stamp; with `tone`, the colors lightened or darkened (Dodge, Burn).
    Clone {
        offset: [f64; 2],
        to_document: Projective,
        tone: Option<crate::clone::Tone>,
    },
}

impl PaintOp {
    /// What `self` lays at pixel (`x`, `y`) of tile `coord` of a layer: the color (`color`,
    /// from [`op_color`], unless it varies across the layer) and the share of the amount.
    fn at(
        &self,
        math: &PaintPixels,
        color: Option<[f64; 4]>,
        source: Option<&crate::clone::CloneSource>,
        coord: TileCoord,
        x: usize,
        y: usize,
    ) -> (Option<[f64; 4]>, f64) {
        let place = |to_document: &Projective| {
            let t = f64::from(TILE_SIZE);
            to_document.apply(
                f64::from(coord.col) * t + x as f64 + 0.5,
                f64::from(coord.row) * t + y as f64 + 0.5,
            )
        };
        match self {
            PaintOp::Gradient { field, to_document } => {
                let (dx, dy) = place(to_document);
                let ([r, g, b], alpha) = field.at(dx, dy);
                let c = LinearRgba::from_srgb_encoded_to_working(r as f32, g as f32, b as f32, 1.0);
                (op_color(math, PaintOp::Color(c)), alpha)
            }
            PaintOp::Clone {
                offset,
                to_document,
                tone,
            } => {
                let Some(source) = source else {
                    return (None, 0.0);
                };
                let (dx, dy) = place(to_document);
                let [r, g, b, a] = source.at(dx + offset[0], dy + offset[1]);
                if a <= 0.0 {
                    return (None, 0.0);
                }
                let mut rgb = [r / a, g / a, b / a];
                if let Some(tone) = tone {
                    rgb = tone.apply(rgb);
                }
                let c = LinearRgba::new(rgb[0], rgb[1], rgb[2], 1.0);
                (op_color(math, PaintOp::Color(c)), f64::from(a.min(1.0)))
            }
            _ => (color, 1.0),
        }
    }
}

/// An applied adjustment (Image > Adjustments).
#[derive(Debug, Clone, PartialEq)]
pub struct Effect {
    pub adjustment: Adjustment,
    /// Where it applies (a coverage at the document origin); everywhere when `None`.
    pub selection: Option<Selection>,
    /// The layer's pixels → document when it was applied: where the selection is read, so that
    /// the effect stays where it was applied when the layer moves (projective for a layer in
    /// perspective, ADR 0038).
    pub to_document: Projective,
    pub space: BlendSpace,
}

impl Effect {
    pub fn is_valid(&self) -> bool {
        self.adjustment.is_valid()
            && self.to_document.is_finite()
            && self.to_document.inverse().is_some()
    }

    /// This effect then `next` as one (ADR 0034), when that is exact: Exposure's stops added
    /// (without offset nor gamma), Hue/Saturation's hue shifts added (without saturation nor
    /// lightness), two Inverts nothing (`Some(None)`). `None`: they stay two entries (Curves,
    /// Levels…, or another selection).
    fn combined(&self, next: &Effect) -> Option<Option<Effect>> {
        if self.selection != next.selection
            || self.to_document != next.to_document
            || self.space != next.space
        {
            return None;
        }
        let adjustment = match (self.adjustment, next.adjustment) {
            (Adjustment::Invert, Adjustment::Invert) => return Some(None),
            (
                Adjustment::Exposure {
                    exposure: a,
                    offset: 0.0,
                    gamma: 1.0,
                },
                Adjustment::Exposure {
                    exposure: b,
                    offset: 0.0,
                    gamma: 1.0,
                },
            ) => Adjustment::Exposure {
                exposure: a + b,
                offset: 0.0,
                gamma: 1.0,
            },
            (
                Adjustment::HueSaturation {
                    hue: a,
                    saturation: 0.0,
                    lightness: 0.0,
                },
                Adjustment::HueSaturation {
                    hue: b,
                    saturation: 0.0,
                    lightness: 0.0,
                },
            ) => {
                let mut hue = a + b;
                if hue > 180.0 {
                    hue -= 360.0;
                } else if hue < -180.0 {
                    hue += 360.0;
                }
                Adjustment::HueSaturation {
                    hue,
                    saturation: 0.0,
                    lightness: 0.0,
                }
            }
            _ => return None,
        };
        let combined = Effect {
            adjustment,
            ..self.clone()
        };
        // Out of range combined (more stops than Exposure allows): two entries.
        combined.is_valid().then_some(Some(combined))
    }
}

/// Effects of one kind applied in a row: one entry (deleting a paint between two effects of the
/// same kind joins them). Each step stays editable (ADR 0034).
#[derive(Debug)]
pub struct EffectEntry {
    steps: Vec<Arc<Effect>>,
    /// Hidden by its eye (ADR 0034): kept, not evaluated.
    hidden: bool,
}

impl EffectEntry {
    /// One entry of `steps`, in order: at least one, all valid and of the same kind.
    pub fn new(steps: Vec<Arc<Effect>>) -> Result<Self, StackError> {
        let first = steps.first().ok_or(StackError::InvalidEffect)?;
        let kind = first.adjustment.id();
        if steps
            .iter()
            .any(|s| !s.is_valid() || s.adjustment.id() != kind)
        {
            return Err(StackError::InvalidEffect);
        }
        Ok(Self {
            steps,
            hidden: false,
        })
    }

    pub fn steps(&self) -> &[Arc<Effect>] {
        &self.steps
    }

    /// Hidden by its eye (ADR 0034): kept in the stack, not evaluated.
    pub fn hidden(&self) -> bool {
        self.hidden
    }

    /// This entry, hidden or shown.
    pub fn with_hidden(self, hidden: bool) -> Self {
        Self { hidden, ..self }
    }

    /// This entry and `above` (above it) as one, when they meet (ADR 0034): both shown or both
    /// hidden, one step each, combined exactly (see [`Effect::combined`]); `Some(None)` when
    /// they cancel. `None`: they stay two entries.
    fn combined(&self, above: &EffectEntry) -> Option<Option<EffectEntry>> {
        let ([below], [top]) = (&self.steps[..], &above.steps[..]) else {
            return None;
        };
        if self.hidden != above.hidden {
            return None;
        }
        Some(below.combined(top)?.map(|one| EffectEntry {
            steps: vec![Arc::new(one)],
            hidden: self.hidden,
        }))
    }

    /// The adjustment's kind ([`Adjustment::id`]).
    pub fn kind(&self) -> &'static str {
        self.steps[0].adjustment.id()
    }
}

/// An applied filter (Filter > …, ADR 0034), as an [`Effect`] is an applied adjustment.
#[derive(Debug, Clone, PartialEq)]
pub struct FilterStep {
    pub filter: Filter,
    /// Where it applies (a coverage at the document origin); everywhere when `None`. The filter
    /// reads around it as well.
    pub selection: Option<Selection>,
    /// The layer's pixels → document when it was applied (see [`Effect::to_document`]).
    pub to_document: Projective,
    pub space: BlendSpace,
}

impl FilterStep {
    pub fn is_valid(&self) -> bool {
        self.filter.is_valid()
            && self.to_document.is_finite()
            && self.to_document.inverse().is_some()
    }

    /// This filter then `next` as one, when that is exact: two Gaussian Blurs are one of the
    /// root of the sum of their squared radii (ADR 0034). `None`: two entries.
    fn combined(&self, next: &FilterStep) -> Option<FilterStep> {
        if self.selection != next.selection
            || self.to_document != next.to_document
            || self.space != next.space
        {
            return None;
        }
        let filter = match (self.filter, next.filter) {
            (Filter::GaussianBlur { radius: a }, Filter::GaussianBlur { radius: b }) => {
                Filter::GaussianBlur {
                    radius: (a * a + b * b).sqrt(),
                }
            }
            // Sharpening twice is not one sharpening of some other settings.
            _ => return None,
        };
        let combined = FilterStep {
            filter,
            ..self.clone()
        };
        combined.is_valid().then_some(combined)
    }
}

/// Filters of one kind applied in a row: one entry, each step editable (ADR 0034). What it was
/// applied to and its result are cached: the entries above evaluate from that result.
#[derive(Debug)]
pub struct FilterEntry {
    steps: Vec<Arc<FilterStep>>,
    hidden: bool,
    cache: Mutex<Option<FilterCache>>,
    /// Held while its result is computed (the cache is not: what reads it never waits).
    computing: Mutex<()>,
}

/// What is below a filter entry (its stack), the result of that stack, and the entry's own
/// result once computed.
#[derive(Debug, Clone)]
struct FilterCache {
    below: LayerStack,
    input: Arc<RasterImage>,
    output: Option<Arc<RasterImage>>,
}

impl FilterEntry {
    /// One entry of `steps`, in order: at least one, all valid and of the same kind.
    pub fn new(steps: Vec<Arc<FilterStep>>) -> Result<Self, StackError> {
        let first = steps.first().ok_or(StackError::InvalidEffect)?;
        let kind = first.filter.id();
        if steps.iter().any(|s| !s.is_valid() || s.filter.id() != kind) {
            return Err(StackError::InvalidEffect);
        }
        Ok(Self {
            steps,
            hidden: false,
            computing: Mutex::new(()),
            cache: Mutex::new(None),
        })
    }

    pub fn steps(&self) -> &[Arc<FilterStep>] {
        &self.steps
    }

    /// The filter's kind ([`Filter::id`]).
    pub fn kind(&self) -> &'static str {
        self.steps[0].filter.id()
    }

    /// Hidden by its eye: kept in the stack, not evaluated.
    pub fn hidden(&self) -> bool {
        self.hidden
    }

    /// This entry, hidden or shown.
    pub fn with_hidden(self, hidden: bool) -> Self {
        Self { hidden, ..self }
    }

    /// This entry and `above` (above it) as one, when they meet (ADR 0034): both shown or both
    /// hidden, one step each, combined exactly (see [`FilterStep::combined`]); it knows what
    /// this one was applied to. `None`: they stay two entries.
    fn combined(&self, above: &FilterEntry) -> Option<FilterEntry> {
        let ([below], [top]) = (&self.steps[..], &above.steps[..]) else {
            return None;
        };
        if self.hidden != above.hidden {
            return None;
        }
        let one = FilterEntry {
            steps: vec![Arc::new(below.combined(top)?)],
            hidden: self.hidden,
            computing: Mutex::new(()),
            cache: Mutex::new(None),
        };
        Some(match self.known_input() {
            Some((stack, input)) => one.knowing(stack, input),
            None => one,
        })
    }

    /// This entry knowing what it is applied to: `below`'s result is `input` (the pixels a layer
    /// shows, when a filter is applied on top of it).
    fn knowing(self, below: LayerStack, input: Arc<RasterImage>) -> Self {
        Self {
            computing: Mutex::new(()),
            cache: Mutex::new(Some(FilterCache {
                below,
                input,
                output: None,
            })),
            ..self
        }
    }

    /// What this entry knows of what it was applied to, for an entry made from it (edited,
    /// joined): the same below, the same input.
    fn known_input(&self) -> Option<(LayerStack, Arc<RasterImage>)> {
        let cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        cache
            .as_ref()
            .map(|c| (c.below.clone(), Arc::clone(&c.input)))
    }

    /// Its result over `below`, if it is known.
    fn known_output(&self, below: &LayerStack) -> Option<Arc<RasterImage>> {
        let cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        cache
            .as_ref()
            .filter(|c| c.below == *below)
            .and_then(|c| c.output.clone())
    }

    /// The result of the entry over `below` (the stack of the entries below it): from its
    /// cache, or computed now (and `below` evaluated first when it was not known).
    fn output(&self, below: &LayerStack) -> Result<Arc<RasterImage>, StackError> {
        // One computation at a time, the cache free meanwhile (a new state of the layer reads
        // what it knows, the display its input): another may have computed it while this waited.
        let _computing = self
            .computing
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let known = known_cache(&self.cache, below);
        if let Some(output) = known.as_ref().and_then(|c| c.output.clone()) {
            return Ok(output);
        }
        let input = match known {
            Some(c) => c.input,
            None => below.evaluate()?,
        };
        let mut image = Arc::clone(&input);
        for step in &self.steps {
            image = Arc::new(filtered(step, &image, true)?);
        }
        *self.cache.lock().unwrap_or_else(PoisonError::into_inner) = Some(FilterCache {
            below: below.clone(),
            input,
            output: Some(Arc::clone(&image)),
        });
        Ok(image)
    }
}

/// What `cache` knows of the result of `below`, if it is about that stack.
fn known_cache(cache: &Mutex<Option<FilterCache>>, below: &LayerStack) -> Option<FilterCache> {
    let cache = cache.lock().unwrap_or_else(PoisonError::into_inner);
    cache.as_ref().filter(|c| c.below == *below).cloned()
}

/// A Liquify entry (ADR 0037): a displacement field warping the result of what is below it, the
/// layer's pixels read from where the field says. Like a filter it is a materialization point:
/// what it was applied to and its result are cached, the entries above evaluate from that result,
/// and the display shows a look at the part it shows. Edited again in its own workspace, which
/// replaces its field.
#[derive(Debug)]
pub struct LiquifyEntry {
    field: Arc<Field>,
    /// The blend space the pixels are interpolated in.
    space: BlendSpace,
    /// How far the field reads from, in the layer's pixels (the margin a crop needs).
    reach: f64,
    hidden: bool,
    cache: Mutex<Option<FilterCache>>,
    /// Held while its result is computed (the cache is not: what reads it never waits).
    computing: Mutex<()>,
    /// The field and its freeze as whole images, made once when asked (files store them so).
    images: OnceLock<(Arc<RasterImage>, Arc<RasterImage>)>,
}

impl LiquifyEntry {
    pub fn new(field: Arc<Field>, space: BlendSpace) -> Self {
        Self {
            reach: field.reach(),
            field,
            space,
            hidden: false,
            computing: Mutex::new(()),
            cache: Mutex::new(None),
            images: OnceLock::new(),
        }
    }

    /// An entry read back from its images ([`Self::images`]) over a layer of `size`, `cell`
    /// pixels a node.
    pub fn from_images(
        size: Size,
        cell: u32,
        space: BlendSpace,
        displacement: Arc<RasterImage>,
        frozen: Arc<RasterImage>,
    ) -> Result<Self, StackError> {
        let field = Field::from_images(size, cell, &displacement, &frozen)?;
        let entry = Self::new(Arc::new(field), space);
        let _ = entry.images.set((displacement, frozen));
        Ok(entry)
    }

    /// The field's displacements and freeze as whole images of one pixel per node (what files
    /// store): every tile it did not touch is one shared zero tile. Made once.
    pub fn images(&self) -> Result<(Arc<RasterImage>, Arc<RasterImage>), StackError> {
        if let Some(images) = self.images.get() {
            return Ok(images.clone());
        }
        let (displacement, frozen) = self.field.to_images()?;
        let images = (Arc::new(displacement), Arc::new(frozen));
        Ok(self.images.get_or_init(|| images).clone())
    }

    /// This entry with the images it has made (they describe the same field).
    fn sharing_images(self, from: &LiquifyEntry) -> Self {
        if let Some(images) = from.images.get() {
            let _ = self.images.set(images.clone());
        }
        self
    }

    pub fn field(&self) -> &Arc<Field> {
        &self.field
    }

    pub fn space(&self) -> BlendSpace {
        self.space
    }

    /// Hidden by its eye: kept in the stack, not evaluated.
    pub fn hidden(&self) -> bool {
        self.hidden
    }

    /// This entry, hidden or shown.
    pub fn with_hidden(self, hidden: bool) -> Self {
        Self { hidden, ..self }
    }

    /// This entry knowing what it is applied to (see [`FilterEntry::knowing`]).
    fn knowing(self, below: LayerStack, input: Arc<RasterImage>) -> Self {
        Self {
            computing: Mutex::new(()),
            cache: Mutex::new(Some(FilterCache {
                below,
                input,
                output: None,
            })),
            ..self
        }
    }

    /// What this entry knows of what it was applied to.
    fn known_input(&self) -> Option<(LayerStack, Arc<RasterImage>)> {
        let cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        cache
            .as_ref()
            .map(|c| (c.below.clone(), Arc::clone(&c.input)))
    }

    /// The result of the entry over `below`: from its cache, or computed now.
    fn output(&self, below: &LayerStack) -> Result<Arc<RasterImage>, StackError> {
        // As a filter's (see `FilterEntry::output`).
        let _computing = self
            .computing
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let known = known_cache(&self.cache, below);
        if let Some(output) = known.as_ref().and_then(|c| c.output.clone()) {
            return Ok(output);
        }
        let input = match known {
            Some(c) => c.input,
            None => below.evaluate()?,
        };
        let image = Arc::new(crate::liquify::warp_layer(&input, &self.field, self.space)?);
        if !crate::raster::wanted() {
            return Err(StackError::Cancelled);
        }
        *self.cache.lock().unwrap_or_else(PoisonError::into_inner) = Some(FilterCache {
            below: below.clone(),
            input,
            output: Some(Arc::clone(&image)),
        });
        Ok(image)
    }
}

/// Why a stack, an entry or a tile was refused.
#[derive(Debug, Clone, PartialEq)]
pub enum StackError {
    /// Paint of another size than the layer.
    SizeMismatch,
    /// Paint of another format than the layer's (with alpha).
    FormatMismatch,
    /// A tile of the wrong length.
    TileLength,
    /// A tile outside the layer.
    TileOutside(TileCoord),
    /// An effect that is not valid, or an entry mixing kinds.
    InvalidEffect,
    /// A Liquify field that does not fit its layer.
    Field(FieldError),
    /// No entry at this index.
    IndexOutOfRange(usize),
    Raster(RasterError),
    /// Given up: not wanted any more (see `raster::while_wanted`).
    Cancelled,
}

impl fmt::Display for StackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StackError::SizeMismatch => write!(f, "paint of another size than its layer"),
            StackError::FormatMismatch => write!(f, "paint of another format than its layer"),
            StackError::TileLength => write!(f, "paint tile of the wrong length"),
            StackError::TileOutside(coord) => write!(f, "paint tile {coord:?} outside its layer"),
            StackError::InvalidEffect => write!(f, "invalid effect"),
            StackError::Field(e) => write!(f, "{e}"),
            StackError::IndexOutOfRange(index) => write!(f, "no entry at index {index}"),
            StackError::Raster(e) => write!(f, "{e}"),
            StackError::Cancelled => write!(f, "given up: not wanted any more"),
        }
    }
}

impl std::error::Error for StackError {}

impl From<FieldError> for StackError {
    fn from(e: FieldError) -> Self {
        StackError::Field(e)
    }
}

impl From<RasterError> for StackError {
    fn from(e: RasterError) -> Self {
        StackError::Raster(e)
    }
}

/// The format of the paint of a layer of `format`: the same, with an alpha channel.
pub fn paint_format(format: PixelFormat) -> PixelFormat {
    let layout = if format.layout.is_gray() {
        ChannelLayout::GrayAlpha
    } else {
        ChannelLayout::Rgba
    };
    PixelFormat { layout, ..format }
}

/// The sample type of `k` beside `P`'s: the same grid for integers, so that `P`'s alpha and `k`
/// round to complementary values; `f32` for floats.
fn keep_sample(sample: SampleType) -> SampleType {
    match sample {
        SampleType::U8 => SampleType::U8,
        SampleType::U16 => SampleType::U16,
        SampleType::F16 | SampleType::F32 => SampleType::F32,
    }
}

fn read_keep(sample: SampleType, keep: &[u8], i: usize) -> f64 {
    let v = match sample {
        SampleType::U8 => f64::from(keep[i]) / 255.0,
        SampleType::U16 => f64::from(u16::from_ne_bytes([keep[2 * i], keep[2 * i + 1]])) / 65535.0,
        SampleType::F16 => f64::from(f16_to_f32(u16::from_ne_bytes([
            keep[2 * i],
            keep[2 * i + 1],
        ]))),
        SampleType::F32 => {
            let b = &keep[4 * i..4 * i + 4];
            f64::from(f32::from_ne_bytes([b[0], b[1], b[2], b[3]]))
        }
    };
    if v.is_nan() { 1.0 } else { v.clamp(0.0, 1.0) }
}

fn write_keep(sample: SampleType, v: f64, keep: &mut [u8], i: usize) {
    let v = v.clamp(0.0, 1.0);
    match sample {
        SampleType::U8 => keep[i] = (v * 255.0).round() as u8,
        SampleType::U16 => {
            keep[2 * i..2 * i + 2].copy_from_slice(&((v * 65535.0).round() as u16).to_ne_bytes())
        }
        SampleType::F16 => {
            keep[2 * i..2 * i + 2].copy_from_slice(&f32_to_f16(v as f32).to_ne_bytes())
        }
        SampleType::F32 => keep[4 * i..4 * i + 4].copy_from_slice(&(v as f32).to_ne_bytes()),
    }
}

/// Half a step of `sample`: what `P`'s alpha + `k` may miss 1 by and still keep a pixel opaque.
fn half_step(sample: SampleType) -> f64 {
    match sample {
        SampleType::U8 => 0.5 / 255.0,
        SampleType::U16 => 0.5 / 65535.0,
        SampleType::F16 | SampleType::F32 => 1e-6,
    }
}

/// Whether pixels of `format` hold the very values `space` blends: 8-bit sRGB color with
/// straight alpha in a perceptual document, the common case. They are then read and written as
/// they are, without decoding and encoding their transfer curve: the same values, much faster.
fn blends_as_stored(format: PixelFormat, space: BlendSpace) -> bool {
    format.sample == SampleType::U8
        && !format.layout.is_gray()
        && format.layout.has_alpha()
        && format.alpha == crate::color::AlphaMode::Straight
        && format.color_space == crate::color::ColorSpace::SRGB
        && space == BlendSpace::Perceptual
}

/// An 8-bit straight RGBA pixel (see [`blends_as_stored`]) as premultiplied blend values.
fn read_stored(px: &[u8]) -> [f64; 4] {
    let a = f64::from(px[3]) / 255.0;
    let v = |i: usize| f64::from(px[i]) / 255.0 * a;
    [v(0), v(1), v(2), a]
}

/// Premultiplied blend values as an 8-bit straight RGBA pixel (see [`blends_as_stored`]),
/// rounded and clamped as the codec writes them.
fn write_stored(values: [f64; 4], out: &mut [u8]) {
    let alpha = values[3].clamp(0.0, 1.0);
    if alpha <= 0.0 {
        out[..4].fill(0);
        return;
    }
    let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    for (n, sample) in out[..3].iter_mut().enumerate() {
        *sample = byte(values[n] / values[3]);
    }
    out[3] = byte(alpha);
}

/// The pixels of a layer of one format as premultiplied working-space colors, and back.
#[derive(Debug, Clone, Copy)]
struct LayerColors {
    gray: bool,
    to_working: Mat3,
    from_working: Mat3,
    luma: [f64; 3],
}

impl LayerColors {
    fn new(format: PixelFormat) -> Self {
        let space = format.color_space;
        let gray = format.layout.is_gray();
        Self {
            gray,
            // A gray value is its own luminance (as painting reads it, ADR 0027).
            to_working: if gray {
                IDENTITY
            } else {
                space.matrix_to(&WORKING_SPACE)
            },
            from_working: if gray {
                IDENTITY
            } else {
                WORKING_SPACE.matrix_to(&space)
            },
            luma: WORKING_SPACE.primaries.to_xyz()[1],
        }
    }

    fn read(&self, codec: &Codec, px: &[u8]) -> [f64; 4] {
        let (color, alpha) = codec.read_mapped(px, &mut |v| v);
        let color = mat_vec(&self.to_working, color.map(f64::from));
        [color[0], color[1], color[2], f64::from(alpha)]
    }

    fn write(&self, codec: &Codec, [r, g, b, a]: [f64; 4], out: &mut [u8]) {
        let rgb = if self.gray {
            [r * self.luma[0] + g * self.luma[1] + b * self.luma[2]; 3]
        } else {
            mat_vec(&self.from_working, [r, g, b])
        };
        codec.write(rgb.map(|v| v as f32), a as f32, out);
    }
}

/// A layer's pixels read as premultiplied values of a blend space, and written back.
#[derive(Debug)]
pub(crate) struct PremulPixels<'a> {
    level: &'a crate::raster::RasterLevel,
    size: Size,
    codec: Codec,
    colors: LayerColors,
    blender: Blender,
    /// The stored values are the blend values (see [`blends_as_stored`]).
    as_stored: bool,
}

impl<'a> PremulPixels<'a> {
    pub(crate) fn new(image: &'a RasterImage, space: BlendSpace) -> Self {
        Self::at_level(image, space, 0)
    }

    /// The pixels of pyramid level `level` (clamped to the coarsest).
    pub(crate) fn at_level(image: &'a RasterImage, space: BlendSpace, level: usize) -> Self {
        let stored = image.stored_format();
        let level = &image.levels()[level.min(image.levels().len() - 1)];
        Self {
            level,
            size: level.size(),
            codec: Codec::new(stored),
            colors: LayerColors::new(image.format()),
            blender: Blender::new(space),
            as_stored: blends_as_stored(stored, space),
        }
    }

    /// The stored pixels are 8-bit sRGB straight RGBA blended as stored: what a display
    /// shows without any conversion.
    pub(crate) fn is_displayed(&self) -> bool {
        self.as_stored
    }

    pub(crate) fn bytes_per_pixel(&self) -> usize {
        self.codec.bytes_per_pixel
    }

    /// The bytes of pixel (`x`, `y`), the edges repeating outward.
    pub(crate) fn raw(&self, x: i64, y: i64) -> Option<&[u8]> {
        let x = x.clamp(0, i64::from(self.size.width) - 1) as u32;
        let y = y.clamp(0, i64::from(self.size.height) - 1) as u32;
        let tile = self.level.tile(TileCoord {
            col: x / TILE_SIZE,
            row: y / TILE_SIZE,
        })?;
        let bpp = self.codec.bytes_per_pixel;
        let at = ((y % TILE_SIZE) * TILE_SIZE + x % TILE_SIZE) as usize * bpp;
        tile.get(at..at + bpp)
    }

    /// Pixel (`x`, `y`) as blend values, the edges repeating outward.
    pub(crate) fn tap(&self, x: i64, y: i64) -> [f64; 4] {
        self.raw(x, y).map_or([0.0; 4], |px| self.read(px))
    }

    /// Premultiplied blend values as premultiplied linear working-space color: what the
    /// display's conversion takes.
    pub(crate) fn to_linear(&self, values: [f64; 4]) -> [f32; 4] {
        self.blender.decode_premultiplied(&values).map(|v| v as f32)
    }

    pub(crate) fn read(&self, px: &[u8]) -> [f64; 4] {
        if self.as_stored {
            read_stored(px)
        } else {
            self.blender
                .encode_premultiplied(&self.colors.read(&self.codec, px))
        }
    }

    pub(crate) fn write(&self, values: [f64; 4], px: &mut [u8]) {
        if self.as_stored {
            write_stored(values, px);
        } else {
            let color = self.blender.decode_premultiplied(&values);
            self.colors.write(&self.codec, color, px);
        }
    }

    /// Pixels `x0..x0 + out.len()` of row `y` into `out`, the layer's edges repeating outward:
    /// each tile looked up once per run of its pixels.
    fn row(&self, x0: i64, y: i64, out: &mut [[f32; 4]]) {
        let (width, height) = (i64::from(self.size.width), i64::from(self.size.height));
        let y = y.clamp(0, height - 1) as u32;
        let (row, ty) = (y / TILE_SIZE, (y % TILE_SIZE) as usize);
        let bpp = self.codec.bytes_per_pixel;
        let t = TILE_SIZE as usize;
        let mut i = 0;
        while i < out.len() {
            let x = (x0 + i as i64).clamp(0, width - 1) as u32;
            let coord = TileCoord {
                col: x / TILE_SIZE,
                row,
            };
            // The run of `out` reading this tile: up to its right edge, or all of the repeated
            // edge pixels beyond the layer.
            let run = if x0 + (i as i64) < 0 {
                ((-x0) as usize - i).min(out.len() - i)
            } else if x0 + (i as i64) >= width {
                out.len() - i
            } else {
                ((TILE_SIZE - x % TILE_SIZE) as usize).min(out.len() - i)
            };
            match self.level.tile(coord) {
                Some(tile) => {
                    let start = (x % TILE_SIZE) as usize;
                    let edge = x0 + (i as i64) < 0 || x0 + (i as i64) >= width;
                    for n in 0..run {
                        let tx = if edge { start } else { start + n };
                        let at = (ty * t + tx) * bpp;
                        out[i + n] = self.read(&tile[at..at + bpp]).map(|v| v as f32);
                    }
                }
                None => out[i..i + run].fill([0.0; 4]),
            }
            i += run;
        }
    }

    /// Pixel (`x`, `y`), the layer's edges repeating outward.
    #[cfg(test)]
    fn at(&self, x: i64, y: i64) -> [f32; 4] {
        let x = x.clamp(0, i64::from(self.size.width) - 1) as u32;
        let y = y.clamp(0, i64::from(self.size.height) - 1) as u32;
        let coord = TileCoord {
            col: x / TILE_SIZE,
            row: y / TILE_SIZE,
        };
        let Some(tile) = self.level.tile(coord) else {
            return [0.0; 4];
        };
        let bpp = self.codec.bytes_per_pixel;
        let i = ((y % TILE_SIZE) * TILE_SIZE + x % TILE_SIZE) as usize;
        self.read(&tile[i * bpp..(i + 1) * bpp]).map(|v| v as f32)
    }
}

/// `input` (a layer's pixels) with `step` applied (ADR 0034): filtered in premultiplied values
/// of the step's blend space (the layer's edges repeating outward): blurred, then each pixel
/// made from itself and its blur (`Filter::finish`), mixed with what it was by the selection's
/// coverage, written in the image's own format. Tile by tile: the memory it takes does not grow
/// with the layer (see `filter::GaussianPlan`).
fn filtered(
    step: &FilterStep,
    input: &RasterImage,
    pyramid: bool,
) -> Result<RasterImage, StackError> {
    let size = input.size();
    let format = input.format();
    let pixels = PremulPixels::new(input, step.space);
    let bpp = pixels.codec.bytes_per_pixel;
    let t = TILE_SIZE as usize;
    let level = &input.levels()[0];
    let filter = step.filter;
    let plans = filter.plans();
    // A large radius: the layer reduced by the plan's factor (averages of blocks), blurred.
    let reduced: Vec<Option<Reduced>> = plans
        .iter()
        .map(|plan| (plan.factor > 1).then(|| Reduced::new(&pixels, size, plan)))
        .collect();
    if !crate::raster::wanted() {
        return Err(StackError::Cancelled);
    }

    let selection = step.selection.as_ref().map(|s| {
        let image = s.image().as_ref();
        (image, Codec::new(image.stored_format()))
    });
    let mut tiles: Vec<(TileCoord, Option<Arc<[u8]>>)> =
        grid_coords(size).map(|coord| (coord, None)).collect();
    parallel_for_each(&mut tiles, |(coord, out)| {
        let Some(tile) = level.tile(*coord).filter(|_| crate::raster::wanted()) else {
            return;
        };
        let (w, h) = valid_area(size, *coord);
        let (x0, y0) = (coord.col as usize * t, coord.row as usize * t);
        // A small radius: the tile and its margin, blurred on this thread.
        let margin = plans
            .iter()
            .zip(&reduced)
            .filter(|(_, reduced)| reduced.is_none())
            .map(|(plan, _)| plan.kernel.reach())
            .max()
            .unwrap_or(0);
        let (rw, rh) = (w + 2 * margin, h + 2 * margin);
        let mut source = vec![[0.0f32; 4]; rw * rh];
        for (y, row) in source.chunks_mut(rw).enumerate() {
            let (x, y) = (x0 as i64 - margin as i64, (y0 + y) as i64 - margin as i64);
            pixels.row(x, y, row);
        }
        let regions: Vec<Option<Vec<[f32; 4]>>> = plans
            .iter()
            .zip(&reduced)
            .map(|(plan, reduced)| {
                reduced.is_none().then(|| {
                    let mut region = source.clone();
                    plan.kernel.region(&mut region, rw, rh);
                    region
                })
            })
            .collect();
        let mut bytes = tile.to_vec();
        for y in 0..h {
            for x in 0..w {
                // The document pixel this one shows.
                let (dx, dy) = step
                    .to_document
                    .apply((x0 + x) as f64 + 0.5, (y0 + y) as f64 + 0.5);
                let (dx, dy) = (dx.floor(), dy.floor());
                let coverage = match &selection {
                    Some((image, codec)) => f64::from(MaskReader { image, codec }.at(dx, dy)),
                    None => 1.0,
                }
                .min(1.0);
                if coverage <= 0.0 {
                    continue;
                }
                let mut blurs = [[0.0f64; 4]; MAX_PLANS];
                for (n, (region, reduced)) in regions.iter().zip(&reduced).enumerate() {
                    blurs[n] = match (region, reduced) {
                        (Some(region), _) => region[(y + margin) * rw + x + margin].map(f64::from),
                        (None, Some(small)) => small.at(x0 + x, y0 + y),
                        (None, None) => [0.0; 4],
                    };
                }
                let blurs = &blurs[..plans.len()];
                let px = &mut bytes[(y * t + x) * bpp..][..bpp];
                let r = if coverage < 1.0 || filter.reads_original() {
                    let b = pixels.read(px);
                    let f = filter.finish(b, blurs, [dx as i64, dy as i64]);
                    std::array::from_fn(|n| b[n] + (f[n] - b[n]) * coverage)
                } else {
                    blurs[0]
                };
                pixels.write(r, px);
            }
        }
        pad_tile(&mut bytes, w, h, bpp);
        *out = Some(Arc::from(bytes));
    });
    // Given up meanwhile: tiles are missing.
    if !crate::raster::wanted() {
        return Err(StackError::Cancelled);
    }
    let tiles = tiles
        .into_iter()
        .map(|(coord, tile)| match tile {
            Some(tile) => tile,
            None => level
                .tile(coord)
                .map_or_else(|| Arc::from(vec![0; t * t * bpp]), Arc::clone),
        })
        .collect();
    Ok(if pyramid {
        RasterImage::from_level0_tiles(size, format, tiles)?
    } else {
        RasterImage::from_level0_tiles_only(size, format, tiles)?
    })
}

/// The most blurs a filter takes (Clarity and Texture's two).
const MAX_PLANS: usize = 2;

/// A layer reduced by a plan's factor (averages of blocks of pixels) and blurred by its kernel:
/// how a large radius is computed (see `filter::Plan`).
struct Reduced {
    image: Vec<[f32; 4]>,
    width: usize,
    height: usize,
    factor: usize,
}

impl Reduced {
    fn new(pixels: &PremulPixels<'_>, size: Size, plan: &crate::filter::Plan) -> Self {
        let factor = plan.factor;
        let (width, height) = (
            (size.width as usize).div_ceil(factor),
            (size.height as usize).div_ceil(factor),
        );
        let mut small = vec![[0.0f32; 4]; width * height];
        let mut rows: Vec<(usize, &mut [[f32; 4]])> = small.chunks_mut(width).enumerate().collect();
        let weight = 1.0 / (factor * factor) as f32;
        parallel_for_each(&mut rows, |(sy, row)| {
            let mut line = vec![[0.0f32; 4]; width * factor];
            let mut sums = vec![[0.0f32; 4]; width];
            for j in 0..factor {
                pixels.row(0, (*sy * factor + j) as i64, &mut line);
                for (sum, block) in sums.iter_mut().zip(line.chunks(factor)) {
                    for px in block {
                        for c in 0..4 {
                            sum[c] += px[c];
                        }
                    }
                }
            }
            for (out, sum) in row.iter_mut().zip(&sums) {
                *out = sum.map(|v| v * weight);
            }
        });
        Self {
            image: plan.kernel.image(small, width, height),
            width,
            height,
            factor,
        }
    }

    /// Pixel (`x`, `y`) of the layer, read back between the reduced pixels' centers.
    fn at(&self, x: usize, y: usize) -> [f64; 4] {
        let f = self.factor as f64;
        let u = ((x as f64 + 0.5) / f - 0.5).clamp(0.0, (self.width - 1) as f64);
        let v = ((y as f64 + 0.5) / f - 0.5).clamp(0.0, (self.height - 1) as f64);
        let (x0, y0) = (u.floor() as usize, v.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
        let (a, b) = (u - x0 as f64, v - y0 as f64);
        let px = |x: usize, y: usize| self.image[y * self.width + x].map(f64::from);
        let (p00, p10, p01, p11) = (px(x0, y0), px(x1, y0), px(x0, y1), px(x1, y1));
        std::array::from_fn(|c| {
            (p00[c] * (1.0 - a) + p10[c] * a) * (1.0 - b) + (p01[c] * (1.0 - a) + p11[c] * a) * b
        })
    }
}

/// The valid pixels (width, height) of tile `coord` of an image of `size`.
fn valid_area(size: Size, coord: TileCoord) -> (usize, usize) {
    (
        (size.width - coord.col * TILE_SIZE).min(TILE_SIZE) as usize,
        (size.height - coord.row * TILE_SIZE).min(TILE_SIZE) as usize,
    )
}

fn grid_coords(size: Size) -> impl Iterator<Item = TileCoord> {
    let (columns, rows) = (
        size.width.div_ceil(TILE_SIZE),
        size.height.div_ceil(TILE_SIZE),
    );
    (0..rows).flat_map(move |row| (0..columns).map(move |col| TileCoord { col, row }))
}

fn contains(size: Size, coord: TileCoord) -> bool {
    coord.col < size.width.div_ceil(TILE_SIZE) && coord.row < size.height.div_ceil(TILE_SIZE)
}

/// Encodes and decodes the paint of one entry.
struct PaintPixels {
    /// `P`'s format.
    format: PixelFormat,
    /// `P`'s pixels are the blend values as they are (see [`blends_as_stored`]).
    stored: bool,
    pixels: LayerColors,
    codec: Codec,
    keep: SampleType,
    bpp: usize,
    blender: Blender,
}

impl PaintPixels {
    fn new(format: PixelFormat, space: BlendSpace) -> Self {
        let codec = Codec::new(format);
        Self {
            format,
            stored: blends_as_stored(stored_format(format), space),
            pixels: LayerColors::new(format),
            bpp: codec.bytes_per_pixel,
            codec,
            keep: keep_sample(format.sample),
            blender: Blender::new(space),
        }
    }

    /// `P` (premultiplied blend-space values) and `k` of pixel `i`.
    fn read(&self, color: &[u8], keep: &[u8], i: usize) -> ([f64; 4], f64) {
        let px = &color[i * self.bpp..(i + 1) * self.bpp];
        if self.stored {
            return (read_stored(px), read_keep(self.keep, keep, i));
        }
        let p = self.pixels.read(&self.codec, px);
        (
            self.blender.encode_premultiplied(&p),
            read_keep(self.keep, keep, i),
        )
    }

    fn write(&self, p: [f64; 4], k: f64, color: &mut [u8], keep: &mut [u8], i: usize) {
        if self.stored {
            write_stored(p, &mut color[i * self.bpp..(i + 1) * self.bpp]);
            write_keep(self.keep, k, keep, i);
            return;
        }
        let p = self.blender.decode_premultiplied(&p);
        self.pixels
            .write(&self.codec, p, &mut color[i * self.bpp..(i + 1) * self.bpp]);
        write_keep(self.keep, k, keep, i);
    }

    /// The identity tiles: `P = 0`, `k = 1`.
    fn identity(&self) -> (Vec<u8>, Vec<u8>) {
        let mut keep = vec![0; TILE_PIXELS * self.keep.bytes() as usize];
        for i in 0..TILE_PIXELS {
            write_keep(self.keep, 1.0, &mut keep, i);
        }
        (vec![0; TILE_PIXELS * self.bpp], keep)
    }

    /// `tile` once its valid pixels (`from`: width, height) become the first ones of a tile
    /// whose valid pixels are `to`: the identity where it had only padding, padded again.
    fn reframed(&self, tile: &PaintTile, from: (usize, usize), to: (usize, usize)) -> PaintTile {
        if from == to {
            return tile.clone();
        }
        let (mut color, mut keep) = self.identity();
        let t = TILE_SIZE as usize;
        let kb = self.keep.bytes() as usize;
        for y in 0..from.1 {
            let (a, b) = (y * t, y * t + from.0);
            color[a * self.bpp..b * self.bpp]
                .copy_from_slice(&tile.color[a * self.bpp..b * self.bpp]);
            keep[a * kb..b * kb].copy_from_slice(&tile.keep[a * kb..b * kb]);
        }
        pad_tile(&mut color, to.0, to.1, self.bpp);
        pad_tile(&mut keep, to.0, to.1, kb);
        let mut out = self.tile(color, keep);
        out.lowers_alpha |= tile.lowers_alpha;
        out
    }

    /// A tile from its padded buffers.
    fn tile(&self, color: Vec<u8>, keep: Vec<u8>) -> PaintTile {
        let lowers_alpha = (0..TILE_PIXELS).any(|i| self.lowers_alpha(&color, &keep, i));
        self.tile_lowering(color, keep, lowers_alpha)
    }

    /// A tile from its padded buffers, whether it lowers alpha known: its flag is never below
    /// what its pixels say, so a tile changed in some pixels lowers alpha when it did before or
    /// when one of them does (what a stroke's frames and Fill know without reading the others).
    fn tile_lowering(&self, color: Vec<u8>, keep: Vec<u8>, lowers_alpha: bool) -> PaintTile {
        PaintTile {
            color: Arc::from(color),
            keep: Arc::from(keep),
            lowers_alpha,
        }
    }

    /// Whether pixel `i` lowers the alpha of an opaque pixel: `P`'s alpha + `k` below 1.
    fn lowers_alpha(&self, color: &[u8], keep: &[u8], i: usize) -> bool {
        let alpha = f64::from(self.codec.alpha(&color[i * self.bpp..(i + 1) * self.bpp]));
        alpha + read_keep(self.keep, keep, i) < 1.0 - half_step(self.keep)
    }
}

impl PaintEntry {
    /// No paint yet, on a layer of `layer_format` and `size`, blending in `space`.
    pub fn empty(layer_format: PixelFormat, size: Size, space: BlendSpace) -> Self {
        Self {
            size,
            format: paint_format(layer_format),
            space,
            images: OnceLock::new(),
            below: Mutex::new(None),
            hidden: false,
            tiles: BTreeMap::new(),
        }
    }

    /// Paint read back (e.g. from a file): `P` and `k` of each tile it touched.
    pub fn from_tiles(
        layer_format: PixelFormat,
        size: Size,
        space: BlendSpace,
        tiles: Vec<RawPaintTile>,
    ) -> Result<Self, StackError> {
        let empty = Self::empty(layer_format, size, space);
        let math = empty.math();
        let mut out = BTreeMap::new();
        for (coord, color, keep) in tiles {
            out.insert(
                coord,
                empty.checked(&math, coord, color.to_vec(), keep.to_vec())?,
            );
        }
        Ok(Self {
            tiles: out,
            ..empty
        })
    }

    /// The paint of an image painted the way ADR 0027 did (a `.slop` v6 file): `P` the painted
    /// pixels and `k = 0` where they differ from the original, the identity elsewhere. The
    /// layer then shows exactly the painted pixels.
    pub fn from_painted(
        original: &RasterImage,
        painted: &RasterImage,
        space: BlendSpace,
    ) -> Result<Self, StackError> {
        Self::between(original.format(), original, painted, space)
    }

    /// The paint that turns `before` into `after` (two images of a layer of `layer_format`):
    /// `P` the pixels of `after` and `k = 0` where they differ, the identity elsewhere. What
    /// tools that read pixels write (moved pixels, ADR 0029): the values they took are baked.
    /// Only the tiles that are not shared are compared.
    pub fn between(
        layer_format: PixelFormat,
        before: &RasterImage,
        after: &RasterImage,
        space: BlendSpace,
    ) -> Result<Self, StackError> {
        let size = before.size();
        if after.size() != size {
            return Err(StackError::SizeMismatch);
        }
        let empty = Self::empty(layer_format, size, space);
        let math = empty.math();
        let (painted, original) = (after, before);
        let (before, after) = (&original.levels()[0], &painted.levels()[0]);
        let (old, new) = (
            Codec::new(original.stored_format()),
            Codec::new(painted.stored_format()),
        );
        let same_format = painted.stored_format() == empty.format;
        let (old_bpp, new_bpp) = (old.bytes_per_pixel, new.bytes_per_pixel);
        let mut work: Vec<(TileCoord, Option<PaintTile>)> = grid_coords(size)
            .filter(|&coord| match (before.tile(coord), after.tile(coord)) {
                (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
                _ => false,
            })
            .map(|coord| (coord, None))
            .collect();
        parallel_for_each(&mut work, |(coord, out)| {
            let (Some(a), Some(b)) = (before.tile(*coord), after.tile(*coord)) else {
                return;
            };
            let (mut color, mut keep) = math.identity();
            let (width, height) = valid_area(size, *coord);
            let mut changed = false;
            for y in 0..height {
                for x in 0..width {
                    let i = y * TILE_SIZE as usize + x;
                    let (pa, pb) = (
                        &a[i * old_bpp..(i + 1) * old_bpp],
                        &b[i * new_bpp..(i + 1) * new_bpp],
                    );
                    if old.read_mapped(pa, &mut |v| v) == new.read_mapped(pb, &mut |v| v) {
                        continue;
                    }
                    changed = true;
                    let p = &mut color[i * math.bpp..(i + 1) * math.bpp];
                    if same_format {
                        p.copy_from_slice(pb);
                    } else {
                        math.pixels
                            .write(&math.codec, math.pixels.read(&new, pb), p);
                    }
                    write_keep(math.keep, 0.0, &mut keep, i);
                }
            }
            if changed {
                pad_tile(&mut color, width, height, math.bpp);
                pad_tile(&mut keep, width, height, math.keep.bytes() as usize);
                *out = Some(math.tile(color, keep));
            }
        });
        let tiles = work
            .into_iter()
            .filter_map(|(coord, tile)| tile.map(|t| (coord, t)))
            .collect();
        Ok(Self { tiles, ..empty })
    }

    pub fn size(&self) -> Size {
        self.size
    }

    /// The format of `P`.
    pub fn format(&self) -> PixelFormat {
        self.format
    }

    /// The sample type of `k`.
    pub fn keep_sample(&self) -> SampleType {
        keep_sample(self.format.sample)
    }

    pub fn space(&self) -> BlendSpace {
        self.space
    }

    /// The tiles it touched; every other tile is the identity.
    pub fn tiles(&self) -> &BTreeMap<TileCoord, PaintTile> {
        &self.tiles
    }

    /// The format of `k` as an image: gray samples read as they are.
    pub fn keep_format(&self) -> PixelFormat {
        PixelFormat {
            layout: ChannelLayout::Gray,
            sample: self.keep_sample(),
            color_space: crate::color::ColorSpace::LINEAR_SRGB,
            alpha: crate::color::AlphaMode::Straight,
        }
    }

    /// `P` and `k` as whole images of the layer's size (what files store): every tile it did
    /// not touch is one shared identity tile. Made once.
    pub fn images(&self) -> Result<(Arc<RasterImage>, Arc<RasterImage>), StackError> {
        if let Some(images) = self.images.get() {
            return Ok(images.clone());
        }
        let math = self.math();
        let (color, keep) = math.identity();
        let (color, keep): (Arc<[u8]>, Arc<[u8]>) = (Arc::from(color), Arc::from(keep));
        let coords: Vec<TileCoord> = grid_coords(self.size).collect();
        let pick = |of: fn(&PaintTile) -> &Arc<[u8]>, identity: &Arc<[u8]>| -> Vec<Arc<[u8]>> {
            coords
                .iter()
                .map(|c| {
                    self.tiles
                        .get(c)
                        .map_or_else(|| Arc::clone(identity), |t| Arc::clone(of(t)))
                })
                .collect()
        };
        let images = (
            Arc::new(RasterImage::from_level0_tiles(
                self.size,
                self.format,
                pick(PaintTile::color, &color),
            )?),
            Arc::new(RasterImage::from_level0_tiles(
                self.size,
                self.keep_format(),
                pick(PaintTile::keep, &keep),
            )?),
        );
        Ok(self.images.get_or_init(|| images).clone())
    }

    /// [`Self::images`], borrowed: made now if they are not yet; `None` if they cannot be.
    pub fn image_refs(&self) -> Option<&(Arc<RasterImage>, Arc<RasterImage>)> {
        self.images().ok()?;
        self.images.get()
    }

    /// Paint read back from its images ([`Self::images`]) on a layer of `layer_format`.
    pub fn from_images(
        layer_format: PixelFormat,
        color: &RasterImage,
        keep: &RasterImage,
        space: BlendSpace,
    ) -> Result<Self, StackError> {
        let empty = Self::empty(layer_format, color.size(), space);
        if keep.size() != color.size() {
            return Err(StackError::SizeMismatch);
        }
        if color.format() != empty.format || keep.format() != empty.keep_format() {
            return Err(StackError::FormatMismatch);
        }
        let math = empty.math();
        let (identity_color, identity_keep) = math.identity();
        let (colors, keeps) = (&color.levels()[0], &keep.levels()[0]);
        let mut tiles = Vec::new();
        for coord in grid_coords(empty.size) {
            let (Some(c), Some(k)) = (colors.tile(coord), keeps.tile(coord)) else {
                return Err(StackError::TileOutside(coord));
            };
            if **c != *identity_color || **k != *identity_keep {
                tiles.push((coord, Arc::clone(c), Arc::clone(k)));
            }
        }
        Self::from_tiles(layer_format, empty.size, space, tiles)
    }

    /// Bytes its tiles hold (tiles shared between positions counted once).
    pub fn memory_bytes(&self) -> u64 {
        let mut seen = HashSet::new();
        self.tiles
            .values()
            .flat_map(|t| [&t.color, &t.keep])
            .filter(|tile| seen.insert(tile.as_ptr() as usize))
            .map(|tile| tile.len() as u64)
            .sum()
    }

    /// Whether it changes nothing.
    pub fn is_identity(&self) -> bool {
        self.tiles.is_empty()
    }

    /// Whether it can lower the alpha of an opaque pixel (the layer then needs an alpha
    /// channel).
    pub fn lowers_alpha(&self) -> bool {
        self.tiles.values().any(|t| t.lowers_alpha)
    }

    fn math(&self) -> PaintPixels {
        PaintPixels::new(self.format, self.space)
    }

    fn checked(
        &self,
        math: &PaintPixels,
        coord: TileCoord,
        color: Vec<u8>,
        keep: Vec<u8>,
    ) -> Result<PaintTile, StackError> {
        if !contains(self.size, coord) {
            return Err(StackError::TileOutside(coord));
        }
        if color.len() != TILE_PIXELS * math.bpp
            || keep.len() != TILE_PIXELS * math.keep.bytes() as usize
        {
            return Err(StackError::TileLength);
        }
        Ok(math.tile(color, keep))
    }

    /// Tile `coord` of this paint with `op` applied at `amount(x, y)` (clamped to `[0, 1]`) to
    /// each valid pixel of it: a stroke recomputes the tiles it reaches from the paint it
    /// started from. The tile must be inside the layer.
    pub fn painted_tile(
        &self,
        coord: TileCoord,
        op: PaintOp,
        amount: impl Fn(usize, usize) -> f32,
    ) -> PaintTile {
        let math = self.math();
        let (mut color, mut keep) = match self.tiles.get(&coord) {
            Some(tile) => (tile.color.to_vec(), tile.keep.to_vec()),
            None => math.identity(),
        };
        let paint = op_color(&math, op);
        let (width, height) = valid_area(self.size, coord);
        // Erased once, a tile keeps asking for an alpha channel, so that the layer's format
        // does not depend on how small the erasing was.
        let mut erased = self.tiles.get(&coord).is_some_and(|t| t.lowers_alpha);
        // Only the pixels written can lower alpha where the tile did not.
        let mut lowers = erased;
        for y in 0..height {
            for x in 0..width {
                let a = f64::from(amount(x, y));
                if a.is_nan() || a <= 0.0 {
                    continue;
                }
                let (paint, share) = op.at(&math, paint, None, coord, x, y);
                let a = (a * share).min(1.0);
                if a <= 0.0 {
                    continue;
                }
                erased |= op == PaintOp::Erase;
                let i = y * TILE_SIZE as usize + x;
                let (p, k) = math.read(&color, &keep, i);
                let (p, k) = lay(op, paint, a, p, k);
                math.write(p, k, &mut color, &mut keep, i);
                lowers = lowers || math.lowers_alpha(&color, &keep, i);
            }
        }
        pad_tile(&mut color, width, height, math.bpp);
        pad_tile(&mut keep, width, height, math.keep.bytes() as usize);
        math.tile_lowering(color, keep, lowers || erased)
    }

    /// This paint with `tiles` replaced (tiles made by [`Self::painted_tile`] of a paint of the
    /// same format, or read back).
    pub fn with_tiles(&self, tiles: Vec<(TileCoord, PaintTile)>) -> Result<Self, StackError> {
        let math = self.math();
        let mut out = self.tiles.clone();
        for (coord, tile) in tiles {
            if !contains(self.size, coord) {
                return Err(StackError::TileOutside(coord));
            }
            if tile.color.len() != TILE_PIXELS * math.bpp
                || tile.keep.len() != TILE_PIXELS * math.keep.bytes() as usize
            {
                return Err(StackError::TileLength);
            }
            out.insert(coord, tile);
        }
        Ok(Self {
            tiles: out,
            size: self.size,
            format: self.format,
            space: self.space,
            images: OnceLock::new(),
            below: Mutex::new(None),
            hidden: self.hidden,
        })
    }

    /// Whether `above` can merge with this paint (below it): same layer and blend space, both
    /// shown or both hidden.
    fn merges_with(&self, above: &PaintEntry) -> bool {
        self.size == above.size
            && self.format == above.format
            && self.space == above.space
            && self.hidden == above.hidden
    }

    /// Hidden by its eye (ADR 0034): kept in the stack, not evaluated.
    pub fn hidden(&self) -> bool {
        self.hidden
    }

    /// This paint, hidden or shown (read back from a file).
    pub fn with_hidden(self, hidden: bool) -> Self {
        Self { hidden, ..self }
    }

    /// A copy of this paint, hidden or shown: its tiles shared.
    fn hidden_copy(&self, hidden: bool) -> Self {
        let images = OnceLock::new();
        if let Some(made) = self.images.get() {
            let _ = images.set(made.clone());
        }
        Self {
            size: self.size,
            format: self.format,
            space: self.space,
            tiles: self.tiles.clone(),
            images,
            below: Mutex::new(None),
            hidden,
        }
    }

    /// This paint then `above`, as one: `P₂ + k₂·P₁`, `k₂·k₁`.
    fn merged(&self, above: &PaintEntry) -> PaintEntry {
        let math = self.math();
        let coords: BTreeSet<TileCoord> = self
            .tiles
            .keys()
            .chain(above.tiles.keys())
            .copied()
            .collect();
        let mut work: Vec<(TileCoord, Option<PaintTile>)> =
            coords.into_iter().map(|coord| (coord, None)).collect();
        parallel_for_each(&mut work, |(coord, out)| {
            let tile = match (self.tiles.get(coord), above.tiles.get(coord)) {
                (Some(below), Some(top)) => {
                    let (mut color, mut keep) = math.identity();
                    let (width, height) = valid_area(self.size, *coord);
                    for y in 0..height {
                        for x in 0..width {
                            let i = y * TILE_SIZE as usize + x;
                            let (p1, k1) = math.read(&below.color, &below.keep, i);
                            let (p2, k2) = math.read(&top.color, &top.keep, i);
                            let p = std::array::from_fn(|n| p2[n] + k2 * p1[n]);
                            math.write(p, k2 * k1, &mut color, &mut keep, i);
                        }
                    }
                    pad_tile(&mut color, width, height, math.bpp);
                    pad_tile(&mut keep, width, height, math.keep.bytes() as usize);
                    let mut tile = math.tile(color, keep);
                    tile.lowers_alpha |= below.lowers_alpha || top.lowers_alpha;
                    tile
                }
                (Some(only), None) | (None, Some(only)) => only.clone(),
                (None, None) => return,
            };
            *out = Some(tile);
        });
        PaintEntry {
            size: self.size,
            format: self.format,
            space: self.space,
            images: OnceLock::new(),
            below: Mutex::new(None),
            hidden: self.hidden,
            tiles: work
                .into_iter()
                .filter_map(|(coord, tile)| tile.map(|t| (coord, t)))
                .collect(),
        }
    }
}

/// `op`'s color as premultiplied blend-space values (`None` for the erasers).
fn op_color(math: &PaintPixels, op: PaintOp) -> Option<[f64; 4]> {
    match op {
        PaintOp::Color(c) => Some(math.blender.encode_premultiplied(&[
            f64::from(c.r),
            f64::from(c.g),
            f64::from(c.b),
            1.0,
        ])),
        PaintOp::Erase | PaintOp::Restore | PaintOp::Gradient { .. } | PaintOp::Clone { .. } => {
            None
        }
    }
}

/// `P` and `k` once `op` (its color from [`op_color`]) is laid at `a` in `(0, 1]` over them.
fn lay(op: PaintOp, color: Option<[f64; 4]>, a: f64, p: [f64; 4], k: f64) -> ([f64; 4], f64) {
    match (op, color) {
        (PaintOp::Color(_) | PaintOp::Gradient { .. } | PaintOp::Clone { .. }, Some(c)) => (
            std::array::from_fn(|n| a * c[n] + (1.0 - a) * p[n]),
            (1.0 - a) * k,
        ),
        (PaintOp::Restore, _) => (p.map(|v| (1.0 - a) * v), (1.0 - a) * k + a),
        _ => (p.map(|v| (1.0 - a) * v), (1.0 - a) * k),
    }
}

/// The tiles of a layer that one paint or one effect step reaches.
#[derive(Debug, Clone, PartialEq)]
enum Footprint {
    All,
    Tiles(BTreeSet<TileCoord>),
}

impl Footprint {
    fn of_paint(paint: &PaintEntry) -> Self {
        Self::Tiles(paint.tiles.keys().copied().collect())
    }

    /// The tiles under the selection's bounds (one pixel more around them), all without one.
    fn of_effect(effect: &Effect, size: Size) -> Self {
        let Some(selection) = &effect.selection else {
            return Self::All;
        };
        let Some(bounds) = crate::selection::bounds(selection.image()) else {
            return Self::Tiles(BTreeSet::new());
        };
        let Some(to_image) = effect.to_document.inverse() else {
            return Self::Tiles(BTreeSet::new());
        };
        let [x0, y0, x1, y1] = to_image.map_rect([
            f64::from(bounds.x),
            f64::from(bounds.y),
            bounds.right() as f64,
            bounds.bottom() as f64,
        ]);
        let (w, h) = (f64::from(size.width), f64::from(size.height));
        let (x0, y0) = ((x0 - 1.0).clamp(0.0, w), (y0 - 1.0).clamp(0.0, h));
        let (x1, y1) = ((x1 + 1.0).clamp(0.0, w), (y1 + 1.0).clamp(0.0, h));
        let t = f64::from(TILE_SIZE);
        let mut tiles = BTreeSet::new();
        if x0 < x1 && y0 < y1 {
            for row in (y0 / t) as u32..(y1 / t).ceil() as u32 {
                for col in (x0 / t) as u32..(x1 / t).ceil() as u32 {
                    tiles.insert(TileCoord { col, row });
                }
            }
        }
        Self::Tiles(tiles)
    }

    fn reaches(&self, coord: TileCoord) -> bool {
        match self {
            Self::All => true,
            Self::Tiles(tiles) => tiles.contains(&coord),
        }
    }
}

/// One paint, or one step of an effect entry: what the stack evaluates in order, and what it
/// compares to know which tiles changed.
#[derive(Debug, Clone, Copy)]
enum Atom<'a> {
    Paint(&'a Arc<PaintEntry>),
    Effect(&'a Arc<Effect>),
}

impl Atom<'_> {
    fn address(self) -> usize {
        match self {
            Atom::Paint(p) => Arc::as_ptr(p) as *const () as usize,
            Atom::Effect(e) => Arc::as_ptr(e) as *const () as usize,
        }
    }

    fn footprint(self, size: Size) -> Footprint {
        match self {
            Atom::Paint(p) => Footprint::of_paint(p),
            Atom::Effect(e) => Footprint::of_effect(e, size),
        }
    }
}

fn atoms(entries: &[Entry]) -> Vec<Atom<'_>> {
    let mut out = Vec::new();
    for entry in entries.iter().filter(|e| !e.hidden()) {
        match entry {
            Entry::Paint(p) => out.push(Atom::Paint(p)),
            Entry::Effect(e) => out.extend(e.steps.iter().map(Atom::Effect)),
            // Evaluated whole, before: see `LayerStack::flat`.
            Entry::Filter(_) | Entry::Liquify(_) => {}
        }
    }
    out
}

/// One atom, ready to evaluate.
enum Step<'a> {
    Paint {
        paint: &'a PaintEntry,
        math: PaintPixels,
    },
    Effect {
        effect: &'a Effect,
        prepared: Prepared,
        blender: Blender,
        selection: Option<(&'a RasterImage, Codec)>,
        footprint: Footprint,
    },
}

impl Step<'_> {
    fn reaches(&self, coord: TileCoord) -> bool {
        match self {
            Step::Paint { paint, .. } => paint.tiles.contains_key(&coord),
            Step::Effect { footprint, .. } => footprint.reaches(coord),
        }
    }
}

/// Evaluates tiles of a stack's result.
struct Evaluator<'a> {
    size: Size,
    original: &'a RasterImage,
    pixels: LayerColors,
    /// The original's codec, and the result's.
    source: Codec,
    target: Codec,
    /// The result's stored format.
    format: PixelFormat,
    /// The original's tiles must be converted to the result's format (a gray layer given an
    /// alpha channel).
    convert: bool,
    steps: Vec<Step<'a>>,
}

impl<'a> Evaluator<'a> {
    /// The evaluator of `atoms` over `stack`'s original, into `format` (the stack's, or the
    /// format a stroke shows).
    fn new(stack: &'a LayerStack, atoms: &[Atom<'a>], format: PixelFormat) -> Self {
        let original = stack.original.as_ref();
        let size = original.size();
        let steps = atoms
            .iter()
            .map(|&atom| match atom {
                Atom::Paint(paint) => Step::Paint {
                    paint,
                    math: paint.math(),
                },
                Atom::Effect(effect) => Step::Effect {
                    effect,
                    prepared: effect.adjustment.prepare(),
                    blender: Blender::new(effect.space),
                    selection: effect.selection.as_ref().map(|s| {
                        let image = s.image().as_ref();
                        (image, Codec::new(image.stored_format()))
                    }),
                    footprint: Footprint::of_effect(effect, size),
                },
            })
            .collect();
        Self {
            size,
            original,
            pixels: LayerColors::new(format),
            source: Codec::new(original.stored_format()),
            target: Codec::new(stored_format(format)),
            format: stored_format(format),
            convert: original.stored_format() != stored_format(format),
            steps,
        }
    }

    /// Tile `coord` of the result: from `start` (the result of the steps before `from`), or
    /// from the original when `None`. Shared with it when no step reaches the tile.
    fn tile(&self, coord: TileCoord, from: usize, start: Option<&Arc<[u8]>>) -> Arc<[u8]> {
        let current = match start {
            Some(tile) => Arc::clone(tile),
            None => self.original_tile(coord),
        };
        let mut buffer: Option<Vec<u8>> = None;
        for step in &self.steps[from..] {
            if step.reaches(coord) {
                let rows = buffer.get_or_insert_with(|| current.to_vec());
                self.apply_rows(step, coord, rows, 0);
            }
        }
        self.finished(coord, buffer).unwrap_or(current)
    }

    /// [`Self::tile`] from the original, each step on every core (bands of rows): one tile
    /// as fast as many, for a stroke reaching a new tile.
    fn tile_on_every_core(&self, coord: TileCoord) -> Arc<[u8]> {
        let current = self.original_tile(coord);
        let row_bytes = TILE_SIZE as usize * self.target.bytes_per_pixel;
        let height = valid_area(self.size, coord).1;
        let mut buffer: Option<Vec<u8>> = None;
        for step in &self.steps {
            if step.reaches(coord) {
                let rows = buffer.get_or_insert_with(|| current.to_vec());
                let mut bands: Vec<(usize, &mut [u8])> = rows[..height * row_bytes]
                    .chunks_mut(BAND_ROWS * row_bytes)
                    .enumerate()
                    .map(|(n, band)| (n * BAND_ROWS, band))
                    .collect();
                parallel_for_each(&mut bands, |(first, band)| {
                    self.apply_rows(step, coord, band, *first);
                });
            }
        }
        self.finished(coord, buffer).unwrap_or(current)
    }

    /// A tile computed into `buffer`, padded.
    fn finished(&self, coord: TileCoord, buffer: Option<Vec<u8>>) -> Option<Arc<[u8]>> {
        let mut buffer = buffer?;
        let (width, height) = valid_area(self.size, coord);
        pad_tile(&mut buffer, width, height, self.target.bytes_per_pixel);
        Some(Arc::from(buffer))
    }

    fn original_tile(&self, coord: TileCoord) -> Arc<[u8]> {
        let Some(tile) = self.original.levels()[0].tile(coord) else {
            return Arc::from(vec![0; TILE_PIXELS * self.target.bytes_per_pixel]);
        };
        if !self.convert {
            return Arc::clone(tile);
        }
        let (from, to) = (self.source.bytes_per_pixel, self.target.bytes_per_pixel);
        let mut out = vec![0; TILE_PIXELS * to];
        for i in 0..TILE_PIXELS {
            let (color, alpha) = self
                .source
                .read_mapped(&tile[i * from..(i + 1) * from], &mut |v| v);
            self.target
                .write(color, alpha, &mut out[i * to..(i + 1) * to]);
        }
        Arc::from(out)
    }

    /// `step` applied in place to `rows`, whole rows of tile `coord` from row `first` on (its
    /// valid pixels; the caller pads).
    fn apply_rows(&self, step: &Step<'_>, coord: TileCoord, rows: &mut [u8], first: usize) {
        let bpp = self.target.bytes_per_pixel;
        let t = TILE_SIZE as usize;
        let (width, height) = valid_area(self.size, coord);
        let last = (first + rows.len() / (t * bpp)).min(height);
        match step {
            Step::Paint { paint, math } => {
                let Some(tile) = paint.tiles.get(&coord) else {
                    return;
                };
                let mut below = [0u8; MAX_PIXEL_BYTES];
                for y in first..last {
                    for x in 0..width {
                        let px = &mut rows[((y - first) * t + x) * bpp..][..bpp];
                        below[..bpp].copy_from_slice(px);
                        self.paint_pixel(
                            math,
                            &tile.color,
                            &tile.keep,
                            y * t + x,
                            &below[..bpp],
                            px,
                        );
                    }
                }
            }
            Step::Effect {
                effect,
                prepared,
                blender,
                selection,
                ..
            } => {
                let (x0, y0) = (
                    f64::from(coord.col * TILE_SIZE),
                    f64::from(coord.row * TILE_SIZE),
                );
                // Adjustments in linear light decode the pixels whatever they are.
                let stored = blends_as_stored(self.format, effect.space)
                    && !prepared.adjustment().is_linear();
                for y in first..last {
                    for x in 0..width {
                        let coverage = match selection {
                            Some((image, codec)) => {
                                let (dx, dy) = effect
                                    .to_document
                                    .apply(x0 + x as f64 + 0.5, y0 + y as f64 + 0.5);
                                f64::from(MaskReader { image, codec }.at(dx.floor(), dy.floor()))
                            }
                            None => 1.0,
                        };
                        if coverage <= 0.0 {
                            continue;
                        }
                        let px = &mut rows[((y - first) * t + x) * bpp..][..bpp];
                        if stored {
                            // The straight color is the blend values: adjusted, then mixed by
                            // the coverage (the same alpha on both sides).
                            if px[3] == 0 {
                                continue;
                            }
                            let below = [px[0], px[1], px[2]].map(|v| f64::from(v) / 255.0);
                            let adjusted = prepared.apply(below);
                            let coverage = coverage.min(1.0);
                            for (n, sample) in px[..3].iter_mut().enumerate() {
                                let v = below[n] + (adjusted[n] - below[n]) * coverage;
                                *sample = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                            }
                            continue;
                        }
                        let b = self.pixels.read(&self.target, px);
                        let r = blender.adjust(prepared, &b, coverage);
                        self.pixels.write(&self.target, r, px);
                    }
                }
            }
        }
    }

    /// Pixel `i` of the paint (`color`, `keep`: `P` and `k` tiles of `math`'s format) over
    /// `below` (a pixel of the result's format), into `out`.
    fn paint_pixel(
        &self,
        math: &PaintPixels,
        color: &[u8],
        keep: &[u8],
        i: usize,
        below: &[u8],
        out: &mut [u8],
    ) {
        let k = read_keep(math.keep, keep, i);
        let p_bytes = &color[i * math.bpp..(i + 1) * math.bpp];
        if k >= 1.0 && math.codec.alpha(p_bytes) <= 0.0 {
            out.copy_from_slice(below);
            return;
        }
        // `P` copies as is where it replaces everything (`k = 0`), in the same format.
        if k <= 0.0 && stored_format(math.format) == self.format {
            out.copy_from_slice(p_bytes);
            return;
        }
        let (p, k) = math.read(color, keep, i);
        let stored = blends_as_stored(self.format, math.blender.space());
        let b = if stored {
            read_stored(below)
        } else {
            math.blender
                .encode_premultiplied(&self.pixels.read(&self.target, below))
        };
        let r = std::array::from_fn(|n| p[n] + k * b[n]);
        if stored {
            write_stored(r, out);
            return;
        }
        let r = math.blender.decode_premultiplied(&r);
        self.pixels.write(&self.target, r, out);
    }

    /// The tiles `coords` of the result, evaluated from scratch, on every core.
    fn tiles(&self, coords: Vec<TileCoord>) -> Result<Vec<PlacedTile>, StackError> {
        let mut work: Vec<(TileCoord, Option<Arc<[u8]>>)> =
            coords.into_iter().map(|c| (c, None)).collect();
        parallel_for_each(&mut work, |(coord, out)| {
            if crate::raster::wanted() {
                *out = Some(self.tile(*coord, 0, None));
            }
        });
        // Given up meanwhile: tiles are missing.
        if !crate::raster::wanted() {
            return Err(StackError::Cancelled);
        }
        Ok(work
            .into_iter()
            .filter_map(|(coord, tile)| tile.map(|t| (coord, t)))
            .collect())
    }
}

/// What a raster layer shows: its pixels, or the result of its stack, evaluated on the CPU
/// when first asked (ADR 0029, point 6) and kept, shared by every copy of the layer (history
/// snapshots, the views being rendered). An edit of the stack is then instant; what needs the
/// pixels (tools, thumbnails, export, the clipboard) evaluates them once, off the UI thread;
/// meanwhile the renderer evaluates the stack itself on the GPU ([`Self::ready`] is `None`).
#[derive(Debug, Clone)]
pub struct Pixels(Arc<LazyPixels>);

#[derive(Debug)]
struct LazyPixels {
    /// Changes with the pixels: what thumbnails and views know them by.
    key: crate::raster::ImageId,
    size: Size,
    format: PixelFormat,
    ready: OnceLock<Arc<RasterImage>>,
    /// The stack to evaluate, and the pixels of an earlier state of it to start from (only the
    /// tiles that differ are evaluated then); taken once evaluated.
    pending: Mutex<Option<Recipe>>,
    /// An evaluation was started on a thread of its own.
    started: std::sync::atomic::AtomicBool,
    /// What the layer showed before, shown meanwhile when the stack has a filter, which the
    /// display does not evaluate (ADR 0034).
    meanwhile: Option<Arc<RasterImage>>,
    /// A quick look at the result, computed first when the stack has a filter.
    preview: OnceLock<Preview>,
    /// The quick look of an earlier state, shown until this one's: a slider dragged over a
    /// filter shows the last one rather than the layer unfiltered in between.
    inherited: Option<Preview>,
    /// Pixels known at once going back below a filter, a state never shown: the next state
    /// stands in with what was shown before them (`meanwhile`), not with them.
    passes_on: bool,
    /// For those: the filter (`Filter::id`) what they pass on shows. A next state showing
    /// another filter stands in with these pixels instead (a filter cancelled, then another
    /// opened, must not flash the cancelled one).
    passed_filter: Option<&'static str>,
    /// The stack being evaluated, for its quick look: read without waiting for an evaluation
    /// in progress (which holds `pending`).
    stack: Option<LayerStack>,
    /// The look at the part of the layer the display shows (see [`Pixels::look_for`]).
    region: Mutex<Region>,
    /// The look an earlier state showed, shown until this one's is computed: a slider dragged
    /// over a filter, or its dialog's OK, keeps what was shown instead of the layer unfiltered.
    inherited_look: Option<Arc<Preview>>,
}

/// The look at part of a layer, the part asked for last, and whether one is being computed.
#[derive(Debug, Default)]
struct Region {
    look: Option<Arc<Preview>>,
    asked: Option<([f64; 4], usize)>,
    computing: bool,
}

/// Background evaluations, one at a time (each on every core): one superseded meanwhile (a
/// slider dragged over a filter) is skipped rather than computed for nothing.
static BACKGROUND: Mutex<()> = Mutex::new(());

/// Quick looks at filtered stacks being computed, likewise one at a time.
static PREVIEWS: Mutex<()> = Mutex::new(());

/// A tile of an image and where it goes.
type PlacedTile = (TileCoord, Arc<[u8]>);

/// A stack to evaluate, and where its evaluation starts from.
type Recipe = (LayerStack, Earlier);

/// What the evaluation of a state of a stack starts from (only the tiles that differ are
/// evaluated then): the state before it, held weakly so that a state replaced at once (a slider
/// dragged) is dropped rather than kept, and evaluated, for the next one; and the last state
/// evaluated before it, with its pixels, for when the state before was never evaluated.
#[derive(Debug, Default, Clone)]
struct Earlier {
    state: Option<(std::sync::Weak<LazyPixels>, LayerStack)>,
    evaluated: Option<(Arc<RasterImage>, LayerStack)>,
}

impl Earlier {
    fn of(earlier: Option<(Pixels, LayerStack)>) -> Self {
        let Some((pixels, before)) = earlier else {
            return Self::default();
        };
        if let Some(image) = pixels.ready_image() {
            return Self {
                state: None,
                evaluated: Some((Arc::clone(image), before)),
            };
        }
        // Being evaluated (its recipe taken): its pixels, once there, are found through `state`.
        let evaluated = match pixels.0.pending.try_lock() {
            Ok(recipe) => recipe.as_ref().and_then(|(_, e)| e.evaluated.clone()),
            Err(std::sync::TryLockError::Poisoned(recipe)) => recipe
                .into_inner()
                .as_ref()
                .and_then(|(_, e)| e.evaluated.clone()),
            Err(std::sync::TryLockError::WouldBlock) => None,
        };
        Self {
            state: Some((Arc::downgrade(&pixels.0), before)),
            evaluated,
        }
    }

    /// An earlier state and its pixels: the one before if it was evaluated meanwhile, else the
    /// last one evaluated before it.
    fn start(self) -> Option<(Arc<RasterImage>, LayerStack)> {
        self.state
            .and_then(|(state, before)| Some((Arc::clone(state.upgrade()?.ready.get()?), before)))
            .or(self.evaluated)
    }
}

/// A quick look at a stack's result while it is evaluated (see [`LayerStack::preview`] and
/// [`LayerStack::look_at`]): what its topmost filter gives at a pyramid level, `factor` layer
/// pixels a side per pixel, over the whole layer or a part of it starting at `origin` (pixels of
/// that level); the display evaluates the entries `above` the filter over it.
#[derive(Debug, Clone)]
pub struct Preview {
    pub image: Arc<RasterImage>,
    pub factor: u32,
    pub origin: [u32; 2],
    pub above: Vec<Entry>,
    /// The filter it shows (`Filter::id`): a look stands in for another state's only when that
    /// state shows the same filter (a setting changed), never another one.
    pub filter: &'static str,
}

impl Preview {
    /// Its pixels → the layer's.
    pub fn placement(&self) -> Affine {
        let factor = f64::from(self.factor);
        Affine::translation(f64::from(self.origin[0]), f64::from(self.origin[1]))
            .then(Affine::scale(factor, factor))
    }

    /// Whether it shows `rect` (`[x0, y0, x1, y1]`, the layer's pixels, within it) at `level`
    /// (a look has no coarser levels: another level is another look).
    fn covers(&self, rect: [f64; 4], level: usize) -> bool {
        let factor = f64::from(self.factor);
        let size = self.image.size();
        let [x0, y0] = self.origin.map(|v| f64::from(v) * factor);
        let (x1, y1) = (
            x0 + f64::from(size.width) * factor,
            y0 + f64::from(size.height) * factor,
        );
        self.factor == 1 << level
            && x0 <= rect[0]
            && y0 <= rect[1]
            && x1 >= rect[2]
            && y1 >= rect[3]
    }
}

/// The looks the display shows in place of layers whose stack has a filter, by layer.
pub type Looks = HashMap<crate::document::LayerId, Arc<Preview>>;

/// What a look at part of a layer takes to compute (see [`LayerStack::look_job`]): the tiles
/// of what is below the topmost filter there (row-major, `size` pixels of `format`, at the
/// look's level), and the filter's steps scaled to that level and placed from the crop's
/// pixels. Computed by the CPU ([`Self::run`]) or by the renderer on the GPU (ADR 0035).
#[derive(Debug, Clone)]
pub struct LookJob {
    pub tiles: Vec<Arc<[u8]>>,
    pub size: Size,
    pub format: PixelFormat,
    pub steps: Vec<FilterStep>,
    pub factor: u32,
    pub origin: [u32; 2],
    pub above: Vec<Entry>,
    /// A Liquify entry's look: `steps` is empty, the tiles are what the field reads from.
    pub warp: Option<WarpJob>,
}

/// What a Liquify look draws (see [`LayerStack::look_job`]): the field to read through and the
/// part of the level it draws (the job's tiles hold that part and the margin the field reads
/// from).
#[derive(Debug, Clone)]
pub struct WarpJob {
    pub field: Arc<Field>,
    pub space: BlendSpace,
    /// The drawn part's first pixel at the look's level, and its size.
    pub out_origin: [u32; 2],
    pub out_size: Size,
    /// With a pyramid of its own (a quick look of the whole layer).
    pub pyramid: bool,
}

impl LookJob {
    /// The look, computed on the CPU.
    pub fn run(&self) -> Result<Preview, StackError> {
        if let Some(warp) = &self.warp {
            return self.warped(warp);
        }
        // Read and made at level 0 only: a look is shown at the level it is made for.
        let mut image = Arc::new(RasterImage::from_level0_tiles_only(
            self.size,
            self.format,
            self.tiles.clone(),
        )?);
        for step in &self.steps {
            image = Arc::new(filtered(step, &image, false)?);
        }
        Ok(self.finished(image))
    }

    /// A Liquify look, computed on the CPU.
    fn warped(&self, warp: &WarpJob) -> Result<Preview, StackError> {
        let crop = RasterImage::from_level0_tiles_only(self.size, self.format, self.tiles.clone())?;
        let factor = f64::from(self.factor);
        let source = crate::liquify::Source::new(
            &crop,
            warp.space,
            0,
            [
                f64::from(self.origin[0]) * factor,
                f64::from(self.origin[1]) * factor,
            ],
            factor,
            warp.field.size(),
        );
        let placement = crate::liquify::Placement {
            origin: [
                f64::from(warp.out_origin[0]) * factor,
                f64::from(warp.out_origin[1]) * factor,
            ],
            step: factor,
        };
        let image = crate::liquify::warp_image(
            &source,
            &warp.field,
            placement,
            warp.out_size,
            self.format,
            warp.pyramid,
        )?;
        Ok(self.finished(Arc::new(image)))
    }

    /// The look showing `image` (the filtered crop, computed elsewhere).
    pub fn finished(&self, image: Arc<RasterImage>) -> Preview {
        Preview {
            filter: match &self.warp {
                Some(_) => "liquify",
                None => self.steps.last().map_or("", |step| step.filter.id()),
            },
            image,
            factor: self.factor,
            origin: self.warp.as_ref().map_or(self.origin, |w| w.out_origin),
            above: self.above.clone(),
        }
    }
}

/// A look computed elsewhere than on the CPU (the renderer's GPU, ADR 0035): the filtered
/// crop, or `None` when it does not take that job (the CPU computes it then).
pub type LookFilter = Arc<dyn Fn(&LookJob) -> Option<RasterImage> + Send + Sync>;

/// How long a state with a filter shows before its whole layer is evaluated in the background
/// (see `Pixels::evaluate_in_background`).
const SETTLE: std::time::Duration = std::time::Duration::from_millis(500);

/// The same for a stack without a filter, which the display evaluates itself meanwhile: long
/// enough to skip the states of a slider being dragged, short enough that the pixels are there
/// soon after it stops.
const SETTLE_WITHOUT_FILTER: std::time::Duration = std::time::Duration::from_millis(150);

/// The largest preview, in pixels: a few tens of milliseconds to compute.
const PREVIEW_PIXELS: u64 = 4_000_000;

impl Pixels {
    /// Pixels already there.
    pub fn ready(image: Arc<RasterImage>) -> Self {
        let lazy = LazyPixels {
            key: image.id(),
            size: image.size(),
            format: image.format(),
            ready: OnceLock::new(),
            pending: Mutex::new(None),
            started: std::sync::atomic::AtomicBool::new(true),
            meanwhile: None,
            preview: OnceLock::new(),
            inherited: None,
            passes_on: false,
            passed_filter: None,
            stack: None,
            region: Mutex::default(),
            inherited_look: None,
        };
        let _ = lazy.ready.set(image);
        Self(Arc::new(lazy))
    }

    /// The result of `stack`, evaluated when first asked; from `earlier` (pixels of another
    /// state of the stack, and that state) when they are evaluated by then.
    pub fn pending(stack: LayerStack, earlier: Option<(Pixels, LayerStack)>) -> Self {
        // Kept where a filter comes or goes: a preview replaced at each setting goes back to the
        // stack without its filter, then on, and must not flash the layer unfiltered meanwhile.
        let meanwhile = earlier
            .as_ref()
            .filter(|(_, before)| stack.has_shown_filter() || before.has_shown_filter())
            .and_then(|(pixels, _)| pixels.stand_in_for(&stack).cloned());
        // An earlier quick look stands while the filter and the entries above it are the same
        // (a setting changed); a state without a filter (a preview replaced, going back before
        // applying again) passes it on. Another filter (one cancelled, then another opened) starts
        // from what the layer shows.
        let stands = |preview: &Preview| match stack.last_filter() {
            None => true,
            Some(index) => {
                stack.entries[index + 1..] == preview.above[..]
                    && stack.filter_shown(index) == Some(preview.filter)
            }
        };
        let inherited = earlier
            .as_ref()
            .and_then(|(pixels, _)| pixels.preview().filter(|p| stands(p)).cloned());
        // The look shown before stands likewise.
        let inherited_look = earlier
            .as_ref()
            .and_then(|(pixels, _)| pixels.shown_look().filter(|l| stands(l)));
        // Back to what a filter of the earlier state applied to: known already. What was shown
        // is passed on to the next state (a preview replaced goes back, then on again).
        if let Some(image) = earlier
            .as_ref()
            .and_then(|(_, before)| before.known_result(&stack))
        {
            let lazy = LazyPixels {
                key: image.id(),
                size: image.size(),
                format: image.format(),
                ready: OnceLock::new(),
                pending: Mutex::new(None),
                started: std::sync::atomic::AtomicBool::new(true),
                meanwhile: earlier
                    .as_ref()
                    .and_then(|(pixels, _)| pixels.stand_in().cloned()),
                preview: OnceLock::new(),
                inherited,
                passes_on: true,
                passed_filter: earlier
                    .as_ref()
                    .and_then(|(_, before)| before.filter_shown(before.last_filter()?)),
                stack: None,
                region: Mutex::default(),
                inherited_look: inherited_look.clone(),
            };
            let _ = lazy.ready.set(image);
            return Self(Arc::new(lazy));
        }
        Self(Arc::new(LazyPixels {
            key: crate::raster::ImageId::next(),
            size: stack.original().size(),
            format: stack.format(),
            ready: OnceLock::new(),
            stack: Some(stack.clone()),
            pending: Mutex::new(Some((stack, Earlier::of(earlier)))),
            started: std::sync::atomic::AtomicBool::new(false),
            meanwhile,
            preview: OnceLock::new(),
            inherited,
            passes_on: false,
            passed_filter: None,
            region: Mutex::default(),
            inherited_look,
        }))
    }

    /// The pixels, evaluated now if they are not yet (on the calling thread: not the UI's).
    pub fn get(&self) -> Arc<RasterImage> {
        if let Some(image) = self.0.ready.get() {
            return Arc::clone(image);
        }
        // A poisoned lock only means another evaluation panicked: evaluate again.
        let mut pending = self
            .0
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(image) = self.0.ready.get() {
            return Arc::clone(image);
        }
        let image = match pending.take() {
            Some((stack, earlier)) => evaluated(&stack, earlier),
            // Invariant: pending until ready, and ready is checked under the lock.
            None => unreachable!("pixels are pending until they are ready"),
        };
        Arc::clone(self.0.ready.get_or_init(|| image))
    }

    /// [`Self::get`], given up as soon as `wanted` says they are not wanted any more (checked
    /// tile by tile): `None` then, the pixels still pending, for whoever needs them later.
    fn get_while(&self, wanted: crate::raster::Wanted) -> Option<Arc<RasterImage>> {
        if let Some(image) = self.0.ready.get() {
            return Some(Arc::clone(image));
        }
        let mut pending = self
            .0
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(image) = self.0.ready.get() {
            return Some(Arc::clone(image));
        }
        // Invariant: pending until ready, and ready is checked under the lock.
        let (stack, earlier) = pending.take()?;
        let result = crate::raster::while_wanted(wanted, || try_evaluated(&stack, earlier.clone()));
        let image = match result {
            Err(StackError::Cancelled) => {
                *pending = Some((stack, earlier));
                return None;
            }
            // As `evaluated`: the original rather than nothing.
            other => other.unwrap_or_else(|_| Arc::clone(stack.original())),
        };
        Some(Arc::clone(self.0.ready.get_or_init(|| image)))
    }

    /// Start evaluating the pixels on a thread of their own, once, unless they are evaluated:
    /// what shows the stack meanwhile (the renderer) gets the exact pixels soon, and what
    /// needs them later does not wait.
    pub fn evaluate_in_background(&self) {
        use std::sync::atomic::Ordering;
        if self.0.ready.get().is_some() || self.0.started.swap(true, Ordering::AcqRel) {
            return;
        }
        let pixels = self.clone();
        let spawned = std::thread::Builder::new()
            .name("stack pixels".to_owned())
            .spawn(move || {
                // A filter's quick look first, not waiting for the other evaluations: what a
                // slider dragged over a filter shows at once.
                let settle = match pixels.0.stack.as_ref().filter(|s| s.has_shown_filter()) {
                    Some(stack) => {
                        // One at a time, on every core: one superseded meanwhile (a slider
                        // dragged) is skipped, nobody showing it.
                        let _one_at_a_time =
                            PREVIEWS.lock().unwrap_or_else(PoisonError::into_inner);
                        if Arc::strong_count(&pixels.0) > 1
                            && let Ok(Some(preview)) = stack.preview(PREVIEW_PIXELS)
                        {
                            let _ = pixels.0.preview.set(preview);
                        }
                        SETTLE
                    }
                    None => SETTLE_WITHOUT_FILTER,
                };
                // The display looks at what it shows (`look_for`) or evaluates the stack itself.
                // The whole layer, which painting it or moving its pixels needs, once this
                // state has lasted: a slider dragged replaces it at each setting, and nobody
                // holds those any more by then.
                std::thread::sleep(settle);
                if Arc::strong_count(&pixels.0) <= 1 {
                    return;
                }
                let _one_at_a_time = BACKGROUND.lock().unwrap_or_else(PoisonError::into_inner);
                // Only this thread holds them any more: nobody needs them; and given up as
                // soon as that is so (the next state replaced this one meanwhile).
                let held = Arc::downgrade(&pixels.0);
                let wanted: crate::raster::Wanted = Arc::new(move || held.strong_count() > 1);
                if Arc::strong_count(&pixels.0) > 1 {
                    pixels.get_while(wanted);
                }
            });
        // Without a thread, whoever needs the pixels evaluates them.
        if spawned.is_err() {
            self.0.started.store(false, Ordering::Release);
        }
    }

    /// The pixels if they are evaluated, without waiting.
    pub fn ready_image(&self) -> Option<&Arc<RasterImage>> {
        self.0.ready.get()
    }

    /// A quick look at the result of a stack with a filter, while its pixels are evaluated:
    /// its own, or an earlier state's until its own is computed.
    pub fn preview(&self) -> Option<&Preview> {
        self.0.preview.get().or(self.0.inherited.as_ref())
    }

    /// What the layer showed before, while pixels of a stack with a filter are evaluated.
    pub fn meanwhile(&self) -> Option<&Arc<RasterImage>> {
        self.0.meanwhile.as_ref()
    }

    /// The pixels if they are evaluated, else what stands in for them meanwhile.
    pub fn shown(&self) -> Option<&Arc<RasterImage>> {
        self.ready_image().or(self.meanwhile())
    }

    /// What the display shows of a stack with a filter while its pixels are not evaluated, for
    /// `rect` (`[x0, y0, x1, y1]`, the layer's pixels) seen at pyramid `level`: the look at
    /// that part once computed, else the quick look while it is computed on a thread of its own
    /// (only the part last asked for), and whether the display must ask again. `None` when the
    /// pixels are there or the stack has no filter.
    pub fn look_for(
        &self,
        rect: [f64; 4],
        level: usize,
        gpu: Option<&LookFilter>,
    ) -> (Option<Arc<Preview>>, bool) {
        if self.ready_image().is_some() {
            return (None, false);
        }
        let Some(stack) = self.0.stack.as_ref().filter(|s| s.has_shown_filter()) else {
            return (None, false);
        };
        let size = stack.original().size();
        let rect = [
            rect[0].clamp(0.0, f64::from(size.width)),
            rect[1].clamp(0.0, f64::from(size.height)),
            rect[2].clamp(0.0, f64::from(size.width)),
            rect[3].clamp(0.0, f64::from(size.height)),
        ];
        let mut region = self.0.region.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(look) = region.look.as_ref().filter(|l| l.covers(rect, level)) {
            return (Some(Arc::clone(look)), false);
        }
        region.asked = Some((rect, level));
        if !region.computing {
            region.computing = true;
            let pixels = self.clone();
            let stack = stack.clone();
            let gpu = gpu.cloned();
            let spawned = std::thread::Builder::new()
                .name("stack look".to_owned())
                .spawn(move || {
                    loop {
                        let asked = {
                            let mut region = pixels
                                .0
                                .region
                                .lock()
                                .unwrap_or_else(PoisonError::into_inner);
                            match region.asked.take() {
                                // Asked again while it was computed: shown already.
                                Some((rect, level))
                                    if region
                                        .look
                                        .as_ref()
                                        .is_some_and(|l| l.covers(rect, level)) =>
                                {
                                    continue;
                                }
                                Some(asked) => asked,
                                None => {
                                    region.computing = false;
                                    return;
                                }
                            }
                        };
                        // Nobody shows these pixels any more: nothing to look at.
                        if Arc::strong_count(&pixels.0) <= 1 {
                            continue;
                        }
                        let Some(job) = stack.look_job(asked.0, asked.1) else {
                            continue;
                        };
                        let held = Arc::downgrade(&pixels.0);
                        let wanted: crate::raster::Wanted =
                            Arc::new(move || held.strong_count() > 1);
                        let look = match gpu.as_ref().and_then(|gpu| gpu(&job)) {
                            Some(image) => Ok(job.finished(Arc::new(image))),
                            // Given up when nobody shows these pixels any more.
                            None => crate::raster::while_wanted(wanted, || job.run()),
                        };
                        if let Ok(look) = look {
                            pixels
                                .0
                                .region
                                .lock()
                                .unwrap_or_else(PoisonError::into_inner)
                                .look = Some(Arc::new(look));
                        }
                    }
                });
            if spawned.is_err() {
                region.computing = false;
            }
        }
        // Meanwhile: the look shown before when it shows this part at this level (the setting
        // being changed shows the last one, never the layer unfiltered), else the quick look.
        let fallback = self
            .0
            .inherited_look
            .as_ref()
            .filter(|l| l.covers(rect, level))
            .cloned()
            .or_else(|| self.preview().cloned().map(Arc::new));
        (fallback, true)
    }

    /// The look these pixels show of their layer: the one computed for them, else the one they
    /// inherited, passed on to the next state.
    fn shown_look(&self) -> Option<Arc<Preview>> {
        let region = self.0.region.lock().unwrap_or_else(PoisonError::into_inner);
        region
            .look
            .clone()
            .or_else(|| self.0.inherited_look.clone())
    }

    /// The pixels if they are evaluated, else a quick look at them of at most `max_pixels`
    /// when the stack's topmost filter is its last entry (a filter being applied): what a
    /// thumbnail needs, without waiting for the whole layer. `None`: only [`Self::get`] tells.
    pub fn quick_look(&self, max_pixels: u64) -> Option<Arc<RasterImage>> {
        if let Some(image) = self.ready_image() {
            return Some(Arc::clone(image));
        }
        if let Some(preview) = self.0.preview.get()
            && preview.above.is_empty()
        {
            return Some(Arc::clone(&preview.image));
        }
        let stack = self.0.stack.as_ref()?;
        let preview = stack.preview(max_pixels).ok()??;
        preview.above.is_empty().then_some(preview.image)
    }

    /// What the next state shows while its own pixels are evaluated (see `passes_on`).
    /// What stands in for these pixels in the next state, `next`: what they pass on, unless it
    /// shows another filter than `next` does (then these pixels themselves).
    fn stand_in_for(&self, next: &LayerStack) -> Option<&Arc<RasterImage>> {
        let shown = next.last_filter().and_then(|i| next.filter_shown(i));
        match (self.0.passed_filter, shown) {
            (Some(passed), Some(shown)) if passed != shown => {
                self.ready_image().or(self.meanwhile())
            }
            _ => self.stand_in(),
        }
    }

    fn stand_in(&self) -> Option<&Arc<RasterImage>> {
        if self.0.passes_on {
            self.meanwhile().or(self.ready_image())
        } else {
            self.shown()
        }
    }

    /// Changes whenever the pixels do (a new `Pixels`): what thumbnails are known by.
    pub fn key(&self) -> u64 {
        self.0.key.get()
    }

    pub fn size(&self) -> Size {
        self.0.size
    }

    pub fn format(&self) -> PixelFormat {
        self.0.format
    }

    /// The same pixels (the same allocation).
    pub fn ptr_eq(&self, other: &Pixels) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// `stack`'s result, from an earlier state when one is evaluated (else from the original).
fn evaluated(stack: &LayerStack, earlier: Earlier) -> Arc<RasterImage> {
    let result = try_evaluated(stack, earlier);
    // Invariant: a stack's tiles are of its own making (lengths and places checked when made),
    // so evaluating it cannot fail; were it to, the original is shown rather than nothing.
    result.unwrap_or_else(|_| Arc::clone(stack.original()))
}

/// [`evaluated`], or why not (given up: see `raster::while_wanted`).
fn try_evaluated(stack: &LayerStack, earlier: Earlier) -> Result<Arc<RasterImage>, StackError> {
    match earlier.start() {
        Some((shown, before)) => stack.reevaluate(&before, &shown),
        None => stack.evaluate(),
    }
}

impl PartialEq for LayerStack {
    fn eq(&self, other: &Self) -> bool {
        // Immutable and shared: the same allocations, the same stack.
        Arc::ptr_eq(&self.original, &other.original) && self.entries == other.entries
    }
}

impl LayerStack {
    /// A layer's original pixels, nothing applied yet.
    pub fn new(original: Arc<RasterImage>) -> Self {
        Self {
            original,
            entries: Vec::new(),
        }
    }

    /// A stack read back (e.g. from a file): every paint fits the layer, every effect is valid.
    pub fn with_entries(
        original: Arc<RasterImage>,
        entries: Vec<Entry>,
    ) -> Result<Self, StackError> {
        let stack = Self {
            original,
            entries: Vec::new(),
        };
        for entry in &entries {
            stack.check(entry)?;
        }
        Ok(Self { entries, ..stack })
    }

    fn check(&self, entry: &Entry) -> Result<(), StackError> {
        match entry {
            Entry::Paint(paint) => {
                if paint.size != self.original.size() {
                    return Err(StackError::SizeMismatch);
                }
                if paint.format != paint_format(self.original.format()) {
                    return Err(StackError::FormatMismatch);
                }
            }
            Entry::Effect(effect) => {
                if effect.steps.is_empty() || effect.steps.iter().any(|s| !s.is_valid()) {
                    return Err(StackError::InvalidEffect);
                }
            }
            Entry::Filter(filter) => {
                if filter.steps.is_empty() || filter.steps.iter().any(|s| !s.is_valid()) {
                    return Err(StackError::InvalidEffect);
                }
            }
            Entry::Liquify(liquify) => {
                if liquify.field.size() != self.original.size() {
                    return Err(StackError::SizeMismatch);
                }
            }
        }
        Ok(())
    }

    pub fn original(&self) -> &Arc<RasterImage> {
        &self.original
    }

    /// Bottom to top.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The top entry when it is paint, shown: what a new stroke continues.
    pub fn top_paint(&self) -> Option<&Arc<PaintEntry>> {
        match self.entries.last() {
            Some(Entry::Paint(paint)) if !paint.hidden => Some(paint),
            _ => None,
        }
    }

    /// The stack with `paint` as its top paint: in place of the top entry when it is paint (a
    /// stroke continues it), on top otherwise. Paint that changes nothing is dropped.
    pub fn with_top_paint(&self, paint: Arc<PaintEntry>) -> Result<Self, StackError> {
        let entry = Entry::Paint(paint);
        self.check(&entry)?;
        let mut entries = self.entries.clone();
        if self.top_paint().is_some() {
            entries.pop();
        }
        if matches!(&entry, Entry::Paint(p) if !p.is_identity()) {
            entries.push(entry);
        }
        Ok(Self {
            original: Arc::clone(&self.original),
            entries,
        })
    }

    /// The stack with the paint that turns `before` into `after` on top (both of this layer's
    /// size; see [`PaintEntry::between`]), continuing the top paint when it can: what moving
    /// selected pixels leaves.
    pub fn with_painted(
        &self,
        before: &RasterImage,
        after: &RasterImage,
        space: BlendSpace,
    ) -> Result<Self, StackError> {
        let delta = PaintEntry::between(self.original.format(), before, after, space)?;
        let paint = match self.top_paint() {
            Some(top) if top.merges_with(&delta) => top.merged(&delta),
            _ => {
                let mut entries = self.entries.clone();
                let entry = Entry::Paint(Arc::new(delta));
                self.check(&entry)?;
                entries.push(entry);
                return Ok(Self {
                    original: Arc::clone(&self.original),
                    entries,
                });
            }
        };
        self.with_top_paint(Arc::new(paint))
    }

    /// The stack of the layer grown by `offset` whole tiles (columns, rows) before its pixels
    /// to `size` (painting beyond its bounds, ADR 0027): the original given an alpha channel
    /// and grown, transparent around; the paint moved with it; the effects' selections still
    /// read where they were applied.
    pub fn grown(&self, offset: (u32, u32), size: Size) -> Result<Self, StackError> {
        let with_alpha = match self.original.with_alpha() {
            Some(converted) => Arc::new(converted?),
            None => Arc::clone(&self.original),
        };
        let original = Arc::new(
            with_alpha
                .grown(offset, size)
                .ok_or(StackError::SizeMismatch)??,
        );
        let (left, top) = offset;
        let t = f64::from(TILE_SIZE);
        let shift = Affine::translation(-f64::from(left) * t, -f64::from(top) * t);
        let format = paint_format(original.format());
        let old_size = self.original.size();
        let entries = self
            .entries
            .iter()
            .map(|entry| match entry {
                Entry::Paint(paint) => Entry::Paint(Arc::new(PaintEntry {
                    size,
                    format,
                    space: paint.space,
                    images: OnceLock::new(),
                    below: Mutex::new(None),
                    hidden: paint.hidden,
                    tiles: paint
                        .tiles
                        .iter()
                        .map(|(coord, tile)| {
                            let moved = TileCoord {
                                col: coord.col + left,
                                row: coord.row + top,
                            };
                            let math = paint.math();
                            (
                                moved,
                                math.reframed(
                                    tile,
                                    valid_area(old_size, *coord),
                                    valid_area(size, moved),
                                ),
                            )
                        })
                        .collect(),
                })),
                Entry::Filter(filter) => Entry::Filter(Arc::new(FilterEntry {
                    hidden: filter.hidden,
                    computing: Mutex::new(()),
                    cache: Mutex::new(None),
                    steps: filter
                        .steps
                        .iter()
                        .map(|step| {
                            Arc::new(FilterStep {
                                to_document: Projective::from(shift).then(step.to_document),
                                ..(**step).clone()
                            })
                        })
                        .collect(),
                })),
                Entry::Liquify(liquify) => Entry::Liquify(Arc::new(LiquifyEntry {
                    field: Arc::new(liquify.field.grown(offset, size)),
                    space: liquify.space,
                    reach: liquify.reach,
                    hidden: liquify.hidden,
                    computing: Mutex::new(()),
                    cache: Mutex::new(None),
                    images: OnceLock::new(),
                })),
                Entry::Effect(effect) => Entry::Effect(Arc::new(EffectEntry {
                    hidden: effect.hidden,
                    steps: effect
                        .steps
                        .iter()
                        .map(|step| {
                            Arc::new(Effect {
                                to_document: Projective::from(shift).then(step.to_document),
                                ..(**step).clone()
                            })
                        })
                        .collect(),
                })),
            })
            .collect();
        Self::with_entries(original, entries)
    }

    /// The stack with `effect` applied on top: an entry of its own, or combined with the top
    /// entry when their settings combine exactly (ADR 0034: Exposure's stops, a hue shift); an
    /// Invert on an Invert cancels it.
    pub fn with_effect(&self, effect: Effect) -> Result<Self, StackError> {
        let added = EffectEntry::new(vec![Arc::new(effect)])?;
        let mut entries = self.entries.clone();
        let combined = match entries.last() {
            Some(Entry::Effect(top)) => top.combined(&added),
            _ => None,
        };
        match combined {
            Some(one) => {
                entries.pop();
                entries.extend(one.map(|one| Entry::Effect(Arc::new(one))));
            }
            None => entries.push(Entry::Effect(Arc::new(added))),
        }
        Ok(Self {
            original: Arc::clone(&self.original),
            entries,
        })
    }

    /// The stack with `step` applied on top: an entry of its own, or combined with the top
    /// entry when they combine exactly (two Gaussian Blurs, ADR 0034). `shown`: what the layer
    /// shows (this stack's result, when it is evaluated), what the filter is applied to.
    pub fn with_filter(
        &self,
        step: FilterStep,
        shown: Option<Arc<RasterImage>>,
    ) -> Result<Self, StackError> {
        let added = FilterEntry::new(vec![Arc::new(step)])?;
        let mut entries = self.entries.clone();
        let combined = match entries.last() {
            Some(Entry::Filter(top)) => top.combined(&added),
            _ => None,
        };
        let entry = match combined {
            Some(one) => {
                entries.pop();
                one
            }
            None => match shown {
                Some(input) if input.size() == self.original.size() => {
                    added.knowing(self.clone(), input)
                }
                _ => added,
            },
        };
        entries.push(Entry::Filter(Arc::new(entry)));
        Ok(Self {
            original: Arc::clone(&self.original),
            entries,
        })
    }

    /// The stack with a Liquify entry of `field` on top (ADR 0037), warping what is below it in
    /// `space`. `shown`: what the layer shows (this stack's result, when it is evaluated), what
    /// the warp is applied to.
    pub fn with_liquify(
        &self,
        field: Arc<Field>,
        space: BlendSpace,
        shown: Option<Arc<RasterImage>>,
    ) -> Result<Self, StackError> {
        let mut entry = LiquifyEntry::new(field, space);
        if let Some(input) = shown.filter(|input| input.size() == self.original.size()) {
            entry = entry.knowing(self.clone(), input);
        }
        let entry = Entry::Liquify(Arc::new(entry));
        self.check(&entry)?;
        let mut entries = self.entries.clone();
        entries.push(entry);
        Ok(Self {
            original: Arc::clone(&self.original),
            entries,
        })
    }

    /// What entry `index` is applied to: the result of the entries below it (its own cache when
    /// it is a Liquify entry that knows it).
    pub fn evaluate_below(&self, index: usize) -> Result<Arc<RasterImage>, StackError> {
        if index > self.entries.len() {
            return Err(StackError::IndexOutOfRange(index));
        }
        let below = self.below(index);
        if let Some(Entry::Liquify(liquify)) = self.entries.get(index)
            && let Some((known, input)) = liquify.known_input()
            && known == below
        {
            return Ok(input);
        }
        below.evaluate()
    }

    /// The stack with the field of Liquify entry `index` replaced by `field`: an entry edited
    /// again in its workspace. It keeps its eye and what it is applied to; `input` (what the
    /// entry is applied to, as [`Self::evaluate_below`] gives it) when it did not know.
    pub fn with_field(
        &self,
        index: usize,
        field: Arc<Field>,
        input: Option<Arc<RasterImage>>,
    ) -> Result<Self, StackError> {
        let Entry::Liquify(old) = self
            .entries
            .get(index)
            .ok_or(StackError::IndexOutOfRange(index))?
        else {
            return Err(StackError::InvalidEffect);
        };
        let mut edited = LiquifyEntry::new(field, old.space).with_hidden(old.hidden);
        match (old.known_input(), input) {
            (Some((below, input)), _) => edited = edited.knowing(below, input),
            (None, Some(input)) if input.size() == self.original.size() => {
                edited = edited.knowing(self.below(index), input);
            }
            _ => {}
        }
        let edited = Entry::Liquify(Arc::new(edited));
        self.check(&edited)?;
        let mut entries = self.entries.clone();
        entries[index] = edited;
        Ok(Self {
            original: Arc::clone(&self.original),
            entries,
        })
    }

    /// The stack without entry `index`; the neighbours that meet merge: two paints exactly, two
    /// effects or filters whose settings combine exactly as one entry, two Inverts cancel.
    pub fn without(&self, index: usize) -> Result<Self, StackError> {
        if index >= self.entries.len() {
            return Err(StackError::IndexOutOfRange(index));
        }
        let mut entries = self.entries.clone();
        entries.remove(index);
        let mut seam = index;
        while seam > 0 && seam < entries.len() {
            match (&entries[seam - 1], &entries[seam]) {
                (Entry::Paint(below), Entry::Paint(above)) if below.merges_with(above) => {
                    let merged = Entry::Paint(Arc::new(below.merged(above)));
                    entries.splice(seam - 1..=seam, [merged]);
                    break;
                }
                (Entry::Filter(below), Entry::Filter(above)) => {
                    let Some(merged) = below.combined(above) else {
                        break;
                    };
                    entries.splice(seam - 1..=seam, [Entry::Filter(Arc::new(merged))]);
                    break;
                }
                (Entry::Effect(below), Entry::Effect(above)) => match below.combined(above) {
                    None => break,
                    Some(None) => {
                        entries.drain(seam - 1..=seam);
                        seam -= 1;
                    }
                    Some(Some(merged)) => {
                        entries.splice(seam - 1..=seam, [Entry::Effect(Arc::new(merged))]);
                        break;
                    }
                },
                _ => break,
            }
        }
        Ok(Self {
            original: Arc::clone(&self.original),
            entries,
        })
    }

    /// The stack with entry `index` hidden or shown by its eye (ADR 0034). Nothing merges: an
    /// entry shown again next to one alike stays an entry of its own.
    pub fn with_hidden(&self, index: usize, hidden: bool) -> Result<Self, StackError> {
        let entry = self
            .entries
            .get(index)
            .ok_or(StackError::IndexOutOfRange(index))?;
        if entry.hidden() == hidden {
            return Ok(self.clone());
        }
        let changed = match entry {
            Entry::Paint(paint) => Entry::Paint(Arc::new(paint.hidden_copy(hidden))),
            Entry::Effect(effect) => Entry::Effect(Arc::new(EffectEntry {
                steps: effect.steps.clone(),
                hidden,
            })),
            // What it computed stays good: hiding a filter changes what is above it only.
            Entry::Liquify(liquify) => Entry::Liquify(Arc::new(
                LiquifyEntry {
                    field: Arc::clone(&liquify.field),
                    space: liquify.space,
                    reach: liquify.reach,
                    hidden,
                    computing: Mutex::new(()),
                    cache: Mutex::new(
                        liquify
                            .cache
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .clone(),
                    ),
                    images: OnceLock::new(),
                }
                .sharing_images(liquify),
            )),
            Entry::Filter(filter) => Entry::Filter(Arc::new(FilterEntry {
                steps: filter.steps.clone(),
                hidden,
                computing: Mutex::new(()),
                cache: Mutex::new(
                    filter
                        .cache
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .clone(),
                ),
            })),
        };
        let mut entries = self.entries.clone();
        entries[index] = changed;
        Ok(Self {
            original: Arc::clone(&self.original),
            entries,
        })
    }

    /// The stack with the steps of entry `index` set to `operations` (one per step: adjustments
    /// of an effect entry's kind, filters of a filter entry's), each keeping its selection,
    /// placement and blend space: an entry edited again (ADR 0034). Nothing merges nor splits;
    /// the entry keeps its eye.
    pub fn with_steps(&self, index: usize, operations: &[Operation]) -> Result<Self, StackError> {
        let entry = self
            .entries
            .get(index)
            .ok_or(StackError::IndexOutOfRange(index))?;
        let edited = match entry {
            Entry::Effect(effect) if operations.len() == effect.steps.len() => {
                let mut steps = Vec::with_capacity(operations.len());
                for (step, operation) in effect.steps.iter().zip(operations) {
                    let Operation::Adjustment(adjustment) = *operation else {
                        return Err(StackError::InvalidEffect);
                    };
                    steps.push(if step.adjustment == adjustment {
                        Arc::clone(step)
                    } else {
                        Arc::new(Effect {
                            adjustment,
                            ..(**step).clone()
                        })
                    });
                }
                // Valid steps of the entry's kind only.
                let edited = EffectEntry::new(steps)?;
                if edited.kind() != effect.kind() {
                    return Err(StackError::InvalidEffect);
                }
                Entry::Effect(Arc::new(edited.with_hidden(effect.hidden)))
            }
            Entry::Filter(filter) if operations.len() == filter.steps.len() => {
                let mut steps = Vec::with_capacity(operations.len());
                for (step, operation) in filter.steps.iter().zip(operations) {
                    let Operation::Filter(changed) = *operation else {
                        return Err(StackError::InvalidEffect);
                    };
                    steps.push(if step.filter == changed {
                        Arc::clone(step)
                    } else {
                        Arc::new(FilterStep {
                            filter: changed,
                            ..(**step).clone()
                        })
                    });
                }
                let edited = FilterEntry::new(steps)?;
                if edited.kind() != filter.kind() {
                    return Err(StackError::InvalidEffect);
                }
                // What it is applied to has not changed: only the filter is computed again.
                let edited = edited.with_hidden(filter.hidden);
                let edited = match filter.known_input() {
                    Some((below, input)) => edited.knowing(below, input),
                    None => edited,
                };
                Entry::Filter(Arc::new(edited))
            }
            _ => return Err(StackError::InvalidEffect),
        };
        let mut entries = self.entries.clone();
        entries[index] = edited;
        Ok(Self {
            original: Arc::clone(&self.original),
            entries,
        })
    }

    /// The format of the result: the original's, with an alpha channel when paint can lower
    /// alpha (shared tiles for RGB, which is stored with one).
    pub fn format(&self) -> PixelFormat {
        let format = self.original.format();
        let lowers_alpha = self.entries.iter().any(|e| match e {
            Entry::Paint(p) => p.lowers_alpha(),
            Entry::Effect(_) | Entry::Filter(_) | Entry::Liquify(_) => false,
        });
        if format.layout.has_alpha() || !lowers_alpha {
            return format;
        }
        paint_format(format)
    }

    /// Whether a filter is shown in the stack: its result cannot be evaluated tile by tile.
    /// The filter entry `index` shows (its last step's `Filter::id`), if it is one.
    fn filter_shown(&self, index: usize) -> Option<&'static str> {
        match self.entries.get(index)? {
            Entry::Filter(filter) => filter.steps.last().map(|step| step.filter.id()),
            Entry::Liquify(_) => Some("liquify"),
            _ => None,
        }
    }

    pub fn has_shown_filter(&self) -> bool {
        self.last_filter().is_some()
    }

    /// The result of `stack`, when a filter entry of this stack knows it as what it applies to:
    /// going back from a filter to what was below it (a preview replaced, undo) needs no
    /// evaluation.
    fn known_result(&self, stack: &LayerStack) -> Option<Arc<RasterImage>> {
        self.entries.iter().find_map(|entry| match entry {
            Entry::Filter(filter) => filter
                .known_input()
                .filter(|(below, input)| {
                    below == stack
                        && input.format() == stack.format()
                        && input.size() == stack.original.size()
                })
                .map(|(_, input)| input),
            Entry::Liquify(liquify) => liquify
                .known_input()
                .filter(|(below, input)| {
                    below == stack
                        && input.format() == stack.format()
                        && input.size() == stack.original.size()
                })
                .map(|(_, input)| input),
            _ => None,
        })
    }

    /// The index of the topmost filter entry shown.
    fn last_filter(&self) -> Option<usize> {
        self.entries
            .iter()
            .rposition(|e| matches!(e, Entry::Filter(_) | Entry::Liquify(_)) && !e.hidden())
    }

    /// The stack of the entries below `index`.
    fn below(&self, index: usize) -> LayerStack {
        LayerStack {
            original: Arc::clone(&self.original),
            entries: self.entries[..index].to_vec(),
        }
    }

    /// A quick look at the result while it is evaluated (ADR 0034): the topmost shown filter
    /// applied to what is below it at the coarsest pyramid level of at most `max_pixels`
    /// (its radius scaled alike), for the display to evaluate the entries above it from. `None`
    /// without a filter, when what is below it is not known yet, or when the layer is small
    /// enough to evaluate whole at once.
    pub fn preview(&self, max_pixels: u64) -> Result<Option<Preview>, StackError> {
        let Some(index) = self.last_filter() else {
            return Ok(None);
        };
        let below = self.below(index);
        let filter = match &self.entries[index] {
            Entry::Filter(filter) => filter,
            Entry::Liquify(liquify) => {
                return self.liquify_preview(index, liquify, &below, max_pixels);
            }
            _ => return Ok(None),
        };
        let Some((input, run)) = self.known_run(index) else {
            return Ok(None);
        };
        let Some(level) = input
            .levels()
            .iter()
            .position(|l| u64::from(l.size().width) * u64::from(l.size().height) <= max_pixels)
            .filter(|&level| level > 0)
        else {
            return Ok(None);
        };
        let factor = (1u32 << level) as f32;
        let coarse = &input.levels()[level];
        // Filters read level 0 only: a pyramid for the result alone, which the display samples.
        let mut image = Arc::new(RasterImage::from_level0_tiles_only(
            coarse.size(),
            input.format(),
            coarse.tiles().to_vec(),
        )?);
        // A coarse pixel is `factor` pixels of the layer: where the selection is read from, and
        // how far the filter reaches.
        let scale = Affine::scale(f64::from(factor), f64::from(factor));
        for (n, step) in run.iter().enumerate() {
            let coarse_step = FilterStep {
                filter: step.filter.scaled(factor),
                to_document: Projective::from(scale).then(step.to_document),
                ..(*step).clone()
            };
            let last = n + 1 == run.len();
            image = Arc::new(filtered(&coarse_step, &image, last)?);
        }
        Ok(Some(Preview {
            filter: filter.steps.last().map_or("", |step| step.filter.id()),
            image,
            factor: 1 << level,
            origin: [0, 0],
            above: self.entries[index + 1..].to_vec(),
        }))
    }

    /// The look at `rect` (`[x0, y0, x1, y1]`, the layer's pixels) at pyramid `level`: the
    /// topmost shown filter applied to the tiles of what is below it there, with a margin of
    /// what the filter reaches; what the display needs when it shows part of a large layer
    /// close up. `None` without a shown filter, when what is below it is not known, or when
    /// `rect` misses the layer.
    pub fn look_at(&self, rect: [f64; 4], level: usize) -> Result<Option<Preview>, StackError> {
        match self.look_job(rect, level) {
            Some(job) => job.run().map(Some),
            None => Ok(None),
        }
    }

    /// What [`Self::look_at`] computes, to be computed elsewhere (on the GPU, ADR 0035).
    /// What a look at filter entry `index` computes from: the steps of the filters from the
    /// lowest one needed up to it (filters shown one on another), and what that lowest one
    /// applies to, known already (its input, or the result of the filter below it). A filter
    /// applied over another whose pixels are not evaluated yet is then shown at once, both
    /// computed on the part shown. `None` when nothing below is known.
    fn known_run(&self, index: usize) -> Option<(Arc<RasterImage>, Vec<&FilterStep>)> {
        let mut first = index;
        loop {
            let Entry::Filter(filter) = &self.entries[first] else {
                return None;
            };
            if first < index && filter.hidden {
                return None;
            }
            let below = self.below(first);
            let known = filter
                .known_input()
                .filter(|(stack, _)| *stack == below)
                .map(|(_, input)| input)
                .or_else(|| match first.checked_sub(1).map(|i| &self.entries[i]) {
                    Some(Entry::Filter(under)) if !under.hidden => {
                        under.known_output(&self.below(first - 1))
                    }
                    _ => None,
                });
            if let Some(input) = known {
                let steps = self.entries[first..=index]
                    .iter()
                    .filter_map(|entry| match entry {
                        Entry::Filter(filter) => Some(filter.steps.iter().map(|s| &**s)),
                        _ => None,
                    })
                    .flatten()
                    .collect();
                return Some((input, steps));
            }
            first = first.checked_sub(1)?;
        }
    }

    pub fn look_job(&self, rect: [f64; 4], level: usize) -> Option<LookJob> {
        let index = self.last_filter()?;
        match &self.entries[index] {
            Entry::Filter(_) => {}
            Entry::Liquify(liquify) => {
                let (known, input) = liquify.known_input()?;
                if known != self.below(index) {
                    return None;
                }
                return self.liquify_job(index, liquify, &input, rect, level);
            }
            _ => return None,
        }
        let (input, run) = self.known_run(index)?;
        let whole = input.size();
        if rect[0] >= f64::from(whole.width)
            || rect[1] >= f64::from(whole.height)
            || rect[2] <= 0.0
            || rect[3] <= 0.0
            || rect[0] >= rect[2]
            || rect[1] >= rect[3]
        {
            return None;
        }
        let level = level.min(input.levels().len() - 1);
        let factor = (1u32 << level) as f32;
        let coarse = &input.levels()[level];
        let size = coarse.size();
        // How far the filters read around a pixel at this level.
        let reach: f64 = run
            .iter()
            .map(|step| step.filter.scaled(factor).reach())
            .sum();
        let t = f64::from(TILE_SIZE);
        let f = f64::from(factor);
        let (columns, rows) = (coarse.grid().columns(), coarse.grid().rows());
        let col0 = ((rect[0] / f - reach) / t).floor().max(0.0) as u32;
        let row0 = ((rect[1] / f - reach) / t).floor().max(0.0) as u32;
        let col1 = (((rect[2] / f + reach) / t).ceil().max(0.0) as u32).min(columns);
        let row1 = (((rect[3] / f + reach) / t).ceil().max(0.0) as u32).min(rows);
        if col0 >= col1 || row0 >= row1 {
            return None;
        }
        let mut tiles = Vec::with_capacity(((col1 - col0) * (row1 - row0)) as usize);
        for row in row0..row1 {
            for col in col0..col1 {
                tiles.push(Arc::clone(coarse.tile(TileCoord { col, row })?));
            }
        }
        let origin = [col0 * TILE_SIZE, row0 * TILE_SIZE];
        let crop = Size::new(
            (col1 * TILE_SIZE).min(size.width) - origin[0],
            (row1 * TILE_SIZE).min(size.height) - origin[1],
        );
        // A pixel of the crop is `factor` pixels of the layer, from `origin`.
        let placed = Affine::translation(f64::from(origin[0]), f64::from(origin[1]))
            .then(Affine::scale(f, f));
        let steps = run
            .iter()
            .map(|step| FilterStep {
                filter: step.filter.scaled(factor),
                to_document: Projective::from(placed).then(step.to_document),
                ..(*step).clone()
            })
            .collect();
        Some(LookJob {
            tiles,
            size: crop,
            format: input.format(),
            steps,
            factor: 1 << level,
            origin,
            above: self.entries[index + 1..].to_vec(),
            warp: None,
        })
    }

    /// [`Self::preview`] of a Liquify entry: its warp of what is below it at the coarsest
    /// pyramid level of at most `max_pixels`, whole.
    fn liquify_preview(
        &self,
        index: usize,
        liquify: &LiquifyEntry,
        below: &LayerStack,
        max_pixels: u64,
    ) -> Result<Option<Preview>, StackError> {
        let Some((known, input)) = liquify.known_input() else {
            return Ok(None);
        };
        if known != *below {
            return Ok(None);
        }
        let Some(level) = input
            .levels()
            .iter()
            .position(|l| u64::from(l.size().width) * u64::from(l.size().height) <= max_pixels)
            .filter(|&level| level > 0)
        else {
            return Ok(None);
        };
        let size = input.size();
        let whole = [0.0, 0.0, f64::from(size.width), f64::from(size.height)];
        let Some(mut job) = self.liquify_job(index, liquify, &input, whole, level) else {
            return Ok(None);
        };
        // Shown at any zoom out to the whole layer: with its own pyramid.
        if let Some(warp) = job.warp.as_mut() {
            warp.pyramid = true;
        }
        job.run().map(Some)
    }

    /// The look at `rect` of a Liquify entry (see [`Self::look_job`]): the tiles of what is
    /// below it that `rect` covers at `level`, with a margin of the field's reach, and the field
    /// to read through.
    fn liquify_job(
        &self,
        index: usize,
        liquify: &LiquifyEntry,
        input: &Arc<RasterImage>,
        rect: [f64; 4],
        level: usize,
    ) -> Option<LookJob> {
        let whole = input.size();
        if rect[0] >= f64::from(whole.width)
            || rect[1] >= f64::from(whole.height)
            || rect[2] <= 0.0
            || rect[3] <= 0.0
            || rect[0] >= rect[2]
            || rect[1] >= rect[3]
        {
            return None;
        }
        let level = level.min(input.levels().len() - 1);
        let factor = 1u32 << level;
        let f = f64::from(factor);
        let coarse = &input.levels()[level];
        let size = coarse.size();
        let t = f64::from(TILE_SIZE);
        let (columns, rows) = (coarse.grid().columns(), coarse.grid().rows());
        // What is drawn: the tiles of this level that `rect` covers.
        let col0 = ((rect[0] / f / t).floor().max(0.0) as u32).min(columns);
        let row0 = ((rect[1] / f / t).floor().max(0.0) as u32).min(rows);
        let col1 = ((rect[2] / f / t).ceil().max(0.0) as u32).min(columns);
        let row1 = ((rect[3] / f / t).ceil().max(0.0) as u32).min(rows);
        if col0 >= col1 || row0 >= row1 {
            return None;
        }
        let out_origin = [col0 * TILE_SIZE, row0 * TILE_SIZE];
        let out_size = Size::new(
            (col1 * TILE_SIZE).min(size.width) - out_origin[0],
            (row1 * TILE_SIZE).min(size.height) - out_origin[1],
        );
        // What is read: those tiles and a margin of how far the field reads from.
        let margin = liquify.reach / f + 2.0;
        let (x0, y0) = (
            f64::from(out_origin[0]) - margin,
            f64::from(out_origin[1]) - margin,
        );
        let (x1, y1) = (
            f64::from(out_origin[0] + out_size.width) + margin,
            f64::from(out_origin[1] + out_size.height) + margin,
        );
        let icol0 = ((x0 / t).floor().max(0.0) as u32).min(columns);
        let irow0 = ((y0 / t).floor().max(0.0) as u32).min(rows);
        let icol1 = ((x1 / t).ceil().max(0.0) as u32).min(columns);
        let irow1 = ((y1 / t).ceil().max(0.0) as u32).min(rows);
        let mut tiles = Vec::with_capacity(((icol1 - icol0) * (irow1 - irow0)) as usize);
        for row in irow0..irow1 {
            for col in icol0..icol1 {
                tiles.push(Arc::clone(coarse.tile(TileCoord { col, row })?));
            }
        }
        let origin = [icol0 * TILE_SIZE, irow0 * TILE_SIZE];
        let crop = Size::new(
            (icol1 * TILE_SIZE).min(size.width) - origin[0],
            (irow1 * TILE_SIZE).min(size.height) - origin[1],
        );
        Some(LookJob {
            tiles,
            size: crop,
            format: input.format(),
            steps: Vec::new(),
            factor,
            origin,
            above: self.entries[index + 1..].to_vec(),
            warp: Some(WarpJob {
                field: Arc::clone(&liquify.field),
                space: liquify.space,
                out_origin,
                out_size,
                pyramid: false,
            }),
        })
    }

    /// The same result without shown filters: the result of the topmost one as the original,
    /// the entries above it on top (computed now, unless the filter has it already). What
    /// evaluates tile by tile starts from it.
    fn flat(&self) -> Result<LayerStack, StackError> {
        let Some(index) = self.last_filter() else {
            return Ok(self.clone());
        };
        let output = match &self.entries[index] {
            Entry::Filter(filter) => filter.output(&self.below(index))?,
            Entry::Liquify(liquify) => liquify.output(&self.below(index))?,
            _ => return Ok(self.clone()),
        };
        Ok(LayerStack {
            original: output,
            entries: self.entries[index + 1..].to_vec(),
        })
    }

    /// The result, evaluated from the original: every tile no entry reaches is the original's.
    pub fn evaluate(&self) -> Result<Arc<RasterImage>, StackError> {
        let format = self.format();
        self.flat()?.evaluate_as(format)
    }

    /// [`Self::evaluate`] of a stack without shown filters, in `format`.
    fn evaluate_as(&self, format: PixelFormat) -> Result<Arc<RasterImage>, StackError> {
        if self.entries.is_empty() && format == self.original.format() {
            return Ok(Arc::clone(&self.original));
        }
        let atoms = atoms(&self.entries);
        let evaluator = Evaluator::new(self, &atoms, format);
        let size = self.original.size();
        let tiles = evaluator
            .tiles(grid_coords(size).collect())?
            .into_iter()
            .map(|(_, tile)| tile)
            .collect();
        Ok(Arc::new(RasterImage::from_level0_tiles(
            size, format, tiles,
        )?))
    }

    /// The result, from `shown`, the result of `before` (an earlier state of this stack):
    /// entries added on top apply to `shown`'s tiles; otherwise only the tiles that the entries
    /// added or removed reach are evaluated again. The same pixels as [`Self::evaluate`].
    pub fn reevaluate(
        &self,
        before: &LayerStack,
        shown: &Arc<RasterImage>,
    ) -> Result<Arc<RasterImage>, StackError> {
        let format = self.format();
        match (self.last_filter(), before.last_filter()) {
            (None, None) => self.reevaluate_as(before, shown, format),
            // The same filter over the same entries: from its result, as a stack without filters.
            (Some(index), Some(then))
                if index == then
                    && Arc::ptr_eq(&self.original, &before.original)
                    && self.entries[..=index] == before.entries[..=index] =>
            {
                self.flat()?.reevaluate_as(&before.flat()?, shown, format)
            }
            _ => self.flat()?.evaluate_as(format),
        }
    }

    /// [`Self::reevaluate`] of a stack without shown filters, in `format`.
    fn reevaluate_as(
        &self,
        before: &LayerStack,
        shown: &Arc<RasterImage>,
        format: PixelFormat,
    ) -> Result<Arc<RasterImage>, StackError> {
        if !Arc::ptr_eq(&self.original, &before.original)
            || shown.format() != format
            || shown.size() != self.original.size()
        {
            return self.evaluate_as(format);
        }
        if self.entries.is_empty() && format == self.original.format() {
            return Ok(Arc::clone(&self.original));
        }
        let size = self.original.size();
        let (old, new) = (atoms(&before.entries), atoms(&self.entries));
        let common = old
            .iter()
            .zip(&new)
            .take_while(|(a, b)| a.address() == b.address())
            .count();
        let evaluator = Evaluator::new(self, &new, format);
        let replaced: Vec<(TileCoord, Arc<[u8]>)> = if common == old.len() {
            // Added on top: from what is shown.
            let mut reached = Footprint::Tiles(BTreeSet::new());
            for atom in &new[common..] {
                reached = union(reached, atom.footprint(size));
            }
            let coords = coords_of(&reached, size);
            let level = &shown.levels()[0];
            let mut work: Vec<(TileCoord, Option<Arc<[u8]>>)> =
                coords.into_iter().map(|c| (c, None)).collect();
            parallel_for_each(&mut work, |(coord, out)| {
                *out = level
                    .tile(*coord)
                    .map(|tile| evaluator.tile(*coord, common, Some(tile)));
            });
            work.into_iter()
                .filter_map(|(coord, tile)| tile.map(|t| (coord, t)))
                .collect()
        } else {
            let kept: BTreeSet<usize> = new.iter().map(|a| a.address()).collect();
            let was: BTreeSet<usize> = old.iter().map(|a| a.address()).collect();
            let removed: Vec<Atom<'_>> = old
                .iter()
                .copied()
                .filter(|a| !kept.contains(&a.address()))
                .collect();
            let added: Vec<Atom<'_>> = new
                .iter()
                .copied()
                .filter(|a| !was.contains(&a.address()))
                .collect();
            let reached = match (&removed[..], &added[..]) {
                // A paint for another of the same layer (undo or redo of a stroke continuing
                // it): only the tiles where they differ.
                ([Atom::Paint(then)], [Atom::Paint(now)])
                    if then.size == now.size
                        && then.format == now.format
                        && then.space == now.space =>
                {
                    Footprint::Tiles(changed_tiles(then, now))
                }
                _ => {
                    let mut reached = Footprint::Tiles(BTreeSet::new());
                    for atom in removed.iter().chain(&added) {
                        reached = union(reached, atom.footprint(size));
                    }
                    reached
                }
            };
            evaluator.tiles(coords_of(&reached, size))?
        };
        if replaced.is_empty() {
            return Ok(Arc::clone(shown));
        }
        Ok(Arc::new(shown.with_tiles(replaced)?))
    }
}

/// Paint being laid on top of a stack, tile by tile: a stroke's frames, Fill, Delete. It
/// continues the top paint (or starts one) and keeps the paint laid so far; what is below that
/// paint is evaluated once per tile and cached for the whole stroke, so that a frame computes
/// only the pixels that changed, as painting an image does (ADR 0027).
#[derive(Debug)]
pub struct TopPaint {
    /// The stack without the paint being laid.
    below: LayerStack,
    /// The same without shown filters (their result as its original): what tiles evaluate from.
    flat: LayerStack,
    /// The paint it continues (empty if none).
    start: PaintEntry,
    /// The paint laid so far: the tiles changed since the start.
    painted: BTreeMap<TileCoord, PaintTile>,
    /// What the layer shows meanwhile.
    format: PixelFormat,
    /// What the layer showed when the paint started: what is below it where the paint it
    /// continues did not reach.
    shown: Option<Arc<RasterImage>>,
    /// Tiles of what is below the paint, evaluated once each for the whole stroke.
    cache: HashMap<TileCoord, Arc<[u8]>>,
}

impl TopPaint {
    /// Paint on `stack` blending in `space`, whose result the layer shows as `shown`; `erase`:
    /// the paint may lower alpha (the layer shows an alpha channel from the start).
    pub fn new(
        stack: &LayerStack,
        shown: &Arc<RasterImage>,
        space: BlendSpace,
        erase: bool,
    ) -> Self {
        let (below, start) = match stack.top_paint() {
            Some(top) if top.space == space => (
                LayerStack {
                    original: Arc::clone(&stack.original),
                    entries: stack.entries[..stack.entries.len() - 1].to_vec(),
                },
                PaintEntry {
                    size: top.size,
                    format: top.format,
                    space,
                    images: OnceLock::new(),
                    below: Mutex::new(None),
                    hidden: false,
                    tiles: top.tiles.clone(),
                },
            ),
            _ => (
                stack.clone(),
                PaintEntry::empty(stack.original.format(), stack.original.size(), space),
            ),
        };
        // Invariant: evaluating a stack's own tiles cannot fail (see `evaluated`); were it to,
        // the paint is laid over the stack without its filters rather than not at all.
        let flat = below.flat().unwrap_or_else(|_| below.clone());
        let format = stack.format();
        let format = if erase && !format.layout.has_alpha() {
            paint_format(format)
        } else {
            format
        };
        // What is below the paint: what the layer shows where the paint it continues (if any)
        // did not reach, else what that paint kept, while it still applies.
        let continues = below.entries.len() < stack.entries.len();
        let shown = (shown.format() == format && shown.size() == stack.original.size())
            .then(|| Arc::clone(shown));
        let cache = match stack.top_paint() {
            Some(top) if continues => top
                .below
                .lock()
                .ok()
                .and_then(|kept| kept.clone())
                .filter(|kept| kept.stack == below && kept.format == format)
                .map(|kept| kept.tiles)
                .unwrap_or_default(),
            _ => HashMap::new(),
        };
        Self {
            below,
            flat,
            start,
            painted: BTreeMap::new(),
            format,
            shown,
            cache,
        }
    }

    /// The format of the tiles shown while painting.
    pub fn format(&self) -> PixelFormat {
        self.format
    }

    /// Whether some paint was laid.
    pub fn has_paint(&self) -> bool {
        !self.painted.is_empty()
    }

    /// Lay `op` at `amount(coord, x, y)` on the pixels `area` (`[x0, y0, x1, y1)` within the
    /// tile) of each tile of `dirty`, from the paint it started from: the tiles the layer then
    /// shows, from those of `shown` (what it showed so far, of [`Self::format`]), on every core
    /// (bands of rows). Amounts only grow during a stroke, so a pixel not reached yet keeps its
    /// start.
    /// With [`PaintOp::Clone`], `source` holds the colors laid (its tiles prepared).
    #[allow(clippy::too_many_arguments)]
    pub fn lay(
        &mut self,
        dirty: &[(TileCoord, [usize; 4])],
        op: PaintOp,
        amount: impl Fn(TileCoord, usize, usize) -> f32 + Sync,
        shown: &RasterImage,
        source: Option<&crate::clone::CloneSource>,
    ) -> Vec<(TileCoord, Arc<[u8]>)> {
        let atoms = atoms(&self.flat.entries);
        let evaluator = Evaluator::new(&self.flat, &atoms, self.format);
        let math = self.start.math();
        let color = op_color(&math, op);
        let level = &shown.levels()[0];
        let size = self.start.size;
        // Each tile with its area and what it shows so far.
        type Dirty<'a> = (TileCoord, [usize; 4], &'a Arc<[u8]>);
        let dirty: Vec<Dirty<'_>> = dirty
            .iter()
            .filter(|(c, _)| contains(size, *c))
            .filter_map(|&(c, area)| Some((c, area, level.tile(c)?)))
            .collect();
        // What is below the paint: evaluated once per tile for the whole stroke.
        let shown_below = self.shown.as_ref().map(|image| &image.levels()[0]);
        let below: Vec<Arc<[u8]>> = dirty
            .iter()
            .map(|(coord, _, _)| {
                let untouched = !self.start.tiles.contains_key(coord);
                Arc::clone(self.cache.entry(*coord).or_insert_with(|| {
                    match shown_below
                        .filter(|_| untouched)
                        .and_then(|level| level.tile(*coord))
                    {
                        Some(tile) => Arc::clone(tile),
                        None => evaluator.tile_on_every_core(*coord),
                    }
                }))
            })
            .collect();
        let identity = math.identity();
        let starts: Vec<(&[u8], &[u8], bool)> = dirty
            .iter()
            .map(|(coord, _, _)| match self.start.tiles.get(coord) {
                Some(t) => (&t.color[..], &t.keep[..], t.lowers_alpha),
                None => (&identity.0[..], &identity.1[..], false),
            })
            .collect();
        // Each tile's `P`, `k`, what it shows, and whether it was erased.
        type Buffers = (Vec<u8>, Vec<u8>, Vec<u8>, bool);
        let mut buffers: Vec<Buffers> = dirty
            .iter()
            .zip(&starts)
            .map(
                |((coord, _, current), start)| match self.painted.get(coord) {
                    Some(t) => (
                        t.color.to_vec(),
                        t.keep.to_vec(),
                        current.to_vec(),
                        t.lowers_alpha,
                    ),
                    None => (
                        start.0.to_vec(),
                        start.1.to_vec(),
                        current.to_vec(),
                        start.2,
                    ),
                },
            )
            .collect();
        let t = TILE_SIZE as usize;
        let (bpp, kb, sb) = (
            math.bpp,
            math.keep.bytes() as usize,
            evaluator.target.bytes_per_pixel,
        );
        // Bands of the rows each area reaches, every tile's together.
        type Band<'a> = (usize, usize, &'a mut [u8], &'a mut [u8], &'a mut [u8]);
        let mut work: Vec<Band<'_>> = Vec::new();
        for (index, ((p, k, px, _), (coord, area, _))) in buffers.iter_mut().zip(&dirty).enumerate()
        {
            let height = valid_area(size, *coord).1;
            let (y0, y1) = (area[1].min(height), area[3].min(height));
            if y0 >= y1 {
                continue;
            }
            let p_rows = p[y0 * t * bpp..y1 * t * bpp].chunks_mut(BAND_ROWS * t * bpp);
            let k_rows = k[y0 * t * kb..y1 * t * kb].chunks_mut(BAND_ROWS * t * kb);
            let px_rows = px[y0 * t * sb..y1 * t * sb].chunks_mut(BAND_ROWS * t * sb);
            for (n, ((pr, kr), xr)) in p_rows.zip(k_rows).zip(px_rows).enumerate() {
                work.push((index, y0 + n * BAND_ROWS, pr, kr, xr));
            }
        }
        let erased = parallel_for_each(&mut work, |(index, first, pr, kr, xr)| {
            let (coord, area, _) = dirty[*index];
            let (p0, k0, _) = starts[*index];
            let below = &below[*index];
            let width = valid_area(size, coord).0;
            let rows = pr.len() / (t * bpp);
            // Erased, or some pixel written lowers alpha: the others are the start's or the last
            // frame's, whose tiles say so already (see `PaintPixels::tile_lowering`).
            let mut erased = false;
            for r in 0..rows {
                let y = *first + r;
                for x in area[0]..area[2].min(width) {
                    let (i, local) = (y * t + x, r * t + x);
                    // From the start: an amount never re-applies over what the last frame laid.
                    pr[local * bpp..(local + 1) * bpp].copy_from_slice(&p0[i * bpp..(i + 1) * bpp]);
                    kr[local * kb..(local + 1) * kb].copy_from_slice(&k0[i * kb..(i + 1) * kb]);
                    let mut a = f64::from(amount(coord, x, y));
                    let mut color = color;
                    if a > 0.0 {
                        let (c, share) = op.at(&math, color, source, coord, x, y);
                        (color, a) = (c, a * share);
                    }
                    if a > 0.0 {
                        let (pv, kv) = math.read(pr, kr, local);
                        let (pv, kv) = lay(op, color, a.min(1.0), pv, kv);
                        math.write(pv, kv, pr, kr, local);
                        erased = erased || op == PaintOp::Erase || math.lowers_alpha(pr, kr, local);
                    }
                    evaluator.paint_pixel(
                        &math,
                        pr,
                        kr,
                        local,
                        &below[i * sb..(i + 1) * sb],
                        &mut xr[local * sb..(local + 1) * sb],
                    );
                }
            }
            (*index, erased)
        });
        drop(work);
        for (index, erased) in erased {
            buffers[index].3 |= erased;
        }
        let mut shown_tiles = Vec::with_capacity(dirty.len());
        for ((p, k, mut px, erased), (coord, _, _)) in buffers.into_iter().zip(&dirty) {
            let (width, height) = valid_area(size, *coord);
            let (mut p, mut k) = (p, k);
            pad_tile(&mut p, width, height, bpp);
            pad_tile(&mut k, width, height, kb);
            pad_tile(&mut px, width, height, sb);
            self.painted
                .insert(*coord, math.tile_lowering(p, k, erased));
            shown_tiles.push((*coord, Arc::from(px)));
        }
        shown_tiles
    }

    /// The paint laid so far, continuing the start.
    pub fn paint(&self) -> Result<PaintEntry, StackError> {
        self.start
            .with_tiles(self.painted.iter().map(|(c, t)| (*c, t.clone())).collect())
    }

    /// The stack with the paint laid so far on top; without it when it changes nothing. The
    /// paint keeps what was below the tiles it reached, for the next stroke.
    pub fn stack(&self) -> Result<LayerStack, StackError> {
        let mut paint = self.paint()?;
        let reached = self
            .cache
            .iter()
            .filter(|(coord, _)| paint.tiles.contains_key(coord))
            .map(|(coord, tile)| (*coord, Arc::clone(tile)))
            .collect();
        paint.below = Mutex::new(Some(BelowTiles {
            stack: self.below.clone(),
            format: self.format,
            tiles: reached,
        }));
        let entry = Entry::Paint(Arc::new(paint));
        self.below.check(&entry)?;
        let mut entries = self.below.entries.clone();
        if matches!(&entry, Entry::Paint(p) if !p.is_identity()) {
            entries.push(entry);
        }
        Ok(LayerStack {
            original: Arc::clone(&self.below.original),
            entries,
        })
    }
}

/// The Restore Eraser on a stack (ADR 0029): every paint entry brought back towards the
/// identity where it rubs (`P ← (1−r)·P`, `k ← (1−r)·k + r`), the effects staying applied, and
/// the tiles it reaches evaluated again through the stack, each on every core.
#[derive(Debug)]
pub struct RestorePaint {
    stack: LayerStack,
    /// By paint entry (its index in the stack), its tiles restored so far.
    restored: BTreeMap<usize, BTreeMap<TileCoord, PaintTile>>,
}

impl RestorePaint {
    pub fn new(stack: &LayerStack) -> Self {
        Self {
            stack: stack.clone(),
            restored: BTreeMap::new(),
        }
    }

    /// The format of the tiles shown while restoring: the stack's.
    pub fn format(&self) -> PixelFormat {
        self.stack.format()
    }

    /// Whether some paint was restored.
    pub fn has_paint(&self) -> bool {
        !self.restored.is_empty()
    }

    /// Restore at `amount(coord, x, y)` on the tiles of `dirty` (amounts only grow during a
    /// stroke: every paint tile is restored from the stroke's start); the tiles the layer then
    /// shows.
    pub fn lay(
        &mut self,
        dirty: &[(TileCoord, [usize; 4])],
        amount: impl Fn(TileCoord, usize, usize) -> f32 + Sync,
    ) -> Vec<(TileCoord, Arc<[u8]>)> {
        let size = self.stack.original.size();
        let mut work: Vec<(usize, TileCoord, Option<PaintTile>)> = Vec::new();
        // Paint below a filter would make it compute again at every frame: the Restore Eraser
        // reaches the paint above the topmost shown filter only (ADR 0034).
        let first = self.stack.last_filter().map_or(0, |index| index + 1);
        for (index, entry) in self.stack.entries.iter().enumerate().skip(first) {
            // A hidden paint is left as it is: the stroke cannot be seen on it.
            if let Entry::Paint(paint) = entry
                && !paint.hidden
            {
                for (coord, _) in dirty {
                    if contains(size, *coord) && paint.tiles.contains_key(coord) {
                        work.push((index, *coord, None));
                    }
                }
            }
        }
        if work.is_empty() {
            return Vec::new();
        }
        let entries = &self.stack.entries;
        parallel_for_each(&mut work, |(index, coord, out)| {
            if let Entry::Paint(paint) = &entries[*index] {
                let coord = *coord;
                *out =
                    Some(paint.painted_tile(coord, PaintOp::Restore, |x, y| amount(coord, x, y)));
            }
        });
        for (index, coord, tile) in work {
            if let Some(tile) = tile {
                self.restored.entry(index).or_default().insert(coord, tile);
            }
        }
        let current = self.current();
        // Invariant: as in `TopPaint::new`; the filters' results are known by then.
        let stack = current.flat().unwrap_or(current);
        let atoms = atoms(&stack.entries);
        let evaluator = Evaluator::new(&stack, &atoms, self.format());
        dirty
            .iter()
            .filter(|(coord, _)| contains(size, *coord))
            .map(|(coord, _)| (*coord, evaluator.tile_on_every_core(*coord)))
            .collect()
    }

    /// The stack with the paint restored so far (the same format as the original stack's).
    fn current(&self) -> LayerStack {
        let entries = self
            .stack
            .entries
            .iter()
            .enumerate()
            .map(|(index, entry)| match (entry, self.restored.get(&index)) {
                (Entry::Paint(paint), Some(tiles)) => {
                    let mut all = paint.tiles.clone();
                    all.extend(tiles.iter().map(|(c, t)| (*c, t.clone())));
                    Entry::Paint(Arc::new(PaintEntry {
                        size: paint.size,
                        format: paint.format,
                        space: paint.space,
                        tiles: all,
                        images: OnceLock::new(),
                        below: Mutex::new(None),
                        hidden: paint.hidden,
                    }))
                }
                _ => entry.clone(),
            })
            .collect();
        LayerStack {
            original: Arc::clone(&self.stack.original),
            entries,
        }
    }

    /// The stack with the paint restored: tiles back to the identity are dropped, and paint
    /// entries left with none are deleted (their neighbours merging as when deleted).
    pub fn stack(&self) -> Result<LayerStack, StackError> {
        let mut stack = self.current();
        let mut emptied = Vec::new();
        for (index, entry) in stack.entries.iter_mut().enumerate() {
            let Entry::Paint(paint) = entry else {
                continue;
            };
            if !self.restored.contains_key(&index) {
                continue;
            }
            let math = paint.math();
            let (color, keep) = math.identity();
            let tiles: BTreeMap<TileCoord, PaintTile> = paint
                .tiles
                .iter()
                .filter(|(_, t)| *t.color != *color || *t.keep != *keep)
                .map(|(c, t)| (*c, t.clone()))
                .collect();
            if tiles.is_empty() {
                emptied.push(index);
            }
            *entry = Entry::Paint(Arc::new(PaintEntry {
                size: paint.size,
                format: paint.format,
                space: paint.space,
                tiles,
                images: OnceLock::new(),
                below: Mutex::new(None),
                hidden: paint.hidden,
            }));
        }
        // From the top, so that the indices below stay right.
        for index in emptied.into_iter().rev() {
            stack = stack.without(index)?;
        }
        Ok(stack)
    }
}

/// The tiles where two paints of a layer differ: those of one only, and those whose pixels are
/// not the same (tiles a stroke did not touch share them).
fn changed_tiles(a: &PaintEntry, b: &PaintEntry) -> BTreeSet<TileCoord> {
    let same = |x: &PaintTile, y: &PaintTile| {
        Arc::ptr_eq(&x.color, &y.color) && Arc::ptr_eq(&x.keep, &y.keep)
    };
    let mut changed: BTreeSet<TileCoord> = a
        .tiles
        .iter()
        .filter(|(coord, tile)| b.tiles.get(coord).is_none_or(|other| !same(tile, other)))
        .map(|(coord, _)| *coord)
        .collect();
    changed.extend(b.tiles.keys().filter(|coord| !a.tiles.contains_key(coord)));
    changed
}

fn union(a: Footprint, b: Footprint) -> Footprint {
    match (a, b) {
        (Footprint::Tiles(mut a), Footprint::Tiles(b)) => {
            a.extend(b);
            Footprint::Tiles(a)
        }
        _ => Footprint::All,
    }
}

fn coords_of(footprint: &Footprint, size: Size) -> Vec<TileCoord> {
    match footprint {
        Footprint::All => grid_coords(size).collect(),
        Footprint::Tiles(tiles) => tiles
            .iter()
            .copied()
            .filter(|&c| contains(size, c))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection::SELECTION_FORMAT;

    /// Two tiles by two, the last ones partial.
    const W: u32 = 300;
    const H: u32 = 260;
    const T: u32 = TILE_SIZE;

    fn size() -> Size {
        Size::new(W, H)
    }

    fn image(format: PixelFormat, pixel: impl Fn(u32, u32) -> Vec<u8>) -> Arc<RasterImage> {
        let mut bytes = Vec::new();
        for y in 0..H {
            for x in 0..W {
                bytes.extend(pixel(x, y));
            }
        }
        Arc::new(RasterImage::from_pixels(size(), format, &bytes).unwrap())
    }

    /// An opaque 8-bit sRGB gradient, with an alpha channel or not.
    fn gradient(alpha: bool) -> Arc<RasterImage> {
        let format = if alpha {
            PixelFormat::RGBA8_SRGB
        } else {
            PixelFormat {
                layout: ChannelLayout::Rgb,
                ..PixelFormat::RGBA8_SRGB
            }
        };
        image(format, move |x, y| {
            let mut px = vec![(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8];
            if alpha {
                px.push(255);
            }
            px
        })
    }

    /// Pixel (`x`, `y`) of level 0 as stored.
    fn pixel(image: &RasterImage, x: u32, y: u32) -> Vec<u8> {
        let coord = TileCoord {
            col: x / T,
            row: y / T,
        };
        let tile = image.levels()[0].tile(coord).unwrap();
        let bpp = image.stored_format().bytes_per_pixel() as usize;
        let i = ((y % T) * T + x % T) as usize;
        tile[i * bpp..(i + 1) * bpp].to_vec()
    }

    /// The largest difference between two images' samples (same format).
    fn difference(a: &RasterImage, b: &RasterImage) -> u8 {
        assert_eq!(a.format(), b.format());
        let mut most = 0;
        for y in 0..H {
            for x in 0..W {
                for (s, t) in pixel(a, x, y).iter().zip(pixel(b, x, y)) {
                    most = most.max(s.abs_diff(t));
                }
            }
        }
        most
    }

    fn tile_of(image: &RasterImage, col: u32, row: u32) -> &Arc<[u8]> {
        image.levels()[0].tile(TileCoord { col, row }).unwrap()
    }

    /// `paint` with `op` at `amount(x, y)` (layer pixels), on the tiles it reaches.
    fn painted(
        paint: &PaintEntry,
        op: PaintOp,
        amount: impl Fn(u32, u32) -> f32 + Copy,
    ) -> Arc<PaintEntry> {
        let tiles = grid_coords(paint.size())
            .filter(|&coord| {
                (0..T).any(|y| {
                    (0..T).any(|x| {
                        let (px, py) = (coord.col * T + x, coord.row * T + y);
                        px < W && py < H && amount(px, py) > 0.0
                    })
                })
            })
            .map(|coord| {
                let tile = paint.painted_tile(coord, op, |x, y| {
                    amount(coord.col * T + x as u32, coord.row * T + y as u32)
                });
                (coord, tile)
            })
            .collect();
        Arc::new(paint.with_tiles(tiles).unwrap())
    }

    fn empty(original: &RasterImage) -> PaintEntry {
        PaintEntry::empty(original.format(), original.size(), BlendSpace::Perceptual)
    }

    fn gray(v: f32) -> PaintOp {
        PaintOp::Color(LinearRgba::new(v, v, v, 1.0))
    }

    fn effect(adjustment: Adjustment, selection: Option<Selection>) -> Effect {
        Effect {
            adjustment,
            selection,
            to_document: Affine::IDENTITY.into(),
            space: BlendSpace::Perceptual,
        }
    }

    /// A hard selection of the document pixels where `inside(x, y)`.
    fn selection(inside: impl Fn(u32, u32) -> bool) -> Selection {
        let image = image(SELECTION_FORMAT, |x, y| {
            let v: u16 = if inside(x, y) { u16::MAX } else { 0 };
            v.to_ne_bytes().to_vec()
        });
        Selection::new(image).unwrap()
    }

    #[test]
    fn an_empty_stack_shows_its_original() {
        let original = gradient(true);
        let stack = LayerStack::new(Arc::clone(&original));
        assert!(Arc::ptr_eq(&stack.evaluate().unwrap(), &original));
    }

    #[test]
    fn paint_replaces_where_it_covers_and_shares_the_tiles_it_does_not_reach() {
        let original = gradient(true);
        let paint = painted(&empty(&original), gray(1.0), |x, y| {
            if x < 100 && y < 100 { 1.0 } else { 0.0 }
        });
        assert_eq!(paint.tiles().len(), 1);
        let stack = LayerStack::new(Arc::clone(&original))
            .with_top_paint(paint)
            .unwrap();
        let shown = stack.evaluate().unwrap();
        assert_eq!(pixel(&shown, 10, 10), [255, 255, 255, 255]);
        assert_eq!(pixel(&shown, 150, 10), pixel(&original, 150, 10));
        assert!(Arc::ptr_eq(tile_of(&shown, 1, 1), tile_of(&original, 1, 1)));
    }

    #[test]
    fn undoing_a_stroke_evaluates_only_the_tiles_it_changed() {
        let original = gradient(true);
        let plain = LayerStack::new(Arc::clone(&original));
        let first = painted(&empty(&original), gray(0.3), |x, y| {
            if x < 40 && y < 40 { 1.0 } else { 0.0 }
        });
        // A stroke continuing it, in another tile.
        let second = painted(&first, gray(0.8), |x, y| {
            if x > 270 && y > 230 { 1.0 } else { 0.0 }
        });
        let before = plain.with_top_paint(first).unwrap();
        let after = plain.with_top_paint(second).unwrap();
        let shown = after.evaluate().unwrap();
        // Undo: back to the paint before the stroke.
        let undone = before.reevaluate(&after, &shown).unwrap();
        assert_eq!(difference(&undone, &before.evaluate().unwrap()), 0);
        // The first stroke's tile, the same in both paints, is not evaluated again.
        assert!(Arc::ptr_eq(tile_of(&undone, 0, 0), tile_of(&shown, 0, 0)));
        assert!(!Arc::ptr_eq(tile_of(&undone, 1, 1), tile_of(&shown, 1, 1)));
        // Redo likewise.
        let redone = after.reevaluate(&before, &undone).unwrap();
        assert_eq!(difference(&redone, &shown), 0);
        assert!(Arc::ptr_eq(tile_of(&redone, 0, 0), tile_of(&undone, 0, 0)));
    }

    #[test]
    fn a_stroke_knows_its_tiles_lower_alpha_as_their_pixels_say() {
        // A stroke's frames tell from the pixels they write whether a tile lowers alpha: the
        // same as reading every pixel of it, for the Eraser, the Brush over it, and again.
        let original = gradient(true);
        let brush = PaintOp::Color(LinearRgba::new(1.0, 0.2, 0.1, 1.0));
        let mut stack = LayerStack::new(Arc::clone(&original));
        for op in [PaintOp::Erase, brush, PaintOp::Erase] {
            let shown = stack.evaluate().unwrap();
            let mut top =
                TopPaint::new(&stack, &shown, BlendSpace::Perceptual, op == PaintOp::Erase);
            let mut current = Arc::clone(&shown);
            // Two frames over parts of two tiles, the second over the first.
            for area in [[10, 20, 120, 90], [60, 40, 256, 200]] {
                let dirty: Vec<(TileCoord, [usize; 4])> = [(0, 0), (1, 0)]
                    .map(|(col, row)| (TileCoord { col, row }, area))
                    .to_vec();
                let amount = |_, x: usize, y: usize| ((x + y) % 7) as f32 / 6.0;
                let replaced = top.lay(&dirty, op, amount, &current, None);
                current = Arc::new(current.with_tiles(replaced).unwrap());
            }
            stack = top.stack().unwrap();
            let paint = stack.top_paint().unwrap();
            let math = paint.math();
            for (coord, tile) in paint.tiles() {
                let read = math.tile(tile.color.to_vec(), tile.keep.to_vec());
                assert_eq!(tile.lowers_alpha, read.lowers_alpha, "{op:?} {coord:?}");
            }
        }
    }

    #[test]
    fn soft_paint_keeps_opaque_pixels_opaque() {
        for alpha in [true, false] {
            let original = gradient(alpha);
            let paint = painted(&empty(&original), gray(0.3), |x, y| {
                ((x * 7 + y * 13) % 256) as f32 / 255.0
            });
            assert!(!paint.lowers_alpha());
            let stack = LayerStack::new(Arc::clone(&original))
                .with_top_paint(paint)
                .unwrap();
            // No alpha channel needed: RGB stays RGB.
            assert_eq!(stack.format(), original.format());
            let shown = stack.evaluate().unwrap();
            for y in 0..H {
                for x in 0..W {
                    assert_eq!(pixel(&shown, x, y)[3], 255, "({x}, {y})");
                }
            }
        }
    }

    #[test]
    fn the_eraser_gives_rgb_layers_an_alpha_channel() {
        let original = gradient(false);
        let paint = painted(&empty(&original), PaintOp::Erase, |x, _| {
            if x < 50 { 0.5 } else { 0.0 }
        });
        assert!(paint.lowers_alpha());
        let stack = LayerStack::new(Arc::clone(&original))
            .with_top_paint(paint)
            .unwrap();
        assert_eq!(stack.format().layout, ChannelLayout::Rgba);
        let shown = stack.evaluate().unwrap();
        assert!(pixel(&shown, 10, 10)[3].abs_diff(128) <= 1);
        assert_eq!(pixel(&shown, 10, 10)[..3], pixel(&original, 10, 10)[..3]);
        assert_eq!(pixel(&shown, 100, 10), pixel(&original, 100, 10));
        // RGB is stored with alpha: the tiles the eraser did not reach are shared.
        assert!(Arc::ptr_eq(tile_of(&shown, 1, 0), tile_of(&original, 1, 0)));
    }

    #[test]
    fn gray_layers_are_painted_in_gray() {
        let original = image(
            PixelFormat {
                layout: ChannelLayout::Gray,
                ..PixelFormat::RGBA8_SRGB
            },
            |x, _| vec![(x % 256) as u8],
        );
        let white = painted(
            &empty(&original),
            gray(1.0),
            |x, _| {
                if x < 10 { 1.0 } else { 0.0 }
            },
        );
        let stack = LayerStack::new(Arc::clone(&original))
            .with_top_paint(Arc::clone(&white))
            .unwrap();
        assert_eq!(stack.format(), original.format());
        assert_eq!(pixel(&stack.evaluate().unwrap(), 5, 5), [255]);
        let erased = painted(
            &white,
            PaintOp::Erase,
            |x, _| {
                if x < 10 { 1.0 } else { 0.0 }
            },
        );
        let stack = stack.with_top_paint(erased).unwrap();
        assert_eq!(stack.format().layout, ChannelLayout::GrayAlpha);
        let shown = stack.evaluate().unwrap();
        assert_eq!(pixel(&shown, 5, 5)[1], 0);
        assert_eq!(pixel(&shown, 50, 5), [50, 255]);
    }

    #[test]
    fn paint_follows_when_an_effect_below_it_is_deleted() {
        let original = gradient(true);
        let paint = painted(&empty(&original), gray(1.0), |x, _| {
            if x < 120 { 0.5 } else { 0.0 }
        });
        let stack = LayerStack::new(Arc::clone(&original))
            .with_effect(effect(Adjustment::Invert, None))
            .unwrap()
            .with_top_paint(Arc::clone(&paint))
            .unwrap();
        let inverted = stack.evaluate().unwrap();
        assert_eq!(pixel(&inverted, 200, 10)[0], 255 - 200);
        let deleted = stack.without(0).unwrap();
        assert_eq!(deleted.entries(), &[Entry::Paint(Arc::clone(&paint))]);
        let direct = LayerStack::new(Arc::clone(&original))
            .with_top_paint(paint)
            .unwrap()
            .evaluate()
            .unwrap();
        assert_eq!(difference(&deleted.evaluate().unwrap(), &direct), 0);
    }

    #[test]
    fn paint_follows_when_an_effect_below_it_is_edited() {
        let original = gradient(true);
        let paint = painted(&empty(&original), gray(1.0), |x, _| {
            if x < 120 { 0.5 } else { 0.0 }
        });
        let built = |levels: f32| {
            LayerStack::new(Arc::clone(&original))
                .with_effect(effect(Adjustment::Posterize { levels }, None))
                .unwrap()
                .with_top_paint(Arc::clone(&paint))
                .unwrap()
        };
        let stack = built(2.0);
        let edited = stack
            .with_steps(
                0,
                &[Operation::Adjustment(Adjustment::Posterize { levels: 4.0 })],
            )
            .unwrap();
        // The paint is the same entry, laid over the new result.
        assert_eq!(edited.entries()[1], stack.entries()[1]);
        let direct = built(4.0).evaluate().unwrap();
        assert_eq!(difference(&edited.evaluate().unwrap(), &direct), 0);
        let shown = stack.evaluate().unwrap();
        assert_eq!(
            difference(&edited.reevaluate(&stack, &shown).unwrap(), &direct),
            0
        );
    }

    #[test]
    fn each_step_of_an_entry_read_from_an_older_file_is_edited_on_its_own() {
        let original = gradient(true);
        let stack = LayerStack::with_entries(
            Arc::clone(&original),
            vec![Entry::Effect(Arc::new(
                EffectEntry::new(vec![
                    Arc::new(effect(Adjustment::Posterize { levels: 2.0 }, None)),
                    Arc::new(effect(Adjustment::Posterize { levels: 8.0 }, None)),
                ])
                .unwrap(),
            ))],
        )
        .unwrap();
        let Entry::Effect(entry) = &stack.entries()[0] else {
            panic!("an effect entry");
        };
        assert_eq!(entry.steps().len(), 2);
        let edited = stack
            .with_steps(
                0,
                &[
                    Operation::Adjustment(Adjustment::Posterize { levels: 3.0 }),
                    Operation::Adjustment(entry.steps()[1].adjustment),
                ],
            )
            .unwrap();
        let Entry::Effect(changed) = &edited.entries()[0] else {
            panic!("an effect entry");
        };
        assert_eq!(
            changed.steps()[0].adjustment,
            Adjustment::Posterize { levels: 3.0 }
        );
        // The step left alone is the same allocation.
        assert!(Arc::ptr_eq(&changed.steps()[1], &entry.steps()[1]));
        assert_eq!(
            stack.with_steps(
                0,
                &[Operation::Adjustment(Adjustment::Posterize { levels: 3.0 })]
            ),
            Err(StackError::InvalidEffect)
        );
    }

    #[test]
    fn hidden_entries_are_kept_but_not_applied_and_do_not_merge() {
        let original = gradient(true);
        let below = painted(&empty(&original), gray(0.5), |x, _| {
            if x < 200 { 0.6 } else { 0.0 }
        });
        let above = painted(&empty(&original), gray(1.0), |_, y| {
            if y < 150 { 0.3 } else { 0.0 }
        });
        let stack = LayerStack::with_entries(
            Arc::clone(&original),
            vec![
                Entry::Paint(Arc::clone(&below)),
                Entry::Effect(Arc::new(
                    EffectEntry::new(vec![Arc::new(effect(Adjustment::Invert, None))]).unwrap(),
                )),
                Entry::Paint(Arc::clone(&above)),
            ],
        )
        .unwrap();
        let hidden = stack.with_hidden(2, true).unwrap();
        assert!(hidden.entries()[2].hidden());
        // A stroke does not continue a hidden paint.
        assert!(hidden.top_paint().is_none());
        // Deleting what separates them: a hidden paint and a shown one stay apart.
        let deleted = hidden.without(1).unwrap();
        assert_eq!(deleted.entries().len(), 2);
        let only_below = LayerStack::new(Arc::clone(&original))
            .with_top_paint(Arc::clone(&below))
            .unwrap()
            .evaluate()
            .unwrap();
        assert_eq!(difference(&deleted.evaluate().unwrap(), &only_below), 0);
        // Shown again: as before.
        let shown = hidden.with_hidden(2, false).unwrap();
        assert_eq!(
            difference(&shown.evaluate().unwrap(), &stack.evaluate().unwrap()),
            0
        );
        assert_eq!(
            stack.with_hidden(3, true).unwrap_err(),
            StackError::IndexOutOfRange(3)
        );
    }

    #[test]
    fn neighbouring_paints_merge_when_what_separates_them_is_deleted() {
        let original = gradient(true);
        let below = painted(&empty(&original), gray(0.5), |x, _| {
            if x < 200 { 0.6 } else { 0.0 }
        });
        let above = painted(&empty(&original), gray(1.0), |_, y| {
            if y < 150 { 0.3 } else { 0.0 }
        });
        let stack = LayerStack::with_entries(
            Arc::clone(&original),
            vec![
                Entry::Paint(Arc::clone(&below)),
                Entry::Effect(Arc::new(
                    EffectEntry::new(vec![Arc::new(effect(Adjustment::Invert, None))]).unwrap(),
                )),
                Entry::Paint(Arc::clone(&above)),
            ],
        )
        .unwrap();
        let merged = stack.without(1).unwrap();
        assert_eq!(merged.entries().len(), 1);
        assert!(matches!(merged.entries()[0], Entry::Paint(_)));
        let apart = LayerStack::with_entries(
            Arc::clone(&original),
            vec![Entry::Paint(below), Entry::Paint(above)],
        )
        .unwrap();
        // Exact but for one rounding.
        assert!(difference(&merged.evaluate().unwrap(), &apart.evaluate().unwrap()) <= 1);
    }

    #[test]
    fn effects_of_a_kind_merge_and_two_inverts_cancel() {
        let original = gradient(true);
        let paint = painted(
            &empty(&original),
            gray(1.0),
            |x, _| {
                if x < 10 { 1.0 } else { 0.0 }
            },
        );
        let brighter = |amount| {
            effect(
                Adjustment::BrightnessContrast {
                    brightness: amount,
                    contrast: 0.0,
                },
                None,
            )
        };
        let base = LayerStack::new(Arc::clone(&original));
        let stack = base
            .with_effect(brighter(20.0))
            .unwrap()
            .with_top_paint(Arc::clone(&paint))
            .unwrap()
            .with_effect(brighter(-40.0))
            .unwrap();
        // Brightness/Contrast does not combine: two entries, as if applied in a row.
        let merged = stack.without(1).unwrap();
        assert_eq!(merged.entries().len(), 2);
        let apart = base
            .with_effect(brighter(20.0))
            .unwrap()
            .with_effect(brighter(-40.0))
            .unwrap();
        // The same steps, each rounded as before: the same pixels.
        assert_eq!(
            difference(&merged.evaluate().unwrap(), &apart.evaluate().unwrap()),
            0
        );

        let inverts = base
            .with_effect(effect(Adjustment::Invert, None))
            .unwrap()
            .with_top_paint(paint)
            .unwrap()
            .with_effect(effect(Adjustment::Invert, None))
            .unwrap();
        let cancelled = inverts.without(1).unwrap();
        assert!(cancelled.is_empty());
        assert!(Arc::ptr_eq(&cancelled.evaluate().unwrap(), &original));
    }

    #[test]
    fn effects_that_do_not_combine_are_entries_of_their_own_and_inverts_cancel() {
        let original = gradient(true);
        let brighter = |amount| {
            effect(
                Adjustment::BrightnessContrast {
                    brightness: amount,
                    contrast: 0.0,
                },
                None,
            )
        };
        let base = LayerStack::new(Arc::clone(&original));
        let once = base.with_effect(brighter(20.0)).unwrap();
        let twice = once.with_effect(brighter(-40.0)).unwrap();
        assert_eq!(twice.entries().len(), 2);
        // Applied on top of what is shown: the first entry is not evaluated again.
        let shown = once.reevaluate(&base, &original).unwrap();
        let shown = twice.reevaluate(&once, &shown).unwrap();
        assert_eq!(difference(&shown, &twice.evaluate().unwrap()), 0);
        // Another kind starts an entry; an Invert on an Invert cancels.
        let other = twice.with_effect(effect(Adjustment::Invert, None)).unwrap();
        assert_eq!(other.entries().len(), 3);
        let back = other.with_effect(effect(Adjustment::Invert, None)).unwrap();
        assert_eq!(back.entries(), twice.entries());
    }

    #[test]
    fn settings_that_combine_exactly_make_one_entry() {
        let original = gradient(true);
        let base = LayerStack::new(Arc::clone(&original));
        let exposure = |exposure| Adjustment::Exposure {
            exposure,
            offset: 0.0,
            gamma: 1.0,
        };
        let stops = base
            .with_effect(effect(exposure(1.0), None))
            .unwrap()
            .with_effect(effect(exposure(0.5), None))
            .unwrap();
        let [Entry::Effect(one)] = stops.entries() else {
            panic!("one entry expected");
        };
        assert_eq!(one.steps()[0].adjustment, exposure(1.5));
        let hue = |hue| Adjustment::HueSaturation {
            hue,
            saturation: 0.0,
            lightness: 0.0,
        };
        let turned = base
            .with_effect(effect(hue(150.0), None))
            .unwrap()
            .with_effect(effect(hue(60.0), None))
            .unwrap();
        let [Entry::Effect(one)] = turned.entries() else {
            panic!("one entry expected");
        };
        assert_eq!(one.steps()[0].adjustment, hue(-150.0));
        // With a saturation, or within another selection: two entries.
        let saturated = Adjustment::HueSaturation {
            hue: 10.0,
            saturation: 20.0,
            lightness: 0.0,
        };
        let two = base
            .with_effect(effect(saturated, None))
            .unwrap()
            .with_effect(effect(saturated, None))
            .unwrap();
        assert_eq!(two.entries().len(), 2);
        let elsewhere = base
            .with_effect(effect(exposure(1.0), None))
            .unwrap()
            .with_effect(effect(exposure(1.0), Some(selection(|x, _| x < 50))))
            .unwrap();
        assert_eq!(elsewhere.entries().len(), 2);
        // Two blurs: one, of the root of the sum of the squares.
        let blurred = base
            .with_filter(blur(3.0, None), None)
            .unwrap()
            .with_filter(blur(4.0, None), None)
            .unwrap();
        let [Entry::Filter(one)] = blurred.entries() else {
            panic!("one entry expected");
        };
        assert_eq!(one.steps()[0].filter, Filter::GaussianBlur { radius: 5.0 });
    }

    #[test]
    fn an_effect_stays_within_its_selection() {
        let original = gradient(true);
        let stack = LayerStack::new(Arc::clone(&original))
            .with_effect(effect(Adjustment::Invert, Some(selection(|x, _| x < 100))))
            .unwrap();
        let shown = stack.evaluate().unwrap();
        assert_eq!(pixel(&shown, 10, 10)[..3], [245, 245, 235]);
        assert_eq!(pixel(&shown, 150, 10), pixel(&original, 150, 10));
        assert!(Arc::ptr_eq(tile_of(&shown, 1, 0), tile_of(&original, 1, 0)));

        // Applied to a layer placed 50 pixels to the right: the selection is read where the
        // layer's pixels were then.
        let moved = LayerStack::new(Arc::clone(&original))
            .with_effect(Effect {
                to_document: Affine::translation(50.0, 0.0).into(),
                ..effect(Adjustment::Invert, Some(selection(|x, _| x < 100)))
            })
            .unwrap()
            .evaluate()
            .unwrap();
        assert_eq!(pixel(&moved, 40, 10)[0], 255 - 40);
        assert_eq!(pixel(&moved, 60, 10), pixel(&original, 60, 10));
    }

    #[test]
    fn reevaluating_gives_what_evaluating_gives() {
        let original = gradient(false);
        let first = painted(&empty(&original), gray(0.2), |x, y| {
            if x < 140 && y > 30 { 0.7 } else { 0.0 }
        });
        let erased = painted(&empty(&original), PaintOp::Erase, |x, _| {
            if (100..280).contains(&x) { 0.4 } else { 0.0 }
        });
        let continued = painted(&erased, gray(0.9), |_, y| if y > 200 { 0.8 } else { 0.0 });
        let s0 = LayerStack::new(Arc::clone(&original));
        let s1 = s0.with_top_paint(first).unwrap();
        let s2 = s1
            .with_effect(effect(
                Adjustment::Posterize { levels: 4.0 },
                Some(selection(|x, y| x > 20 && y < 120)),
            ))
            .unwrap();
        let s3 = s2.with_top_paint(erased).unwrap();
        let s4 = s3.with_top_paint(continued).unwrap();
        let s5 = s4.without(1).unwrap();
        let s6 = s5.without(0).unwrap();
        let states = [s0, s1, s2, s3, s4, s5, s6];
        let mut shown = states[0].evaluate().unwrap();
        // Forward, then back as undo goes.
        let order: Vec<usize> = (1..states.len())
            .chain((0..states.len() - 1).rev())
            .collect();
        let mut previous = 0;
        for index in order {
            let stack = &states[index];
            shown = stack.reevaluate(&states[previous], &shown).unwrap();
            let fresh = stack.evaluate().unwrap();
            assert_eq!(shown.format(), fresh.format(), "state {index}");
            assert_eq!(difference(&shown, &fresh), 0, "state {index}");
            previous = index;
        }
        assert!(Arc::ptr_eq(&shown, &original));
    }

    #[test]
    fn images_painted_the_old_way_convert_exactly() {
        let original = gradient(true);
        let painted_image = image(PixelFormat::RGBA8_SRGB, |x, y| {
            if x < 40 && y < 40 {
                vec![200, 30, 90, ((x * 6 + y) % 256) as u8]
            } else {
                vec![(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8, 255]
            }
        });
        let paint =
            PaintEntry::from_painted(&original, &painted_image, BlendSpace::Perceptual).unwrap();
        assert_eq!(paint.tiles().len(), 1);
        let stack = LayerStack::new(Arc::clone(&original))
            .with_top_paint(Arc::new(paint))
            .unwrap();
        assert_eq!(difference(&stack.evaluate().unwrap(), &painted_image), 0);
    }

    #[test]
    fn the_restore_eraser_brings_back_the_original() {
        let original = gradient(true);
        let paint = painted(&empty(&original), gray(0.8), |x, _| {
            if x < 100 { 0.9 } else { 0.0 }
        });
        let restored = painted(
            &paint,
            PaintOp::Restore,
            |x, _| {
                if x < 100 { 1.0 } else { 0.0 }
            },
        );
        let stack = LayerStack::new(Arc::clone(&original))
            .with_top_paint(restored)
            .unwrap();
        assert_eq!(difference(&stack.evaluate().unwrap(), &original), 0);
    }

    #[test]
    fn a_stroke_shows_what_its_stack_evaluates_to() {
        let original = gradient(false);
        let first = painted(&empty(&original), gray(0.2), |x, _| {
            if x < 140 { 0.7 } else { 0.0 }
        });
        let stack = LayerStack::new(Arc::clone(&original))
            .with_effect(effect(Adjustment::Invert, Some(selection(|_, y| y < 100))))
            .unwrap()
            .with_top_paint(first)
            .unwrap();
        for (op, erase) in [(gray(0.9), false), (PaintOp::Erase, true)] {
            let mut top = TopPaint::new(
                &stack,
                &stack.evaluate().unwrap(),
                BlendSpace::Perceptual,
                erase,
            );
            let mut shown = stack.evaluate().unwrap();
            if erase {
                // RGB is stored with alpha: the same tiles.
                shown = Arc::new(shown.with_alpha().unwrap().unwrap());
            }
            assert_eq!(shown.format(), top.format());
            let t = T as usize;
            // Two frames, the second reaching further with more.
            for (reach, more) in [(100, 0.0), (200, 0.3)] {
                let dirty = [
                    (TileCoord { col: 0, row: 0 }, [0, 0, t, t]),
                    (TileCoord { col: 1, row: 1 }, [0, 0, 20, 4]),
                ];
                let amount = move |coord: TileCoord, x: usize, _| {
                    if coord.col == 0 && x > reach {
                        0.0
                    } else {
                        0.25 + more
                    }
                };
                let tiles = top.lay(&dirty, op, amount, &shown, None);
                shown = Arc::new(shown.with_tiles(tiles).unwrap());
            }
            let result = top.stack().unwrap();
            assert_eq!(result.entries().len(), 2, "the top paint continues");
            assert_eq!(result.format(), top.format());
            assert_eq!(difference(&shown, &result.evaluate().unwrap()), 0);
        }
    }

    fn blur(radius: f32, selection: Option<Selection>) -> FilterStep {
        FilterStep {
            filter: Filter::GaussianBlur { radius },
            selection,
            to_document: Affine::IDENTITY.into(),
            space: BlendSpace::Perceptual,
        }
    }

    /// Opaque black left of `x = 150`, white from there.
    fn halves() -> Arc<RasterImage> {
        image(PixelFormat::RGBA8_SRGB, |x, _| {
            let v = if x < 150 { 0 } else { 255 };
            vec![v, v, v, 255]
        })
    }

    #[test]
    fn a_filtered_layer_is_evaluated_in_the_background_once_its_state_lasts() {
        // What painting it needs is there soon after a filter is applied, not waited for then.
        let stack = LayerStack::new(halves())
            .with_filter(blur(3.0, None), None)
            .unwrap();
        let pixels = Pixels::pending(stack, None);
        pixels.evaluate_in_background();
        assert!(
            pixels.ready_image().is_none(),
            "not at once: the state may be replaced"
        );
        let start = std::time::Instant::now();
        while pixels.ready_image().is_none() && start.elapsed().as_secs() < 10 {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(pixels.ready_image().is_some());
    }

    #[test]
    fn a_state_replaced_before_its_quick_look_gets_none_computed() {
        let original = halves();
        let stack = LayerStack::new(Arc::clone(&original))
            .with_filter(blur(3.0, None), Some(original))
            .unwrap();
        let pixels = Pixels::pending(stack, None);
        let held = Arc::downgrade(&pixels.0);
        {
            // Another quick look being computed meanwhile; then the state is replaced.
            let _other = PREVIEWS.lock().unwrap();
            pixels.evaluate_in_background();
            drop(pixels);
        }
        // Its thread waits for the state to last before evaluating it whole, holding it.
        std::thread::sleep(std::time::Duration::from_millis(100));
        let state = Pixels(held.upgrade().expect("held by its thread until then"));
        assert!(state.0.preview.get().is_none());
    }

    #[test]
    fn a_filter_over_a_filter_not_evaluated_yet_has_a_look_at_once() {
        let original = halves();
        let once = LayerStack::new(Arc::clone(&original))
            .with_filter(blur(3.0, None), Some(Arc::clone(&original)))
            .unwrap();
        // Another filter (two blurs would make one entry), applied before the first one's pixels
        // were evaluated: its own input is not known.
        let high = FilterStep {
            filter: Filter::HighPass { radius: 2.0 },
            ..blur(2.0, None)
        };
        let twice = once.with_filter(high.clone(), None).unwrap();
        let rect = [100.0, 50.0, 200.0, 150.0];
        let job = twice.look_job(rect, 0).expect("a look at once");
        assert_eq!(job.steps.len(), 2, "both filters, on the part shown");
        let look = job.run().unwrap();
        let exact = twice.evaluate().unwrap();
        let [ox, oy] = look.origin;
        for (x, y) in [(100, 50), (149, 80), (150, 100), (199, 149)] {
            let (a, b) = (pixel(&look.image, x - ox, y - oy), pixel(&exact, x, y));
            assert!(
                a.iter().zip(&b).all(|(a, b)| a.abs_diff(*b) <= 1),
                "({x}, {y}): {a:?} {b:?}"
            );
        }
        assert!(
            twice.preview(150 * 130).unwrap().is_some(),
            "and a quick look"
        );
        // Once the first one's result is known, the look starts from it.
        let output = once.evaluate().unwrap();
        let known = once.with_filter(high, Some(output)).unwrap();
        assert_eq!(known.look_job(rect, 0).unwrap().steps.len(), 1);
    }

    #[test]
    fn a_look_asked_again_while_it_is_computed_is_computed_once() {
        // The display asks at every frame while a look is computed: the same part, once.
        let original = halves();
        let stack = LayerStack::new(Arc::clone(&original))
            .with_filter(blur(3.0, None), Some(original))
            .unwrap();
        let pixels = Pixels::pending(stack, None);
        let (go, wait) = std::sync::mpsc::channel::<()>();
        let wait = Mutex::new(wait);
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = Arc::clone(&calls);
        // Held until the display has asked again; then the CPU computes it.
        let gpu: LookFilter = Arc::new(move |_| {
            if counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                let _ = wait
                    .lock()
                    .unwrap()
                    .recv_timeout(std::time::Duration::from_secs(10));
            }
            None
        });
        let rect = [10.0, 10.0, 120.0, 90.0];
        assert!(pixels.look_for(rect, 0, Some(&gpu)).1);
        let start = std::time::Instant::now();
        while calls.load(std::sync::atomic::Ordering::SeqCst) == 0 && start.elapsed().as_secs() < 10
        {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        // The next frame, while it is computed.
        assert!(pixels.look_for(rect, 0, Some(&gpu)).1);
        go.send(()).unwrap();
        let start = std::time::Instant::now();
        while pixels.0.region.lock().unwrap().computing && start.elapsed().as_secs() < 10 {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let (look, again) = pixels.look_for(rect, 0, Some(&gpu));
        assert!(look.is_some() && !again);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn an_evaluation_not_wanted_any_more_is_given_up_and_left_pending() {
        let original = halves();
        let stack = LayerStack::new(Arc::clone(&original))
            .with_filter(blur(3.0, None), Some(Arc::clone(&original)))
            .unwrap()
            .with_effect(effect(Adjustment::Invert, None))
            .unwrap();
        let pixels = Pixels::pending(stack.clone(), None);
        // Wanted for the first few tiles only: replaced while it was evaluated.
        let asked = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = Arc::clone(&asked);
        let wanted: crate::raster::Wanted =
            Arc::new(move || counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst) < 2);
        assert!(pixels.get_while(wanted).is_none());
        assert!(pixels.ready_image().is_none(), "still pending");
        // Whoever needs them later gets them whole, and the filter's result was not kept half.
        assert_eq!(difference(&pixels.get(), &stack.evaluate().unwrap()), 0);
        // Wanted throughout: evaluated.
        let again = Pixels::pending(stack.clone(), None);
        let image = again.get_while(Arc::new(|| true)).unwrap();
        assert!(Arc::ptr_eq(&image, again.ready_image().unwrap()));
    }

    #[test]
    fn a_state_replaced_before_it_is_evaluated_is_dropped_not_kept_by_the_next() {
        // A slider dragged over an entry: every setting replaces the state before at once.
        let original = halves();
        let plain = LayerStack::new(Arc::clone(&original));
        let once = plain.with_effect(effect(Adjustment::Invert, None)).unwrap();
        let first = Pixels::pending(once.clone(), Some((Pixels::ready(original), plain)));
        let held = Arc::downgrade(&first.0);
        let twice = once.with_effect(effect(Adjustment::Invert, None)).unwrap();
        let second = Pixels::pending(twice.clone(), Some((first, once)));
        // Nobody holds the first any more: its background evaluation is skipped, and a long
        // drag leaves no chain of states behind.
        assert!(held.upgrade().is_none());
        let thrice = twice.with_effect(effect(Adjustment::Invert, None)).unwrap();
        let third = Pixels::pending(thrice.clone(), Some((second, twice)));
        // Evaluated from the last state evaluated (the original's): the same pixels as whole.
        assert_eq!(difference(&third.get(), &thrice.evaluate().unwrap()), 0);
    }

    #[test]
    fn a_state_evaluated_meanwhile_is_where_the_next_one_starts_from() {
        let original = halves();
        let plain = LayerStack::new(Arc::clone(&original));
        let inside = selection(|x, _| x < 40);
        let once = plain
            .with_effect(effect(Adjustment::Invert, Some(inside.clone())))
            .unwrap();
        let first = Pixels::pending(once.clone(), Some((Pixels::ready(original), plain)));
        let twice = once
            .with_effect(effect(Adjustment::Invert, Some(inside)))
            .unwrap();
        let second = Pixels::pending(twice.clone(), Some((first.clone(), once)));
        // Held elsewhere (the document) and evaluated after the next state was made.
        let evaluated = first.get();
        let result = second.get();
        assert_eq!(difference(&result, &twice.evaluate().unwrap()), 0);
        // Only the tiles under the selection were evaluated again: the others are the first's.
        assert!(Arc::ptr_eq(
            tile_of(&result, 1, 0),
            tile_of(&evaluated, 1, 0)
        ));
    }

    #[test]
    fn a_layer_without_a_filter_is_evaluated_in_the_background_once_its_state_lasts() {
        let stack = LayerStack::new(halves())
            .with_effect(effect(Adjustment::Invert, None))
            .unwrap();
        let pixels = Pixels::pending(stack, None);
        pixels.evaluate_in_background();
        assert!(
            pixels.ready_image().is_none(),
            "not at once: the state may be replaced"
        );
        let start = std::time::Instant::now();
        while pixels.ready_image().is_none() && start.elapsed().as_secs() < 10 {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(pixels.ready_image().is_some());
    }

    #[test]
    fn a_blur_softens_an_edge_and_stays_opaque_within_the_layer() {
        let original = halves();
        let stack = LayerStack::new(Arc::clone(&original))
            .with_filter(blur(4.0, None), None)
            .unwrap();
        let shown = stack.evaluate().unwrap();
        // Far from the edge (and at the layer's edges, which repeat), unchanged.
        for (x, y) in [(0, 0), (100, 10), (299, 259), (200, 130)] {
            assert_eq!(pixel(&shown, x, y), pixel(&original, x, y), "({x}, {y})");
        }
        // Across it, a ramp: about half at the edge, rising, opaque.
        let ramp: Vec<u8> = (144..156).map(|x| pixel(&shown, x, 50)[0]).collect();
        assert!(ramp.windows(2).all(|w| w[0] <= w[1]), "{ramp:?}");
        assert!(ramp[0] > 0 && ramp[11] < 255, "{ramp:?}");
        assert!(pixel(&shown, 149, 50)[0].abs_diff(128) < 30);
        assert!((140..160).all(|x| pixel(&shown, x, 50)[3] == 255));
    }

    #[test]
    fn unsharp_mask_and_high_pass_work_on_the_layer_around_its_edges() {
        let original = halves();
        let step = |filter| FilterStep {
            filter,
            ..blur(1.0, None)
        };
        let sharpen = Filter::UnsharpMask {
            amount: 100.0,
            radius: 2.0,
            threshold: 0.0,
        };
        let sharp = LayerStack::new(Arc::clone(&original))
            .with_filter(step(sharpen), None)
            .unwrap()
            .evaluate()
            .unwrap();
        // Black and white already: sharpening clips, and leaves what is far from the edge.
        for (x, y) in [
            (0, 0),
            (140, 50),
            (149, 50),
            (150, 50),
            (160, 50),
            (299, 259),
        ] {
            assert_eq!(pixel(&sharp, x, y), pixel(&original, x, y), "({x}, {y})");
        }
        let high = LayerStack::new(Arc::clone(&original))
            .with_filter(step(Filter::HighPass { radius: 2.0 }), None)
            .unwrap()
            .evaluate()
            .unwrap();
        // Middle gray far from the edge, darker just before it, lighter just after; opaque.
        for (x, y) in [(0, 0), (100, 10), (200, 130), (299, 259)] {
            assert!(pixel(&high, x, y)[0].abs_diff(128) <= 1, "({x}, {y})");
        }
        assert!(pixel(&high, 148, 50)[0] < 100);
        assert!(pixel(&high, 151, 50)[0] > 156);
        assert!((140..160).all(|x| pixel(&high, x, 50)[3] == 255));
    }

    #[test]
    fn a_motion_blur_softens_edges_across_its_line_only() {
        // A vertical edge: blurred along the rows, it softens; along the columns, it stays.
        let original = halves();
        let motion = |angle, distance| {
            let filter = Filter::MotionBlur { angle, distance };
            LayerStack::new(Arc::clone(&original))
                .with_filter(
                    FilterStep {
                        filter,
                        ..blur(1.0, None)
                    },
                    None,
                )
                .unwrap()
                .evaluate()
                .unwrap()
        };
        let along = motion(90.0, 40.0);
        assert_eq!(difference(&along, &original), 0);
        // Long enough to be sampled on the layer reduced (4 times): softened across by a few
        // pixels only, along the edge.
        let long = motion(-90.0, 600.0);
        for x in (0..144).chain(156..300) {
            for y in [0, 130, 259] {
                assert_eq!(pixel(&long, x, y), pixel(&original, x, y), "({x}, {y})");
            }
        }
        let across = motion(0.0, 20.0);
        let ramp: Vec<u8> = (138..162).map(|x| pixel(&across, x, 50)[0]).collect();
        assert!(ramp.windows(2).all(|w| w[0] <= w[1]), "{ramp:?}");
        assert!(ramp[2] > 0 && ramp[21] < 255, "{ramp:?}");
        // The same on every row, and untouched beyond the line's reach.
        for y in [0, 120, 259] {
            assert_eq!(pixel(&across, 145, y), pixel(&across, 145, 50));
        }
        assert_eq!(pixel(&across, 120, 50), pixel(&original, 120, 50));
    }

    #[test]
    fn dust_and_scratches_takes_specks_out_of_a_layer_above_its_threshold() {
        // A gray speck of 2 × 2 on black, by the edge with white.
        let original = image(PixelFormat::RGBA8_SRGB, |x, y| {
            let v = if (40..42).contains(&x) && (60..62).contains(&y) {
                100
            } else if x < 150 {
                0
            } else {
                255
            };
            vec![v, v, v, 255]
        });
        let cleaned = |radius, threshold| {
            let filter = Filter::DustAndScratches { radius, threshold };
            LayerStack::new(Arc::clone(&original))
                .with_filter(
                    FilterStep {
                        filter,
                        ..blur(1.0, None)
                    },
                    None,
                )
                .unwrap()
                .evaluate()
                .unwrap()
        };
        let gone = cleaned(2.0, 0.0);
        assert_eq!(pixel(&gone, 40, 60), [0, 0, 0, 255]);
        // The edge stays where it was.
        assert_eq!(pixel(&gone, 149, 60), [0, 0, 0, 255]);
        assert_eq!(pixel(&gone, 150, 60), [255, 255, 255, 255]);
        // A threshold above the speck's difference keeps it.
        assert_eq!(pixel(&cleaned(2.0, 120.0), 40, 60), [100, 100, 100, 255]);
    }

    #[test]
    fn texture_and_clarity_work_on_a_layer_with_their_two_blurs() {
        let original = halves();
        let filtered = |texture, clarity| {
            let filter = Filter::ClarityTexture {
                texture,
                clarity,
                scale: 1.0,
            };
            LayerStack::new(Arc::clone(&original))
                .with_filter(
                    FilterStep {
                        filter,
                        ..blur(1.0, None)
                    },
                    None,
                )
                .unwrap()
                .evaluate()
                .unwrap()
        };
        // Texture at -100 is the fine blur: the edge softens over a few pixels only.
        let smooth = filtered(-100.0, 0.0);
        let fine = LayerStack::new(Arc::clone(&original))
            .with_filter(blur(3.0, None), None)
            .unwrap()
            .evaluate()
            .unwrap();
        assert!(difference(&smooth, &fine) <= 1);
        // Black and white have no midtones: Clarity leaves them, at any setting.
        assert_eq!(difference(&filtered(0.0, 100.0), &original), 0);
        assert_eq!(difference(&filtered(0.0, -100.0), &original), 0);
    }

    #[test]
    fn rows_are_read_as_pixels_are_the_edges_repeating() {
        let original = gradient(true);
        let reader = PremulPixels::new(&original, BlendSpace::Perceptual);
        // Across both edges and the tiles between them, above and below the layer.
        for y in [-3i64, 0, 130, 259, 300] {
            let mut row = vec![[0.0f32; 4]; 340];
            reader.row(-20, y, &mut row);
            for (i, px) in row.iter().enumerate() {
                assert_eq!(*px, reader.at(i as i64 - 20, y), "({}, {y})", i as i64 - 20);
            }
        }
    }

    #[test]
    fn a_large_blur_made_on_the_reduced_layer_is_close_to_the_direct_one() {
        let original = halves();
        let (w, h) = (W as usize, H as usize);
        for radius in [70.0, 150.0] {
            let out = filtered(&blur(radius, None), &original, true).unwrap();
            let reader = PremulPixels::new(&original, BlendSpace::Perceptual);
            let buffer: Vec<[f32; 4]> = (0..w * h)
                .map(|i| reader.at((i % w) as i64, (i / w) as i64))
                .collect();
            let direct = crate::filter::Blur::gaussian(f64::from(radius)).image(buffer, w, h);
            let shown = PremulPixels::new(&out, BlendSpace::Perceptual);
            let mut worst = 0.0f32;
            for y in 0..h {
                for x in 0..w {
                    let (a, b) = (shown.at(x as i64, y as i64), direct[y * w + x]);
                    for c in 0..4 {
                        worst = worst.max((a[c] - b[c]).abs());
                    }
                }
            }
            assert!(worst < 0.02, "radius {radius}: {worst}");
        }
    }

    #[test]
    fn a_blur_applies_within_its_selection_only() {
        let original = halves();
        let stack = LayerStack::new(Arc::clone(&original))
            .with_filter(blur(4.0, Some(selection(|_, y| y < 100))), None)
            .unwrap();
        let shown = stack.evaluate().unwrap();
        assert_ne!(pixel(&shown, 149, 50), pixel(&original, 149, 50));
        for x in 140..160 {
            assert_eq!(pixel(&shown, x, 150), pixel(&original, x, 150));
        }
    }

    #[test]
    fn paint_above_a_blur_is_laid_on_its_result_and_follows_its_edits() {
        let original = halves();
        let paint = painted(&empty(&original), gray(1.0), |x, _| {
            if (100..200).contains(&x) { 0.5 } else { 0.0 }
        });
        let stack = LayerStack::new(Arc::clone(&original))
            .with_filter(blur(4.0, None), None)
            .unwrap()
            .with_top_paint(Arc::clone(&paint))
            .unwrap();
        let blurred = LayerStack::new(Arc::clone(&original))
            .with_filter(blur(4.0, None), None)
            .unwrap()
            .evaluate()
            .unwrap();
        let direct = LayerStack::new(Arc::clone(&blurred))
            .with_top_paint(Arc::clone(&paint))
            .unwrap()
            .evaluate()
            .unwrap();
        let shown = stack.evaluate().unwrap();
        assert_eq!(difference(&shown, &direct), 0);

        // The radius edited: as a stack made with it, from what the filter knew it applied to.
        let edited = stack
            .with_steps(
                0,
                &[Operation::Filter(Filter::GaussianBlur { radius: 9.0 })],
            )
            .unwrap();
        let Entry::Filter(entry) = &edited.entries()[0] else {
            panic!("a filter entry");
        };
        assert!(entry.known_input().is_some());
        let made = LayerStack::new(Arc::clone(&original))
            .with_filter(blur(9.0, None), None)
            .unwrap()
            .with_top_paint(paint)
            .unwrap();
        assert_eq!(
            difference(
                &edited.reevaluate(&stack, &shown).unwrap(),
                &made.evaluate().unwrap()
            ),
            0
        );
        // Hidden: as if it were not there.
        let hidden = edited.with_hidden(0, true).unwrap();
        assert!(!hidden.has_shown_filter());
        let without = LayerStack::new(Arc::clone(&original))
            .with_top_paint(match &stack.entries()[1] {
                Entry::Paint(p) => Arc::clone(p),
                _ => panic!("a paint entry"),
            })
            .unwrap();
        assert_eq!(
            difference(&hidden.evaluate().unwrap(), &without.evaluate().unwrap()),
            0
        );
    }

    #[test]
    fn adding_on_top_of_a_blur_starts_from_its_result() {
        let original = halves();
        let stack = LayerStack::new(Arc::clone(&original))
            .with_filter(blur(3.0, None), None)
            .unwrap();
        let shown = stack.evaluate().unwrap();
        let inverted = stack
            .with_effect(effect(Adjustment::Invert, Some(selection(|x, _| x > 140))))
            .unwrap();
        assert_eq!(
            difference(
                &inverted.reevaluate(&stack, &shown).unwrap(),
                &inverted.evaluate().unwrap()
            ),
            0
        );
        // Applied over what the layer shows: the filter knows its input.
        let again = inverted
            .with_filter(blur(2.0, None), Some(inverted.evaluate().unwrap()))
            .unwrap();
        let Entry::Filter(top) = again.entries().last().unwrap() else {
            panic!("a filter entry");
        };
        assert!(top.known_input().is_some());
        // Two blurs in a row are one entry, one blur.
        let twice = stack.with_filter(blur(2.0, None), None).unwrap();
        assert_eq!(twice.entries().len(), 1);
        let Entry::Filter(both) = &twice.entries()[0] else {
            panic!("a filter entry");
        };
        assert_eq!(both.steps().len(), 1);
    }

    #[test]
    fn a_stroke_over_a_blur_shows_what_its_stack_evaluates_to() {
        let original = halves();
        let stack = LayerStack::new(Arc::clone(&original))
            .with_filter(blur(5.0, None), None)
            .unwrap();
        let mut shown = stack.evaluate().unwrap();
        let mut top = TopPaint::new(&stack, &shown, BlendSpace::Perceptual, false);
        let t = T as usize;
        let dirty = [(TileCoord { col: 0, row: 0 }, [0, 0, t, t])];
        let tiles = top.lay(
            &dirty,
            gray(0.5),
            |_, x, _| if x > 140 { 0.4 } else { 0.0 },
            &shown,
            None,
        );
        shown = Arc::new(shown.with_tiles(tiles).unwrap());
        let result = top.stack().unwrap();
        assert_eq!(result.entries().len(), 2);
        assert_eq!(difference(&shown, &result.evaluate().unwrap()), 0);
    }

    #[test]
    fn a_preview_filters_a_coarse_level_of_what_the_filter_applies_to() {
        let original = halves();
        let plain = LayerStack::new(Arc::clone(&original));
        let stack = plain
            .with_filter(blur(8.0, None), Some(Arc::clone(&original)))
            .unwrap()
            .with_effect(effect(Adjustment::Invert, None))
            .unwrap();
        // 300 × 260: level 1 is 150 × 130.
        let preview = stack.preview(150 * 130).unwrap().unwrap();
        assert_eq!(preview.factor, 2);
        assert_eq!(preview.above, stack.entries()[1..]);
        assert_eq!(preview.image.size(), Size::new(150, 130));
        // As the filter at half the radius on the coarse level.
        let coarse = &original.levels()[1];
        let input = Arc::new(
            RasterImage::from_level0_tiles(
                coarse.size(),
                original.format(),
                coarse.tiles().to_vec(),
            )
            .unwrap(),
        );
        let direct = LayerStack::new(input)
            .with_filter(blur(4.0, None), None)
            .unwrap()
            .evaluate()
            .unwrap();
        let shown = PremulPixels::new(&preview.image, BlendSpace::Perceptual);
        let expected = PremulPixels::new(&direct, BlendSpace::Perceptual);
        for (x, y) in [(0, 0), (74, 60), (75, 60), (149, 129)] {
            assert_eq!(shown.at(x, y), expected.at(x, y), "({x}, {y})");
        }
        // Small enough whole, or nothing known below: no preview.
        assert!(stack.preview(u64::MAX).unwrap().is_none());
        let unknown = plain.with_filter(blur(8.0, None), None).unwrap();
        assert!(unknown.preview(150 * 130).unwrap().is_none());
    }

    #[test]
    fn a_look_at_part_of_a_layer_is_the_filter_there() {
        let original = halves();
        let plain = LayerStack::new(Arc::clone(&original));
        let blurred = plain
            .with_filter(blur(4.0, None), Some(Arc::clone(&original)))
            .unwrap();
        // The middle of the layer at full size: the tiles around it, with the filter's margin.
        let look = blurred
            .look_at([140.0, 100.0, 160.0, 120.0], 0)
            .unwrap()
            .unwrap();
        assert_eq!(look.factor, 1);
        assert!(look.covers([140.0, 100.0, 160.0, 120.0], 0));
        assert!(!look.covers([140.0, 100.0, 160.0, 120.0], 1) || look.factor <= 2);
        let whole = blurred.evaluate().unwrap();
        let part = PremulPixels::new(&look.image, BlendSpace::Perceptual);
        let exact = PremulPixels::new(&whole, BlendSpace::Perceptual);
        let [ox, oy] = look.origin.map(i64::from);
        for (x, y) in [(140, 100), (149, 110), (150, 110), (159, 119)] {
            assert_eq!(part.at(x - ox, y - oy), exact.at(x, y), "({x}, {y})");
        }
        // Half size: half as many pixels, a factor of 2.
        let half = blurred
            .look_at([0.0, 0.0, 300.0, 260.0], 1)
            .unwrap()
            .unwrap();
        assert_eq!((half.factor, half.image.size()), (2, Size::new(150, 130)));
        // Outside the layer: nothing.
        assert!(
            blurred
                .look_at([400.0, 400.0, 500.0, 500.0], 0)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_filter_being_applied_has_a_quick_look_for_thumbnails() {
        let original = halves();
        let plain = LayerStack::new(Arc::clone(&original));
        let blurred = plain
            .with_filter(blur(4.0, None), Some(Arc::clone(&original)))
            .unwrap();
        let pixels = Pixels::pending(blurred.clone(), None);
        let look = pixels.quick_look(150 * 130).unwrap();
        assert_eq!(look.size(), Size::new(150, 130));
        assert!(pixels.ready_image().is_none(), "nothing evaluated whole");
        // Paint above the filter: only the pixels tell.
        let paint = painted(
            &empty(&original),
            gray(1.0),
            |x, _| if x < 9 { 1.0 } else { 0.0 },
        );
        let painted_pixels = Pixels::pending(blurred.with_top_paint(paint).unwrap(), None);
        assert!(painted_pixels.quick_look(150 * 130).is_none());
        // Evaluated: the pixels themselves.
        let image = pixels.get();
        assert!(Arc::ptr_eq(&pixels.quick_look(150 * 130).unwrap(), &image));
    }

    #[test]
    fn the_look_shown_stands_until_the_next_one_is_computed() {
        let original = halves();
        let plain = LayerStack::new(Arc::clone(&original));
        let first_stack = plain
            .with_filter(blur(3.0, None), Some(Arc::clone(&original)))
            .unwrap();
        let first = Pixels::pending(first_stack.clone(), None);
        let rect = [100.0, 50.0, 200.0, 150.0];
        let look = Arc::new(first_stack.look_at(rect, 0).unwrap().unwrap());
        first.0.region.lock().unwrap().look = Some(Arc::clone(&look));
        // A setting changed: back below the filter, then the filter again at another radius.
        let back = Pixels::pending(plain.clone(), Some((first, first_stack)));
        let second_stack = plain
            .with_filter(blur(7.0, None), Some(Arc::clone(&original)))
            .unwrap();
        let second = Pixels::pending(second_stack, Some((back, plain)));
        // Shown meanwhile: the look the first showed, not the layer unfiltered.
        let (shown, again) = second.look_for(rect, 0, None);
        assert!(again);
        assert!(Arc::ptr_eq(&shown.unwrap(), &look));
        // Elsewhere, or at another level, it does not stand.
        let (other, _) = second.look_for([0.0, 0.0, 40.0, 40.0], 1, None);
        assert!(other.is_none_or(|o| !Arc::ptr_eq(&o, &look)));
    }

    #[test]
    fn going_back_below_a_filter_needs_no_evaluation() {
        let original = halves();
        let painted_stack = LayerStack::new(Arc::clone(&original))
            .with_effect(effect(Adjustment::Invert, None))
            .unwrap();
        let shown = painted_stack.evaluate().unwrap();
        let blurred = painted_stack
            .with_filter(blur(3.0, None), Some(Arc::clone(&shown)))
            .unwrap();
        let pixels = Pixels::pending(blurred.clone(), None);
        // A preview replaced: back to the stack the filter applied to, then the filter again,
        // which knows its input from what the layer shows.
        let back = Pixels::pending(painted_stack.clone(), Some((pixels, blurred)));
        assert!(Arc::ptr_eq(back.ready_image().unwrap(), &shown));
        let again = painted_stack
            .with_filter(blur(5.0, None), back.ready_image().cloned())
            .unwrap();
        assert!(again.preview(150 * 130).unwrap().is_some());
    }

    #[test]
    fn an_earlier_preview_stands_while_the_next_is_computed() {
        let original = halves();
        let plain = LayerStack::new(Arc::clone(&original));
        let shown = Pixels::ready(Arc::clone(&original));
        let first_stack = plain
            .with_filter(blur(3.0, None), Some(Arc::clone(&original)))
            .unwrap();
        let first = Pixels::pending(first_stack.clone(), Some((shown, plain.clone())));
        first
            .0
            .preview
            .set(first_stack.preview(150 * 130).unwrap().unwrap())
            .unwrap();
        // Replaced: back to the stack without the filter, then another radius.
        let back = Pixels::pending(plain.clone(), Some((first.clone(), first_stack)));
        let again = plain
            .with_filter(blur(6.0, None), Some(Arc::clone(&original)))
            .unwrap();
        let second = Pixels::pending(again.clone(), Some((back, plain.clone())));
        assert!(Arc::ptr_eq(
            &second.preview().unwrap().image,
            &first.preview().unwrap().image
        ));
        // Paint added above the filter: the earlier look no longer stands.
        let paint = painted(
            &empty(&original),
            gray(1.0),
            |x, _| if x < 9 { 1.0 } else { 0.0 },
        );
        let painted_stack = again.with_top_paint(paint).unwrap();
        let third = Pixels::pending(painted_stack, Some((second, again)));
        assert!(third.preview().is_none());
    }

    #[test]
    fn what_a_layer_showed_stands_in_while_a_filter_is_evaluated() {
        let original = halves();
        let plain = LayerStack::new(Arc::clone(&original));
        let shown = Pixels::ready(Arc::clone(&original));
        let blurred = plain.with_filter(blur(3.0, None), None).unwrap();
        let first = Pixels::pending(blurred.clone(), Some((shown.clone(), plain.clone())));
        assert!(Arc::ptr_eq(first.meanwhile().unwrap(), &original));
        let first_image = first.get();
        // A preview replaced: back to the stack without the filter, then another radius on it.
        let back = Pixels::pending(plain.clone(), Some((first.clone(), blurred.clone())));
        let again = plain.with_filter(blur(6.0, None), None).unwrap();
        let second = Pixels::pending(again, Some((back.clone(), plain.clone())));
        assert!(Arc::ptr_eq(second.shown().unwrap(), &first_image));
        // Cancelled, then another filter (the maintainer's report: the cancelled one flashed;
        // it did once the blur had been evaluated): the layer as it is below, not the blur.
        let high = plain
            .with_filter(
                FilterStep {
                    filter: Filter::HighPass { radius: 4.0 },
                    ..blur(1.0, None)
                },
                None,
            )
            .unwrap();
        let other = Pixels::pending(high, Some((back.clone(), plain.clone())));
        assert!(Arc::ptr_eq(other.shown().unwrap(), &original));
        // No filter on either side: nothing kept.
        let painted = plain.with_effect(effect(Adjustment::Invert, None)).unwrap();
        assert!(
            Pixels::pending(painted, Some((shown, plain)))
                .meanwhile()
                .is_none()
        );
    }

    #[test]
    fn a_grown_stack_shows_the_same_pixels_moved() {
        let original = gradient(true);
        let paint = painted(&empty(&original), gray(1.0), |x, _| {
            if x > 280 { 0.6 } else { 0.0 }
        });
        let stack = LayerStack::new(Arc::clone(&original))
            .with_top_paint(paint)
            .unwrap()
            .with_effect(effect(Adjustment::Invert, Some(selection(|x, _| x > 150))))
            .unwrap();
        let shown = stack.evaluate().unwrap();
        let grown = stack.grown((1, 0), Size::new(W + 2 * T, H)).unwrap();
        let moved = grown.evaluate().unwrap();
        for y in [0, 100, H - 1] {
            for x in [0, 149, 151, 290, W - 1] {
                assert_eq!(pixel(&moved, x + T, y), pixel(&shown, x, y), "({x}, {y})");
            }
            // Transparent around, the paint's padding included.
            assert_eq!(pixel(&moved, 10, y)[3], 0);
            assert_eq!(pixel(&moved, W + T + 5, y)[3], 0);
        }
    }

    #[test]
    fn moved_pixels_are_baked_into_the_top_paint() {
        let original = gradient(true);
        let paint = painted(
            &empty(&original),
            gray(0.5),
            |x, _| {
                if x < 50 { 0.5 } else { 0.0 }
            },
        );
        let stack = LayerStack::new(Arc::clone(&original))
            .with_top_paint(paint)
            .unwrap();
        let shown = stack.evaluate().unwrap();
        let after = image(PixelFormat::RGBA8_SRGB, |x, y| {
            let mut px = pixel(&shown, x, y);
            if (100..120).contains(&x) {
                px = vec![1, 2, 3, 128];
            }
            px
        });
        let moved = stack
            .with_painted(&shown, &after, BlendSpace::Perceptual)
            .unwrap();
        assert_eq!(moved.entries().len(), 1, "it continues the top paint");
        assert!(difference(&moved.evaluate().unwrap(), &after) <= 1);
    }

    #[test]
    fn the_restore_eraser_reaches_every_paint_and_keeps_the_effects() {
        let original = gradient(true);
        let below = painted(&empty(&original), gray(0.9), |x, _| {
            if x < 200 { 1.0 } else { 0.0 }
        });
        let above = painted(&empty(&original), gray(0.1), |_, y| {
            if y < 100 { 0.5 } else { 0.0 }
        });
        let stack = LayerStack::new(Arc::clone(&original))
            .with_top_paint(below)
            .unwrap()
            .with_effect(effect(Adjustment::Invert, None))
            .unwrap()
            .with_top_paint(above)
            .unwrap();
        let mut restore = RestorePaint::new(&stack);
        let t = T as usize;
        let dirty: Vec<(TileCoord, [usize; 4])> =
            grid_coords(size()).map(|c| (c, [0, 0, t, t])).collect();
        // Everywhere at once: the original, inverted.
        let shown = restore.lay(&dirty, |_, _, _| 1.0);
        let result = restore.stack().unwrap();
        let [Entry::Effect(_)] = result.entries() else {
            panic!("only the effect is left");
        };
        let evaluated = result.evaluate().unwrap();
        for (coord, tile) in &shown {
            assert_eq!(**tile, **tile_of(&evaluated, coord.col, coord.row));
        }
        assert_eq!(pixel(&evaluated, 10, 10)[0], 255 - 10);
    }

    #[test]
    fn paint_of_another_layer_is_refused() {
        let original = gradient(true);
        let other = PaintEntry::empty(original.format(), Size::new(10, 10), BlendSpace::Perceptual);
        assert_eq!(
            LayerStack::new(original)
                .with_top_paint(Arc::new(other))
                .unwrap_err(),
            StackError::SizeMismatch
        );
    }

    /// A field with a stroke across the middle of the layer, moving the pixels right.
    fn pushed() -> Arc<Field> {
        use crate::liquify::{Brush, Stroke, Tool};
        let mut field = Field::new(size());
        let brush = Brush {
            size: 120.0,
            density: 100.0,
            pressure: 100.0,
            rate: 100.0,
        };
        let mut stroke = Stroke::new(Tool::ForwardWarp, brush);
        stroke.move_to(&mut field, [100.0, 130.0]);
        stroke.move_to(&mut field, [190.0, 130.0]);
        stroke.finish(&mut field);
        Arc::new(field)
    }

    fn liquified(original: &Arc<RasterImage>) -> LayerStack {
        LayerStack::new(Arc::clone(original))
            .with_liquify(pushed(), BlendSpace::Perceptual, Some(Arc::clone(original)))
            .unwrap()
    }

    #[test]
    fn a_liquify_entry_warps_what_is_below_it_and_what_is_above_follows() {
        let original = gradient(true);
        let stack = liquified(&original);
        assert!(stack.has_shown_filter());
        let warped = stack.evaluate().unwrap();
        // Where the stroke moved pixels they differ, elsewhere they are the original's own.
        assert_ne!(pixel(&warped, 150, 130), pixel(&original, 150, 130));
        assert_eq!(pixel(&warped, 5, 5), pixel(&original, 5, 5));
        assert_eq!(pixel(&warped, 295, 255), pixel(&original, 295, 255));
        // Paint above it lands on the warped pixels.
        let paint = painted(
            &empty(&original),
            gray(1.0),
            |x, _| {
                if x < 9 { 1.0 } else { 0.0 }
            },
        );
        let over = stack.with_top_paint(paint).unwrap();
        let result = over.evaluate().unwrap();
        assert_eq!(pixel(&result, 150, 130), pixel(&warped, 150, 130));
        assert_eq!(pixel(&result, 3, 3), vec![255, 255, 255, 255]);
        // And an effect above it is applied over the warp.
        let inverted = stack
            .with_effect(effect(Adjustment::Invert, None))
            .unwrap()
            .evaluate()
            .unwrap();
        let w = pixel(&warped, 150, 130);
        assert_eq!(
            pixel(&inverted, 150, 130),
            vec![255 - w[0], 255 - w[1], 255 - w[2], 255]
        );
    }

    #[test]
    fn a_liquify_entry_is_edited_hidden_and_keeps_what_it_knows() {
        let original = gradient(true);
        let stack = liquified(&original);
        let Entry::Liquify(entry) = &stack.entries()[0] else {
            panic!("a liquify entry");
        };
        assert!(entry.known_input().is_some());
        // Hidden: kept, not shown.
        let hidden = stack.with_hidden(0, true).unwrap();
        assert!(!hidden.has_shown_filter() && hidden.entries()[0].hidden());
        assert_eq!(difference(&hidden.evaluate().unwrap(), &original), 0);
        // Another field in its place: the eye and what it was applied to stay.
        let other = hidden
            .with_field(0, Arc::new(Field::new(size())), None)
            .unwrap();
        let Entry::Liquify(edited) = &other.entries()[0] else {
            panic!("a liquify entry");
        };
        assert!(edited.hidden());
        assert!(Arc::ptr_eq(&edited.known_input().unwrap().1, &original));
        // Only a liquify entry takes a field; and only one of the layer's size.
        assert!(matches!(
            LayerStack::new(Arc::clone(&original))
                .with_effect(effect(Adjustment::Invert, None))
                .unwrap()
                .with_field(0, pushed(), None),
            Err(StackError::InvalidEffect)
        ));
        assert_eq!(
            stack
                .with_field(0, Arc::new(Field::new(Size::new(10, 10))), None)
                .unwrap_err(),
            StackError::SizeMismatch
        );
        assert_eq!(
            stack.with_field(3, pushed(), None).unwrap_err(),
            StackError::IndexOutOfRange(3)
        );
        // Deleted: the layer is as it was.
        assert!(stack.without(0).unwrap().is_empty());
    }

    #[test]
    fn a_liquify_look_at_level_zero_is_the_whole_evaluation_there() {
        let original = gradient(true);
        let stack = liquified(&original);
        let whole = stack.evaluate().unwrap();
        let rect = [90.0, 100.0, 210.0, 160.0];
        let look = stack.look_at(rect, 0).unwrap().unwrap();
        assert_eq!(look.factor, 1);
        assert_eq!(look.filter, "liquify");
        assert!(look.covers(rect, 0));
        let [ox, oy] = look.origin.map(i64::from);
        let part = PremulPixels::new(&look.image, BlendSpace::Perceptual);
        let exact = PremulPixels::new(&whole, BlendSpace::Perceptual);
        for (x, y) in [(90, 100), (150, 130), (200, 150), (209, 159)] {
            assert_eq!(part.at(x - ox, y - oy), exact.at(x, y), "({x}, {y})");
        }
        // Outside the layer: nothing.
        assert!(
            stack
                .look_at([400.0, 400.0, 500.0, 500.0], 0)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_liquify_look_at_a_coarse_level_agrees_within_the_averaging() {
        let original = gradient(true);
        let stack = liquified(&original);
        let whole = stack.evaluate().unwrap();
        let look = stack.look_at([0.0, 0.0, 300.0, 260.0], 1).unwrap().unwrap();
        assert_eq!((look.factor, look.image.size()), (2, Size::new(150, 130)));
        for (x, y) in [(40u32, 40u32), (75, 65), (100, 70)] {
            let a = pixel(&look.image, x, y);
            let b = pixel(&whole, x * 2, y * 2);
            for c in 0..3 {
                assert!(a[c].abs_diff(b[c]) <= 8, "({x}, {y}): {a:?} vs {b:?}");
            }
        }
    }

    #[test]
    fn a_liquify_preview_stands_in_for_the_whole_layer() {
        let original = gradient(true);
        let plain = LayerStack::new(Arc::clone(&original));
        let known = || {
            plain
                .with_liquify(
                    pushed(),
                    BlendSpace::Perceptual,
                    Some(Arc::clone(&original)),
                )
                .unwrap()
        };
        let stack = known()
            .with_effect(effect(Adjustment::Invert, None))
            .unwrap();
        let preview = stack.preview(150 * 130).unwrap().unwrap();
        assert_eq!((preview.factor, preview.filter), (2, "liquify"));
        assert_eq!(preview.image.size(), Size::new(150, 130));
        assert_eq!(preview.above, stack.entries()[1..]);
        // Small enough whole, or nothing known below: none.
        assert!(stack.preview(u64::MAX).unwrap().is_none());
        let unknown = plain
            .with_liquify(pushed(), BlendSpace::Perceptual, None)
            .unwrap();
        assert!(unknown.preview(150 * 130).unwrap().is_none());
        // Thumbnails take the quick look while a liquify is the top entry.
        let pixels = Pixels::pending(known(), None);
        assert_eq!(
            pixels.quick_look(150 * 130).unwrap().size(),
            Size::new(150, 130)
        );
        assert!(pixels.ready_image().is_none(), "nothing evaluated whole");
        assert_eq!(pixels.get().size(), size());
    }

    #[test]
    fn a_grown_layer_keeps_its_liquify_where_it_was() {
        let original = gradient(true);
        let stack = liquified(&original);
        let grown = stack.grown((1, 1), Size::new(3 * T, 3 * T + 40)).unwrap();
        let Entry::Liquify(entry) = &grown.entries()[0] else {
            panic!("a liquify entry");
        };
        let Entry::Liquify(before) = &stack.entries()[0] else {
            panic!("a liquify entry");
        };
        assert_eq!(
            entry
                .field()
                .displacement_at([150.0 + f64::from(T), 130.0 + f64::from(T)]),
            before.field().displacement_at([150.0, 130.0])
        );
        assert_eq!(entry.field().size(), Size::new(3 * T, 3 * T + 40));
        let result = grown.evaluate().unwrap();
        assert_eq!(result.size(), Size::new(3 * T, 3 * T + 40));
    }

    #[test]
    fn what_a_liquify_is_applied_to_is_what_is_below_it() {
        let original = gradient(true);
        let plain = LayerStack::new(Arc::clone(&original));
        let unknown = plain
            .with_liquify(pushed(), BlendSpace::Perceptual, None)
            .unwrap();
        let Entry::Liquify(entry) = &unknown.entries()[0] else {
            panic!("a liquify entry");
        };
        assert!(entry.known_input().is_none());
        assert!(Arc::ptr_eq(&unknown.evaluate_below(0).unwrap(), &original));
        // Above a paint: the paint's result.
        let paint = painted(
            &empty(&original),
            gray(1.0),
            |x, _| {
                if x < 9 { 1.0 } else { 0.0 }
            },
        );
        let stacked = plain
            .with_top_paint(paint)
            .unwrap()
            .with_liquify(pushed(), BlendSpace::Perceptual, None)
            .unwrap();
        let below = stacked.evaluate_below(1).unwrap();
        assert_eq!(pixel(&below, 3, 3), vec![255, 255, 255, 255]);
        assert_eq!(
            stacked.evaluate_below(5).unwrap_err(),
            StackError::IndexOutOfRange(5)
        );
        // An entry edited with what it is applied to learns it.
        let seeded = unknown
            .with_field(0, pushed(), Some(Arc::clone(&original)))
            .unwrap();
        let Entry::Liquify(entry) = &seeded.entries()[0] else {
            panic!("a liquify entry");
        };
        assert!(Arc::ptr_eq(&entry.known_input().unwrap().1, &original));
        assert!(seeded.look_at([0.0, 0.0, 50.0, 50.0], 0).unwrap().is_some());
    }
}
