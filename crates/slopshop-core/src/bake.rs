//! Layer > Bake to Pixels (ADR 0031): the commands that turn what stays editable into plain
//! pixels, on purpose. Rasterize keeps each layer (its transform, opacity, blend mode, mask and
//! clipping) but bakes its content: a layer's stack, a fill's color, a group's layers. Merge
//! Layers, Merge Down, Merge Visible and Flatten Image replace layers by the one pixel layer
//! they composite into.
//!
//! A merge shows at once: [`merge_preview`] moves the layers into a new group in the place of
//! the result (the same pixels on screen), and that group is then rasterized. The pixels are
//! composited by the caller (on the GPU when there is one, on a worker): a [`BakePlan`] holds a
//! scratch document whose whole canvas is what to composite, and [`BakePlan::finish`] puts the
//! image in place of the layer's content, unless it changed meanwhile. Raster layers whose stack
//! is baked need no compositing ([`rasterize_in_place`]).

use std::sync::Arc;

use crate::blend::BlendMode;
use crate::document::{Document, Layer, LayerContent, LayerId, LayerMask};
use crate::edit::{Edit, EditError};
use crate::geom::{Rect, Size};
use crate::pick::{self, Bounds};
use crate::raster::{RasterImage, TILE_SIZE};
use crate::transform::Affine;

/// A composite to make, and the layer whose content it replaces.
#[derive(Debug, Clone)]
pub struct BakePlan {
    /// What to composite: its whole canvas, in its blend space.
    pub scratch: Document,
    /// The layer rasterized (a fill or a group).
    id: LayerId,
    /// Where the content shows on the scratch canvas.
    region: Rect,
    /// Whole tiles (columns, rows) the image starts before the layer's content space's origin:
    /// the content space moves by as much, its mask grown alike.
    shift: (u32, u32),
    /// The content baked: if it is not the layer's any more (undone, edited), nothing is.
    expected: LayerContent,
}

impl BakePlan {
    /// The layer this plan rasterizes.
    pub fn layer(&self) -> LayerId {
        self.id
    }

    /// Where the content shows on the scratch canvas: what the baked layer's thumbnail shows.
    pub fn region(&self) -> Rect {
        self.region
    }

    /// The edit that puts `image` (the scratch canvas composited, of its size) in place of the
    /// layer's content, keeping the rest of the layer as it is now; `None` when the layer is
    /// gone or its content changed since the plan (nothing to bake any more).
    pub fn finish(
        self,
        doc: &Document,
        image: Arc<RasterImage>,
    ) -> Result<Option<Edit>, EditError> {
        if image.size() != self.scratch.size() {
            return Err(EditError::InvalidPaint);
        }
        let Some(layer) = doc.layer(self.id) else {
            return Ok(None);
        };
        if layer.content != self.expected {
            return Ok(None);
        }
        let mut baked = layer.clone();
        baked.content = LayerContent::raster(image);
        baked.transform = shifted(layer.transform, self.shift);
        baked.mask = match &layer.mask {
            Some(mask) => Some(grown_mask(mask, self.shift)?),
            None => None,
        };
        replace(doc, baked).map(Some)
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
        LayerContent::Adjustment { .. } | LayerContent::Filter { .. } => false,
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
                visible: true,
                opacity: 1.0,
                blend_mode: BlendMode::Normal,
                mask: None,
                clipped: false,
                transform: Affine::IDENTITY,
                // Its effects stay the layer's (ADR 0032), drawn from the pixels it becomes.
                style: None,
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
        // Nothing shows (hidden or empty layers): a transparent pixel.
        let region = content_bounds(doc, &content, canvas)?.unwrap_or(Bounds {
            left: 0,
            top: 0,
            right: 1,
            bottom: 1,
        });
        // Whole tiles before the origin when the content starts before it.
        let tiles = |v: i64| (v.min(0).unsigned_abs()).div_ceil(u64::from(TILE_SIZE)) as u32;
        let shift = (tiles(region.left), tiles(region.top));
        let t = i64::from(TILE_SIZE);
        let (ox, oy) = (i64::from(shift.0) * t, i64::from(shift.1) * t);
        // The image covers the new content space from its origin to the content's far edge.
        let size = size_of(region.right + ox, region.bottom + oy)?;
        let moved = Affine::translation(ox as f64, oy as f64);
        let scratch = scratch(doc, size, content, moved)?;
        // Within the canvas by construction (the shift makes the region's start non-negative).
        let region = Rect::new(
            (region.left + ox).max(0) as u32,
            (region.top + oy).max(0) as u32,
            (region.right - region.left).max(1) as u32,
            (region.bottom - region.top).max(1) as u32,
        );
        plans.push(BakePlan {
            scratch,
            id,
            region,
            shift,
            expected: layer.content.clone(),
        });
    }
    Ok(plans)
}

