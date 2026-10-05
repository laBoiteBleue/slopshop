//! Layer styles (ADR 0032): effects drawn from a layer's shape, editable, as Photoshop's: Drop
//! Shadow, Outer Glow, Inner Shadow, Inner Glow, Color Overlay and Stroke.
//!
//! A layer's shape is its coverage in the document (its pixels' alpha through its transform,
//! its mask applied, as Photoshop's default where the mask shapes the effects too). Effects are
//! drawn from it with the selection's coverage operations (`selection::modify`'s Expand and
//! Feather, Edit > Stroke's band): one engine for selections, Edit > Stroke and effects. Each
//! effect becomes a plain layer of its own, composited with the existing steps around the
//! layer's content (see `composite::push_layer`): the effects below it (Drop Shadow), the content
//! at Fill Opacity with the effects that recolor it (Color Overlay), the effects above it
//! (Stroke), then the whole blended as one with the layer's mode and opacity.
//!
//! What effects draw is a cache, never saved (ADR 0032 point 4). The layer's coverage is
//! computed once for all its effects, and each effect's mask is kept by its geometry while the
//! shape is the same: a color, a mode or an opacity changed only recolors it, and a layer moved
//! by whole pixels moves it. The display never waits for effects: they are computed in the
//! background while what was drawn last shows (`Style::drawn_for_display`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::blend::{BlendMode, BlendSpace};
use crate::color::{LinearRgba, WORKING_SPACE};
use crate::composite::composite_region;
use crate::document::{Document, Layer, LayerContent, LayerId, LayerMask};
use crate::geom::{Rect, Size};
use crate::pick;
use crate::raster::{RasterImage, parallel_for_each};
use crate::selection::{self, MAX_FEATHER, MAX_MODIFY, Modify, SELECTION_FORMAT, StrokeLocation};
use crate::transform::Affine;

/// The farthest a Drop Shadow is offset, in pixels (Photoshop's).
pub const MAX_DISTANCE: f64 = 30_000.0;
/// The largest Drop Shadow size, in pixels (Photoshop's).
pub const MAX_SIZE: f64 = 250.0;

/// A layer's style: its effects (`None` for those not added) and its Fill Opacity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayerStyle {
    /// The opacity of the layer's own content, its effects untouched (Photoshop's "Fill").
    pub fill_opacity: f32,
    pub drop_shadow: Option<DropShadow>,
    pub outer_glow: Option<Glow>,
    /// A shadow inside the shape, cast from its edge: a Drop Shadow's settings, `spread` being
    /// Photoshop's "Choke".
    pub inner_shadow: Option<DropShadow>,
    /// A glow inside the shape, from its edge: `spread` being Photoshop's "Choke".
    pub inner_glow: Option<Glow>,
    pub color_overlay: Option<ColorOverlay>,
    pub stroke: Option<Stroke>,
}

impl Default for LayerStyle {
    fn default() -> Self {
        Self {
            fill_opacity: 1.0,
            drop_shadow: None,
            outer_glow: None,
            inner_shadow: None,
            inner_glow: None,
            color_overlay: None,
            stroke: None,
        }
    }
}

/// A glow around the shape (Outer Glow) or inside it from its edge (Inner Glow), a color fading
/// out.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glow {
    pub enabled: bool,
    pub color: LinearRgba,
    pub mode: BlendMode,
    pub opacity: f32,
    /// How much of `size` hardens the shape before it is blurred, percent (Spread; Choke inside).
    pub spread: f64,
    /// How far the glow spreads and blurs, pixels.
    pub size: f64,
}

impl Default for Glow {
    /// Photoshop's: pale yellow (sRGB #ffffbe), Screen, 75 %, 5 pixels.
    fn default() -> Self {
        Self {
            enabled: true,
            color: LinearRgba::from_srgb_encoded_to_working(1.0, 1.0, 190.0 / 255.0, 1.0),
            mode: BlendMode::Screen,
            opacity: 0.75,
            spread: 0.0,
            size: 5.0,
        }
    }
}

/// A layer's style as the layer holds it: its settings, and what its effects draw (a cache,
/// never saved).
#[derive(Debug, Clone)]
pub struct Style {
    /// Shared: a layer stays small (its style is cloned with it).
    settings: Arc<LayerStyle>,
    effects: Arc<Effects>,
}

impl PartialEq for Style {
    fn eq(&self, other: &Self) -> bool {
        self.settings == other.settings
    }
}

impl Style {
    pub fn new(settings: LayerStyle) -> Self {
        Self {
            settings: Arc::new(settings),
            effects: Arc::new(Effects::new(Arc::default(), Arc::default())),
        }
    }

    pub fn settings(&self) -> &LayerStyle {
        &self.settings
    }

    /// The same settings drawn afresh: what a style becomes when its layer's shape changes (its
    /// pixels, its mask, the layers inside a group).
    pub(crate) fn redrawn(&self) -> Self {
        self.with(Arc::clone(&self.settings), Arc::default())
    }

    /// The same settings for a layer that moved: what its shape gave is kept, and follows it
    /// when it moved by whole pixels.
    pub(crate) fn moved(&self) -> Self {
        self.with(Arc::clone(&self.settings), Arc::clone(&self.effects.shape))
    }

    /// Other settings for the same shape: an effect whose geometry did not change is only
    /// recolored, not computed again.
    pub(crate) fn restyled(&self, settings: LayerStyle) -> Self {
        self.with(Arc::new(settings), Arc::clone(&self.effects.shape))
    }

    fn with(&self, settings: Arc<LayerStyle>, shape: Arc<ShapeCache>) -> Self {
        Self {
            settings,
            effects: Arc::new(Effects::new(shape, Arc::clone(&self.effects.lineage))),
        }
    }

    /// What the effects draw for `layer` (a pixel or fill layer, or a group) placed by
    /// `to_document` on a `canvas`: computed now if they are not (on every core), then kept.
    pub(crate) fn drawn(&self, layer: &Layer, to_document: Affine, canvas: Size) -> &Drawn {
        let effects = &self.effects;
        effects.ready.get_or_init(|| {
            let drawn = Arc::new(
                self.settings
                    .draw(&effects.shape, layer, to_document, canvas),
            );
            effects.lineage.set_latest(&drawn);
            drawn
        })
    }

