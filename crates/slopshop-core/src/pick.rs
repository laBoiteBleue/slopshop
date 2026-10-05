//! Finding layers from the image: the layer under a point (the Move tool's Auto-Select, as in
//! Photoshop) and where each layer's pixels are (snapping while moving, ADR 0017).

use crate::document::{Document, Layer, LayerContent, LayerId};
use crate::raster::RasterImage;
use crate::transform::Projective;

/// A rectangle in document pixels, `[left, right) × [top, bottom)`; it may extend past the
/// canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    pub left: i64,
    pub top: i64,
    pub right: i64,
    pub bottom: i64,
}

impl Bounds {
    /// The box around `rect` (in a layer's pixels) placed by `transform`, in whole pixels.
    fn placed(rect: crate::geom::Rect, transform: Projective) -> Bounds {
        let [x0, y0, x1, y1] = transform.map_rect([
            f64::from(rect.x),
            f64::from(rect.y),
            rect.right() as f64,
            rect.bottom() as f64,
        ]);
        Bounds {
            left: x0.floor() as i64,
            top: y0.floor() as i64,
            right: x1.ceil() as i64,
            bottom: y1.ceil() as i64,
        }
    }

    /// The common part, `None` when there is none.
    fn intersection(self, other: Bounds) -> Option<Bounds> {
        let b = Bounds {
            left: self.left.max(other.left),
            top: self.top.max(other.top),
            right: self.right.min(other.right),
            bottom: self.bottom.min(other.bottom),
        };
        (b.left < b.right && b.top < b.bottom).then_some(b)
    }

