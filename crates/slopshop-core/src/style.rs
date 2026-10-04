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
//! What effects draw is a cache, never saved: computed when first composited, kept while the
//! layer is not edited (every edit reaching a layer gives its style a fresh cache).

use std::sync::{Arc, OnceLock};

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
/// never saved, drawn again when the layer is edited).
#[derive(Debug, Clone)]
pub struct Style {
    /// Shared: a layer stays small (its style is cloned with it).
    settings: Arc<LayerStyle>,
    drawn: Arc<OnceLock<Drawn>>,
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
            drawn: Arc::default(),
        }
    }

    pub fn settings(&self) -> &LayerStyle {
        &self.settings
    }

    /// The same settings, with nothing drawn yet: what a layer's style becomes when the layer
    /// is edited.
    pub(crate) fn redrawn(&self) -> Self {
        Self {
            settings: Arc::clone(&self.settings),
            drawn: Arc::default(),
        }
    }

    /// What the effects draw for `layer` (a pixel or fill layer) placed by `to_document` on a
    /// `canvas`: computed the first time (on every core), then kept.
    pub(crate) fn drawn(&self, layer: &Layer, to_document: Affine, canvas: Size) -> &Drawn {
        self.drawn
            .get_or_init(|| self.settings.draw(layer, to_document, canvas))
    }
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

    /// Photoshop's order, bottom to top: Drop Shadow, Outer Glow, the content, Color Overlay,
    /// Inner Glow, Inner Shadow, Stroke.
    fn draw(&self, layer: &Layer, to_document: Affine, canvas: Size) -> Drawn {
        let mut drawn = Drawn::default();
        let shape = Shape {
            layer,
            to_document,
            canvas,
        };
        if let Some(s) = self.drop_shadow.filter(|s| s.enabled)
            && let Some(effect) = shape.shadow(s, false)
        {
            drawn.below.push(effect);
        }
        if let Some(g) = self.outer_glow.filter(|g| g.enabled)
            && let Some(effect) = shape.glow(g, false)
        {
            drawn.below.push(effect);
        }
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
        if let Some(g) = self.inner_glow.filter(|g| g.enabled)
            && let Some(effect) = shape.glow(g, true)
        {
            drawn.over.push(effect);
        }
        if let Some(s) = self.inner_shadow.filter(|s| s.enabled)
            && let Some(effect) = shape.shadow(s, true)
        {
            drawn.over.push(effect);
        }
        if let Some(stroke) = self.stroke.filter(|s| s.enabled)
            && let Some(effect) = stroke_effect(layer, to_document, canvas, stroke)
        {
            drawn.above.push(effect);
        }
        drawn
    }
}

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

/// `layer`'s coverage in the document (placed by `to_document`), where it lies grown by `reach`
/// pixels on every side, on a `canvas` grown by `margin` pixels (so that a shape just off the
/// canvas still casts what reaches it): the area (in the grown canvas, whose origin is
/// `-margin` in the document) and the coverage there, a gray image of its size. `None` when
/// nothing shows.
fn coverage(
    layer: &Layer,
    to_document: Affine,
    canvas: Size,
    reach: f64,
    margin: u32,
) -> Option<(Rect, Arc<RasterImage>)> {
    let m = f64::from(margin);
    let grown = Size::new(
        canvas.width.saturating_add(2 * margin),
        canvas.height.saturating_add(2 * margin),
    );
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
    let reach = reach.ceil() as i64;
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
    let mut pixels = vec![0.0f32; area.size().pixel_count() as usize * 4];
    composite_region(&scratch, area, &mut pixels).ok()?;
    let mut gray = vec![0u8; pixels.len() / 4 * 2];
    // Rows on every core.
    let row = area.width as usize;
    let mut rows: Vec<(&[f32], &mut [u8])> = pixels
        .chunks(row * 4)
        .zip(gray.chunks_mut(row * 2))
        .collect();
    parallel_for_each(&mut rows, |(src, dst)| {
        for (px, out) in src
            .as_chunks::<4>()
            .0
            .iter()
            .zip(dst.as_chunks_mut::<2>().0)
        {
            *out = ((px[3].clamp(0.0, 1.0) * 65535.0).round() as u16).to_ne_bytes();
        }
    });
    let image = RasterImage::from_pixels(area.size(), SELECTION_FORMAT, &gray).ok()?;
    Some((area, Arc::new(image)))
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

/// A layer's shape on a canvas: what its blurred effects are drawn from.
struct Shape<'a> {
    layer: &'a Layer,
    to_document: Affine,
    canvas: Size,
}