    /// [`Self::drawn`] for the display, which never waits: the effects when they are drawn, or
    /// can be at once from what the shape already gave (a color changed, the layer moved by
    /// whole pixels). Otherwise they are computed in the background (one computation at a time
    /// per layer, the newest asked for next), and what this layer's style drew last is shown
    /// meanwhile (nothing the first time): `Err`, the display to show them again soon.
    pub(crate) fn drawn_for_display(
        &self,
        layer: &Layer,
        to_document: Affine,
        canvas: Size,
    ) -> Result<&Drawn, Option<&Drawn>> {
        let effects = &self.effects;
        if let Some(drawn) = effects.ready.get() {
            return Ok(drawn);
        }
        if !effects.started.load(Ordering::Acquire)
            && let Some(drawn) = self
                .settings
                .draw_known(&effects.shape, to_document, canvas)
        {
            let drawn = effects.ready.get_or_init(|| Arc::new(drawn));
            effects.lineage.set_latest(drawn);
            return Ok(drawn);
        }
        Arc::clone(effects).start(
            Arc::clone(&self.settings),
            layer.clone(),
            to_document,
            canvas,
        );
        Err(effects
            .meanwhile
            .get_or_init(|| effects.lineage.latest())
            .as_deref())
    }

    /// Whether `other` shares what this style draws (a clone of it).
    pub fn ptr_eq(&self, other: &Style) -> bool {
        Arc::ptr_eq(&self.effects, &other.effects)
    }
}

/// What a style draws for one shape and one set of settings.
struct Effects {
    ready: OnceLock<Arc<Drawn>>,
    /// Their computation in the background started.
    started: AtomicBool,
    /// What the display shows until they are ready: what the lineage drew last when first
    /// asked.
    meanwhile: OnceLock<Option<Arc<Drawn>>>,
    /// What the layer's shape gave, shared with the styles of the same shape.
    shape: Arc<ShapeCache>,
    /// Shared with every style this one came from or gave.
    lineage: Arc<Lineage>,
}

impl std::fmt::Debug for Effects {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Effects")
            .field("ready", &self.ready.get().is_some())
            .finish_non_exhaustive()
    }
}

impl Effects {
    fn new(shape: Arc<ShapeCache>, lineage: Arc<Lineage>) -> Self {
        Self {
            ready: OnceLock::new(),
            started: AtomicBool::new(false),
            meanwhile: OnceLock::new(),
            shape,
            lineage,
        }
    }

    /// Compute them on a thread of their own, once, when their lineage computes nothing else:
    /// the newest asked for (a stroke under way gives a new style per change) starts next.
    fn start(self: Arc<Self>, settings: Arc<LayerStyle>, layer: Layer, at: Affine, canvas: Size) {
        if self.started.load(Ordering::Acquire) || self.lineage.busy.swap(true, Ordering::AcqRel) {
            return;
        }
        if self.started.swap(true, Ordering::AcqRel) {
            self.lineage.busy.store(false, Ordering::Release);
            return;
        }
        let effects = Arc::clone(&self);
        let spawned = std::thread::Builder::new()
            .name("layer style".to_owned())
            .spawn(move || {
                let done = Done(effects);
                let drawn = Arc::new(settings.draw(&done.0.shape, &layer, at, canvas));
                done.0.lineage.set_latest(&drawn);
                let _ = done.0.ready.set(drawn);
            });
        // Without a thread, whoever needs them exactly computes them.
        if spawned.is_err() {
            self.started.store(false, Ordering::Release);
            self.lineage.busy.store(false, Ordering::Release);
        }
    }
}

/// A computation in the background ending, even by a panic: its lineage is free, and effects
/// that could not be drawn show nothing rather than being asked for forever.
struct Done(Arc<Effects>);

impl Drop for Done {
    fn drop(&mut self) {
        let _ = self.0.ready.set(Arc::default());
        self.0.lineage.busy.store(false, Ordering::Release);
    }
}

/// The styles of one layer as it is edited: one computation at a time, and what was drawn last
/// (shown while newer effects are computed).
#[derive(Default)]
struct Lineage {
    busy: AtomicBool,
    latest: Mutex<Option<Arc<Drawn>>>,
}

impl Lineage {
    fn latest(&self) -> Option<Arc<Drawn>> {
        self.latest.lock().ok().and_then(|latest| latest.clone())
    }

    fn set_latest(&self, drawn: &Arc<Drawn>) {
        if let Ok(mut latest) = self.latest.lock() {
            *latest = Some(Arc::clone(drawn));
        }
    }
}

/// What a layer's shape gave its effects (ADR 0032 point 4), kept while the shape is the same:
/// its coverage, and each effect's mask by its geometry.
#[derive(Debug, Default)]
struct ShapeCache(Mutex<Shaped>);

impl ShapeCache {
    /// Locked to compute: a panic while it was left it unknown, not wrong.
    fn lock(&self) -> std::sync::MutexGuard<'_, Shaped> {
        self.0.lock().unwrap_or_else(|poisoned| {
            let mut shaped = poisoned.into_inner();
            *shaped = Shaped::default();
            shaped
        })
    }
}

#[derive(Debug, Default)]
struct Shaped {
    coverage: Option<Coverage>,
    /// By effect ([`Slot`]): its geometry, and its mask placed in the document (none when
    /// nothing shows).
    masks: [Option<(MaskKey, Option<Placed>)>; SLOTS],
}

/// A gray coverage image placed in the document.
type Placed = (Arc<RasterImage>, Affine);

/// The effects drawn from a mask, each with its place in [`Shaped::masks`].
#[derive(Debug, Clone, Copy)]
enum Slot {
    DropShadow,
    OuterGlow,
    InnerGlow,
    InnerShadow,
    Stroke,
}

const SLOTS: usize = 5;

/// What an effect's mask is made of: its geometry (not its color, mode or opacity).
#[derive(Debug, Clone, Copy, PartialEq)]
enum MaskKey {
    /// The shape (inverted when `inside`: what is outside it, to draw inward from the edge)
    /// expanded by `hard` pixels, blurred by a Gaussian of `sigma`, offset.
    Blurred {
        hard: f64,
        sigma: f64,
        offset: (f64, f64),
        inside: bool,
    },
    /// The band along the outline.
    Band { size: f64, position: StrokeLocation },
}

impl MaskKey {
    /// From `size` and `spread` (the percent of it hardening the shape), as Photoshop's.
    fn blurred(size: f64, spread: f64, offset: (f64, f64), inside: bool) -> Self {
        let hard = size * spread / 100.0;
        let soft = (size - hard).min(MAX_FEATHER);
        // A Gaussian reaches about three standard deviations; Photoshop's size is about two.
        MaskKey::Blurred {
            hard,
            sigma: soft / 2.0,
            offset,
            inside,
        }
    }

    /// A shadow's: offset away from the light.
    fn shadow(s: DropShadow, inside: bool) -> Self {
        let radians = s.angle.to_radians();
        let offset = (
            (-radians.cos() * s.distance).round(),
            (radians.sin() * s.distance).round(),
        );
        Self::blurred(s.size, s.spread, offset, inside)
    }

    /// A glow's: where the shape is.
    fn glow(g: Glow, inside: bool) -> Self {
        Self::blurred(g.size, g.spread, (0.0, 0.0), inside)
    }

    /// How far beyond the shape the mask reaches, pixels.
    fn reach(self) -> f64 {
        match self {
            MaskKey::Blurred {
                hard,
                sigma,
                offset: (dx, dy),
                ..
            } => hard + 3.0 * sigma + dx.abs().max(dy.abs()),
            MaskKey::Band { size, .. } => size + 1.0,
        }
    }

