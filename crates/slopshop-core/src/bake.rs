//! Layer > Bake to Pixels (ADR 0030): the commands that turn what stays editable into plain
//! pixels, on purpose. Rasterize keeps each layer (its transform, opacity, blend mode, mask and
//! clipping) but bakes its content: a layer's stack, a fill's color, a group's layers. Merge
//! Layers, Merge Down, Merge Visible and Flatten Image replace layers by the one pixel layer
//! they composite into.
//!
//! The pixels are composited by the caller (on the GPU when there is one): a [`BakePlan`] holds
//! a scratch document whose whole canvas is what to composite, and [`BakePlan::finish`] turns
//! the composited image into the edit, one undo entry. Raster layers whose stack is baked need
//! no compositing ([`rasterize_in_place`]).

use std::sync::Arc;

use crate::blend::BlendMode;
use crate::document::{Document, Layer, LayerContent, LayerId, LayerMask};
use crate::edit::{Edit, EditError};
use crate::geom::Size;
use crate::pick::{self, Bounds};
use crate::raster::{RasterImage, TILE_SIZE};
use crate::transform::Affine;

/// A composite to make, and where its result goes.
#[derive(Debug, Clone)]
pub struct BakePlan {
    /// What to composite: its whole canvas, in its blend space.
    pub scratch: Document,
    target: Target,
}

#[derive(Debug, Clone)]
enum Target {
    /// Rasterize `id` (a fill or a group): its content becomes the image, placed `shift` whole
    /// tiles (columns, rows) further from its content space's origin, its mask grown alike.
    Layer { id: LayerId, shift: (u32, u32) },
    /// Merge: `removed` go, the image comes as a layer named `name` at `index` among the layers
    /// of `parent` (once they are gone), its pixels at `at` (document pixels).
    Merge {
        removed: Vec<LayerId>,
        parent: Option<LayerId>,
        index: usize,
        name: String,
        at: (i64, i64),
    },
}

impl BakePlan {
    /// The edit that puts `image` (the scratch canvas composited, of its size) in place; a
    /// merge's layer takes the id `new_id` (allocated by the document).
    pub fn finish(
        self,
        doc: &Document,
        image: Arc<RasterImage>,
        new_id: LayerId,
    ) -> Result<Edit, EditError> {
        if image.size() != self.scratch.size() {
            return Err(EditError::InvalidPaint);
        }
        match self.target {
            Target::Layer { id, shift } => {
                let layer = doc.layer(id).ok_or(EditError::UnknownLayer(id))?;
                let mut baked = layer.clone();
                baked.content = LayerContent::raster(image);
                baked.transform = shifted(layer.transform, shift);
                baked.mask = match &layer.mask {
                    Some(mask) => Some(grown_mask(mask, shift)?),
                    None => None,
                };
                replace(doc, baked)
            }
            Target::Merge {
                removed,
                parent,
                index,
                name,
                at,
            } => {
                let to_parent = match parent {
                    Some(p) => doc
                        .layer(p)
                        .map(|l| l.transform.then(doc.parent_transform(p)))
                        .ok_or(EditError::UnknownLayer(p))?
                        .inverse()
                        .ok_or(EditError::InvalidTransform)?,
                    None => Affine::IDENTITY,
                };
                let layer = Layer {
                    id: new_id,
                    name,
                    visible: true,
                    opacity: 1.0,
                    blend_mode: BlendMode::Normal,
                    content: LayerContent::raster(image),
                    mask: None,
                    clipped: false,
                    transform: Affine::translation(at.0 as f64, at.1 as f64).then(to_parent),
                };
                let mut edits: Vec<Edit> = removed
                    .into_iter()
                    .map(|id| Edit::RemoveLayer { id })
                    .collect();
                edits.push(Edit::InsertLayer {
                    parent,
                    index,
                    layer,
                });
                Ok(Edit::Batch(edits))
            }
        }
    }
}