impl Shape<'_> {
    /// Drop Shadow, or Inner Shadow when `inside`: offset away from the light.
    fn shadow(&self, s: DropShadow, inside: bool) -> Option<Layer> {
        let radians = s.angle.to_radians();
        let offset = (
            (-radians.cos() * s.distance).round(),
            (radians.sin() * s.distance).round(),
        );
        self.blurred(s.size, s.spread, offset, inside, s.color, s.mode, s.opacity)
    }

    /// Outer Glow, or Inner Glow when `inside`: where it is.
    fn glow(&self, g: Glow, inside: bool) -> Option<Layer> {
        self.blurred(
            g.size,
            g.spread,
            (0.0, 0.0),
            inside,
            g.color,
            g.mode,
            g.opacity,
        )
    }

    /// The shape (inverted when `inside`: what is outside it, to draw inward from the edge)
    /// expanded by `spread` percent of `size`, blurred by the rest, offset, colored. An inside
    /// effect is composited atop the content: the shape clips it.
    #[allow(clippy::too_many_arguments)]
    fn blurred(
        &self,
        size: f64,
        spread: f64,
        (dx, dy): (f64, f64),
        inside: bool,
        color: LinearRgba,
        mode: BlendMode,
        opacity: f32,
    ) -> Option<Layer> {
        let hard = size * spread / 100.0;
        let soft = (size - hard).min(MAX_FEATHER);
        // A Gaussian reaches about three standard deviations; Photoshop's size is about two.
        let sigma = soft / 2.0;
        let reach = hard + 3.0 * sigma + dx.abs().max(dy.abs());
        let m = margin(reach);
        let (area, mut shape) = coverage(self.layer, self.to_document, self.canvas, reach, m)?;
        let area_size = area.size();
        if inside {
            shape = Arc::new(selection::invert(area_size, Some(&shape)).ok()??);
        }
        if hard > 0.0 {
            shape = Arc::new(selection::modify(area_size, &shape, Modify::Expand(hard)).ok()??);
        }
        if sigma > 0.0 {
            shape = Arc::new(selection::modify(area_size, &shape, Modify::Feather(sigma)).ok()??);
        }
        let at = Affine::translation(
            f64::from(area.x) - f64::from(m) + dx,
            f64::from(area.y) - f64::from(m) + dy,
        );
        Some(colored(shape, color, mode, opacity, at))
    }
}

/// Stroke: the band along the shape's outline, inside, centered on or outside it, colored.
fn stroke_effect(
    layer: &Layer,
    to_document: Affine,
    canvas: Size,
    stroke: Stroke,
) -> Option<Layer> {
    let reach = stroke.size + 1.0;
    let m = margin(reach);
    let (area, shape) = coverage(layer, to_document, canvas, reach, m)?;
    let band = selection::stroke_band(area.size(), &shape, stroke.size, stroke.position).ok()??;
    let at = Affine::translation(
        f64::from(area.x) - f64::from(m),
        f64::from(area.y) - f64::from(m),
    );
    Some(colored(
        Arc::new(band),
        stroke.color,
        stroke.mode,
        stroke.opacity,
        at,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::{AlphaMode, ChannelLayout, ColorSpace, PixelFormat, SampleType};
    use crate::edit::{Edit, EditError};

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