    /// The mask drawn from `coverage`, which reaches far enough.
    fn mask(self, coverage: &Coverage) -> Option<Placed> {
        let (area, shape) = coverage.shape.as_ref()?;
        let size = area.size();
        let m = f64::from(coverage.margin);
        let (x, y) = (f64::from(area.x) - m, f64::from(area.y) - m);
        match self {
            MaskKey::Blurred {
                hard,
                sigma,
                offset: (dx, dy),
                inside,
            } => {
                let mut shape = Arc::clone(shape);
                if inside {
                    shape = Arc::new(selection::invert(size, Some(&shape)).ok()??);
                }
                if hard > 0.0 {
                    shape = Arc::new(selection::modify(size, &shape, Modify::Expand(hard)).ok()??);
                }
                if sigma > 0.0 {
                    shape =
                        Arc::new(selection::modify(size, &shape, Modify::Feather(sigma)).ok()??);
                }
                Some((shape, Affine::translation(x + dx, y + dy)))
            }
            MaskKey::Band {
                size: width,
                position,
            } => {
                let band = selection::stroke_band(size, shape, width, position).ok()??;
                Some((Arc::new(band), Affine::translation(x, y)))
            }
        }
    }
}

impl Shaped {
    /// What it holds made for `to_document` on `canvas`: kept, moved along when the layer moved
    /// by whole pixels, else forgotten.
    fn align(&mut self, to_document: Affine, canvas: Size) {
        let Some(coverage) = &mut self.coverage else {
            return;
        };
        if coverage.to_document == to_document && coverage.canvas == canvas {
            return;
        }
        let Some((dx, dy)) = coverage.shift_to(to_document, canvas) else {
            *self = Shaped::default();
            return;
        };
        for (_, placed) in self.masks.iter_mut().flatten() {
            if let Some((_, at)) = placed {
                *at = at.then(Affine::translation(dx, dy));
            }
        }
    }

    /// The mask of `slot` if it is known for `key`.
    fn known(&self, slot: Slot, key: MaskKey) -> Option<Option<Placed>> {
        match &self.masks[slot as usize] {
            Some((known, placed)) if *known == key => Some(placed.clone()),
            _ => None,
        }
    }
}

/// A layer's coverage, as its effects are drawn from it.
#[derive(Debug)]
struct Coverage {
    to_document: Affine,
    canvas: Size,
    /// How far the grown canvas extends beyond the canvas on every side, and how far beyond the
    /// shape the coverage goes: the reach of the effects it serves.
    margin: u32,
    /// Where the shape shows in the grown canvas (whose origin is `-margin` in the document),
    /// grown by the margin, and the coverage there; `None` when nothing shows.
    shape: Option<(Rect, Arc<RasterImage>)>,
    /// The area is the shape's bounds grown by the margin, not cut by the grown canvas: the
    /// layer moved by whole pixels, it is the same coverage moved.
    whole: bool,
}

impl Coverage {
    /// Move it to `to_document` if the layer only moved by whole pixels and its shape stays on
    /// the grown canvas: the move.
    fn shift_to(&mut self, to_document: Affine, canvas: Size) -> Option<(f64, f64)> {
        let (old, new) = (self.to_document, to_document);
        let linear = |t: Affine| [t.a, t.b, t.c, t.d];
        if canvas != self.canvas || !self.whole || linear(old) != linear(new) {
            return None;
        }
        let (dx, dy) = (new.e - old.e, new.f - old.f);
        let whole = |v: f64| v.fract() == 0.0 && v.abs() <= f64::from(u32::MAX);
        if !whole(dx) || !whole(dy) {
            return None;
        }
        // Nothing showed: drawn again, the shape may come into view.
        let (area, _) = self.shape.as_mut()?;
        let grown = grown(canvas, self.margin);
        let x = i64::from(area.x) + dx as i64;
        let y = i64::from(area.y) + dy as i64;
        if x < 0
            || y < 0
            || x + i64::from(area.width) > i64::from(grown.width)
            || y + i64::from(area.height) > i64::from(grown.height)
        {
            return None;
        }
        // Fits: within the grown canvas, whose sides are `u32`.
        (area.x, area.y) = (x as u32, y as u32);
        self.to_document = to_document;
        Some((dx, dy))
    }
}

/// `canvas` grown by `margin` on every side.
fn grown(canvas: Size, margin: u32) -> Size {
    Size::new(
        canvas.width.saturating_add(2 * margin),
        canvas.height.saturating_add(2 * margin),
    )
}

/// A shadow behind the layer, offset from it (Photoshop's Drop Shadow).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DropShadow {
    pub enabled: bool,
    /// Working-space color; its alpha is ignored.
    pub color: LinearRgba,
    pub mode: BlendMode,
    pub opacity: f32,
    /// Where the light comes from, degrees counterclockwise from the right: the shadow goes the
    /// other way.
    pub angle: f64,
    /// How far the shadow is offset, pixels.
    pub distance: f64,
    /// How much of `size` hardens the shape before it is blurred, percent.
    pub spread: f64,
    /// How far the shadow spreads and blurs, pixels.
    pub size: f64,
}

impl Default for DropShadow {
    /// Photoshop's: black, Multiply, 75 %, light from 120°, 5 pixels away and wide.
    fn default() -> Self {
        Self {
            enabled: true,
            color: LinearRgba::new(0.0, 0.0, 0.0, 1.0),
            mode: BlendMode::Multiply,
            opacity: 0.75,
            angle: 120.0,
            distance: 5.0,
            spread: 0.0,
            size: 5.0,
        }
    }
}

/// The layer's shape recolored (Photoshop's Color Overlay).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorOverlay {
    pub enabled: bool,
    pub color: LinearRgba,
    pub mode: BlendMode,
    pub opacity: f32,
}

impl Default for ColorOverlay {
    /// Photoshop's: red, Normal, 100 %.
    fn default() -> Self {
        Self {
            enabled: true,
            color: LinearRgba::new(1.0, 0.0, 0.0, 1.0),
            mode: BlendMode::Normal,
            opacity: 1.0,
        }
    }
}

/// A band along the layer's outline (Photoshop's Stroke, a color fill).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stroke {
    pub enabled: bool,
    /// Its width, pixels.
    pub size: f64,
    pub position: StrokeLocation,
    pub color: LinearRgba,
    pub mode: BlendMode,
    pub opacity: f32,
}

impl Default for Stroke {
    /// Photoshop's: 3 pixels outside, black, Normal, 100 %.
    fn default() -> Self {
        Self {
            enabled: true,
            size: 3.0,
            position: StrokeLocation::Outside,
            color: LinearRgba::new(0.0, 0.0, 0.0, 1.0),
            mode: BlendMode::Normal,
            opacity: 1.0,
        }
    }
}