/// `layer` in the place of the layer of its id (same id, same place): one remove and one insert.
fn replace(doc: &Document, layer: Layer) -> Result<Edit, EditError> {
    let (parent, index) = doc
        .locate(layer.id)
        .ok_or(EditError::UnknownLayer(layer.id))?;
    Ok(Edit::Batch(vec![
        Edit::RemoveLayer { id: layer.id },
        Edit::InsertLayer {
            parent,
            index,
            layer,
        },
    ]))
}

/// A content space moved `shift` whole tiles toward negative coordinates: what was at the
/// origin is now at `shift` tiles.
fn shifted(transform: Affine, (cols, rows): (u32, u32)) -> Affine {
    let t = f64::from(TILE_SIZE);
    Affine::translation(-f64::from(cols) * t, -f64::from(rows) * t).then(transform)
}

/// `mask` in a content space shifted by `shift` tiles (its tiles shared), its paint baked.
fn grown_mask(mask: &LayerMask, shift: (u32, u32)) -> Result<LayerMask, EditError> {
    let image = if shift == (0, 0) {
        Arc::clone(&mask.image)
    } else {
        let size = mask.image.size();
        let grown = Size::new(
            size.width + shift.0 * TILE_SIZE,
            size.height + shift.1 * TILE_SIZE,
        );
        Arc::new(
            mask.image
                .grown(shift, grown)
                .ok_or(EditError::InvalidMask)?
                .map_err(|_| EditError::InvalidMask)?,
        )
    };
    Ok(LayerMask {
        image,
        enabled: mask.enabled,
        replaces_alpha: mask.replaces_alpha,
        original: None,
    })
}

/// Rasterize applies to `layer`: a fill, a group, or a pixel layer carrying a stack or a
/// painted mask.
pub fn can_rasterize(layer: &Layer) -> bool {
    match &layer.content {
        LayerContent::Fill { .. } | LayerContent::Group { .. } => true,
        LayerContent::Raster { .. } => layer.is_painted(),
        LayerContent::Adjustment { .. } => false,
    }
}

/// Rasterize, for the pixel layers among `ids` (outermost) that carry a stack or a painted
/// mask: what they show becomes their original, their entries gone. Evaluates their pixels:
/// not on the UI thread.
pub fn rasterize_in_place(doc: &Document, ids: &[LayerId]) -> Result<Vec<Edit>, EditError> {
    let mut edits = Vec::new();
    for id in doc.outermost(ids) {
        let layer = doc.layer(id).ok_or(EditError::UnknownLayer(id))?;
        if !matches!(layer.content, LayerContent::Raster { .. }) || !layer.is_painted() {
            continue;
        }
        let mut baked = layer.clone();
        if let Some(pixels) = layer.content.pixels() {
            baked.content = LayerContent::raster(pixels);
        }
        if let Some(mask) = &layer.mask {
            baked.mask = Some(grown_mask(mask, (0, 0))?);
        }
        edits.push(replace(doc, baked)?);
    }
    Ok(edits)
}