    fn union(self, other: Bounds) -> Bounds {
        Bounds {
            left: self.left.min(other.left),
            top: self.top.min(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
        }
    }
}

/// Pixels below this coverage do not count as the layer's (a faint shadow or a feathered edge
/// is not where users click to take a layer).
const PICK_COVERAGE: f32 = 0.05;

/// The topmost layer showing a pixel at document pixel (`x`, `y`): a layer, not a group (a
/// group is searched inside). Hidden layers (or in hidden groups) are skipped; masks count, and
/// a clipped layer only shows where its base does. A fill layer counts only within an enabled
/// mask: without one it covers the whole canvas and would put every layer below it out of
/// reach (it is chosen in the Layers panel).
pub fn layer_at(document: &Document, x: i64, y: i64) -> Option<LayerId> {
    hit(document.layers(), Projective::IDENTITY, x, y)
}

/// Every layer showing at document pixel (`x`, `y`), top to bottom: what a right-click with
/// the Move tool lists, to choose one among layers on top of each other. As [`layer_at`], but
/// fill layers count wherever they show (with or without a mask) and adjustment layers wherever
/// their mask lets them act: the list is where they are chosen from the image.
pub fn layers_at(document: &Document, x: i64, y: i64) -> Vec<LayerId> {
    let mut out = Vec::new();
    collect_at(document.layers(), Projective::IDENTITY, x, y, &mut out);
    out
}

fn collect_at(layers: &[Layer], parent: Projective, x: i64, y: i64, out: &mut Vec<LayerId>) {
    for (i, layer) in layers.iter().enumerate().rev() {
        if !shown(layer) || (layer.clipped && i > 0 && !base_shows(layers, i, parent, x, y)) {
            continue;
        }
        let transform = layer.transform.then(parent);
        match &layer.content {
            LayerContent::Group { children, .. } => {
                if mask_shows(layer, transform, x, y) {
                    collect_at(children, transform, x, y, out);
                }
            }
            LayerContent::Adjustment { .. } => {
                if mask_shows(layer, transform, x, y) {
                    out.push(layer.id);
                }
            }
            _ if covers(layer, transform, x, y) => out.push(layer.id),
            _ => {}
        }
    }
}

/// Whether the base of the clipped layer `layers[i]` (the nearest layer below it that is not
/// clipped, or the first) shows at (`x`, `y`).
fn base_shows(layers: &[Layer], i: usize, parent: Projective, x: i64, y: i64) -> bool {
    let base = layers[..i].iter().rposition(|l| !l.clipped).unwrap_or(0);
    let base = &layers[base];
    shown(base) && covers(base, base.transform.then(parent), x, y)
}

fn shown(layer: &Layer) -> bool {
    layer.visible && layer.opacity > 0.0
}

fn hit(layers: &[Layer], parent: Projective, x: i64, y: i64) -> Option<LayerId> {
    for (i, layer) in layers.iter().enumerate().rev() {
        if !shown(layer) {
            continue;
        }
        let transform = layer.transform.then(parent);
        if !mask_shows(layer, transform, x, y) {
            continue;
        }
        if layer.clipped && i > 0 && !base_shows(layers, i, parent, x, y) {
            continue;
        }
        match &layer.content {
            LayerContent::Group { children, .. } => {
                if let Some(id) = hit(children, transform, x, y) {
                    return Some(id);
                }
            }
            LayerContent::Fill { .. } | LayerContent::GradientFill { .. }
                if !layer.mask.as_ref().is_some_and(|m| m.enabled) => {}
            _ if covers(layer, transform, x, y) => return Some(layer.id),
            _ => {}
        }
    }
    None
}

/// Whether a layer (placed by `transform`) shows a pixel at (`x`, `y`), its mask included.
fn covers(layer: &Layer, transform: Projective, x: i64, y: i64) -> bool {
    if !mask_shows(layer, transform, x, y) {
        return false;
    }
    match &layer.content {
        LayerContent::Fill { color } => color.a >= PICK_COVERAGE,
        // Opaque everywhere.
        LayerContent::GradientFill { .. } => true,
        LayerContent::Raster { image, .. } => {
            let image = image.get();
            // A mask made from the layer's transparency replaces its alpha (ADR 0014).
            let replaced = layer.mask.as_ref().is_some_and(|m| m.replaces_alpha);
            let alpha = sample_alpha(&image, transform, x, y);
            if replaced {
                inside(&image, transform, x, y)
            } else {
                alpha >= PICK_COVERAGE
            }
        }
        LayerContent::Group { children, .. } => hit(children, transform, x, y).is_some(),
        // No pixels of its own: never what is under the pointer.
        LayerContent::Adjustment { .. } => false,
    }
}

/// Whether the layer's enabled mask, if any, lets (`x`, `y`) show.
fn mask_shows(layer: &Layer, transform: Projective, x: i64, y: i64) -> bool {
    match layer.mask.as_ref().filter(|m| m.enabled) {
        None => true,
        Some(mask) => sample_alpha_gray(&mask.image, transform, x, y) >= PICK_COVERAGE,
    }
}

/// The image's pixel under the center of document pixel (`x`, `y`), in its own coordinates.
fn local(image: &RasterImage, transform: Projective, x: i64, y: i64) -> Option<(u32, u32)> {
    let inverse = transform.inverse()?;
    let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
    // Beyond the horizon line of a layer in perspective, nothing of it shows (ADR 0038).
    if inverse.w(px, py) <= 0.0 {
        return None;
    }
    let (u, v) = inverse.apply(px, py);
    let (lx, ly) = (u.floor(), v.floor());
    let size = image.size();
    (lx >= 0.0 && ly >= 0.0 && lx < f64::from(size.width) && ly < f64::from(size.height))
        .then_some((lx as u32, ly as u32))
}

fn inside(image: &RasterImage, transform: Projective, x: i64, y: i64) -> bool {
    local(image, transform, x, y).is_some()
}

fn sample_alpha(image: &RasterImage, transform: Projective, x: i64, y: i64) -> f32 {
    local(image, transform, x, y).map_or(0.0, |(lx, ly)| image.alpha_at(lx, ly))
}

/// A mask's coverage: its gray value (masks have no alpha, `alpha_at` reads 1 for them).
fn sample_alpha_gray(mask: &RasterImage, transform: Projective, x: i64, y: i64) -> f32 {
    local(mask, transform, x, y).map_or(0.0, |(lx, ly)| mask.gray_at(lx, ly))
}

/// Where the pixels of each visible layer (not a group, not a fill: they have no edges) are in
/// the document, bottom to top: the targets of snapping. Masks do not change a layer's
/// bounds (Photoshop snaps to layer content); fully transparent layers are left out.
pub fn visible_layer_bounds(document: &Document) -> Vec<(LayerId, Bounds)> {
    let mut out = Vec::new();
    collect_bounds(document.layers(), Projective::IDENTITY, &mut out);
    out
}

fn collect_bounds(layers: &[Layer], parent: Projective, out: &mut Vec<(LayerId, Bounds)>) {
    for layer in layers.iter().filter(|l| shown(l)) {
        let transform = layer.transform.then(parent);
        match &layer.content {
            LayerContent::Group { children, .. } => collect_bounds(children, transform, out),
            LayerContent::Raster { image, .. } => {
                if let Some(rect) = image.get().content_bounds() {
                    out.push((layer.id, Bounds::placed(rect, transform)));
                }
            }
            LayerContent::Fill { .. }
            | LayerContent::GradientFill { .. }
            | LayerContent::Adjustment { .. } => {}
        }
    }
}

/// Where some layer has pixels that can show, hidden layers included: the pixels that are not
/// fully transparent, within their layer's enabled mask (and their groups'), in whole document
/// pixels. Fill layers count only within a mask (they have no edges); adjustment layers have
/// no pixels. What Image > Reveal All brings onto the canvas. `None`: no such pixel.
pub fn content_extent(document: &Document) -> Option<Bounds> {
    layers_extent(document.layers(), Projective::IDENTITY).within
}

/// Where layers have pixels.
#[derive(Default)]
struct Extent {
    /// Pixels with edges.
    within: Option<Bounds>,
    /// A fill, which has none (until a mask gives it some).
    everywhere: bool,
}

impl Extent {
    fn within(bounds: Bounds) -> Extent {
        Extent {
            within: Some(bounds),
            everywhere: false,
        }
    }