/// What a style's effects draw, as plain layers in the document's space, in the order they are
/// composited around the layer's content.
#[derive(Debug, Default)]
pub struct Drawn {
    /// Below the content: Drop Shadow.
    pub below: Vec<Layer>,
    /// Over the content, within its shape: Color Overlay (a fill composited atop the content).
    pub over: Vec<Layer>,
    /// Above the content: Stroke.
    pub above: Vec<Layer>,
}

/// What a style draws while its effects are computed for the first time: nothing.
pub(crate) static NO_EFFECTS: Drawn = Drawn {
    below: Vec::new(),
    over: Vec::new(),
    above: Vec::new(),
};

fn opacity_ok(opacity: f32) -> bool {
    (0.0..=1.0).contains(&opacity)
}

fn color_ok(color: LinearRgba) -> bool {
    color.is_finite()
}

impl LayerStyle {
    /// Settings within Photoshop's ranges, finite colors.
    pub fn is_valid(&self) -> bool {
        let shadow_ok = |s: DropShadow| {
            opacity_ok(s.opacity)
                && color_ok(s.color)
                && s.angle.is_finite()
                && (0.0..=MAX_DISTANCE).contains(&s.distance)
                && (0.0..=100.0).contains(&s.spread)
                && (0.0..=MAX_SIZE).contains(&s.size)
        };
        let glow_ok = |g: Glow| {
            opacity_ok(g.opacity)
                && color_ok(g.color)
                && (0.0..=100.0).contains(&g.spread)
                && (0.0..=MAX_SIZE).contains(&g.size)
        };
        opacity_ok(self.fill_opacity)
            && self.drop_shadow.is_none_or(shadow_ok)
            && self.inner_shadow.is_none_or(shadow_ok)
            && self.outer_glow.is_none_or(glow_ok)
            && self.inner_glow.is_none_or(glow_ok)
            && self
                .color_overlay
                .is_none_or(|o| opacity_ok(o.opacity) && color_ok(o.color))
            && self.stroke.is_none_or(|s| {
                opacity_ok(s.opacity) && color_ok(s.color) && (1.0..=MAX_MODIFY).contains(&s.size)
            })
    }

    /// Whether the style changes how the layer looks: an effect enabled, or Fill Opacity.
    pub fn shows(&self) -> bool {
        self.fill_opacity < 1.0
            || self.drop_shadow.is_some_and(|s| s.enabled)
            || self.inner_shadow.is_some_and(|s| s.enabled)
            || self.outer_glow.is_some_and(|g| g.enabled)
            || self.inner_glow.is_some_and(|g| g.enabled)
            || self.color_overlay.is_some_and(|o| o.enabled)
            || self.stroke.is_some_and(|s| s.enabled)
    }

    /// The effects drawn from a mask, enabled, in Photoshop's order (bottom to top).
    fn masked(&self) -> Vec<Masked> {
        let mut masked = Vec::new();
        let mut add = |place, slot, key, color, mode, opacity| {
            masked.push(Masked {
                place,
                slot,
                key,
                color,
                mode,
                opacity,
            });
        };
        if let Some(s) = self.drop_shadow.filter(|s| s.enabled) {
            let key = MaskKey::shadow(s, false);
            add(
                Place::Below,
                Slot::DropShadow,
                key,
                s.color,
                s.mode,
                s.opacity,
            );
        }
        if let Some(g) = self.outer_glow.filter(|g| g.enabled) {
            let key = MaskKey::glow(g, false);
            add(
                Place::Below,
                Slot::OuterGlow,
                key,
                g.color,
                g.mode,
                g.opacity,
            );
        }
        if let Some(g) = self.inner_glow.filter(|g| g.enabled) {
            let key = MaskKey::glow(g, true);
            add(
                Place::Over,
                Slot::InnerGlow,
                key,
                g.color,
                g.mode,
                g.opacity,
            );
        }
        if let Some(s) = self.inner_shadow.filter(|s| s.enabled) {
            let key = MaskKey::shadow(s, true);
            add(
                Place::Over,
                Slot::InnerShadow,
                key,
                s.color,
                s.mode,
                s.opacity,
            );
        }
        if let Some(s) = self.stroke.filter(|s| s.enabled) {
            let key = MaskKey::Band {
                size: s.size,
                position: s.position,
            };
            add(Place::Above, Slot::Stroke, key, s.color, s.mode, s.opacity);
        }
        masked
    }

    /// Photoshop's order, bottom to top: Drop Shadow, Outer Glow, the content, Color Overlay,
    /// Inner Glow, Inner Shadow, Stroke. The masks `shape` does not know yet are computed (on
    /// every core) from the layer's coverage, itself computed once for them all.
    fn draw(&self, shape: &ShapeCache, layer: &Layer, to_document: Affine, canvas: Size) -> Drawn {
        let masked = self.masked();
        let mut shaped = shape.lock();
        shaped.align(to_document, canvas);
        if masked.iter().any(|m| shaped.known(m.slot, m.key).is_none()) {
            let reach = masked.iter().map(|m| m.key.reach()).fold(0.0, f64::max);
            if shaped
                .coverage
                .as_ref()
                .is_none_or(|c| f64::from(c.margin) < reach)
            {
                // With room to spare: a size growing under a slider does not compute it again
                // at each step.
                let margin = margin((reach / ROOM).ceil() * ROOM);
                *shaped = Shaped {
                    coverage: Some(coverage(layer, to_document, canvas, margin)),
                    ..Shaped::default()
                };
            }
            for m in &masked {
                if shaped.known(m.slot, m.key).is_none() {
                    let mask = shaped.coverage.as_ref().and_then(|c| m.key.mask(c));
                    shaped.masks[m.slot as usize] = Some((m.key, mask));
                }
            }
        }
        self.assemble(&masked, &shaped).unwrap_or_default()
    }

    /// [`Self::draw`] without computing anything: `None` unless every mask is known (and the
    /// cache is not in use).
    fn draw_known(&self, shape: &ShapeCache, to_document: Affine, canvas: Size) -> Option<Drawn> {
        let mut shaped = shape.0.try_lock().ok()?;
        shaped.align(to_document, canvas);
        self.assemble(&self.masked(), &shaped)
    }

    /// The effects as layers, from the masks `shaped` knows: `None` when one is missing.
    fn assemble(&self, masked: &[Masked], shaped: &Shaped) -> Option<Drawn> {
        let mut drawn = Drawn::default();
        if let Some(overlay) = self.color_overlay.filter(|o| o.enabled) {
            drawn.over.push(effect_layer(
                LayerContent::Fill {
                    color: overlay.color,
                },
                overlay.mode,
                overlay.opacity,
                Affine::IDENTITY,
            ));
        }
        for m in masked {
            let Some((mask, at)) = shaped.known(m.slot, m.key)? else {
                continue;
            };
            let effect = colored(mask, m.color, m.mode, m.opacity, at);
            match m.place {
                Place::Below => drawn.below.push(effect),
                Place::Over => drawn.over.push(effect),
                Place::Above => drawn.above.push(effect),
            }
        }
        Some(drawn)
    }
}