/// Rasterize, for the fills and the groups among `ids` (outermost): one composite each.
pub fn rasterize_plans(doc: &Document, ids: &[LayerId]) -> Result<Vec<BakePlan>, EditError> {
    let mut plans = Vec::new();
    for id in doc.outermost(ids) {
        let layer = doc.layer(id).ok_or(EditError::UnknownLayer(id))?;
        let content: Vec<Layer> = match &layer.content {
            // The fill alone, plain: its opacity, mode, mask and clipping stay the layer's.
            LayerContent::Fill { .. } => vec![Layer {
                opacity: 1.0,
                blend_mode: BlendMode::Normal,
                mask: None,
                clipped: false,
                transform: Affine::IDENTITY,
                ..layer.clone()
            }],
            // Its layers, in the group's content space, composited on their own.
            LayerContent::Group { children, .. } => children.clone(),
            _ => continue,
        };
        // Where the content shows, in the layer's content space: a fill wherever the canvas
        // is (it covers everything).
        let to_document = layer.transform.then(doc.parent_transform(id));
        let canvas = to_document
            .inverse()
            .map(|to_content| canvas_bounds(doc, to_content))
            .ok_or(EditError::InvalidTransform)?;
        let region = content_bounds(doc, &content, canvas)?;
        let Some(region) = region else { continue };
        // Whole tiles before the origin when the content starts before it.
        let tiles = |v: i64| (v.min(0).unsigned_abs()).div_ceil(u64::from(TILE_SIZE)) as u32;
        let shift = (tiles(region.left), tiles(region.top));
        let t = i64::from(TILE_SIZE);
        let (ox, oy) = (i64::from(shift.0) * t, i64::from(shift.1) * t);
        // The image covers the new content space from its origin to the content's far edge.
        let size = size_of(region.right + ox, region.bottom + oy)?;
        let moved = Affine::translation(ox as f64, oy as f64);
        let scratch = scratch(doc, size, content, moved)?;
        plans.push(BakePlan {
            scratch,
            target: Target::Layer { id, shift },
        });
    }
    Ok(plans)
}

/// Merge `ids` (Merge Layers): the layers (a group with its layers) composited on their own, as
/// they show, into one pixel layer in the place of the topmost, named after it; hidden ones
/// are dropped. `None` without any of them.
pub fn merge_plan(doc: &Document, ids: &[LayerId]) -> Result<Option<BakePlan>, EditError> {
    let topmost = crate::edit::outermost_in_order(doc, ids)?.last().copied();
    let Some(topmost) = topmost else {
        return Ok(None);
    };
    let name = doc
        .layer(topmost)
        .map(|l| l.name.clone())
        .unwrap_or_default();
    merge_named(doc, ids, topmost, name)
}

/// Merge Down: `id` with the layer right below it in its group, named after that one. `None`
/// when there is none, or when it is hidden (as in Photoshop).
pub fn merge_down_plan(doc: &Document, id: LayerId) -> Result<Option<BakePlan>, EditError> {
    let Some(below) = layer_below(doc, id) else {
        return Ok(None);
    };
    let name = below.name.clone();
    let below = below.id;
    merge_named(doc, &[below, id], id, name)
}

/// The layer right below `id` in its group, when visible.
pub fn layer_below(doc: &Document, id: LayerId) -> Option<&Layer> {
    let (parent, index) = doc.locate(id)?;
    let below = doc.children_of(parent)?.get(index.checked_sub(1)?)?;
    below.visible.then_some(below)
}

/// Merge Visible: every visible layer of the top level into one (named after the topmost of
/// them, in its place); hidden ones stay. `None` without any.
pub fn merge_visible_plan(doc: &Document) -> Result<Option<BakePlan>, EditError> {
    let ids: Vec<LayerId> = doc
        .layers()
        .iter()
        .filter(|l| l.visible)
        .map(|l| l.id)
        .collect();
    merge_plan(doc, &ids)
}

/// Flatten Image: every layer into one named `name`, hidden ones dropped, transparency kept
/// (no background is added). `None` for a document without layers.
pub fn flatten_plan(doc: &Document, name: String) -> Result<Option<BakePlan>, EditError> {
    let ids: Vec<LayerId> = doc.layers().iter().map(|l| l.id).collect();
    let Some(&topmost) = ids.last() else {
        return Ok(None);
    };
    merge_named(doc, &ids, topmost, name)
}