/// What a merge takes.
#[derive(Debug, Clone, PartialEq)]
pub enum Merge {
    /// The layers (a group with its layers), into one in the place of the topmost, named after
    /// it (Merge Layers).
    Layers(Vec<LayerId>),
    /// The layer with the visible layer right below it in its group, named after that one
    /// (Merge Down; refused onto a hidden layer, as in Photoshop).
    Down(LayerId),
    /// Every visible layer of the top level, hidden ones staying (Merge Visible).
    Visible,
    /// Every layer, named as given (Flatten Image).
    Flatten(String),
}

/// The layer right below `id` in its group, when visible.
pub fn layer_below(doc: &Document, id: LayerId) -> Option<&Layer> {
    let (parent, index) = doc.locate(id)?;
    let below = doc.children_of(parent)?.get(index.checked_sub(1)?)?;
    below.visible.then_some(below)
}

/// The instant part of `merge`: the layers it takes moved into a new isolated group `group`
/// (an id the document allocated) in the place of the topmost, named as the result, each
/// placed and clipped so that it shows as it did on its own (clipped only to a base merged
/// too). Rasterizing that group ([`rasterize_plans`]) then gives the merged layer: hidden layers
/// drop, nothing outside the canvas is cut. `None` when there is nothing to merge.
pub fn merge_preview(
    doc: &Document,
    merge: Merge,
    group: LayerId,
) -> Result<Option<Edit>, EditError> {
    let top_level = |visible_only: bool| -> Vec<LayerId> {
        doc.layers()
            .iter()
            .filter(|l| l.visible || !visible_only)
            .map(|l| l.id)
            .collect()
    };
    let (ids, name) = match merge {
        Merge::Layers(ids) => (ids, None),
        Merge::Down(id) => match layer_below(doc, id) {
            Some(below) => (vec![below.id, id], Some(below.name.clone())),
            None => return Ok(None),
        },
        Merge::Visible => (top_level(true), None),
        Merge::Flatten(name) => (top_level(false), Some(name)),
    };
    let removed = crate::edit::outermost_in_order(doc, &ids)?;
    let Some(&topmost) = removed.last() else {
        return Ok(None);
    };
    let name = match name {
        Some(name) => name,
        None => doc
            .layer(topmost)
            .map(|l| l.name.clone())
            .unwrap_or_default(),
    };
    let (parent, _) = doc
        .locate(topmost)
        .ok_or(EditError::UnknownLayer(topmost))?;
    let from_document = match parent {
        Some(p) => doc
            .layer(p)
            .ok_or(EditError::UnknownLayer(p))?
            .transform
            .then(doc.parent_transform(p))
            .inverse()
            .ok_or(EditError::InvalidTransform)?,
        None => Affine::IDENTITY,
    };
    let group = Layer {
        style: None,
        id: group,
        name,
        visible: true,
        opacity: 1.0,
        blend_mode: BlendMode::Normal,
        mask: None,
        clipped: false,
        transform: Affine::IDENTITY,
        content: LayerContent::Group {
            children: Vec::new(),
            pass_through: false,
        },
    };
    let mut edits = vec![Edit::group_layers(doc, group, &removed)?];
    for &id in &removed {
        let layer = doc.layer(id).ok_or(EditError::UnknownLayer(id))?;
        // Where it shows, in the group's space (its new parent's): unchanged on screen.
        let transform = layer
            .transform
            .then(doc.parent_transform(id))
            .then(from_document);
        if transform != layer.transform {
            edits.push(Edit::SetLayerTransform { id, transform });
        }
        let clipped = layer.clipped && base(doc, id).is_some_and(|b| removed.contains(&b));
        if clipped != layer.clipped {
            edits.push(Edit::SetLayerClipped { id, clipped });
        }
    }
    Ok(Some(Edit::Batch(edits)))
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
            style: None,
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
        plan.finish(doc, image)
            .unwrap()
            .unwrap()
            .apply(doc)
            .unwrap()
    }

    /// `merge` previewed, then its group rasterized; the undo of both.
    fn merged(doc: &mut Document, merge: Merge) -> Edit {
        let group = doc.allocate_layer_id();
        let preview = merge_preview(doc, merge, group).unwrap().unwrap();
        let undo_preview = preview.apply(doc).unwrap();
        let plan = rasterize_plans(doc, &[group]).unwrap().remove(0);
        let undo_bake = bake(doc, plan);
        Edit::Batch(vec![undo_bake, undo_preview])
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

        // The preview: the same pixels on screen at once, in a group named as the result.
        let group = doc.allocate_layer_id();
        let preview = merge_preview(&doc, Merge::Layers(vec![b, a]), group)
            .unwrap()
            .unwrap();
        let undo_preview = preview.apply(&mut doc).unwrap();
        assert_eq!(names(&doc), ["bottom", "b", "top"]);
        assert!(doc.layer(group).unwrap().is_group());
        assert_eq!(shown(&doc, 15, 15).as_deref(), Some("a"));
        assert_eq!(shown(&doc, 0, 55).as_deref(), Some("b"));
        undo_preview.apply(&mut doc).unwrap();
        assert_eq!(names(&doc), before);

        let undo = merged(&mut doc, Merge::Layers(vec![b, a]));
        assert_eq!(names(&doc), ["bottom", "b", "top"]);
        let result = doc.layers()[1].id;
        assert!(matches!(
            doc.layer(result).unwrap().content,
            LayerContent::Raster { .. }
        ));
        assert_eq!(shown(&doc, 15, 15).as_deref(), Some("b"));
        assert_eq!(shown(&doc, 0, 55).as_deref(), Some("b"));
        assert_eq!(pick::bounds_of(&doc, &[result]).unwrap().left, -5);
        undo.apply(&mut doc).unwrap();
        assert_eq!(names(&doc), before);

        // Merge Down: with the layer below, named after it.
        merged(&mut doc, Merge::Down(a));
        assert_eq!(names(&doc), ["bottom", "b", "top"]);
        assert_eq!(shown(&doc, 5, 5).as_deref(), Some("bottom"));
        assert_eq!(shown(&doc, 15, 15).as_deref(), Some("bottom"));
    }

    #[test]
    fn a_plan_s_region_is_where_its_composite_shows() {
        let mut doc = Document::new(Size::new(100, 100));
        let a = plain(&mut doc, "a", boxed(Rect::new(10, 20, 30, 15)));
        let a = push(&mut doc, a);
        let mut b = plain(&mut doc, "b", boxed(Rect::new(0, 0, 10, 10)));
        // Partly before the origin: the scratch canvas starts a tile earlier.
        b.transform = Affine::translation(-5.0, 60.0);
        let b = push(&mut doc, b);
        let group = doc.allocate_layer_id();
        merge_preview(&doc, Merge::Layers(vec![a, b]), group)
            .unwrap()
            .unwrap()
            .apply(&mut doc)
            .unwrap();
        let plan = rasterize_plans(&doc, &[group]).unwrap().remove(0);
        assert_eq!(Some(plan.region()), composited(&plan).content_bounds());
    }

    #[test]
    fn a_merge_from_a_moved_group_shows_where_it_was() {
        let mut doc = Document::new(Size::new(100, 100));
        let inner = plain(&mut doc, "inner", boxed(Rect::new(0, 0, 10, 10)));
        let inner_id = inner.id;
        let mut group = plain(
            &mut doc,
            "group",
            LayerContent::Group {
                children: vec![inner],
                pass_through: false,
            },
        );
        group.transform = Affine::translation(30.0, 0.0);
        push(&mut doc, group);
        let outside = plain(&mut doc, "outside", boxed(Rect::new(0, 50, 10, 10)));
        let outside = push(&mut doc, outside);
        merged(&mut doc, Merge::Layers(vec![inner_id, outside]));
        assert_eq!(shown(&doc, 35, 5).as_deref(), Some("outside"));
        assert_eq!(shown(&doc, 5, 55).as_deref(), Some("outside"));
        assert_eq!(shown(&doc, 5, 5), None);
    }

    #[test]
    fn a_stale_plan_bakes_nothing() {
        let mut doc = Document::new(Size::new(100, 100));
        let a = plain(&mut doc, "a", boxed(Rect::new(0, 0, 10, 10)));
        let a = push(&mut doc, a);
        let b = plain(&mut doc, "b", boxed(Rect::new(0, 0, 10, 10)));
        let b = push(&mut doc, b);
        let group = doc.allocate_layer_id();
        let undo = merge_preview(&doc, Merge::Layers(vec![a, b]), group)
            .unwrap()
            .unwrap()
            .apply(&mut doc)
            .unwrap();
        let plan = rasterize_plans(&doc, &[group]).unwrap().remove(0);
        let image = composited(&plan);
        // Undone before the pixels came: the group is gone.
        undo.apply(&mut doc).unwrap();
        assert_eq!(plan.finish(&doc, image).unwrap(), None);
    }

    #[test]
    fn merge_down_needs_a_visible_layer_below() {
        let mut doc = Document::new(Size::new(100, 100));
        let low = plain(&mut doc, "low", boxed(Rect::new(0, 0, 10, 10)));
        let low = push(&mut doc, low);
        let high = plain(&mut doc, "high", boxed(Rect::new(0, 0, 10, 10)));
        let high = push(&mut doc, high);
        let group = doc.allocate_layer_id();
        assert!(
            merge_preview(&doc, Merge::Down(low), group)
                .unwrap()
                .is_none()
        );
        Edit::SetLayerVisible {
            id: low,
            visible: false,
        }
        .apply(&mut doc)
        .unwrap();
        assert!(
            merge_preview(&doc, Merge::Down(high), group)
                .unwrap()
                .is_none()
        );
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

        let undo = merged(&mut doc, Merge::Visible);
        assert_eq!(names(&doc), ["hidden", "b"]);
        undo.apply(&mut doc).unwrap();

        merged(&mut doc, Merge::Flatten("Flat".into()));
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
        let group = doc.allocate_layer_id();
        merge_preview(&doc, Merge::Layers(vec![other, clipped]), group)
            .unwrap()
            .unwrap()
            .apply(&mut doc)
            .unwrap();
        assert!(!doc.layer(clipped).unwrap().clipped);
        // Not clipped: it shows beyond `other`.
        assert_eq!(shown(&doc, 30, 30).as_deref(), Some("clipped"));
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