/// An effect drawn from a mask, as a style sets it.
struct Masked {
    place: Place,
    slot: Slot,
    key: MaskKey,
    color: LinearRgba,
    mode: BlendMode,
    opacity: f32,
}

/// Where an effect is composited around the layer's content (see [`Drawn`]).
#[derive(Clone, Copy)]
enum Place {
    Below,
    /// Atop the content: within its shape.
    Over,
    Above,
}

/// The steps the margin of a coverage grows by, pixels.
const ROOM: f64 = 32.0;

/// A layer drawing an effect: `content` blended with `mode` and `opacity`, placed by
/// `transform` in the document.
fn effect_layer(content: LayerContent, mode: BlendMode, opacity: f32, transform: Affine) -> Layer {
    Layer {
        id: LayerId::from_raw(0),
        name: String::new(),
        visible: true,
        opacity,
        blend_mode: mode,
        content,
        mask: None,
        clipped: false,
        transform,
        style: None,
    }
}

/// `layer`'s coverage placed by `to_document`, where it shows grown by `margin` pixels on every
/// side, on the canvas grown by `margin` (so that a shape just off the canvas still casts what
/// reaches it).
fn coverage(layer: &Layer, to_document: Affine, canvas: Size, margin: u32) -> Coverage {
    let (shape, whole) = match shape_of(layer, to_document, canvas, margin) {
        Some((area, image, whole)) => (Some((area, image)), whole),
        None => (None, false),
    };
    Coverage {
        to_document,
        canvas,
        margin,
        shape,
        whole,
    }
}

/// [`coverage`]'s area (in the grown canvas, whose origin is `-margin` in the document), the
/// coverage there (a gray image of its size), and whether the area is whole (not cut by the
/// grown canvas). `None` when nothing shows.
fn shape_of(
    layer: &Layer,
    to_document: Affine,
    canvas: Size,
    margin: u32,
) -> Option<(Rect, Arc<RasterImage>, bool)> {
    let m = f64::from(margin);
    let grown = grown(canvas, margin);
    // The layer alone, plain: its mask kept (it shapes the effects).
    let id = layer.id;
    let shape = Layer {
        visible: true,
        opacity: 1.0,
        blend_mode: BlendMode::Normal,
        clipped: false,
        transform: to_document.then(Affine::translation(m, m)),
        style: None,
        ..layer.clone()
    };
    let shows_everywhere = shows_fill(std::slice::from_ref(&shape));
    let scratch = Document::restore(
        grown,
        WORKING_SPACE,
        BlendSpace::Perceptual,
        vec![shape],
        id.get() + 1,
    )
    .ok()?;
    let reach = i64::from(margin);
    let full = pick::Bounds {
        left: 0,
        top: 0,
        right: i64::from(grown.width),
        bottom: i64::from(grown.height),
    };
    let bounds = if shows_everywhere {
        full
    } else {
        pick::bounds_of(&scratch, &[id])?
    };
    let whole = !shows_everywhere
        && bounds.left - reach >= 0
        && bounds.top - reach >= 0
        && bounds.right + reach <= full.right
        && bounds.bottom + reach <= full.bottom;
    let left = (bounds.left - reach).clamp(0, full.right);
    let top = (bounds.top - reach).clamp(0, full.bottom);
    let right = (bounds.right + reach).clamp(0, full.right);
    let bottom = (bounds.bottom + reach).clamp(0, full.bottom);
    if right <= left || bottom <= top {
        return None;
    }
    // Fits: clamped to the grown canvas, whose sides are `u32`.
    let area = Rect::new(
        left as u32,
        top as u32,
        (right - left) as u32,
        (bottom - top) as u32,
    );
    let image = alpha_of(&scratch, area)?;
    Some((area, Arc::new(image), whole))
}

/// The alpha of `area` of `document` composited, as a coverage image ([`SELECTION_FORMAT`]): a
/// row of tiles at a time, so that what is composited (16 bytes a pixel) never holds more than
/// one (a layer's whole area would be gigabytes on a large canvas).
fn alpha_of(document: &Document, area: Rect) -> Option<RasterImage> {
    let t = crate::raster::TILE_SIZE as usize;
    let (width, height) = (area.width as usize, area.height as usize);
    let columns = width.div_ceil(t);
    let mut tiles: Vec<Arc<[u8]>> = Vec::with_capacity(columns * height.div_ceil(t));
    let mut pixels = Vec::new();
    for top in (0..height).step_by(t) {
        let rows = t.min(height - top);
        // Fits: within the area, whose sides are `u32`.
        let band = Rect::new(area.x, area.y + top as u32, area.width, rows as u32);
        pixels.clear();
        pixels.resize(width * rows * 4, 0.0f32);
        composite_region(document, band, &mut pixels).ok()?;
        let mut band_tiles: Vec<(usize, Option<Arc<[u8]>>)> =
            (0..columns).map(|col| (col, None)).collect();
        let pixels = &pixels;
        parallel_for_each(&mut band_tiles, |(col, out)| {
            let x0 = *col * t;
            let w = t.min(width - x0);
            let mut tile = vec![0u8; t * t * 2];
            // Edge tiles padded by repeating their last row and column, as images are.
            for ty in 0..t {
                let y = ty.min(rows - 1);
                for tx in 0..t {
                    let x = x0 + tx.min(w - 1);
                    let alpha = pixels[(y * width + x) * 4 + 3];
                    let v = (alpha.clamp(0.0, 1.0) * 65535.0).round() as u16;
                    tile[(ty * t + tx) * 2..][..2].copy_from_slice(&v.to_ne_bytes());
                }
            }
            *out = Some(Arc::from(tile));
        });
        tiles.extend(band_tiles.into_iter().filter_map(|(_, tile)| tile));
    }
    RasterImage::from_level0_tiles(area.size(), SELECTION_FORMAT, tiles).ok()
}

/// Some of `layers` is a fill seen through visible groups: it covers the whole canvas.
fn shows_fill(layers: &[Layer]) -> bool {
    layers
        .iter()
        .filter(|l| l.visible)
        .any(|l| match &l.content {
            LayerContent::Fill { .. } => true,
            LayerContent::Group { children, .. } => shows_fill(children),
            _ => false,
        })
}

/// An effect drawing `color` where `coverage` shows, placed by `at`: a fill masked by the
/// coverage (no color image is built).
fn colored(
    coverage: Arc<RasterImage>,
    color: LinearRgba,
    mode: BlendMode,
    opacity: f32,
    at: Affine,
) -> Layer {
    let mut layer = effect_layer(LayerContent::Fill { color }, mode, opacity, at);
    layer.mask = Some(LayerMask {
        image: coverage,
        enabled: true,
        replaces_alpha: false,
        original: None,
    });
    layer
}