fn merge_named(
    doc: &Document,
    ids: &[LayerId],
    topmost: LayerId,
    name: String,
) -> Result<Option<BakePlan>, EditError> {
    let removed = crate::edit::outermost_in_order(doc, ids)?;
    if removed.is_empty() {
        return Ok(None);
    }
    let (parent, at) = doc
        .locate(topmost)
        .ok_or(EditError::UnknownLayer(topmost))?;
    // Its place once the others are gone: the layers below it that stay.
    let index = doc
        .children_of(parent)
        .ok_or(EditError::UnknownLayer(topmost))?
        .iter()
        .take(at)
        .filter(|l| !removed.contains(&l.id))
        .count();
    // Each layer in the document's space, clipped only to a base that is merged too.
    let mut content = Vec::with_capacity(removed.len());
    for &id in &removed {
        let mut layer = doc.layer(id).ok_or(EditError::UnknownLayer(id))?.clone();
        layer.transform = layer.transform.then(doc.parent_transform(id));
        layer.clipped = layer.clipped && base(doc, id).is_some_and(|b| removed.contains(&b));
        content.push(layer);
    }
    let canvas = canvas_bounds(doc, Affine::IDENTITY);
    let region = content_bounds(doc, &content, canvas)?.unwrap_or(canvas);
    let size = size_of(region.right - region.left, region.bottom - region.top)?;
    let moved = Affine::translation(-region.left as f64, -region.top as f64);
    let scratch = scratch(doc, size, content, moved)?;
    Ok(Some(BakePlan {
        scratch,
        target: Target::Merge {
            removed,
            parent,
            index,
            name,
            at: (region.left, region.top),
        },
    }))
}

/// The base a clipped layer `id` is clipped to: the nearest layer below it in its group that
/// is not clipped.
fn base(doc: &Document, id: LayerId) -> Option<LayerId> {
    let (parent, index) = doc.locate(id)?;
    doc.children_of(parent)?[..index]
        .iter()
        .rev()
        .find(|l| !l.clipped)
        .map(|l| l.id)
}

/// The canvas in a space whose map to the document's is the inverse of `to_space`'s: the box
/// around it, in whole pixels.
fn canvas_bounds(doc: &Document, to_space: Affine) -> Bounds {
    let size = doc.size();
    let [x0, y0, x1, y1] =
        to_space.map_rect([0.0, 0.0, f64::from(size.width), f64::from(size.height)]);
    Bounds {
        left: x0.floor() as i64,
        top: y0.floor() as i64,
        right: x1.ceil() as i64,
        bottom: y1.ceil() as i64,
    }
}

/// Where `layers` (in some space) show: the box around their visible pixels, and `canvas`
/// (the canvas in that space) wherever they hold a visible fill, which covers everything.
fn content_bounds(
    doc: &Document,
    layers: &[Layer],
    canvas: Bounds,
) -> Result<Option<Bounds>, EditError> {
    let probe = Document::restore(
        doc.size(),
        doc.working_space(),
        doc.blend_space(),
        layers.to_vec(),
        doc.next_layer_id(),
    )
    .map_err(|_| EditError::NoLayers)?;
    let mut boxes: Vec<Bounds> = pick::visible_layer_bounds(&probe)
        .into_iter()
        .map(|(_, b)| b)
        .collect();
    if shows_fill(layers) {
        boxes.push(canvas);
    }
    Ok(boxes.into_iter().reduce(|a, b| Bounds {
        left: a.left.min(b.left),
        top: a.top.min(b.top),
        right: a.right.max(b.right),
        bottom: a.bottom.max(b.bottom),
    }))
}

/// Some of `layers` is a fill seen through visible groups.
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

/// A size from whole pixels, at least one pixel each way.
fn size_of(width: i64, height: i64) -> Result<Size, EditError> {
    let side = |v: i64| u32::try_from(v.max(1)).map_err(|_| EditError::EmptyCanvas);
    Ok(Size::new(side(width)?, side(height)?))
}