    fn union(self, other: Extent) -> Extent {
        Extent {
            within: match (self.within, other.within) {
                (Some(a), Some(b)) => Some(a.union(b)),
                (a, b) => a.or(b),
            },
            everywhere: self.everywhere || other.everywhere,
        }
    }
}

fn layers_extent(layers: &[Layer], parent: Projective) -> Extent {
    layers
        .iter()
        .map(|layer| layer_extent(layer, layer.transform.then(parent)))
        .fold(Extent::default(), Extent::union)
}

fn layer_extent(layer: &Layer, transform: Projective) -> Extent {
    let mask = layer.mask.as_ref().filter(|m| m.enabled);
    let own = match &layer.content {
        LayerContent::Fill { .. } | LayerContent::GradientFill { .. } => Extent {
            within: None,
            everywhere: true,
        },
        LayerContent::Adjustment { .. } => Extent::default(),
        LayerContent::Group { children, .. } => layers_extent(children, transform),
        LayerContent::Raster { image, .. } => {
            // A mask made from the layer's transparency replaces its alpha (ADR 0014).
            let rect = if mask.is_some_and(|m| m.replaces_alpha) {
                Some(image.size().bounds())
            } else {
                image.get().content_bounds()
            };
            rect.map_or(Extent::default(), |r| {
                Extent::within(Bounds::placed(r, transform))
            })
        }
    };
    let Some(mask) = mask else {
        return own;
    };
    // Outside its image, a mask hides the layer.
    let Some(shows) = mask
        .image
        .coverage_bounds()
        .map(|r| Bounds::placed(r, transform))
    else {
        return Extent::default();
    };
    if own.everywhere {
        return Extent::within(shows);
    }
    own.within
        .and_then(|b| b.intersection(shows))
        .map_or(Extent::default(), Extent::within)
}

/// The visible layers whose pixels' bounds touch `area` (document pixels), bottom to top: what
/// a rectangle drawn with the Move tool selects (as in Photoshop, by their boxes; layers, not
/// groups; fills and adjustment layers have no edges and are left out).
pub fn layers_touching(document: &Document, area: Bounds) -> Vec<LayerId> {
    visible_layer_bounds(document)
        .into_iter()
        .filter(|(_, b)| b.intersection(area).is_some())
        .map(|(id, _)| id)
        .collect()
}

/// The union of the bounds of `ids` and of everything inside those that are groups: what the
/// Move tool moves.
pub fn bounds_of(document: &Document, ids: &[LayerId]) -> Option<Bounds> {
    let wanted = document.outermost(ids);
    let mut total: Option<Bounds> = None;
    for id in wanted {
        let Some(layer) = document.layer(id) else {
            continue;
        };
        let mut found = Vec::new();
        collect_bounds(
            std::slice::from_ref(layer),
            document.parent_transform(id),
            &mut found,
        );
        for (_, b) in found {
            total = Some(total.map_or(b, |t| t.union(b)));
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::transform::{Affine, Projective};

    #[test]
    fn a_layer_in_perspective_is_picked_inside_its_quad_only() {
        let mut doc = Document::new(Size::new(60, 40));
        let image = RasterImage::from_pixels(
            Size::new(40, 20),
            crate::color::PixelFormat::RGBA8_SRGB,
            &[255; 40 * 20 * 4],
        )
        .unwrap();
        let id = doc.allocate_layer_id();
        let quad = [(20.0, 5.0), (40.0, 5.0), (55.0, 35.0), (5.0, 35.0)];
        crate::edit::Edit::InsertLayer {
            parent: None,
            index: 0,
            layer: Layer {
                style: None,
                transform: Projective::from_rect_to_quad([0.0, 0.0, 40.0, 20.0], quad).unwrap(),
                clipped: false,
                id,
                name: "quad".into(),
                visible: true,
                opacity: 1.0,
                blend_mode: crate::blend::BlendMode::Normal,
                mask: None,
                content: LayerContent::raster(Arc::new(image)),
            },
        }
        .apply(&mut doc)
        .unwrap();
        assert_eq!(layer_at(&doc, 30, 20), Some(id));
        assert_eq!(layer_at(&doc, 8, 8), None);
        assert_eq!(layer_at(&doc, 30, 1), None);
    }

    use super::*;
    use crate::blend::BlendMode;
    use crate::color::{AlphaMode, ChannelLayout, ColorSpace, LinearRgba, PixelFormat, SampleType};
    use crate::document::LayerMask;
    use crate::edit::Edit;
    use crate::geom::{Rect, Size};

    fn layer(doc: &mut Document, content: LayerContent) -> Layer {
        Layer {
            style: None,
            id: doc.allocate_layer_id(),
            name: "l".into(),
            visible: true,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            mask: None,
            clipped: false,
            transform: crate::transform::Projective::IDENTITY,
            content,
        }
    }

    fn push(doc: &mut Document, layer: Layer) -> LayerId {
        let id = layer.id;
        let index = doc.layers().len();
        Edit::InsertLayer {
            parent: None,
            index,
            layer,
        }
        .apply(doc)
        .unwrap();
        id
    }

    /// A 20 × 20 canvas with an opaque square at `rect`, transparent elsewhere.
    fn square(rect: Rect) -> LayerContent {
        let format = PixelFormat {
            layout: ChannelLayout::Rgba,
            sample: SampleType::U8,
            color_space: ColorSpace::SRGB,
            alpha: AlphaMode::Straight,
        };
        let pixels = [200u8, 10, 10, 255].repeat(rect.size().pixel_count() as usize);
        let image =
            RasterImage::from_placed(Size::new(20, 20), format, rect, &pixels, &[0; 4]).unwrap();
        LayerContent::Raster {
            stack: None,
            image: crate::stack::Pixels::ready(Arc::new(image)),
        }
    }

    #[test]
    fn the_topmost_layer_with_a_pixel_there_is_picked() {
        let mut doc = Document::new(Size::new(20, 20));
        let fill = layer(
            &mut doc,
            LayerContent::Fill {
                color: LinearRgba::new(0.1, 0.1, 0.1, 1.0),
            },
        );
        let fill = push(&mut doc, fill);
        let low = layer(&mut doc, square(Rect::new(2, 2, 10, 10)));
        let low = push(&mut doc, low);
        let high = layer(&mut doc, square(Rect::new(8, 8, 10, 10)));
        let high = push(&mut doc, high);
        assert_eq!(layer_at(&doc, 9, 9), Some(high));
        assert_eq!(layer_at(&doc, 3, 3), Some(low));
        assert_eq!(
            layer_at(&doc, 0, 19),
            None,
            "transparent above a fill without a mask: not the fill"
        );

        // Hidden: skipped. Moved: picked where it went.
        Edit::SetLayerVisible {
            id: high,
            visible: false,
        }
        .apply(&mut doc)
        .unwrap();
        assert_eq!(layer_at(&doc, 15, 15), None);
        Edit::SetLayerTransform {
            id: low,
            transform: Affine::translation(6.0, 6.0).into(),
        }
        .apply(&mut doc)
        .unwrap();
        assert_eq!(layer_at(&doc, 3, 3), None);
        assert_eq!(layer_at(&doc, 16, 16), Some(low));

        // A mask hiding the layer there: the pick goes through.
        let gray = PixelFormat {
            layout: ChannelLayout::Gray,
            sample: SampleType::U8,
            color_space: ColorSpace::LINEAR_SRGB,
            alpha: AlphaMode::Straight,
        };
        let mask = RasterImage::from_placed(
            Size::new(20, 20),
            gray,
            Rect::new(0, 0, 10, 20),
            &[255u8; 200],
            &[0],
        )
        .unwrap();
        Edit::SetLayerMask {
            id: low,
            mask: Some(LayerMask {
                original: None,
                image: Arc::new(mask),
                enabled: true,
                replaces_alpha: false,
            }),
        }
        .apply(&mut doc)
        .unwrap();
        // The mask moves with the layer: it shows (6..16) × (6..26) of the document.
        assert_eq!(layer_at(&doc, 12, 12), Some(low));
        assert_eq!(layer_at(&doc, 17, 12), None);

        // A fill within its mask: picked where the mask shows it.
        let fill_mask = RasterImage::from_placed(
            Size::new(20, 20),
            gray,
            Rect::new(0, 0, 4, 20),
            &[255u8; 80],
            &[0],
        )
        .unwrap();
        Edit::SetLayerMask {
            id: fill,
            mask: Some(LayerMask {
                original: None,
                image: Arc::new(fill_mask),
                enabled: true,
                replaces_alpha: false,
            }),
        }
        .apply(&mut doc)
        .unwrap();
        assert_eq!(layer_at(&doc, 1, 19), Some(fill));
        assert_eq!(layer_at(&doc, 5, 19), None);
    }

    #[test]
    fn clipped_layers_and_groups_are_picked_where_they_show() {
        let mut doc = Document::new(Size::new(20, 20));
        let base = layer(&mut doc, square(Rect::new(0, 0, 10, 20)));
        push(&mut doc, base);
        let mut clipped = layer(&mut doc, square(Rect::new(0, 0, 20, 20)));
        clipped.clipped = true;
        let clipped = push(&mut doc, clipped);
        // Where the base shows, the clipped layer is on top; elsewhere it does not show.
        assert_eq!(layer_at(&doc, 5, 5), Some(clipped));
        assert_eq!(layer_at(&doc, 15, 5), None);

        let inner = layer(&mut doc, square(Rect::new(12, 12, 4, 4)));
        let inner_id = inner.id;
        let mut group = layer(
            &mut doc,
            LayerContent::Group {
                children: vec![inner],
                pass_through: true,
            },
        );
        group.transform = Affine::translation(2.0, 0.0).into();
        push(&mut doc, group);
        // Inside a group: the layer, not the group, placed by both transforms.
        assert_eq!(layer_at(&doc, 15, 13), Some(inner_id));
        assert_eq!(layer_at(&doc, 12, 13), None);
    }

    #[test]
    fn every_layer_showing_there_is_listed_top_to_bottom() {
        let mut doc = Document::new(Size::new(20, 20));
        let fill = layer(
            &mut doc,
            LayerContent::Fill {
                color: LinearRgba::new(0.1, 0.1, 0.1, 1.0),
            },
        );
        let fill = push(&mut doc, fill);
        let low = layer(&mut doc, square(Rect::new(2, 2, 10, 10)));
        let low = push(&mut doc, low);
        let mut clipped = layer(&mut doc, square(Rect::new(0, 0, 20, 20)));
        clipped.clipped = true;
        let clipped = push(&mut doc, clipped);
        let mut adjust = layer(
            &mut doc,
            LayerContent::Adjustment {
                adjustment: crate::adjust::Adjustment::Invert,
            },
        );
        adjust.mask = Some(mask_showing(Rect::new(0, 0, 5, 20)));
        let adjust = push(&mut doc, adjust);
        let mut hidden = layer(&mut doc, square(Rect::new(0, 0, 20, 20)));
        hidden.visible = false;
        push(&mut doc, hidden);
        let inner = layer(&mut doc, square(Rect::new(0, 0, 20, 4)));
        let inner_id = inner.id;
        let group = layer(
            &mut doc,
            LayerContent::Group {
                children: vec![inner],
                pass_through: true,
            },
        );
        push(&mut doc, group);

        // The fill (not picked by a click) and the adjustment within its mask are listed; the
        // clipped layer where its base shows; inside a group, the layer; never a hidden one.
        assert_eq!(
            layers_at(&doc, 3, 3),
            vec![inner_id, adjust, clipped, low, fill]
        );
        assert_eq!(layers_at(&doc, 8, 8), vec![clipped, low, fill]);
        assert_eq!(layers_at(&doc, 15, 15), vec![fill]);
        assert_eq!(layer_at(&doc, 15, 15), None);
    }

    #[test]
    fn a_rectangle_touches_the_layers_whose_pixels_it_reaches() {
        let mut doc = Document::new(Size::new(20, 20));
        let a = layer(&mut doc, square(Rect::new(0, 0, 4, 4)));
        let a = push(&mut doc, a);
        let b = layer(&mut doc, square(Rect::new(10, 10, 4, 4)));
        let b = push(&mut doc, b);
        let mut hidden = layer(&mut doc, square(Rect::new(0, 0, 20, 20)));
        hidden.visible = false;
        push(&mut doc, hidden);
        let area = |left, top, right, bottom| Bounds {
            left,
            top,
            right,
            bottom,
        };
        assert_eq!(layers_touching(&doc, area(3, 3, 11, 11)), vec![a, b]);
        assert_eq!(layers_touching(&doc, area(5, 5, 9, 9)), vec![]);
        assert_eq!(layers_touching(&doc, area(12, 0, 30, 11)), vec![b]);
        assert_eq!(
            layers_touching(&doc, area(4, 0, 10, 20)),
            vec![],
            "edges are exclusive"
        );
    }

    #[test]
    fn bounds_are_the_visible_pixels_placed_in_the_document() {
        let mut doc = Document::new(Size::new(20, 20));
        let a = layer(&mut doc, square(Rect::new(3, 4, 5, 6)));
        let a = push(&mut doc, a);
        let b = layer(&mut doc, square(Rect::new(10, 10, 2, 2)));
        let b = push(&mut doc, b);
        Edit::SetLayerTransform {
            id: b,
            transform: Affine::translation(-4.0, 1.0).into(),
        }
        .apply(&mut doc)
        .unwrap();
        let all = visible_layer_bounds(&doc);
        assert_eq!(
            all,
            [
                (
                    a,
                    Bounds {
                        left: 3,
                        top: 4,
                        right: 8,
                        bottom: 10
                    }
                ),
                (
                    b,
                    Bounds {
                        left: 6,
                        top: 11,
                        right: 8,
                        bottom: 13
                    }
                ),
            ]
        );
        assert_eq!(
            bounds_of(&doc, &[a, b]),
            Some(Bounds {
                left: 3,
                top: 4,
                right: 8,
                bottom: 13
            })
        );
    }

    #[test]
    fn scaled_and_rotated_layers_are_picked_and_bounded_where_they_show() {
        let mut doc = Document::new(Size::new(20, 20));
        let a = layer(&mut doc, square(Rect::new(0, 0, 4, 2)));
        let a = push(&mut doc, a);
        // A quarter turn then × 2, moved right: the 4 × 2 square covers x ∈ [6, 10), y ∈ [0, 8).
        let turn = Affine {
            a: 0.0,
            b: 1.0,
            c: -1.0,
            d: 0.0,
            e: 0.0,
            f: 0.0,
        };
        Edit::SetLayerTransform {
            id: a,
            transform: Projective::from(
                turn.then(Affine::scale(2.0, 2.0))
                    .then(Affine::translation(10.0, 0.0)),
            ),
        }
        .apply(&mut doc)
        .unwrap();
        assert_eq!(layer_at(&doc, 7, 7), Some(a));
        assert_eq!(layer_at(&doc, 5, 3), None);
        assert_eq!(layer_at(&doc, 7, 8), None);
        assert_eq!(
            bounds_of(&doc, &[a]),
            Some(Bounds {
                left: 6,
                top: 0,
                right: 10,
                bottom: 8
            })
        );
    }

    /// A gray mask of a 20 × 20 layer showing `rect` only.
    fn mask_showing(rect: Rect) -> LayerMask {
        let gray = PixelFormat {
            layout: ChannelLayout::Gray,
            sample: SampleType::U8,
            color_space: ColorSpace::LINEAR_SRGB,
            alpha: AlphaMode::Straight,
        };
        let pixels = vec![255u8; rect.size().pixel_count() as usize];
        let image = RasterImage::from_placed(Size::new(20, 20), gray, rect, &pixels, &[0]).unwrap();
        LayerMask {
            original: None,
            image: Arc::new(image),
            enabled: true,
            replaces_alpha: false,
        }
    }

    #[test]
    fn the_content_extent_holds_every_pixel_that_can_show() {
        let mut doc = Document::new(Size::new(20, 20));
        assert_eq!(content_extent(&doc), None);
        // A fill has no edges: it does not count, and does not hide the others.
        let fill = layer(
            &mut doc,
            LayerContent::Fill {
                color: LinearRgba::new(1.0, 1.0, 1.0, 1.0),
            },
        );
        push(&mut doc, fill);
        assert_eq!(content_extent(&doc), None);
        // Moved partly off the canvas, and hidden: it counts.
        let mut moved = layer(&mut doc, square(Rect::new(2, 2, 10, 10)));
        moved.transform = Affine::translation(-7.0, 15.0).into();
        moved.visible = false;
        push(&mut doc, moved);
        let extent = |left, top, right, bottom| {
            Some(Bounds {
                left,
                top,
                right,
                bottom,
            })
        };
        assert_eq!(content_extent(&doc), extent(-5, 17, 5, 27));
        // A mask hides part of a layer: only what it shows counts.
        let mut masked = layer(&mut doc, square(Rect::new(0, 0, 20, 20)));
        masked.transform = Affine::translation(10.0, -10.0).into();
        masked.mask = Some(mask_showing(Rect::new(5, 5, 5, 5)));
        let masked = push(&mut doc, masked);
        assert_eq!(content_extent(&doc), extent(-5, -5, 20, 27));
        // A disabled mask hides nothing.
        Edit::SetLayerMaskEnabled {
            id: masked,
            enabled: false,
        }
        .apply(&mut doc)
        .unwrap();
        assert_eq!(content_extent(&doc), extent(-5, -10, 30, 27));
        // A fill inside a masked group shows within the mask.
        let inner = layer(
            &mut doc,
            LayerContent::Fill {
                color: LinearRgba::new(0.0, 0.0, 0.0, 1.0),
            },
        );
        let mut group = layer(
            &mut doc,
            LayerContent::Group {
                children: vec![inner],
                pass_through: false,
            },
        );
        group.mask = Some(mask_showing(Rect::new(0, 0, 1, 1)));
        group.transform = Affine::translation(-40.0, 0.0).into();
        push(&mut doc, group);
        assert_eq!(content_extent(&doc), extent(-40, -10, 30, 27));
    }
}