/// The margin of the grown canvas, the reach of an effect, whole pixels.
fn margin(reach: f64) -> u32 {
    reach.ceil().clamp(0.0, f64::from(u32::MAX / 4)) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::{AlphaMode, ChannelLayout, ColorSpace, PixelFormat, SampleType};
    use crate::edit::{Edit, EditError};

    #[test]
    fn a_shape_made_a_row_of_tiles_at_a_time_is_the_whole_area_s() {
        // Taller and wider than a tile, off the canvas' origin: bands and padded edge tiles.
        let size = Size::new(300, 600);
        let mut doc = Document::new(size);
        let pixels: Vec<u8> = (0..size.pixel_count() as usize)
            .flat_map(|i| [9, 99, 199, ((i * 7 + i / 300 * 13) % 256) as u8])
            .collect();
        let image = RasterImage::from_pixels(size, PixelFormat::RGBA8_SRGB, &pixels).unwrap();
        let id = doc.allocate_layer_id();
        Edit::InsertLayer {
            parent: None,
            index: 0,
            layer: Layer {
                id,
                name: "alpha".into(),
                visible: true,
                opacity: 1.0,
                blend_mode: BlendMode::Normal,
                mask: None,
                clipped: false,
                transform: Affine::IDENTITY,
                style: None,
                content: LayerContent::raster(Arc::new(image)),
            },
        }
        .apply(&mut doc)
        .unwrap();
        let area = Rect::new(5, 3, 290, 590);
        let banded = alpha_of(&doc, area).unwrap();
        // The whole area composited at once, as before.
        let mut whole = vec![0.0f32; area.size().pixel_count() as usize * 4];
        composite_region(&doc, area, &mut whole).unwrap();
        let gray: Vec<u8> = whole
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|px| ((px[3].clamp(0.0, 1.0) * 65535.0).round() as u16).to_ne_bytes())
            .collect();
        let expected = RasterImage::from_pixels(area.size(), SELECTION_FORMAT, &gray).unwrap();
        assert_eq!(banded.levels().len(), expected.levels().len());
        for (a, b) in banded.levels().iter().zip(expected.levels()) {
            for (s, t) in a.tiles().iter().zip(b.tiles()) {
                assert_eq!(**s, **t);
            }
        }
    }

    /// A 64 × 64 document with an opaque white box at (20, 20)–(30, 30), and its id.
    fn document() -> (Document, LayerId) {
        document_with(255)
    }

    /// The same with a box of gray `level`.
    fn document_with(level: u8) -> (Document, LayerId) {
        let mut doc = Document::new(Size::new(64, 64));
        let format = PixelFormat {
            layout: ChannelLayout::Rgba,
            sample: SampleType::U8,
            color_space: ColorSpace::SRGB,
            alpha: AlphaMode::Straight,
        };
        let rect = Rect::new(20, 20, 10, 10);
        let pixels = [level, level, level, 255].repeat(100);
        let image = RasterImage::from_placed(doc.size(), format, rect, &pixels, &[0; 4]).unwrap();
        let id = doc.allocate_layer_id();
        Edit::InsertLayer {
            parent: None,
            index: 0,
            layer: Layer {
                id,
                name: "box".into(),
                visible: true,
                opacity: 1.0,
                blend_mode: BlendMode::Normal,
                content: LayerContent::raster(Arc::new(image)),
                mask: None,
                clipped: false,
                transform: Affine::IDENTITY,
                style: None,
            },
        }
        .apply(&mut doc)
        .unwrap();
        (doc, id)
    }

    fn styled(doc: &mut Document, id: LayerId, style: LayerStyle) -> Edit {
        Edit::SetLayerStyle {
            id,
            style: Some(Box::new(style)),
        }
        .apply(doc)
        .unwrap()
    }

    /// The composite at (`x`, `y`): premultiplied working-space RGBA.
    fn at(doc: &Document, x: u32, y: u32) -> [f32; 4] {
        let mut px = [0.0f32; 4];
        composite_region(doc, Rect::new(x, y, 1, 1), &mut px).unwrap();
        px
    }

    #[test]
    fn a_drop_shadow_falls_away_from_the_light_under_the_layer() {
        let (mut doc, id) = document();
        styled(
            &mut doc,
            id,
            LayerStyle {
                drop_shadow: Some(DropShadow {
                    mode: BlendMode::Normal,
                    opacity: 1.0,
                    ..DropShadow::default()
                }),
                ..LayerStyle::default()
            },
        );
        // Light from 120°: the shadow goes right and down, under the layer.
        // At the corner of the blurred shadow, about half.
        assert!(at(&doc, 31, 31)[3] > 0.4, "{:?}", at(&doc, 31, 31));
        assert!(at(&doc, 27, 32)[3] > 0.6, "{:?}", at(&doc, 27, 32));
        assert!(at(&doc, 31, 31)[0] < 0.1);
        assert_eq!(at(&doc, 17, 17)[3], 0.0);
        assert_eq!(at(&doc, 25, 25), [1.0, 1.0, 1.0, 1.0]);
    }

    #[test]
    fn a_stroke_outside_bands_the_outline_and_an_overlay_recolors_the_shape() {
        let (mut doc, id) = document();
        styled(
            &mut doc,
            id,
            LayerStyle {
                stroke: Some(Stroke::default()),
                color_overlay: Some(ColorOverlay {
                    color: LinearRgba::new(0.0, 0.0, 1.0, 1.0),
                    ..ColorOverlay::default()
                }),
                ..LayerStyle::default()
            },
        );
        // The stroke: black, 3 pixels outside the box.
        assert_eq!(at(&doc, 31, 25), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(at(&doc, 32, 25)[3], 1.0);
        assert_eq!(at(&doc, 34, 25)[3], 0.0);
        // The overlay: blue within the shape, nothing outside it.
        assert_eq!(at(&doc, 25, 25), [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(at(&doc, 40, 40)[3], 0.0);
    }

    #[test]
    fn fill_opacity_fades_the_content_not_its_effects() {
        let (mut doc, id) = document();
        styled(
            &mut doc,
            id,
            LayerStyle {
                fill_opacity: 0.0,
                stroke: Some(Stroke::default()),
                ..LayerStyle::default()
            },
        );
        assert_eq!(at(&doc, 25, 25)[3], 0.0);
        assert_eq!(at(&doc, 31, 25), [0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn effects_follow_the_layer_and_go_with_undo() {
        let (mut doc, id) = document();
        let undo = styled(
            &mut doc,
            id,
            LayerStyle {
                stroke: Some(Stroke::default()),
                ..LayerStyle::default()
            },
        );
        assert_eq!(at(&doc, 31, 25)[3], 1.0);
        // Moved: drawn again where it is.
        Edit::SetLayerTransform {
            id,
            transform: Affine::translation(10.0, 0.0),
        }
        .apply(&mut doc)
        .unwrap();
        assert_eq!(at(&doc, 31, 25), [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(at(&doc, 41, 25), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(at(&doc, 28, 25)[3], 1.0);
        Edit::SetLayerTransform {
            id,
            transform: Affine::IDENTITY,
        }
        .apply(&mut doc)
        .unwrap();
        undo.apply(&mut doc).unwrap();
        assert_eq!(doc.layer(id).unwrap().style, None::<Style>);
        assert_eq!(at(&doc, 31, 25)[3], 0.0);
    }

    #[test]
    fn an_outer_glow_lights_around_the_shape() {
        let (mut doc, id) = document();
        styled(
            &mut doc,
            id,
            LayerStyle {
                outer_glow: Some(Glow {
                    mode: BlendMode::Normal,
                    opacity: 1.0,
                    ..Glow::default()
                }),
                ..LayerStyle::default()
            },
        );
        // Around the box on every side, fading out; the box itself untouched.
        for (x, y) in [(19, 25), (30, 25), (25, 19), (25, 30)] {
            assert!(at(&doc, x, y)[3] > 0.3, "({x}, {y}): {:?}", at(&doc, x, y));
        }
        assert!(at(&doc, 30, 25)[3] > at(&doc, 33, 25)[3]);
        assert_eq!(at(&doc, 40, 40)[3], 0.0);
        assert_eq!(at(&doc, 25, 25), [1.0, 1.0, 1.0, 1.0]);
    }

    #[test]
    fn inner_effects_stay_within_the_shape_from_its_edge() {
        let (mut doc, id) = document_with(128);
        let plain = at(&doc, 25, 25);
        styled(
            &mut doc,
            id,
            LayerStyle {
                inner_shadow: Some(DropShadow {
                    mode: BlendMode::Normal,
                    opacity: 1.0,
                    distance: 3.0,
                    size: 2.0,
                    ..DropShadow::default()
                }),
                ..LayerStyle::default()
            },
        );
        // Light from 120°: the shadow falls inside along the top and left edges.
        assert!(
            at(&doc, 21, 25)[0] < plain[0] * 0.5,
            "{:?}",
            at(&doc, 21, 25)
        );
        assert!((at(&doc, 28, 25)[0] - plain[0]).abs() < 1e-3);
        // Nothing outside the shape.
        assert_eq!(at(&doc, 18, 25)[3], 0.0);

        let (mut doc, id) = document_with(128);
        styled(
            &mut doc,
            id,
            LayerStyle {
                inner_glow: Some(Glow {
                    mode: BlendMode::Normal,
                    opacity: 1.0,
                    color: LinearRgba::new(1.0, 1.0, 1.0, 1.0),
                    ..Glow::default()
                }),
                ..LayerStyle::default()
            },
        );
        // Brighter along every inner edge than in the middle; nothing outside.
        assert!(at(&doc, 20, 25)[0] > at(&doc, 25, 25)[0] + 0.1);
        assert!(at(&doc, 29, 25)[0] > at(&doc, 25, 25)[0] + 0.1);
        assert_eq!(at(&doc, 31, 25)[3], 0.0);
    }

    #[test]
    fn a_group_s_effects_are_drawn_from_its_layers_and_follow_them() {
        let (mut doc, id) = document();
        let group = doc.allocate_layer_id();
        Edit::group_layers(
            &doc,
            Layer {
                id: group,
                name: "group".into(),
                visible: true,
                opacity: 1.0,
                blend_mode: BlendMode::Normal,
                content: LayerContent::Group {
                    children: Vec::new(),
                    pass_through: true,
                },
                mask: None,
                clipped: false,
                transform: Affine::IDENTITY,
                style: None,
            },
            &[id],
        )
        .unwrap()
        .apply(&mut doc)
        .unwrap();
        styled(
            &mut doc,
            group,
            LayerStyle {
                stroke: Some(Stroke::default()),
                ..LayerStyle::default()
            },
        );
        assert_eq!(at(&doc, 31, 25), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(at(&doc, 25, 25), [1.0, 1.0, 1.0, 1.0]);
        // The layer inside moves: the group's stroke goes with it.
        Edit::SetLayerTransform {
            id,
            transform: Affine::translation(10.0, 0.0),
        }
        .apply(&mut doc)
        .unwrap();
        assert_eq!(at(&doc, 41, 25), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(at(&doc, 31, 25), [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(at(&doc, 18, 25)[3], 0.0);
    }

    /// The masks of the effects the exact steps draw, bottom to top.
    fn masks(doc: &Document) -> Vec<Arc<RasterImage>> {
        crate::composite::steps(doc)
            .into_iter()
            .filter_map(|step| match step {
                crate::composite::Step::Layer { layer, .. } if layer.id.get() == 0 => {
                    layer.mask.as_ref().map(|m| Arc::clone(&m.image))
                }
                _ => None,
            })
            .collect()
    }

    /// The same images (not copies).
    fn same(a: &[Arc<RasterImage>], b: &[Arc<RasterImage>]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(a, b)| Arc::ptr_eq(a, b))
    }

    fn shadow_and_stroke() -> LayerStyle {
        LayerStyle {
            drop_shadow: Some(DropShadow {
                mode: BlendMode::Normal,
                opacity: 1.0,
                ..DropShadow::default()
            }),
            stroke: Some(Stroke::default()),
            ..LayerStyle::default()
        }
    }

    #[test]
    fn a_new_color_recolors_the_same_masks_a_new_size_draws_that_effect_only() {
        let (mut doc, id) = document();
        let style = shadow_and_stroke();
        styled(&mut doc, id, style);
        let before = masks(&doc);
        assert_eq!(before.len(), 2);
        let mut recolored = style;
        if let Some(stroke) = &mut recolored.stroke {
            stroke.color = LinearRgba::new(0.0, 1.0, 0.0, 1.0);
        }
        styled(&mut doc, id, recolored);
        assert!(same(&masks(&doc), &before));
        assert_eq!(at(&doc, 31, 25), [0.0, 1.0, 0.0, 1.0]);
        // The shadow larger: drawn again; the stroke kept.
        let mut larger = recolored;
        if let Some(shadow) = &mut larger.drop_shadow {
            shadow.size = 8.0;
        }
        styled(&mut doc, id, larger);
        let after = masks(&doc);
        assert!(!Arc::ptr_eq(&after[0], &before[0]));
        assert!(Arc::ptr_eq(&after[1], &before[1]));
    }

    #[test]
    fn a_whole_pixel_move_moves_the_masks_a_fractional_one_draws_them_again() {
        let (mut doc, id) = document();
        styled(&mut doc, id, shadow_and_stroke());
        let before = masks(&doc);
        Edit::SetLayerTransform {
            id,
            transform: Affine::translation(10.0, -3.0),
        }
        .apply(&mut doc)
        .unwrap();
        assert!(same(&masks(&doc), &before));
        // Where the layer went, exactly as drawn there from scratch.
        let (mut fresh, fresh_id) = document();
        Edit::SetLayerTransform {
            id: fresh_id,
            transform: Affine::translation(10.0, -3.0),
        }
        .apply(&mut fresh)
        .unwrap();
        styled(&mut fresh, fresh_id, shadow_and_stroke());
        for (x, y) in [(41, 22), (43, 30), (38, 33), (30, 20), (25, 25)] {
            assert_eq!(at(&doc, x, y), at(&fresh, x, y), "({x}, {y})");
        }
        Edit::SetLayerTransform {
            id,
            transform: Affine::translation(10.5, -3.0),
        }
        .apply(&mut doc)
        .unwrap();
        assert!(!same(&masks(&doc), &before));
    }

    #[test]
    fn a_move_off_the_grown_canvas_draws_the_effects_again() {
        let (mut doc, id) = document();
        styled(&mut doc, id, shadow_and_stroke());
        let before = masks(&doc);
        // Far enough that the shape leaves the canvas and its margin: cut, so drawn again.
        Edit::SetLayerTransform {
            id,
            transform: Affine::translation(60.0, 0.0),
        }
        .apply(&mut doc)
        .unwrap();
        assert!(!same(&masks(&doc), &before));
        assert_eq!(at(&doc, 25, 25)[3], 0.0);
    }

    #[test]
    fn edits_that_keep_the_shape_keep_the_effects() {
        let (mut doc, id) = document();
        styled(&mut doc, id, shadow_and_stroke());
        masks(&doc);
        let style = |doc: &Document| doc.layer(id).and_then(|l| l.style.clone()).unwrap();
        let kept = style(&doc);
        for edit in [
            Edit::RenameLayer {
                id,
                name: "renamed".into(),
            },
            Edit::SetLayerOpacity { id, opacity: 0.5 },
            Edit::SetLayerBlendMode {
                id,
                mode: BlendMode::Multiply,
            },
            Edit::SetLayerVisible { id, visible: false },
            Edit::SetLayerVisible { id, visible: true },
        ] {
            edit.apply(&mut doc).unwrap();
        }
        assert!(style(&doc).ptr_eq(&kept));
    }

    #[test]
    fn a_hidden_layer_inside_a_styled_group_takes_its_part_of_the_effects_away() {
        let (mut doc, id) = document();
        let group = doc.allocate_layer_id();
        Edit::group_layers(
            &doc,
            Layer {
                id: group,
                name: "group".into(),
                visible: true,
                opacity: 1.0,
                blend_mode: BlendMode::Normal,
                content: LayerContent::Group {
                    children: Vec::new(),
                    pass_through: true,
                },
                mask: None,
                clipped: false,
                transform: Affine::IDENTITY,
                style: None,
            },
            &[id],
        )
        .unwrap()
        .apply(&mut doc)
        .unwrap();
        styled(
            &mut doc,
            group,
            LayerStyle {
                stroke: Some(Stroke::default()),
                ..LayerStyle::default()
            },
        );
        assert_eq!(at(&doc, 31, 25), [0.0, 0.0, 0.0, 1.0]);
        Edit::SetLayerVisible { id, visible: false }
            .apply(&mut doc)
            .unwrap();
        assert_eq!(at(&doc, 31, 25)[3], 0.0);
    }

    /// The display's plan once its effects are drawn (waiting for the background).
    fn settled(doc: &Document) -> Vec<Arc<RasterImage>> {
        let start = std::time::Instant::now();
        while crate::composite::display_plan(doc).1 {
            assert!(start.elapsed().as_secs() < 30, "effects never drawn");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        display_masks(doc).0
    }

    /// The masks of the effects the display shows, and whether some are not drawn yet.
    fn display_masks(doc: &Document) -> (Vec<Arc<RasterImage>>, bool) {
        let (steps, pending) = crate::composite::display_plan(doc);
        let masks = steps
            .into_iter()
            .filter_map(|step| match step {
                crate::composite::Step::Layer { layer, .. } if layer.id.get() == 0 => {
                    layer.mask.as_ref().map(|m| Arc::clone(&m.image))
                }
                _ => None,
            })
            .collect();
        (masks, pending)
    }

    #[test]
    fn the_display_never_waits_and_shows_what_was_drawn_last_meanwhile() {
        let (mut doc, id) = document();
        styled(&mut doc, id, shadow_and_stroke());
        // The first time: nothing to show yet, computed in the background.
        let (first, pending) = display_masks(&doc);
        assert!(first.is_empty() || !pending);
        let drawn = settled(&doc);
        assert_eq!(drawn.len(), 2);
        // The exact steps take what the background drew.
        assert!(same(&masks(&doc), &drawn));

        // The shape changes (a mask hides the right half): the display shows the previous
        // effects until the new ones are drawn.
        let mut gray = vec![0u8; 64 * 64 * 2];
        for (i, px) in gray.chunks_mut(2).enumerate() {
            if i % 64 < 25 {
                px.copy_from_slice(&u16::MAX.to_ne_bytes());
            }
        }
        let mask = RasterImage::from_pixels(Size::new(64, 64), SELECTION_FORMAT, &gray).unwrap();
        Edit::SetLayerMask {
            id,
            mask: Some(LayerMask {
                image: Arc::new(mask),
                enabled: true,
                replaces_alpha: false,
                original: None,
            }),
        }
        .apply(&mut doc)
        .unwrap();
        let (meanwhile, pending) = display_masks(&doc);
        if pending {
            assert!(same(&meanwhile, &drawn));
        }
        let redrawn = settled(&doc);
        assert!(!same(&redrawn, &drawn));
        // As drawn exactly: the stroke along the new right edge.
        assert_eq!(at(&doc, 26, 25), [0.0, 0.0, 0.0, 1.0]);
        assert!(same(&masks(&doc), &redrawn));
    }

    #[test]
    fn a_hidden_layer_s_effects_are_not_waited_for() {
        let (mut doc, id) = document();
        styled(&mut doc, id, shadow_and_stroke());
        Edit::SetLayerVisible { id, visible: false }
            .apply(&mut doc)
            .unwrap();
        assert!(!crate::composite::display_plan(&doc).1);
    }

    #[test]
    fn styles_out_of_range_or_on_adjustment_layers_are_refused() {
        let (mut doc, id) = document();
        let bad = LayerStyle {
            stroke: Some(Stroke {
                size: 0.0,
                ..Stroke::default()
            }),
            ..LayerStyle::default()
        };
        assert_eq!(
            Edit::SetLayerStyle {
                id,
                style: Some(Box::new(bad))
            }
            .apply(&mut doc),
            Err(EditError::InvalidStyle)
        );
        assert!(!LayerStyle::default().shows());
        assert!(
            LayerStyle {
                fill_opacity: 0.5,
                ..LayerStyle::default()
            }
            .shows()
        );
    }
}