/// A document of `size` holding `layers` moved by `moved`, to composite whole.
fn scratch(
    doc: &Document,
    size: Size,
    layers: Vec<Layer>,
    moved: Affine,
) -> Result<Document, EditError> {
    let layers = layers
        .into_iter()
        .map(|mut l| {
            l.transform = l.transform.then(moved);
            l
        })
        .collect();
    Document::restore(
        size,
        doc.working_space(),
        doc.blend_space(),
        layers,
        doc.next_layer_id(),
    )
    .map_err(|_| EditError::NoLayers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::{AlphaMode, ChannelLayout, ColorSpace, LinearRgba, PixelFormat, SampleType};
    use crate::geom::Rect;

    const RGBA8: PixelFormat = PixelFormat {
        layout: ChannelLayout::Rgba,
        sample: SampleType::U8,
        color_space: ColorSpace::SRGB,
        alpha: AlphaMode::Straight,
    };

    fn plain(doc: &mut Document, name: &str, content: LayerContent) -> Layer {
        Layer {
            id: doc.allocate_layer_id(),
            name: name.into(),
            visible: true,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            mask: None,
            clipped: false,
            transform: Affine::IDENTITY,
            content,
        }
    }

    /// An opaque box at `rect` of a 100 x 100 image.
    fn boxed(rect: Rect) -> LayerContent {
        let pixels = [200u8, 10, 10, 255].repeat(rect.size().pixel_count() as usize);
        let image =
            RasterImage::from_placed(Size::new(100, 100), RGBA8, rect, &pixels, &[0; 4]).unwrap();
        LayerContent::raster(Arc::new(image))
    }

    fn fill() -> LayerContent {
        LayerContent::Fill {
            color: LinearRgba::new(0.0, 0.0, 1.0, 1.0),
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

    /// The plan composited on the CPU, as the app does on the GPU.
    fn composited(plan: &BakePlan) -> Arc<RasterImage> {
        let size = plan.scratch.size();
        let mut pixels = vec![0.0f32; size.pixel_count() as usize * 4];
        crate::composite::composite_region(
            &plan.scratch,
            Rect::new(0, 0, size.width, size.height),
            &mut pixels,
        )
        .unwrap();
        Arc::new(crate::copy::merged_image(&pixels, size, plan.scratch.blend_space()).unwrap())
    }

    /// `plan` finished and applied; its undo.
    fn bake(doc: &mut Document, plan: BakePlan) -> Edit {
        let image = composited(&plan);
        let id = doc.allocate_layer_id();
        plan.finish(doc, image, id).unwrap().apply(doc).unwrap()
    }

    fn names(doc: &Document) -> Vec<String> {
        doc.layers().iter().map(|l| l.name.clone()).collect()
    }

    /// The name of the layer showing a pixel at (`x`, `y`), if any.
    fn shown(doc: &Document, x: i64, y: i64) -> Option<String> {
        pick::layer_at(doc, x, y).map(|id| doc.layer(id).unwrap().name.clone())
    }

    #[test]
    fn merging_composites_the_layers_into_one_in_the_place_of_the_topmost() {
        let mut doc = Document::new(Size::new(100, 100));
        let bottom = plain(&mut doc, "bottom", boxed(Rect::new(0, 0, 10, 10)));
        push(&mut doc, bottom);
        let a = plain(&mut doc, "a", boxed(Rect::new(10, 10, 20, 20)));
        let a = push(&mut doc, a);
        let mut b = plain(&mut doc, "b", boxed(Rect::new(0, 0, 10, 10)));
        // Moved off the canvas on the left: kept.
        b.transform = Affine::translation(-5.0, 50.0);
        let b = push(&mut doc, b);
        let top = plain(&mut doc, "top", boxed(Rect::new(90, 90, 10, 10)));
        push(&mut doc, top);
        let before = names(&doc);

        let plan = merge_plan(&doc, &[b, a]).unwrap().unwrap();
        // From (-5, 10) to (30, 60).
        assert_eq!(plan.scratch.size(), Size::new(35, 50));
        let undo = bake(&mut doc, plan);
        assert_eq!(names(&doc), ["bottom", "b", "top"]);
        let merged = doc.layers()[1].id;
        assert_eq!(
            doc.layer(merged).unwrap().transform,
            Affine::translation(-5.0, 10.0)
        );
        assert_eq!(shown(&doc, 15, 15).as_deref(), Some("b"));
        assert_eq!(shown(&doc, 0, 55).as_deref(), Some("b"));
        assert_eq!(pick::bounds_of(&doc, &[merged]).unwrap().left, -5);
        undo.apply(&mut doc).unwrap();
        assert_eq!(names(&doc), before);

        // Merge Down: with the layer below, named after it.
        let plan = merge_down_plan(&doc, a).unwrap().unwrap();
        bake(&mut doc, plan);
        assert_eq!(names(&doc), ["bottom", "b", "top"]);
        assert_eq!(shown(&doc, 5, 5).as_deref(), Some("bottom"));
        assert_eq!(shown(&doc, 15, 15).as_deref(), Some("bottom"));
    }

    #[test]
    fn merge_down_needs_a_visible_layer_below() {
        let mut doc = Document::new(Size::new(100, 100));
        let low = plain(&mut doc, "low", boxed(Rect::new(0, 0, 10, 10)));
        let low = push(&mut doc, low);
        let high = plain(&mut doc, "high", boxed(Rect::new(0, 0, 10, 10)));
        let high = push(&mut doc, high);
        assert!(merge_down_plan(&doc, low).unwrap().is_none());
        Edit::SetLayerVisible {
            id: low,
            visible: false,
        }
        .apply(&mut doc)
        .unwrap();
        assert!(merge_down_plan(&doc, high).unwrap().is_none());
    }

    #[test]
    fn merge_visible_keeps_hidden_layers_and_flatten_drops_them() {
        let mut doc = Document::new(Size::new(100, 100));
        let a = plain(&mut doc, "a", boxed(Rect::new(0, 0, 10, 10)));
        push(&mut doc, a);
        let mut hidden = plain(&mut doc, "hidden", boxed(Rect::new(50, 50, 10, 10)));
        hidden.visible = false;
        push(&mut doc, hidden);
        let b = plain(&mut doc, "b", boxed(Rect::new(20, 0, 10, 10)));
        push(&mut doc, b);

        let plan = merge_visible_plan(&doc).unwrap().unwrap();
        let undo = bake(&mut doc, plan);
        assert_eq!(names(&doc), ["hidden", "b"]);
        undo.apply(&mut doc).unwrap();

        let plan = flatten_plan(&doc, "Flat".into()).unwrap().unwrap();
        bake(&mut doc, plan);
        assert_eq!(names(&doc), ["Flat"]);
        // Transparency is kept: no background.
        assert_eq!(shown(&doc, 15, 5), None);
        assert_eq!(shown(&doc, 25, 5).as_deref(), Some("Flat"));
        assert_eq!(shown(&doc, 55, 55), None);
    }

    #[test]
    fn a_clipped_layer_merged_without_its_base_is_not_clipped_to_another() {
        let mut doc = Document::new(Size::new(100, 100));
        let other = plain(&mut doc, "other", boxed(Rect::new(0, 0, 10, 10)));
        let other = push(&mut doc, other);
        let base = plain(&mut doc, "base", boxed(Rect::new(50, 50, 10, 10)));
        push(&mut doc, base);
        let mut clipped = plain(&mut doc, "clipped", boxed(Rect::new(0, 0, 60, 60)));
        clipped.clipped = true;
        let clipped = push(&mut doc, clipped);
        let plan = merge_plan(&doc, &[other, clipped]).unwrap().unwrap();
        assert!(plan.scratch.layers().iter().all(|l| !l.clipped));
    }

    #[test]
    fn rasterizing_a_moved_fill_keeps_the_canvas_covered_and_its_mask_in_place() {
        let mut doc = Document::new(Size::new(100, 100));
        let mut layer = plain(&mut doc, "fill", fill());
        layer.transform = Affine::translation(10.0, 0.0);
        layer.opacity = 0.5;
        let gray = PixelFormat {
            layout: ChannelLayout::Gray,
            sample: SampleType::U8,
            color_space: ColorSpace::LINEAR_SRGB,
            alpha: AlphaMode::Straight,
        };
        // Shows (0..40) x (0..100) of its content: the document's (10..50).
        let mask = RasterImage::from_placed(
            Size::new(100, 100),
            gray,
            Rect::new(0, 0, 40, 100),
            &[255u8; 4000],
            &[0],
        )
        .unwrap();
        layer.mask = Some(LayerMask {
            image: Arc::new(mask),
            enabled: true,
            replaces_alpha: false,
            original: None,
        });
        let id = push(&mut doc, layer);
        let plans = rasterize_plans(&doc, &[id]).unwrap();
        assert_eq!(plans.len(), 1);
        bake(&mut doc, plans.into_iter().next().unwrap());
        let layer = doc.layer(id).unwrap();
        assert!(matches!(layer.content, LayerContent::Raster { .. }));
        assert_eq!(layer.opacity, 0.5);
        // One tile before the origin: the canvas's left edge is still covered.
        assert_eq!(
            layer.transform,
            Affine::translation(10.0 - f64::from(TILE_SIZE), 0.0)
        );
        assert_eq!(shown(&doc, 10, 50).as_deref(), Some("fill"));
        assert_eq!(shown(&doc, 49, 50).as_deref(), Some("fill"));
        assert_eq!(shown(&doc, 50, 50), None);
        assert_eq!(shown(&doc, 5, 50), None);
        // Without the mask, the whole canvas is covered.
        Edit::SetLayerMaskEnabled { id, enabled: false }
            .apply(&mut doc)
            .unwrap();
        assert_eq!(shown(&doc, 0, 0).as_deref(), Some("fill"));
        assert_eq!(shown(&doc, 99, 99).as_deref(), Some("fill"));
    }

    #[test]
    fn rasterizing_a_group_keeps_the_group_properties() {
        let mut doc = Document::new(Size::new(100, 100));
        let inner = plain(&mut doc, "inner", boxed(Rect::new(20, 20, 10, 10)));
        let mut group = plain(
            &mut doc,
            "group",
            LayerContent::Group {
                children: vec![inner],
                pass_through: false,
            },
        );
        group.opacity = 0.75;
        group.transform = Affine::translation(5.0, 5.0);
        let id = push(&mut doc, group);
        let plans = rasterize_plans(&doc, &[id]).unwrap();
        let undo = bake(&mut doc, plans.into_iter().next().unwrap());
        let layer = doc.layer(id).unwrap();
        assert!(matches!(layer.content, LayerContent::Raster { .. }));
        assert_eq!(layer.opacity, 0.75);
        assert_eq!(layer.transform, Affine::translation(5.0, 5.0));
        assert_eq!(shown(&doc, 25, 25).as_deref(), Some("group"));
        assert_eq!(shown(&doc, 34, 34).as_deref(), Some("group"));
        assert_eq!(shown(&doc, 24, 24), None);
        undo.apply(&mut doc).unwrap();
        assert!(doc.layer(id).unwrap().is_group());
    }

    #[test]
    fn rasterizing_a_stack_makes_what_it_shows_the_original() {
        let mut doc = Document::new(Size::new(100, 100));
        let layer = plain(&mut doc, "pixels", boxed(Rect::new(0, 0, 10, 10)));
        let id = push(&mut doc, layer);
        Edit::apply_effect(&doc, &[id], crate::adjust::Adjustment::Invert)
            .unwrap()
            .apply(&mut doc)
            .unwrap();
        let shows = doc.layer(id).unwrap().content.pixels().unwrap();
        assert!(can_rasterize(doc.layer(id).unwrap()));
        let edits = rasterize_in_place(&doc, &[id]).unwrap();
        Edit::Batch(edits).apply(&mut doc).unwrap();
        let layer = doc.layer(id).unwrap();
        assert!(!layer.is_painted());
        assert!(!can_rasterize(layer));
        assert!(Arc::ptr_eq(layer.content.original().unwrap(), &shows));
    }
}
