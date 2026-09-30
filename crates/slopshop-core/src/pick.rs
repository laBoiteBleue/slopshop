//! Finding layers from the image: the layer under a point (the Move tool's Auto-Select, as in
//! Photoshop) and where each layer's pixels are (snapping while moving, ADR 0017).

use crate::document::{Document, Layer, LayerContent, LayerId};
use crate::raster::RasterImage;
use crate::transform::Affine;

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
/// a clipped layer only shows where its base does.
pub fn layer_at(document: &Document, x: i64, y: i64) -> Option<LayerId> {
    hit(document.layers(), Affine::IDENTITY, x, y)
}

fn shown(layer: &Layer) -> bool {
    layer.visible && layer.opacity > 0.0
}

fn hit(layers: &[Layer], parent: Affine, x: i64, y: i64) -> Option<LayerId> {
    for (i, layer) in layers.iter().enumerate().rev() {
        if !shown(layer) {
            continue;
        }
        let transform = layer.transform.then(parent);
        if !mask_shows(layer, transform, x, y) {
            continue;
        }
        if layer.clipped && i > 0 {
            // Its base: the nearest layer below that is not clipped (or the first).
            let base = layers[..i].iter().rposition(|l| !l.clipped).unwrap_or(0);
            let base = &layers[base];
            let base_transform = base.transform.then(parent);
            if !shown(base) || !covers(base, base_transform, x, y) {
                continue;
            }
        }
        match &layer.content {
            LayerContent::Group { children, .. } => {
                if let Some(id) = hit(children, transform, x, y) {
                    return Some(id);
                }
            }
            _ if covers(layer, transform, x, y) => return Some(layer.id),
            _ => {}
        }
    }
    None
}

/// Whether a layer (placed by `transform`) shows a pixel at (`x`, `y`), its mask included.
fn covers(layer: &Layer, transform: Affine, x: i64, y: i64) -> bool {
    if !mask_shows(layer, transform, x, y) {
        return false;
    }
    match &layer.content {
        LayerContent::Fill { color } => color.a >= PICK_COVERAGE,
        LayerContent::Raster { image } => {
            // A mask made from the layer's transparency replaces its alpha (ADR 0014).
            let replaced = layer.mask.as_ref().is_some_and(|m| m.replaces_alpha);
            let alpha = sample_alpha(image, transform, x, y);
            if replaced {
                inside(image, transform, x, y)
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
fn mask_shows(layer: &Layer, transform: Affine, x: i64, y: i64) -> bool {
    match layer.mask.as_ref().filter(|m| m.enabled) {
        None => true,
        Some(mask) => sample_alpha_gray(&mask.image, transform, x, y) >= PICK_COVERAGE,
    }
}

/// The image's pixel under the center of document pixel (`x`, `y`), in its own coordinates.
fn local(image: &RasterImage, transform: Affine, x: i64, y: i64) -> Option<(u32, u32)> {
    let (u, v) = transform.inverse()?.apply(x as f64 + 0.5, y as f64 + 0.5);
    let (lx, ly) = (u.floor(), v.floor());
    let size = image.size();
    (lx >= 0.0 && ly >= 0.0 && lx < f64::from(size.width) && ly < f64::from(size.height))
        .then_some((lx as u32, ly as u32))
}

fn inside(image: &RasterImage, transform: Affine, x: i64, y: i64) -> bool {
    local(image, transform, x, y).is_some()
}

fn sample_alpha(image: &RasterImage, transform: Affine, x: i64, y: i64) -> f32 {
    local(image, transform, x, y).map_or(0.0, |(lx, ly)| image.alpha_at(lx, ly))
}

/// A mask's coverage: its gray value (masks have no alpha, `alpha_at` reads 1 for them).
fn sample_alpha_gray(mask: &RasterImage, transform: Affine, x: i64, y: i64) -> f32 {
    local(mask, transform, x, y).map_or(0.0, |(lx, ly)| mask.gray_at(lx, ly))
}

/// Where the pixels of each visible layer (not a group, not a fill: they have no edges) are in
/// the document, bottom to top: the targets of snapping. Masks do not change a layer's
/// bounds (Photoshop snaps to layer content); fully transparent layers are left out.
pub fn visible_layer_bounds(document: &Document) -> Vec<(LayerId, Bounds)> {
    let mut out = Vec::new();
    collect_bounds(document.layers(), Affine::IDENTITY, &mut out);
    out
}

fn collect_bounds(layers: &[Layer], parent: Affine, out: &mut Vec<(LayerId, Bounds)>) {
    for layer in layers.iter().filter(|l| shown(l)) {
        let transform = layer.transform.then(parent);
        match &layer.content {
            LayerContent::Group { children, .. } => collect_bounds(children, transform, out),
            LayerContent::Raster { image } => {
                if let Some(rect) = image.content_bounds() {
                    // The box around the transformed content, in whole pixels.
                    let [x0, y0, x1, y1] = transform.map_rect([
                        f64::from(rect.x),
                        f64::from(rect.y),
                        rect.right() as f64,
                        rect.bottom() as f64,
                    ]);
                    out.push((
                        layer.id,
                        Bounds {
                            left: x0.floor() as i64,
                            top: y0.floor() as i64,
                            right: x1.ceil() as i64,
                            bottom: y1.ceil() as i64,
                        },
                    ));
                }
            }
            LayerContent::Fill { .. } | LayerContent::Adjustment { .. } => {}
        }
    }
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

    use super::*;
    use crate::blend::BlendMode;
    use crate::color::{AlphaMode, ChannelLayout, ColorSpace, LinearRgba, PixelFormat, SampleType};
    use crate::document::LayerMask;
    use crate::edit::Edit;
    use crate::geom::{Rect, Size};

    fn layer(doc: &mut Document, content: LayerContent) -> Layer {
        Layer {
            id: doc.allocate_layer_id(),
            name: "l".into(),
            visible: true,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            mask: None,
            clipped: false,
            transform: Affine::IDENTITY,
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
            image: Arc::new(image),
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
            Some(fill),
            "transparent above the fill"
        );
        assert_eq!(
            layer_at(&doc, -5, 3),
            Some(fill),
            "a fill covers everything"
        );

        // Hidden: skipped. Moved: picked where it went.
        Edit::SetLayerVisible {
            id: high,
            visible: false,
        }
        .apply(&mut doc)
        .unwrap();
        assert_eq!(layer_at(&doc, 15, 15), Some(fill));
        Edit::SetLayerTransform {
            id: low,
            transform: Affine::translation(6.0, 6.0),
        }
        .apply(&mut doc)
        .unwrap();
        assert_eq!(layer_at(&doc, 3, 3), Some(fill));
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
                image: Arc::new(mask),
                enabled: true,
                replaces_alpha: false,
            }),
        }
        .apply(&mut doc)
        .unwrap();
        // The mask moves with the layer: it shows (6..16) × (6..26) of the document.
        assert_eq!(layer_at(&doc, 12, 12), Some(low));
        assert_eq!(layer_at(&doc, 17, 12), Some(fill));
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
        group.transform = Affine::translation(2.0, 0.0);
        push(&mut doc, group);
        // Inside a group: the layer, not the group, placed by both transforms.
        assert_eq!(layer_at(&doc, 15, 13), Some(inner_id));
        assert_eq!(layer_at(&doc, 12, 13), None);
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
            transform: Affine::translation(-4.0, 1.0),
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
            transform: turn
                .then(Affine::scale(2.0, 2.0))
                .then(Affine::translation(10.0, 0.0)),
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
}
