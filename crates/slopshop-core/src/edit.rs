//! Reversible document edits.
//!
//! An [`Edit`] is the only way to mutate a [`Document`]. Applying an edit validates it first
//! (a failed edit leaves the document untouched) and returns its exact inverse, which is what
//! the undo/redo history stores.
//!
//! On error the document content is unchanged. (A failing [`Edit::Batch`] rolls back what it
//! applied; its revision still advances, since revisions must never be reused.)

use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;

use crate::blend::{BlendMode, BlendSpace};
use crate::document::{Document, Layer, LayerContent, LayerId, LayerMask, MAX_GROUP_DEPTH};
use crate::geom::Size;
use crate::raster::RasterImage;
use crate::transform::Affine;

#[derive(Debug, Clone, PartialEq)]
pub enum Edit {
    /// Insert `layer` (a group with its children) at `index` among the layers of `parent`
    /// (`None`: the top level; 0 = bottom, `len` = top). Its ids must come from
    /// [`Document::allocate_layer_id`] and not be in use.
    InsertLayer {
        parent: Option<LayerId>,
        index: usize,
        layer: Layer,
    },
    /// Remove a layer, a group with everything inside it.
    RemoveLayer {
        id: LayerId,
    },
    SetLayerVisible {
        id: LayerId,
        visible: bool,
    },
    SetLayerOpacity {
        id: LayerId,
        opacity: f32,
    },
    RenameLayer {
        id: LayerId,
        name: String,
    },
    SetLayerBlendMode {
        id: LayerId,
        mode: BlendMode,
    },
    /// Where the document's layers blend (ADR 0012).
    SetBlendSpace {
        space: BlendSpace,
    },
    /// The document's resolution, pixels per inch (ADR 0028): metadata, the pixels stay.
    SetResolution {
        ppi: f64,
    },
    /// The canvas size (Canvas Size, Image Size, Crop, ADR 0017): layers keep their pixels and
    /// transforms; what falls outside the canvas is kept, not cut.
    SetCanvasSize {
        size: Size,
    },
    /// Add, replace (`Some`) or delete (`None`) a layer's mask (ADR 0014).
    SetLayerMask {
        id: LayerId,
        mask: Option<LayerMask>,
    },
    /// Apply or ignore a layer's mask, which is kept either way.
    SetLayerMaskEnabled {
        id: LayerId,
        enabled: bool,
    },
    /// Move a layer (a group with everything inside it) so that it ends up at `index` among the
    /// layers of `parent` (`None`: the top level; 0 = bottom).
    MoveLayer {
        id: LayerId,
        parent: Option<LayerId>,
        index: usize,
    },
    /// Place a layer (a group with everything inside it) in its parent's space (ADR 0017).
    /// Whole-pixel translations only, until resampled transforms are supported.
    SetLayerTransform {
        id: LayerId,
        transform: Affine,
    },
    /// Give a raster layer `stack` (ADR 0029): its original (grown, for a layer painted beyond
    /// its bounds) and what is applied to it. The layer shows the stack's result: `shown` when
    /// given (what a stroke computed, of the stack's size and format), else evaluated again
    /// where the stack differs from the layer's. The inverse keeps the previous stack only, by
    /// reference: undo evaluates again, so that history never holds evaluated pixels.
    SetLayerStack {
        id: LayerId,
        stack: crate::stack::LayerStack,
        shown: Option<Arc<RasterImage>>,
    },
    /// The same for a layer's mask: `painted` is a gray coverage of the mask's size.
    SetMaskPaint {
        id: LayerId,
        painted: Option<Arc<RasterImage>>,
    },
    /// Replace an adjustment layer's adjustment (its kind or its parameters, ADR 0020).
    SetAdjustment {
        id: LayerId,
        adjustment: crate::adjust::Adjustment,
    },
    /// Clip a layer to the layer below it, or release it (ADR 0016).
    SetLayerClipped {
        id: LayerId,
        clipped: bool,
    },
    /// Let a group's children blend through it, or isolate them (ADR 0015).
    SetGroupPassThrough {
        id: LayerId,
        pass_through: bool,
    },
    /// Select (`Some`) or deselect (`None`), ADR 0024.
    SetSelection {
        selection: Option<crate::selection::Selection>,
    },
    /// Several edits applied in order as a single unit: all of them or none.
    Batch(Vec<Edit>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum EditError {
    UnknownLayer(LayerId),
    /// The id was not produced by this document's allocator.
    LayerIdNotAllocated(LayerId),
    LayerIdInUse(LayerId),
    IndexOutOfRange {
        index: usize,
        len: usize,
    },
    /// Opacity must be finite and within `[0, 1]`.
    InvalidOpacity(f32),
    /// Colors must be finite (out-of-gamut and HDR values are allowed).
    InvalidColor,
    /// Adjustment parameters out of range, or not an adjustment layer (ADR 0020).
    InvalidAdjustment,
    /// Masks are gray images.
    InvalidMask,
    /// Paint of another size than the pixels it covers, or a mask's paint that is not gray
    /// (ADR 0027).
    InvalidPaint,
    /// Painting needs a raster layer (ADR 0027).
    NotRaster(LayerId),
    /// A layer's stack could not be evaluated (ADR 0029).
    Stack(crate::stack::StackError),
    /// The layer has no mask.
    NoMask(LayerId),
    /// The layer is not a group (as a parent, or for a group edit).
    NotAGroup(LayerId),
    /// A group cannot go inside itself or one of its descendants.
    MoveIntoItself(LayerId),
    /// Groups would nest deeper than [`MAX_GROUP_DEPTH`].
    TooDeep {
        depth: usize,
    },
    /// An operation on several layers was given none.
    NoLayers,
    /// A transform that is not finite and invertible (ADR 0018).
    InvalidTransform,
    /// A canvas without pixels.
    EmptyCanvas,
    /// A resolution out of range (ADR 0028).
    InvalidResolution,
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EditError::UnknownLayer(id) => write!(f, "unknown layer {id}"),
            EditError::LayerIdNotAllocated(id) => {
                write!(f, "{id} was not allocated by this document")
            }
            EditError::LayerIdInUse(id) => write!(f, "{id} is already in the document"),
            EditError::IndexOutOfRange { index, len } => {
                write!(
                    f,
                    "layer index {index} out of range (stack has {len} layers)"
                )
            }
            EditError::InvalidOpacity(o) => write!(f, "invalid opacity {o} (expected 0..=1)"),
            EditError::InvalidColor => write!(f, "color components must be finite"),
            EditError::InvalidAdjustment => {
                write!(
                    f,
                    "adjustment parameters out of range, or not an adjustment layer"
                )
            }
            EditError::InvalidMask => write!(f, "a mask must be a gray image"),
            EditError::InvalidPaint => {
                write!(
                    f,
                    "paint must have the size (and, in a mask, the gray) of its pixels"
                )
            }
            EditError::NotRaster(id) => write!(f, "{id} is not a raster layer"),
            EditError::Stack(e) => write!(f, "{e}"),
            EditError::NoMask(id) => write!(f, "{id} has no mask"),
            EditError::NotAGroup(id) => write!(f, "{id} is not a group"),
            EditError::MoveIntoItself(id) => write!(f, "{id} cannot go inside itself"),
            EditError::TooDeep { depth } => {
                write!(f, "{depth} nested groups (at most {MAX_GROUP_DEPTH})")
            }
            EditError::NoLayers => write!(f, "no layers given"),
            EditError::InvalidResolution => write!(f, "invalid resolution"),
            EditError::InvalidTransform => {
                write!(f, "a transform must be finite and invertible")
            }
            EditError::EmptyCanvas => write!(f, "a canvas must have pixels"),
        }
    }
}

impl std::error::Error for EditError {}

impl Edit {
    /// Apply the edit and return its inverse. On error the document content is unchanged.
    pub fn apply(self, doc: &mut Document) -> Result<Edit, EditError> {
        let inverse = match self {
            Edit::InsertLayer {
                parent,
                index,
                layer,
            } => {
                validate_new_layer(doc, parent, index, &layer)?;
                let id = layer.id;
                siblings_mut(doc, parent)?.insert(index, layer);
                Edit::RemoveLayer { id }
            }
            Edit::RemoveLayer { id } => {
                let (parent, index) = doc.locate(id).ok_or(EditError::UnknownLayer(id))?;
                let layer = siblings_mut(doc, parent)?.remove(index);
                Edit::InsertLayer {
                    parent,
                    index,
                    layer,
                }
            }
            Edit::SetLayerVisible { id, visible } => {
                let layer = doc.layer_mut(id).ok_or(EditError::UnknownLayer(id))?;
                let previous = std::mem::replace(&mut layer.visible, visible);
                Edit::SetLayerVisible {
                    id,
                    visible: previous,
                }
            }
            Edit::SetLayerOpacity { id, opacity } => {
                validate_opacity(opacity)?;
                let layer = doc.layer_mut(id).ok_or(EditError::UnknownLayer(id))?;
                let previous = std::mem::replace(&mut layer.opacity, opacity);
                Edit::SetLayerOpacity {
                    id,
                    opacity: previous,
                }
            }
            Edit::RenameLayer { id, name } => {
                let layer = doc.layer_mut(id).ok_or(EditError::UnknownLayer(id))?;
                let previous = std::mem::replace(&mut layer.name, name);
                Edit::RenameLayer { id, name: previous }
            }
            Edit::SetLayerBlendMode { id, mode } => {
                let layer = doc.layer_mut(id).ok_or(EditError::UnknownLayer(id))?;
                let previous = std::mem::replace(&mut layer.blend_mode, mode);
                Edit::SetLayerBlendMode { id, mode: previous }
            }
            Edit::SetBlendSpace { space } => Edit::SetBlendSpace {
                space: doc.set_blend_space(space),
            },
            Edit::SetResolution { ppi } => {
                if !crate::document::valid_resolution(ppi) {
                    return Err(EditError::InvalidResolution);
                }
                Edit::SetResolution {
                    ppi: doc.set_resolution(ppi),
                }
            }
            Edit::SetSelection { selection } => Edit::SetSelection {
                selection: doc.set_selection(selection),
            },
            Edit::SetCanvasSize { size } => {
                if size.is_empty() {
                    return Err(EditError::EmptyCanvas);
                }
                Edit::SetCanvasSize {
                    size: doc.set_size(size),
                }
            }
            Edit::SetLayerMask { id, mask } => {
                if mask
                    .as_ref()
                    .is_some_and(|m| !LayerMask::is_valid_image(&m.image))
                {
                    return Err(EditError::InvalidMask);
                }
                let layer = doc.layer_mut(id).ok_or(EditError::UnknownLayer(id))?;
                let previous = std::mem::replace(&mut layer.mask, mask);
                Edit::SetLayerMask { id, mask: previous }
            }
            Edit::SetLayerStack { id, stack, shown } => {
                let layer = doc.layer_mut(id).ok_or(EditError::UnknownLayer(id))?;
                let LayerContent::Raster { image, stack: kept } = &mut layer.content else {
                    return Err(EditError::NotRaster(id));
                };
                let before = kept
                    .clone()
                    .unwrap_or_else(|| crate::stack::LayerStack::new(Arc::clone(image)));
                let shown = match shown {
                    Some(shown) => {
                        if shown.size() != stack.original().size()
                            || shown.format() != stack.format()
                        {
                            return Err(EditError::InvalidPaint);
                        }
                        shown
                    }
                    None => stack.reevaluate(&before, image).map_err(EditError::Stack)?,
                };
                *image = shown;
                *kept = (!stack.is_empty()).then_some(stack);
                Edit::SetLayerStack {
                    id,
                    stack: before,
                    shown: None,
                }
            }
            Edit::SetMaskPaint { id, painted } => {
                let layer = doc.layer_mut(id).ok_or(EditError::UnknownLayer(id))?;
                let mask = layer.mask.as_mut().ok_or(EditError::NoMask(id))?;
                let painted = swap_paint(&mut mask.image, &mut mask.original, painted, |p, o| {
                    p.size() == o.size() && LayerMask::is_valid_image(p)
                })?;
                Edit::SetMaskPaint { id, painted }
            }
            Edit::SetLayerMaskEnabled { id, enabled } => {
                let layer = doc.layer_mut(id).ok_or(EditError::UnknownLayer(id))?;
                let mask = layer.mask.as_mut().ok_or(EditError::NoMask(id))?;
                let previous = std::mem::replace(&mut mask.enabled, enabled);
                Edit::SetLayerMaskEnabled {
                    id,
                    enabled: previous,
                }
            }
            Edit::MoveLayer { id, parent, index } => {
                let (from_parent, from) = doc.locate(id).ok_or(EditError::UnknownLayer(id))?;
                let layer = doc.layer(id).ok_or(EditError::UnknownLayer(id))?;
                if let Some(target) = parent
                    && layer.subtree().any(|l| l.id == target)
                {
                    return Err(EditError::MoveIntoItself(id));
                }
                let height = layer.group_height();
                let siblings = siblings(doc, parent)?.len();
                // The layer leaves its current place first: among its own siblings, the last
                // index is theirs minus one.
                let last = if parent == from_parent {
                    siblings - 1
                } else {
                    siblings
                };
                if index > last {
                    return Err(EditError::IndexOutOfRange {
                        index,
                        len: siblings,
                    });
                }
                check_depth(doc, parent, height)?;
                let layer = siblings_mut(doc, from_parent)?.remove(from);
                siblings_mut(doc, parent)?.insert(index, layer);
                Edit::MoveLayer {
                    id,
                    parent: from_parent,
                    index: from,
                }
            }
            Edit::SetLayerTransform { id, transform } => {
                validate_transform(transform)?;
                let layer = doc.layer_mut(id).ok_or(EditError::UnknownLayer(id))?;
                let previous = std::mem::replace(&mut layer.transform, transform);
                Edit::SetLayerTransform {
                    id,
                    transform: previous,
                }
            }
            Edit::SetAdjustment { id, adjustment } => {
                if !adjustment.is_valid() {
                    return Err(EditError::InvalidAdjustment);
                }
                let layer = doc.layer_mut(id).ok_or(EditError::UnknownLayer(id))?;
                let LayerContent::Adjustment {
                    adjustment: current,
                } = &mut layer.content
                else {
                    return Err(EditError::InvalidAdjustment);
                };
                let previous = std::mem::replace(current, adjustment);
                Edit::SetAdjustment {
                    id,
                    adjustment: previous,
                }
            }
            Edit::SetLayerClipped { id, clipped } => {
                let layer = doc.layer_mut(id).ok_or(EditError::UnknownLayer(id))?;
                let previous = std::mem::replace(&mut layer.clipped, clipped);
                Edit::SetLayerClipped {
                    id,
                    clipped: previous,
                }
            }
            Edit::SetGroupPassThrough { id, pass_through } => {
                let layer = doc.layer_mut(id).ok_or(EditError::UnknownLayer(id))?;
                let LayerContent::Group {
                    pass_through: current,
                    ..
                } = &mut layer.content
                else {
                    return Err(EditError::NotAGroup(id));
                };
                let previous = std::mem::replace(current, pass_through);
                Edit::SetGroupPassThrough {
                    id,
                    pass_through: previous,
                }
            }
            Edit::Batch(edits) => {
                let mut inverses = Vec::with_capacity(edits.len());
                for edit in edits {
                    match edit.apply(doc) {
                        Ok(inverse) => inverses.push(inverse),
                        Err(err) => {
                            for inverse in inverses.into_iter().rev() {
                                // Exact inverses of edits that just succeeded cannot fail.
                                let rolled_back = inverse.apply(doc);
                                debug_assert!(rolled_back.is_ok(), "rollback failed");
                            }
                            return Err(err);
                        }
                    }
                }
                inverses.reverse();
                Edit::Batch(inverses)
            }
        };
        doc.bump_revision();
        Ok(inverse)
    }
}

/// The layers directly inside `parent` (`None`: the top level).
fn siblings(doc: &Document, parent: Option<LayerId>) -> Result<&[Layer], EditError> {
    doc.children_of(parent)
        .ok_or_else(|| parent_error(doc, parent))
}

fn siblings_mut(doc: &mut Document, parent: Option<LayerId>) -> Result<&mut Vec<Layer>, EditError> {
    if doc.children_of(parent).is_none() {
        return Err(parent_error(doc, parent));
    }
    // Checked just above.
    doc.children_mut(parent)
        .ok_or_else(|| EditError::UnknownLayer(LayerId::from_raw(0)))
}

/// Why `parent` holds no layers: unknown, or not a group.
fn parent_error(doc: &Document, parent: Option<LayerId>) -> EditError {
    match parent {
        Some(id) if doc.layer(id).is_some() => EditError::NotAGroup(id),
        Some(id) => EditError::UnknownLayer(id),
        // The top level always exists.
        None => EditError::UnknownLayer(LayerId::from_raw(0)),
    }
}

/// Refuse placing, inside `parent`, a subtree of `height` nested groups deeper than allowed.
fn check_depth(doc: &Document, parent: Option<LayerId>, height: usize) -> Result<(), EditError> {
    let level = match parent {
        None => 0,
        Some(id) => doc.depth(id).ok_or(EditError::UnknownLayer(id))? + 1,
    };
    if level + height > MAX_GROUP_DEPTH {
        return Err(EditError::TooDeep {
            depth: level + height,
        });
    }
    Ok(())
}

impl Edit {
    /// The edit that puts `ids` into `group`, a new empty group layer (its id from
    /// [`Document::allocate_layer_id`]), as one unit: Photoshop's Layer > Group Layers. The group
    /// takes the place of the topmost of the layers, which keep their stacking order; a layer
    /// inside another of `ids` moves with it.
    pub fn group_layers(doc: &Document, group: Layer, ids: &[LayerId]) -> Result<Edit, EditError> {
        if group.children().is_none_or(|children| !children.is_empty()) {
            return Err(EditError::NotAGroup(group.id));
        }
        let moving = outermost_in_order(doc, ids)?;
        let topmost = *moving.last().ok_or(EditError::NoLayers)?;
        let (parent, index) = doc
            .locate(topmost)
            .ok_or(EditError::UnknownLayer(topmost))?;
        let group_id = group.id;
        let mut edits = vec![Edit::InsertLayer {
            parent,
            index: index + 1,
            layer: group,
        }];
        edits.extend(
            moving
                .iter()
                .enumerate()
                .map(|(index, &id)| Edit::MoveLayer {
                    id,
                    parent: Some(group_id),
                    index,
                }),
        );
        Ok(Edit::Batch(edits))
    }

    /// The edit that deletes the paint of `ids` and of the layers inside them, pixels and masks
    /// (Layer > Delete Paint, ADR 0027): their originals show again. [`EditError::NoLayers`]
    /// when none of them is painted.
    pub fn delete_paint(doc: &Document, ids: &[LayerId]) -> Result<Edit, EditError> {
        let mut edits = Vec::new();
        let mut seen = HashSet::new();
        for &id in ids {
            let layer = doc.layer(id).ok_or(EditError::UnknownLayer(id))?;
            for layer in layer.subtree() {
                if !seen.insert(layer.id) {
                    continue;
                }
                if let LayerContent::Raster {
                    stack: Some(stack), ..
                } = &layer.content
                {
                    edits.push(Edit::SetLayerStack {
                        id: layer.id,
                        stack: crate::stack::LayerStack::new(Arc::clone(stack.original())),
                        shown: Some(Arc::clone(stack.original())),
                    });
                }
                if layer.mask.as_ref().is_some_and(|m| m.original.is_some()) {
                    edits.push(Edit::SetMaskPaint {
                        id: layer.id,
                        painted: None,
                    });
                }
            }
        }
        if edits.is_empty() {
            return Err(EditError::NoLayers);
        }
        Ok(Edit::Batch(edits))
    }

    /// The layers of `ids` that Image > Adjustments applies to (ADR 0029): visible raster
    /// layers, as in Photoshop (groups, fills and adjustment layers are not).
    pub fn effect_targets(doc: &Document, ids: &[LayerId]) -> Vec<LayerId> {
        ids.iter()
            .copied()
            .filter(|&id| {
                doc.layer(id).is_some_and(|layer| {
                    layer.visible && matches!(layer.content, LayerContent::Raster { .. })
                })
            })
            .collect()
    }

    /// Image > Adjustments (ADR 0029): `adjustment` applied to each layer of `ids` it applies to
    /// ([`Self::effect_targets`]), within the selection, in one edit. [`EditError::NoLayers`]
    /// when none.
    pub fn apply_effect(
        doc: &Document,
        ids: &[LayerId],
        adjustment: crate::adjust::Adjustment,
    ) -> Result<Edit, EditError> {
        if !adjustment.is_valid() {
            return Err(EditError::InvalidAdjustment);
        }
        let mut edits = Vec::new();
        for id in Self::effect_targets(doc, ids) {
            let Some(layer) = doc.layer(id) else {
                continue;
            };
            let Some(stack) = layer.content.stack() else {
                continue;
            };
            let effect = crate::stack::Effect {
                adjustment,
                selection: doc.selection().cloned(),
                to_document: layer.transform.then(doc.parent_transform(id)),
                space: doc.blend_space(),
            };
            edits.push(Edit::SetLayerStack {
                id,
                stack: stack.with_effect(effect).map_err(EditError::Stack)?,
                shown: None,
            });
        }
        if edits.is_empty() {
            return Err(EditError::NoLayers);
        }
        Ok(Edit::Batch(edits))
    }

    /// The edit that deletes entry `index` (bottom to top) of raster layer `id`'s stack (ADR
    /// 0029): the neighbours that become alike merge, and what was above it is evaluated
    /// again where it reaches.
    pub fn delete_entry(doc: &Document, id: LayerId, index: usize) -> Result<Edit, EditError> {
        let layer = doc.layer(id).ok_or(EditError::UnknownLayer(id))?;
        let LayerContent::Raster {
            stack: Some(stack), ..
        } = &layer.content
        else {
            return Err(EditError::NotRaster(id));
        };
        Ok(Edit::SetLayerStack {
            id,
            stack: stack.without(index).map_err(EditError::Stack)?,
            shown: None,
        })
    }

    /// The edit that gives raster layer `id` `stack` with the change from `before` (what it
    /// shows) to `after` baked on top as paint (ADR 0029): what tools that read pixels leave,
    /// moved pixels. `stack` is the layer's, grown first if the pixels grew.
    pub fn bake_pixels(
        doc: &Document,
        id: LayerId,
        stack: &crate::stack::LayerStack,
        before: &RasterImage,
        after: &RasterImage,
    ) -> Result<Edit, EditError> {
        let stack = stack
            .with_painted(before, after, doc.blend_space())
            .map_err(EditError::Stack)?;
        Ok(Edit::SetLayerStack {
            id,
            stack,
            shown: None,
        })
    }

    /// The edit that moves `ids` by `(dx, dy)` whole pixels in their parents' space (the Move
    /// tool); a layer inside another of `ids` moves with it.
    pub fn translate_layers(
        doc: &Document,
        ids: &[LayerId],
        dx: i64,
        dy: i64,
    ) -> Result<Edit, EditError> {
        let moving = outermost_in_order(doc, ids)?;
        if moving.is_empty() {
            return Err(EditError::NoLayers);
        }
        let by = Affine::translation(dx as f64, dy as f64);
        Ok(Edit::Batch(
            moving
                .into_iter()
                .filter_map(|id| doc.layer(id).map(|l| (id, l.transform)))
                .map(|(id, transform)| Edit::SetLayerTransform {
                    id,
                    transform: transform.then(by),
                })
                .collect(),
        ))
    }

    /// The edit that applies `by`, a map of the document's space, to `ids` on top of their
    /// transforms (Free Transform); a layer inside another of `ids` goes with it. Results are
    /// [snapped](Affine::snapped), so that a quarter turn built by steps stays exact.
    pub fn transform_layers(
        doc: &Document,
        ids: &[LayerId],
        by: Affine,
    ) -> Result<Edit, EditError> {
        let moving = outermost_in_order(doc, ids)?;
        if moving.is_empty() {
            return Err(EditError::NoLayers);
        }
        let mut edits = Vec::with_capacity(moving.len());
        for id in moving {
            let layer = doc.layer(id).ok_or(EditError::UnknownLayer(id))?;
            // Into the document, `by`, and back into the parent's space.
            let parent = doc.parent_transform(id);
            let back = parent.inverse().ok_or(EditError::InvalidTransform)?;
            let transform = layer.transform.then(parent).then(by).then(back).snapped();
            validate_transform(transform)?;
            edits.push(Edit::SetLayerTransform { id, transform });
        }
        Ok(Edit::Batch(edits))
    }

    /// The edit that gives the canvas `size` and applies `by` (a map of the document's space)
    /// to every top-level layer, a group's layers with it: what Image Size, Canvas Size, Crop
    /// and Image Rotation do (ADR 0017). Pixels are never rewritten, and what falls outside the
    /// canvas is kept. The selection is dropped, as in Photoshop (ADR 0024).
    pub fn reframe_image(doc: &Document, size: Size, by: Affine) -> Result<Edit, EditError> {
        if size.is_empty() {
            return Err(EditError::EmptyCanvas);
        }
        let mut edits = vec![Edit::SetCanvasSize { size }];
        if doc.selection().is_some() {
            edits.push(Edit::SetSelection { selection: None });
        }
        for layer in doc.layers() {
            let transform = layer.transform.then(by).snapped();
            validate_transform(transform)?;
            edits.push(Edit::SetLayerTransform {
                id: layer.id,
                transform,
            });
        }
        Ok(Edit::Batch(edits))
    }

    /// The edit that resamples the whole image to `size` (Image > Image Size, ADR 0018): the
    /// canvas takes that size and every top-level layer is scaled by the same factors. Pixels
    /// are resampled when shown.
    pub fn resize_image(doc: &Document, size: Size) -> Result<Edit, EditError> {
        let old = doc.size();
        let by = Affine::scale(
            f64::from(size.width) / f64::from(old.width.max(1)),
            f64::from(size.height) / f64::from(old.height.max(1)),
        );
        Edit::reframe_image(doc, size, by)
    }

    /// The edit that gives the canvas `size` keeping the image where `anchor` says (Image >
    /// Canvas Size): `(0, 0)` keeps the top-left corner, `(0.5, 0.5)` the center, `(1, 1)` the
    /// bottom-right corner. Layers move by whole pixels (an odd difference puts the extra pixel
    /// on the right or bottom), so they are not resampled.
    pub fn canvas_size(doc: &Document, size: Size, anchor: (f64, f64)) -> Result<Edit, EditError> {
        let old = doc.size();
        let offset = |new: u32, old: u32, at: f64| {
            ((f64::from(new) - f64::from(old)) * at.clamp(0.0, 1.0)).floor()
        };
        let by = Affine::translation(
            offset(size.width, old.width, anchor.0),
            offset(size.height, old.height, anchor.1),
        );
        Edit::reframe_image(doc, size, by)
    }

    /// The edit that keeps only `area` of the canvas, `[x, y, width, height]` in document pixels
    /// (it may extend past the canvas): Crop. Nothing is deleted; layers move by whole pixels.
    pub fn crop(doc: &Document, area: [i64; 4]) -> Result<Edit, EditError> {
        let [x, y, width, height] = area;
        let side = |v: i64| u32::try_from(v).map_err(|_| EditError::EmptyCanvas);
        let size = Size::new(side(width)?, side(height)?);
        Edit::reframe_image(doc, size, Affine::translation(-x as f64, -y as f64))
    }

    /// The edit that turns or flips the whole image (Image > Image Rotation): exact, pixels are
    /// copied, never resampled; a quarter turn swaps the canvas's sides.
    pub fn rotate_image(doc: &Document, turn: ImageTurn) -> Result<Edit, EditError> {
        let size = doc.size();
        let (w, h) = (f64::from(size.width), f64::from(size.height));
        let (a, b, c, d, e, f) = match turn {
            ImageTurn::Clockwise => (0.0, 1.0, -1.0, 0.0, h, 0.0),
            ImageTurn::CounterClockwise => (0.0, -1.0, 1.0, 0.0, 0.0, w),
            ImageTurn::HalfTurn => (-1.0, 0.0, 0.0, -1.0, w, h),
            ImageTurn::FlipHorizontal => (-1.0, 0.0, 0.0, 1.0, w, 0.0),
            ImageTurn::FlipVertical => (1.0, 0.0, 0.0, -1.0, 0.0, h),
        };
        let turned = match turn {
            ImageTurn::Clockwise | ImageTurn::CounterClockwise => {
                Size::new(size.height, size.width)
            }
            _ => size,
        };
        Edit::reframe_image(doc, turned, Affine { a, b, c, d, e, f })
    }

    /// The edit that replaces group `id` by its layers, in its place and order: Layer > Ungroup
    /// Layers. The group's opacity, blend mode and mask go with it.
    pub fn ungroup(doc: &Document, id: LayerId) -> Result<Edit, EditError> {
        let group = doc.layer(id).ok_or(EditError::UnknownLayer(id))?;
        let children = group.children().ok_or(EditError::NotAGroup(id))?;
        let (parent, index) = doc.locate(id).ok_or(EditError::UnknownLayer(id))?;
        // Each child lands just below the group, which moves up one place each time.
        let mut edits: Vec<Edit> = children
            .iter()
            .enumerate()
            .map(|(i, child)| Edit::MoveLayer {
                id: child.id,
                parent,
                index: index + i,
            })
            .collect();
        edits.push(Edit::RemoveLayer { id });
        Ok(Edit::Batch(edits))
    }

    /// The edit that moves `ids` into `parent` (`None`: the top level) at `index` among the layers
    /// of `parent` that do not move (0 = below them all), keeping their stacking order; a layer
    /// inside another of `ids` moves with it. Layers already in place are left alone: the edit
    /// is an empty batch when nothing moves.
    pub fn move_layers(
        doc: &Document,
        ids: &[LayerId],
        parent: Option<LayerId>,
        index: usize,
    ) -> Result<Edit, EditError> {
        let moving = outermost_in_order(doc, ids)?;
        if moving.is_empty() {
            return Err(EditError::NoLayers);
        }
        let staying: Vec<LayerId> = siblings(doc, parent)?
            .iter()
            .map(|l| l.id)
            .filter(|id| !moving.contains(id))
            .collect();
        if index > staying.len() {
            return Err(EditError::IndexOutOfRange {
                index,
                len: staying.len(),
            });
        }
        // Each layer goes right above the previous one (the first above the staying layer below
        // the insertion point); the positions are worked out on a copy as the moves happen.
        let mut plan = doc.clone();
        let mut edits = Vec::new();
        for (k, &id) in moving.iter().enumerate() {
            let below = if k > 0 {
                Some(moving[k - 1])
            } else {
                index.checked_sub(1).map(|i| staying[i])
            };
            let position = match below {
                None => 0,
                Some(below) => siblings(&plan, parent)?
                    .iter()
                    .filter(|l| l.id != id)
                    .position(|l| l.id == below)
                    .map_or(0, |p| p + 1),
            };
            if plan.locate(id) == Some((parent, position)) {
                continue;
            }
            let edit = Edit::MoveLayer {
                id,
                parent,
                index: position,
            };
            edit.clone().apply(&mut plan)?;
            edits.push(edit);
        }
        Ok(Edit::Batch(edits))
    }
}

/// A turn or flip of the whole image (Image > Image Rotation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageTurn {
    /// 90° clockwise.
    Clockwise,
    /// 90° counter clockwise.
    CounterClockwise,
    /// 180°.
    HalfTurn,
    /// Left and right swap.
    FlipHorizontal,
    /// Top and bottom swap.
    FlipVertical,
}

/// `ids` without those inside another of them (they move with it), in stacking order: depth
/// first, each group before its layers, bottom to top. Unknown ids are errors.
pub(crate) fn outermost_in_order(
    doc: &Document,
    ids: &[LayerId],
) -> Result<Vec<LayerId>, EditError> {
    if let Some(&unknown) = ids.iter().find(|&&id| doc.layer(id).is_none()) {
        return Err(EditError::UnknownLayer(unknown));
    }
    Ok(doc.outermost(ids))
}

fn validate_new_layer(
    doc: &Document,
    parent: Option<LayerId>,
    index: usize,
    layer: &Layer,
) -> Result<(), EditError> {
    let len = siblings(doc, parent)?.len();
    if index > len {
        return Err(EditError::IndexOutOfRange { index, len });
    }
    check_depth(doc, parent, layer.group_height())?;
    let mut ids = HashSet::new();
    for layer in layer.subtree() {
        if !doc.is_allocated(layer.id) {
            return Err(EditError::LayerIdNotAllocated(layer.id));
        }
        if doc.layer(layer.id).is_some() || !ids.insert(layer.id) {
            return Err(EditError::LayerIdInUse(layer.id));
        }
        validate_opacity(layer.opacity)?;
        validate_transform(layer.transform)?;
        if layer
            .mask
            .as_ref()
            .is_some_and(|m| !LayerMask::is_valid_image(&m.image))
        {
            return Err(EditError::InvalidMask);
        }
        if let LayerContent::Fill { color } = &layer.content
            && !color.is_finite()
        {
            return Err(EditError::InvalidColor);
        }
        if let LayerContent::Adjustment { adjustment } = &layer.content
            && !adjustment.is_valid()
        {
            return Err(EditError::InvalidAdjustment);
        }
    }
    Ok(())
}

/// Show `painted` in place of `image`, keeping the unpainted pixels in `original` (ADR 0027);
/// `None` shows `original` again. `fits(painted, original)` validates new paint. Returns the
/// inverse's `painted`.
fn swap_paint(
    image: &mut Arc<RasterImage>,
    original: &mut Option<Arc<RasterImage>>,
    painted: Option<Arc<RasterImage>>,
    fits: impl Fn(&RasterImage, &RasterImage) -> bool,
) -> Result<Option<Arc<RasterImage>>, EditError> {
    match painted {
        Some(painted) => {
            let unpainted = original.as_ref().unwrap_or(image);
            if !fits(&painted, unpainted) {
                return Err(EditError::InvalidPaint);
            }
            let shown = std::mem::replace(image, painted);
            Ok(match original {
                // Painted before: the inverse shows the previous paint again.
                Some(_) => Some(shown),
                // First paint: the inverse deletes it.
                None => {
                    *original = Some(shown);
                    None
                }
            })
        }
        None => Ok(original
            .take()
            .map(|unpainted| std::mem::replace(image, unpainted))),
    }
}

/// Transforms layers may have: finite and invertible (ADR 0018).
pub(crate) fn validate_transform(transform: Affine) -> Result<(), EditError> {
    if transform.is_valid_layer_transform() {
        Ok(())
    } else {
        Err(EditError::InvalidTransform)
    }
}

pub(crate) fn validate_opacity(opacity: f32) -> Result<(), EditError> {
    if (0.0..=1.0).contains(&opacity) {
        Ok(())
    } else {
        Err(EditError::InvalidOpacity(opacity))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blend::BlendSpace;
    use crate::color::LinearRgba;
    use crate::stack::{Effect, LayerStack, PaintEntry, PaintOp};

    fn fill_layer(doc: &mut Document, name: &str) -> Layer {
        Layer {
            transform: crate::transform::Affine::IDENTITY,
            clipped: false,
            id: doc.allocate_layer_id(),
            name: name.to_owned(),
            visible: true,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            mask: None,
            content: LayerContent::Fill {
                color: LinearRgba::new(1.0, 0.0, 0.0, 1.0),
            },
        }
    }

    fn names(doc: &Document) -> Vec<&str> {
        doc.layers().iter().map(|l| l.name.as_str()).collect()
    }

    /// Applying an edit then its inverse must restore the layers exactly.
    fn assert_round_trip(doc: &mut Document, edit: Edit) {
        let before = doc.layers().to_vec();
        let inverse = edit.apply(doc).unwrap();
        let reinverse = inverse.apply(doc).unwrap();
        assert_eq!(doc.layers(), before.as_slice());
        // The inverse of the inverse is applicable again.
        reinverse.apply(doc).unwrap();
    }

    #[test]
    fn insert_and_remove_are_inverses() {
        let mut doc = Document::new(Size::new(64, 64));
        let a = fill_layer(&mut doc, "a");
        let b = fill_layer(&mut doc, "b");
        let b_id = b.id;
        Edit::InsertLayer {
            parent: None,
            index: 0,
            layer: a,
        }
        .apply(&mut doc)
        .unwrap();
        let inverse = Edit::InsertLayer {
            parent: None,
            index: 0,
            layer: b,
        }
        .apply(&mut doc)
        .unwrap();
        assert_eq!(names(&doc), ["b", "a"]);
        assert_eq!(inverse, Edit::RemoveLayer { id: b_id });

        let reinsert = inverse.apply(&mut doc).unwrap();
        assert_eq!(names(&doc), ["a"]);
        reinsert.apply(&mut doc).unwrap();
        assert_eq!(names(&doc), ["b", "a"]);
    }

    #[test]
    fn property_edits_round_trip() {
        let mut doc = Document::new(Size::new(64, 64));
        let layer = fill_layer(&mut doc, "a");
        let id = layer.id;
        Edit::InsertLayer {
            parent: None,
            index: 0,
            layer,
        }
        .apply(&mut doc)
        .unwrap();

        for edit in [
            Edit::SetLayerVisible { id, visible: false },
            Edit::SetLayerOpacity { id, opacity: 0.25 },
            Edit::RenameLayer {
                id,
                name: "renamed".into(),
            },
            Edit::SetLayerBlendMode {
                id,
                mode: BlendMode::Multiply,
            },
            Edit::RemoveLayer { id },
        ] {
            assert_round_trip(&mut doc, edit);
        }
    }

    #[test]
    fn resizing_the_image_scales_the_canvas_and_its_layers() {
        let mut doc = Document::new(Size::new(8, 6));
        let ids = stack(&mut doc, &["a", "b"]);
        Edit::SetLayerTransform {
            id: ids[1],
            transform: Affine::translation(2.0, 3.0),
        }
        .apply(&mut doc)
        .unwrap();
        let undo = Edit::resize_image(&doc, Size::new(16, 3))
            .unwrap()
            .apply(&mut doc)
            .unwrap();
        assert_eq!(doc.size(), Size::new(16, 3));
        assert_eq!(
            doc.layer(ids[0]).unwrap().transform,
            Affine::scale(2.0, 0.5)
        );
        assert_eq!(
            doc.layer(ids[1]).unwrap().transform.to_array(),
            [2.0, 0.0, 0.0, 0.5, 4.0, 1.5]
        );
        undo.apply(&mut doc).unwrap();
        assert_eq!(doc.size(), Size::new(8, 6));
        assert!(doc.layer(ids[0]).unwrap().transform.is_identity());
        assert_eq!(
            Edit::SetCanvasSize {
                size: Size::new(0, 4)
            }
            .apply(&mut doc),
            Err(EditError::EmptyCanvas)
        );
    }

    #[test]
    fn adjustments_are_edited_and_validated() {
        use crate::adjust::Adjustment;
        let mut doc = Document::new(Size::new(8, 8));
        let ids = stack(&mut doc, &["a"]);
        let id = doc.allocate_layer_id();
        let layer = Layer {
            id,
            name: "adjust".into(),
            visible: true,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            mask: None,
            clipped: false,
            transform: Affine::IDENTITY,
            content: LayerContent::Adjustment {
                adjustment: Adjustment::DEFAULTS[0],
            },
        };
        Edit::InsertLayer {
            parent: None,
            index: 1,
            layer,
        }
        .apply(&mut doc)
        .unwrap();
        assert_round_trip(
            &mut doc,
            Edit::SetAdjustment {
                id,
                adjustment: Adjustment::Levels {
                    input_black: 0.1,
                    input_white: 0.9,
                    gamma: 1.2,
                    output_black: 0.0,
                    output_white: 1.0,
                },
            },
        );
        let out_of_range = Adjustment::Exposure {
            exposure: 50.0,
            offset: 0.0,
            gamma: 1.0,
        };
        for (target, adjustment) in [(id, out_of_range), (ids[0], Adjustment::DEFAULTS[1])] {
            assert_eq!(
                Edit::SetAdjustment {
                    id: target,
                    adjustment
                }
                .apply(&mut doc),
                Err(EditError::InvalidAdjustment)
            );
        }
    }

    #[test]
    fn image_turns_are_exact_and_swap_the_sides() {
        let mut doc = Document::new(Size::new(8, 6));
        let ids = stack(&mut doc, &["a"]);
        let place =
            |doc: &Document, x: f64, y: f64| doc.layer(ids[0]).unwrap().transform.apply(x, y);
        // Clockwise: the top-left pixel (center 0.5, 0.5) goes to the top-right corner.
        let undo = Edit::rotate_image(&doc, ImageTurn::Clockwise)
            .unwrap()
            .apply(&mut doc)
            .unwrap();
        assert_eq!(doc.size(), Size::new(6, 8));
        assert_eq!(place(&doc, 0.5, 0.5), (5.5, 0.5));
        assert!(doc.layer(ids[0]).unwrap().transform.is_pixel_exact());
        undo.apply(&mut doc).unwrap();
        for (turn, expected) in [
            (ImageTurn::CounterClockwise, (0.5, 7.5)),
            (ImageTurn::HalfTurn, (7.5, 5.5)),
            (ImageTurn::FlipHorizontal, (7.5, 0.5)),
            (ImageTurn::FlipVertical, (0.5, 5.5)),
        ] {
            let undo = Edit::rotate_image(&doc, turn)
                .unwrap()
                .apply(&mut doc)
                .unwrap();
            assert_eq!(place(&doc, 0.5, 0.5), expected, "{turn:?}");
            assert!(doc.layer(ids[0]).unwrap().transform.is_pixel_exact());
            undo.apply(&mut doc).unwrap();
        }
        // Four quarter turns come back exactly.
        for _ in 0..4 {
            Edit::rotate_image(&doc, ImageTurn::Clockwise)
                .unwrap()
                .apply(&mut doc)
                .unwrap();
        }
        assert_eq!(doc.size(), Size::new(8, 6));
        assert!(doc.layer(ids[0]).unwrap().transform.is_identity());
    }

    #[test]
    fn canvas_size_and_crop_move_layers_by_whole_pixels() {
        let mut doc = Document::new(Size::new(8, 6));
        let ids = stack(&mut doc, &["a"]);
        let transform = |doc: &Document| doc.layer(ids[0]).unwrap().transform;
        // Centered, 3 more pixels: 1 on the left, 2 on the right.
        let undo = Edit::canvas_size(&doc, Size::new(11, 6), (0.5, 0.5))
            .unwrap()
            .apply(&mut doc)
            .unwrap();
        assert_eq!(doc.size(), Size::new(11, 6));
        assert_eq!(transform(&doc), Affine::translation(1.0, 0.0));
        undo.apply(&mut doc).unwrap();
        // Anchored bottom-right, smaller: the image moves up and left, nothing is cut.
        Edit::canvas_size(&doc, Size::new(5, 4), (1.0, 1.0))
            .unwrap()
            .apply(&mut doc)
            .unwrap();
        assert_eq!(transform(&doc), Affine::translation(-3.0, -2.0));
        let undo = Edit::crop(&doc, [1, 1, 3, 2])
            .unwrap()
            .apply(&mut doc)
            .unwrap();
        assert_eq!(doc.size(), Size::new(3, 2));
        assert_eq!(transform(&doc), Affine::translation(-4.0, -3.0));
        undo.apply(&mut doc).unwrap();
        assert_eq!(
            Edit::crop(&doc, [0, 0, 0, 3]).and_then(|e| e.apply(&mut doc)),
            Err(EditError::EmptyCanvas)
        );
        assert_eq!(
            Edit::crop(&doc, [0, 0, -2, 3]).and_then(|e| e.apply(&mut doc)),
            Err(EditError::EmptyCanvas)
        );
    }

    #[test]
    fn resolution_is_undoable_metadata() {
        use crate::document::DEFAULT_RESOLUTION;
        let mut doc = Document::new(Size::new(10, 10));
        assert_eq!(doc.resolution(), DEFAULT_RESOLUTION);
        let inverse = Edit::SetResolution { ppi: 300.0 }.apply(&mut doc).unwrap();
        assert_eq!(doc.resolution(), 300.0);
        assert_eq!(
            inverse,
            Edit::SetResolution {
                ppi: DEFAULT_RESOLUTION
            }
        );
        for wrong in [0.0, -1.0, f64::NAN, f64::INFINITY, 1e9] {
            assert_eq!(
                Edit::SetResolution { ppi: wrong }.apply(&mut doc),
                Err(EditError::InvalidResolution)
            );
            assert!(
                Document::new(Size::new(1, 1))
                    .with_resolution(wrong)
                    .is_err()
            );
        }
        assert_eq!(doc.resolution(), 300.0);
    }

    #[test]
    fn blend_space_edits_round_trip() {
        let mut doc = Document::new(Size::new(8, 8));
        assert_eq!(doc.blend_space(), BlendSpace::Perceptual);
        let inverse = Edit::SetBlendSpace {
            space: BlendSpace::Linear,
        }
        .apply(&mut doc)
        .unwrap();
        assert_eq!(doc.blend_space(), BlendSpace::Linear);
        assert_eq!(
            inverse,
            Edit::SetBlendSpace {
                space: BlendSpace::Perceptual
            }
        );
        inverse.apply(&mut doc).unwrap();
        assert_eq!(doc.blend_space(), BlendSpace::Perceptual);
        assert!(matches!(
            Edit::SetLayerBlendMode {
                id: LayerId::from_raw(99),
                mode: BlendMode::Screen
            }
            .apply(&mut doc),
            Err(EditError::UnknownLayer(_))
        ));
    }

    fn stack(doc: &mut Document, names: &[&str]) -> Vec<LayerId> {
        names
            .iter()
            .map(|name| {
                let layer = fill_layer(doc, name);
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
            })
            .collect()
    }

    #[test]
    fn move_layer_and_inverse() {
        let mut doc = Document::new(Size::new(8, 8));
        let ids = stack(&mut doc, &["a", "b", "c", "d"]);

        let inverse = Edit::MoveLayer {
            parent: None,
            id: ids[0],
            index: 3,
        }
        .apply(&mut doc)
        .unwrap();
        assert_eq!(names(&doc), ["b", "c", "d", "a"]);
        inverse.apply(&mut doc).unwrap();
        assert_eq!(names(&doc), ["a", "b", "c", "d"]);

        Edit::MoveLayer {
            parent: None,
            id: ids[3],
            index: 1,
        }
        .apply(&mut doc)
        .unwrap();
        assert_eq!(names(&doc), ["a", "d", "b", "c"]);
        assert_round_trip(
            &mut doc,
            Edit::MoveLayer {
                parent: None,
                id: ids[1],
                index: 0,
            },
        );
    }

    #[test]
    fn move_layer_out_of_range() {
        let mut doc = Document::new(Size::new(8, 8));
        let ids = stack(&mut doc, &["a", "b"]);
        assert_eq!(
            Edit::MoveLayer {
                parent: None,
                id: ids[0],
                index: 2
            }
            .apply(&mut doc),
            Err(EditError::IndexOutOfRange { index: 2, len: 2 })
        );
        assert_eq!(names(&doc), ["a", "b"]);
    }

    #[test]
    fn batch_is_all_or_nothing() {
        let mut doc = Document::new(Size::new(8, 8));
        let ids = stack(&mut doc, &["a", "b"]);
        let batch = Edit::Batch(vec![
            Edit::SetLayerOpacity {
                id: ids[0],
                opacity: 0.5,
            },
            Edit::MoveLayer {
                parent: None,
                id: ids[0],
                index: 1,
            },
            Edit::RemoveLayer {
                id: LayerId::from_raw(999),
            },
        ]);
        let before = doc.layers().to_vec();
        assert!(batch.apply(&mut doc).is_err());
        assert_eq!(doc.layers(), before.as_slice());

        let batch = Edit::Batch(vec![
            Edit::SetLayerOpacity {
                id: ids[0],
                opacity: 0.5,
            },
            Edit::MoveLayer {
                parent: None,
                id: ids[0],
                index: 1,
            },
        ]);
        assert_round_trip(&mut doc, batch);
    }

    #[test]
    fn invalid_edits_leave_document_untouched() {
        let mut doc = Document::new(Size::new(64, 64));
        let layer = fill_layer(&mut doc, "a");
        let id = layer.id;
        Edit::InsertLayer {
            parent: None,
            index: 0,
            layer: layer.clone(),
        }
        .apply(&mut doc)
        .unwrap();
        let revision = doc.revision();

        let ghost = LayerId::from_raw(999);
        let mut unallocated = layer.clone();
        unallocated.id = ghost;
        let mut nan_opacity = fill_layer(&mut doc, "nan");
        nan_opacity.opacity = f32::NAN;
        let mut nan_color = fill_layer(&mut doc, "nan color");
        nan_color.content = LayerContent::Fill {
            color: LinearRgba::new(f32::NAN, 0.0, 0.0, 1.0),
        };

        let cases = [
            (
                Edit::RemoveLayer { id: ghost },
                EditError::UnknownLayer(ghost),
            ),
            (
                Edit::SetLayerOpacity { id, opacity: 1.5 },
                EditError::InvalidOpacity(1.5),
            ),
            (
                Edit::InsertLayer {
                    parent: None,
                    index: 0,
                    layer,
                },
                EditError::LayerIdInUse(id),
            ),
            (
                Edit::InsertLayer {
                    parent: None,
                    index: 0,
                    layer: unallocated,
                },
                EditError::LayerIdNotAllocated(ghost),
            ),
            (
                Edit::InsertLayer {
                    parent: None,
                    index: 5,
                    layer: nan_color.clone(),
                },
                EditError::IndexOutOfRange { index: 5, len: 1 },
            ),
            (
                Edit::InsertLayer {
                    parent: None,
                    index: 0,
                    layer: nan_color,
                },
                EditError::InvalidColor,
            ),
        ];
        for (edit, expected) in cases {
            let before = doc.layers().to_vec();
            assert_eq!(edit.apply(&mut doc), Err(expected));
            assert_eq!(doc.layers(), before.as_slice());
            assert_eq!(doc.revision(), revision);
        }
        // NaN never compares equal, so check this one by pattern.
        assert!(matches!(
            Edit::InsertLayer { parent: None,
                index: 0,
                layer: nan_opacity
            }
            .apply(&mut doc),
            Err(EditError::InvalidOpacity(o)) if o.is_nan()
        ));
    }

    #[test]
    fn every_successful_edit_bumps_revision() {
        let mut doc = Document::new(Size::new(8, 8));
        let layer = fill_layer(&mut doc, "a");
        let id = layer.id;
        assert_eq!(doc.revision(), 0);
        let inverse = Edit::InsertLayer {
            parent: None,
            index: 0,
            layer,
        }
        .apply(&mut doc)
        .unwrap();
        assert_eq!(doc.revision(), 1);
        inverse.apply(&mut doc).unwrap();
        assert_eq!(doc.revision(), 2);
        assert!(doc.layer(id).is_none());
    }

    #[test]
    fn ids_are_never_reused() {
        let mut doc = Document::new(Size::new(8, 8));
        let layer = fill_layer(&mut doc, "a");
        let first = layer.id;
        let undo = Edit::InsertLayer {
            parent: None,
            index: 0,
            layer,
        }
        .apply(&mut doc)
        .unwrap();
        undo.apply(&mut doc).unwrap();
        assert_ne!(doc.allocate_layer_id(), first);
    }

    fn group_layer(doc: &mut Document, name: &str, children: Vec<Layer>) -> Layer {
        Layer {
            content: LayerContent::Group {
                children,
                pass_through: true,
            },
            ..fill_layer(doc, name)
        }
    }

    /// Names of the tree, depth first, children indented.
    fn tree(doc: &Document) -> Vec<String> {
        fn walk(layers: &[Layer], depth: usize, out: &mut Vec<String>) {
            for layer in layers {
                out.push(format!("{}{}", " ".repeat(depth), layer.name));
                if let Some(children) = layer.children() {
                    walk(children, depth + 1, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(doc.layers(), 0, &mut out);
        out
    }

    #[test]
    fn groups_insert_move_and_remove_with_their_subtree() {
        let mut doc = Document::new(Size::new(8, 8));
        let ids = stack(&mut doc, &["a", "b"]);
        let c = fill_layer(&mut doc, "c");
        let c_id = c.id;
        let g = group_layer(&mut doc, "g", vec![c]);
        let g_id = g.id;
        assert_round_trip(
            &mut doc,
            Edit::InsertLayer {
                parent: None,
                index: 1,
                layer: g.clone(),
            },
        );
        assert_eq!(tree(&doc), ["a", "g", " c", "b"]);
        assert_eq!(doc.locate(c_id), Some((Some(g_id), 0)));
        assert_eq!(doc.depth(c_id), Some(1));
        let all: Vec<&str> = doc.all_layers().map(|l| l.name.as_str()).collect();
        assert_eq!(all, ["a", "g", "c", "b"]);

        // Into the group, above c; then back out to the top.
        let into = Edit::MoveLayer {
            id: ids[1],
            parent: Some(g_id),
            index: 1,
        };
        assert_round_trip(&mut doc, into.clone());
        assert_eq!(tree(&doc), ["a", "g", " c", " b"]);
        assert_round_trip(
            &mut doc,
            Edit::MoveLayer {
                id: c_id,
                parent: None,
                index: 0,
            },
        );
        assert_eq!(tree(&doc), ["c", "a", "g", " b"]);

        // Removing a group removes its subtree; undo brings it back whole.
        let inverse = Edit::RemoveLayer { id: g_id }.apply(&mut doc).unwrap();
        assert_eq!(tree(&doc), ["c", "a"]);
        assert!(doc.layer(ids[1]).is_none());
        inverse.apply(&mut doc).unwrap();
        assert_eq!(tree(&doc), ["c", "a", "g", " b"]);

        assert_round_trip(
            &mut doc,
            Edit::SetGroupPassThrough {
                id: g_id,
                pass_through: false,
            },
        );
        assert_eq!(
            Edit::SetGroupPassThrough {
                id: c_id,
                pass_through: false
            }
            .apply(&mut doc),
            Err(EditError::NotAGroup(c_id))
        );
    }

    #[test]
    fn invalid_group_edits_are_refused() {
        let mut doc = Document::new(Size::new(8, 8));
        let inner = group_layer(&mut doc, "inner", Vec::new());
        let inner_id = inner.id;
        let outer = group_layer(&mut doc, "outer", vec![inner]);
        let outer_id = outer.id;
        Edit::InsertLayer {
            parent: None,
            index: 0,
            layer: outer,
        }
        .apply(&mut doc)
        .unwrap();
        let ids = stack(&mut doc, &["a"]);
        let revision = doc.revision();
        let before = doc.layers().to_vec();

        let cases = [
            (
                Edit::MoveLayer {
                    id: outer_id,
                    parent: Some(inner_id),
                    index: 0,
                },
                EditError::MoveIntoItself(outer_id),
            ),
            (
                Edit::MoveLayer {
                    id: outer_id,
                    parent: Some(outer_id),
                    index: 0,
                },
                EditError::MoveIntoItself(outer_id),
            ),
            (
                Edit::MoveLayer {
                    id: inner_id,
                    parent: Some(ids[0]),
                    index: 0,
                },
                EditError::NotAGroup(ids[0]),
            ),
            (
                Edit::MoveLayer {
                    id: ids[0],
                    parent: Some(LayerId::from_raw(999)),
                    index: 0,
                },
                EditError::UnknownLayer(LayerId::from_raw(999)),
            ),
            (
                Edit::MoveLayer {
                    id: ids[0],
                    parent: Some(inner_id),
                    index: 1,
                },
                EditError::IndexOutOfRange { index: 1, len: 0 },
            ),
        ];
        for (edit, expected) in cases {
            assert_eq!(edit.apply(&mut doc), Err(expected));
            assert_eq!(doc.layers(), before.as_slice());
            assert_eq!(doc.revision(), revision);
        }

        // A subtree reusing an id, or one already in the document.
        let mut dup = fill_layer(&mut doc, "dup");
        dup.id = ids[0];
        let bad = group_layer(&mut doc, "bad", vec![dup]);
        let bad_id = bad.id;
        assert_eq!(
            Edit::InsertLayer {
                parent: None,
                index: 0,
                layer: bad,
            }
            .apply(&mut doc),
            Err(EditError::LayerIdInUse(ids[0]))
        );
        assert!(doc.layer(bad_id).is_none());
    }

    #[test]
    fn groups_nest_up_to_the_limit() {
        let mut doc = Document::new(Size::new(8, 8));
        let mut parent = None;
        for depth in 0..MAX_GROUP_DEPTH {
            let g = group_layer(&mut doc, &format!("g{depth}"), Vec::new());
            let id = g.id;
            Edit::InsertLayer {
                parent,
                index: 0,
                layer: g,
            }
            .apply(&mut doc)
            .unwrap();
            parent = Some(id);
        }
        // A layer in the deepest group is fine; one more group is not.
        let leaf = fill_layer(&mut doc, "leaf");
        Edit::InsertLayer {
            parent,
            index: 0,
            layer: leaf,
        }
        .apply(&mut doc)
        .unwrap();
        let extra = group_layer(&mut doc, "extra", Vec::new());
        assert_eq!(
            Edit::InsertLayer {
                parent,
                index: 0,
                layer: extra,
            }
            .apply(&mut doc),
            Err(EditError::TooDeep {
                depth: MAX_GROUP_DEPTH + 1
            })
        );
    }

    #[test]
    fn grouping_takes_the_place_of_the_topmost_layer_and_ungrouping_restores() {
        let mut doc = Document::new(Size::new(8, 8));
        let ids = stack(&mut doc, &["a", "b", "c", "d"]);
        let group = group_layer(&mut doc, "g", Vec::new());
        let g = group.id;
        let edit = Edit::group_layers(&doc, group, &[ids[2], ids[0]]).unwrap();
        let undo = edit.apply(&mut doc).unwrap();
        assert_eq!(tree(&doc), ["b", "g", " a", " c", "d"]);

        let ungroup = Edit::ungroup(&doc, g).unwrap();
        let regroup = ungroup.apply(&mut doc).unwrap();
        assert_eq!(tree(&doc), ["b", "a", "c", "d"]);
        regroup.apply(&mut doc).unwrap();
        assert_eq!(tree(&doc), ["b", "g", " a", " c", "d"]);
        // Undoing the grouping (after the ungroup was undone) restores the original stack.
        undo.apply(&mut doc).unwrap();
        assert_eq!(tree(&doc), ["a", "b", "c", "d"]);

        assert_eq!(
            Edit::ungroup(&doc, ids[0]),
            Err(EditError::NotAGroup(ids[0]))
        );
        let empty = group_layer(&mut doc, "empty", Vec::new());
        assert_eq!(
            Edit::group_layers(&doc, empty, &[]),
            Err(EditError::NoLayers)
        );
    }

    #[test]
    fn grouping_a_group_with_one_of_its_layers_moves_the_group_whole() {
        let mut doc = Document::new(Size::new(8, 8));
        let ids = stack(&mut doc, &["a", "b"]);
        let outer = group_layer(&mut doc, "outer", Vec::new());
        Edit::group_layers(&doc, outer.clone(), &[ids[1]])
            .unwrap()
            .apply(&mut doc)
            .unwrap();
        assert_eq!(tree(&doc), ["a", "outer", " b"]);
        let wrap = group_layer(&mut doc, "wrap", Vec::new());
        // `b` is inside `outer`: it moves with it.
        Edit::group_layers(&doc, wrap, &[ids[1], outer.id, ids[0]])
            .unwrap()
            .apply(&mut doc)
            .unwrap();
        assert_eq!(tree(&doc), ["wrap", " a", " outer", "  b"]);
    }

    #[test]
    fn several_layers_move_together_into_and_out_of_groups() {
        let mut doc = Document::new(Size::new(8, 8));
        let ids = stack(&mut doc, &["a", "b", "c", "d", "e"]);
        let g = group_layer(&mut doc, "g", Vec::new());
        let g_id = g.id;
        Edit::group_layers(&doc, g, &[ids[4]])
            .unwrap()
            .apply(&mut doc)
            .unwrap();
        assert_eq!(tree(&doc), ["a", "b", "c", "d", "g", " e"]);

        // a and c into g, below e (index 0 among g's staying layers).
        let edit = Edit::move_layers(&doc, &[ids[2], ids[0]], Some(g_id), 0).unwrap();
        let before = doc.layers().to_vec();
        let undo = edit.apply(&mut doc).unwrap();
        assert_eq!(tree(&doc), ["b", "d", "g", " a", " c", " e"]);
        undo.apply(&mut doc).unwrap();
        assert_eq!(doc.layers(), before.as_slice());

        // b and d to the top of the stack (above the 3 staying top-level layers: a, c, g).
        Edit::move_layers(&doc, &[ids[1], ids[3]], None, 3)
            .unwrap()
            .apply(&mut doc)
            .unwrap();
        assert_eq!(tree(&doc), ["a", "c", "g", " e", "b", "d"]);

        // Already in place: nothing to do.
        assert_eq!(
            Edit::move_layers(&doc, &[ids[1], ids[3]], None, 3),
            Ok(Edit::Batch(Vec::new()))
        );
        assert_eq!(
            Edit::move_layers(&doc, &[ids[0]], None, 9),
            // Four layers stay at the top level: c, g, b, d.
            Err(EditError::IndexOutOfRange { index: 9, len: 4 })
        );
        assert!(matches!(
            Edit::move_layers(&doc, &[g_id], Some(g_id), 0).and_then(|edit| edit.apply(&mut doc)),
            Err(EditError::MoveIntoItself(_))
        ));
    }

    #[test]
    fn transforms_round_trip_and_must_be_invertible() {
        let mut doc = Document::new(Size::new(8, 8));
        let ids = stack(&mut doc, &["a", "b"]);
        assert_round_trip(
            &mut doc,
            Edit::SetLayerTransform {
                id: ids[0],
                transform: Affine::translation(-3.0, 5.0),
            },
        );
        assert_round_trip(
            &mut doc,
            Edit::SetLayerTransform {
                id: ids[1],
                transform: Affine::rotation(0.3).then(Affine::translation(0.5, 2.25)),
            },
        );
        for transform in [
            Affine {
                a: 0.0,
                ..Affine::IDENTITY
            },
            Affine::translation(f64::NAN, 0.0),
            Affine::translation(1e12, 0.0),
        ] {
            assert_eq!(
                Edit::SetLayerTransform {
                    id: ids[0],
                    transform
                }
                .apply(&mut doc),
                Err(EditError::InvalidTransform)
            );
        }
    }

    #[test]
    fn moving_layers_moves_a_group_whole() {
        let mut doc = Document::new(Size::new(8, 8));
        let ids = stack(&mut doc, &["a", "b"]);
        let g = group_layer(&mut doc, "g", Vec::new());
        let g_id = g.id;
        Edit::group_layers(&doc, g, &[ids[1]])
            .unwrap()
            .apply(&mut doc)
            .unwrap();
        // The group and a layer inside it: only the group moves (its layer with it).
        let undo = Edit::translate_layers(&doc, &[ids[1], g_id, ids[0]], 4, -2)
            .unwrap()
            .apply(&mut doc)
            .unwrap();
        assert_eq!(
            doc.layer(g_id).unwrap().transform,
            Affine::translation(4.0, -2.0)
        );
        assert_eq!(
            doc.layer(ids[0]).unwrap().transform,
            Affine::translation(4.0, -2.0)
        );
        assert!(doc.layer(ids[1]).unwrap().transform.is_identity());
        Edit::translate_layers(&doc, &[ids[0]], 1, 1)
            .unwrap()
            .apply(&mut doc)
            .unwrap();
        assert_eq!(
            doc.layer(ids[0]).unwrap().transform,
            Affine::translation(5.0, -1.0)
        );
        let undo_second = Edit::translate_layers(&doc, &[ids[0]], -1, -1)
            .unwrap()
            .apply(&mut doc)
            .unwrap();
        undo_second.apply(&mut doc).unwrap();
        assert_eq!(
            doc.layer(ids[0]).unwrap().transform,
            Affine::translation(5.0, -1.0)
        );
        // Undoing the first move puts every layer back where it was.
        undo.apply(&mut doc).unwrap();
        assert!(doc.layer(g_id).unwrap().transform.is_identity());
        assert_eq!(doc.layer(ids[0]).unwrap().transform, Affine::IDENTITY);
    }

    #[test]
    fn transforming_layers_applies_in_the_document_space() {
        let mut doc = Document::new(Size::new(8, 8));
        let ids = stack(&mut doc, &["a", "b"]);
        let g = group_layer(&mut doc, "g", Vec::new());
        let g_id = g.id;
        Edit::group_layers(&doc, g, &[ids[1]])
            .unwrap()
            .apply(&mut doc)
            .unwrap();
        // The group is scaled 2×: its layer's own transform is in the group's space.
        Edit::SetLayerTransform {
            id: g_id,
            transform: Affine::scale(2.0, 2.0),
        }
        .apply(&mut doc)
        .unwrap();
        let by = Affine::translation(6.0, 0.0);
        let undo = Edit::transform_layers(&doc, &[ids[1], ids[0]], by)
            .unwrap()
            .apply(&mut doc)
            .unwrap();
        // 6 document pixels are 3 of the group's.
        assert_eq!(
            doc.layer(ids[1]).unwrap().transform,
            Affine::translation(3.0, 0.0)
        );
        assert_eq!(
            doc.layer(ids[0]).unwrap().transform,
            Affine::translation(6.0, 0.0)
        );
        // Four quarter turns by floating-point steps come back to exactly where they were.
        for _ in 0..4 {
            Edit::transform_layers(
                &doc,
                &[ids[0]],
                Affine::rotation(std::f64::consts::FRAC_PI_2),
            )
            .unwrap()
            .apply(&mut doc)
            .unwrap();
        }
        assert_eq!(
            doc.layer(ids[0]).unwrap().transform,
            Affine::translation(6.0, 0.0)
        );
        assert_eq!(
            Edit::transform_layers(&doc, &[ids[0]], Affine::scale(0.0, 1.0))
                .and_then(|e| e.apply(&mut doc)),
            Err(EditError::InvalidTransform)
        );
        undo.apply(&mut doc).unwrap();
        assert!(doc.layer(ids[0]).unwrap().transform.is_identity());
        assert!(doc.layer(ids[1]).unwrap().transform.is_identity());
    }

    /// An image of `size` filled with one gray (`gray`) or RGBA (otherwise) 8-bit value.
    fn image(size: Size, gray: bool, value: u8) -> Arc<RasterImage> {
        let format = crate::color::PixelFormat {
            layout: if gray {
                crate::color::ChannelLayout::Gray
            } else {
                crate::color::ChannelLayout::Rgba
            },
            ..crate::color::PixelFormat::RGBA8_SRGB
        };
        let channels = if gray { 1 } else { 4 };
        let pixels = vec![value; size.pixel_count() as usize * channels];
        Arc::new(RasterImage::from_pixels(size, format, &pixels).unwrap())
    }

    fn raster_layer(doc: &mut Document, image: Arc<RasterImage>) -> LayerId {
        let id = doc.allocate_layer_id();
        let layer = Layer {
            id,
            content: LayerContent::raster(image),
            ..fill_layer(doc, "pixels")
        };
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

    /// What layer `id` shows and its stack.
    fn shown(doc: &Document, id: LayerId) -> (Arc<RasterImage>, Option<LayerStack>) {
        match &doc.layer(id).unwrap().content {
            LayerContent::Raster { image, stack } => (Arc::clone(image), stack.clone()),
            _ => panic!("a raster layer"),
        }
    }

    /// `original` with one paint entry: the first pixels of its first tile in `value`.
    fn painted_stack(original: &Arc<RasterImage>, value: f32) -> LayerStack {
        let empty = PaintEntry::empty(original.format(), original.size(), BlendSpace::Perceptual);
        let coord = crate::tile::TileCoord { col: 0, row: 0 };
        let tile = empty.painted_tile(
            coord,
            PaintOp::Color(LinearRgba::new(value, value, value, 1.0)),
            |x, y| if x + y < 4 { 1.0 } else { 0.0 },
        );
        let paint = empty.with_tiles(vec![(coord, tile)]).unwrap();
        LayerStack::new(Arc::clone(original))
            .with_top_paint(Arc::new(paint))
            .unwrap()
    }

    #[test]
    fn a_stack_shows_its_result_and_comes_off_whole() {
        let size = Size::new(8, 8);
        let mut doc = Document::new(size);
        let original = image(size, false, 10);
        let id = raster_layer(&mut doc, Arc::clone(&original));
        assert!(!doc.layer(id).unwrap().is_painted());

        let first = painted_stack(&original, 1.0);
        let undo_first = Edit::SetLayerStack {
            id,
            stack: first.clone(),
            shown: None,
        }
        .apply(&mut doc)
        .unwrap();
        assert_eq!(
            undo_first,
            Edit::SetLayerStack {
                id,
                stack: LayerStack::new(Arc::clone(&original)),
                shown: None
            }
        );
        let (image_now, kept) = shown(&doc, id);
        assert_eq!(kept.as_ref(), Some(&first));
        assert_eq!(image_now.alpha_at(0, 0), 1.0);
        assert_eq!(image_now.levels()[0].tiles()[0][0], 255, "painted white");
        assert!(doc.layer(id).unwrap().is_painted());

        // An effect on top; undoing it evaluates again.
        let second = first
            .with_effect(Effect {
                adjustment: crate::adjust::Adjustment::Invert,
                selection: None,
                to_document: Affine::IDENTITY,
                space: BlendSpace::Perceptual,
            })
            .unwrap();
        let undo_second = Edit::SetLayerStack {
            id,
            stack: second.clone(),
            shown: None,
        }
        .apply(&mut doc)
        .unwrap();
        assert_eq!(shown(&doc, id).0.levels()[0].tiles()[0][0], 0, "inverted");
        assert_round_trip(&mut doc.clone(), undo_second.clone());

        // Delete Paint, then undo it.
        let undo_delete = Edit::delete_paint(&doc, &[id])
            .unwrap()
            .apply(&mut doc)
            .unwrap();
        let (image_now, kept) = shown(&doc, id);
        assert!(Arc::ptr_eq(&image_now, &original) && kept.is_none());
        assert_eq!(
            Edit::delete_paint(&doc, &[id]),
            Err(EditError::NoLayers),
            "nothing left to delete"
        );
        undo_delete.apply(&mut doc).unwrap();
        assert_eq!(shown(&doc, id).1, Some(second));
        undo_second.apply(&mut doc).unwrap();
        let (image_now, kept) = shown(&doc, id);
        assert_eq!(kept, Some(first));
        assert_eq!(image_now.levels()[0].tiles()[0][0], 255);
        undo_first.apply(&mut doc).unwrap();
        let (image_now, kept) = shown(&doc, id);
        assert!(Arc::ptr_eq(&image_now, &original) && kept.is_none());
    }

    #[test]
    fn deleting_an_entry_merges_its_neighbours_and_undoes() {
        let size = Size::new(8, 8);
        let mut doc = Document::new(size);
        let original = image(size, false, 10);
        let id = raster_layer(&mut doc, Arc::clone(&original));
        let invert = Effect {
            adjustment: crate::adjust::Adjustment::Invert,
            selection: None,
            to_document: Affine::IDENTITY,
            space: BlendSpace::Perceptual,
        };
        let painted = painted_stack(&original, 1.0);
        let stack = painted
            .with_effect(invert)
            .unwrap()
            .with_top_paint(Arc::clone(
                match painted_stack(&original, 0.5).entries().last() {
                    Some(crate::stack::Entry::Paint(paint)) => paint,
                    _ => panic!("a paint entry"),
                },
            ))
            .unwrap();
        Edit::SetLayerStack {
            id,
            stack: stack.clone(),
            shown: None,
        }
        .apply(&mut doc)
        .unwrap();
        let undo = Edit::delete_entry(&doc, id, 1)
            .unwrap()
            .apply(&mut doc)
            .unwrap();
        // The two paints merged.
        assert_eq!(shown(&doc, id).1.unwrap().entries().len(), 1);
        undo.apply(&mut doc).unwrap();
        assert_eq!(shown(&doc, id).1, Some(stack));
        assert!(matches!(
            Edit::delete_entry(&doc, id, 3),
            Err(EditError::Stack(_))
        ));
    }

    #[test]
    fn adjustments_apply_to_visible_raster_layers_within_the_selection() {
        let size = Size::new(8, 8);
        let mut doc = Document::new(size);
        let a = raster_layer(&mut doc, image(size, false, 10));
        let hidden = raster_layer(&mut doc, image(size, false, 10));
        Edit::SetLayerVisible {
            id: hidden,
            visible: false,
        }
        .apply(&mut doc)
        .unwrap();
        let fill = fill_layer(&mut doc, "fill");
        let fill_id = fill.id;
        Edit::InsertLayer {
            parent: None,
            index: 2,
            layer: fill,
        }
        .apply(&mut doc)
        .unwrap();
        let ids = [a, hidden, fill_id];
        assert_eq!(Edit::effect_targets(&doc, &ids), vec![a]);
        let edit = Edit::apply_effect(&doc, &ids, crate::adjust::Adjustment::Invert).unwrap();
        let undo = edit.apply(&mut doc).unwrap();
        // 255 − 10 everywhere: no selection.
        assert_eq!(shown(&doc, a).0.levels()[0].tiles()[0][0], 245);
        assert!(shown(&doc, hidden).1.is_none());
        undo.apply(&mut doc).unwrap();
        assert!(shown(&doc, a).1.is_none());
        assert_eq!(
            Edit::apply_effect(&doc, &[hidden], crate::adjust::Adjustment::Invert),
            Err(EditError::NoLayers)
        );
    }

    #[test]
    fn a_stack_must_fit_its_layer() {
        let size = Size::new(8, 8);
        let mut doc = Document::new(size);
        let original = image(size, false, 10);
        let id = raster_layer(&mut doc, Arc::clone(&original));
        let wrong = Edit::SetLayerStack {
            id,
            stack: painted_stack(&original, 1.0),
            shown: Some(image(Size::new(9, 8), false, 0)),
        };
        assert_eq!(wrong.apply(&mut doc), Err(EditError::InvalidPaint));
        let fill = fill_layer(&mut doc, "fill");
        let fill_id = fill.id;
        Edit::InsertLayer {
            parent: None,
            index: 1,
            layer: fill,
        }
        .apply(&mut doc)
        .unwrap();
        let on_fill = Edit::SetLayerStack {
            id: fill_id,
            stack: painted_stack(&original, 1.0),
            shown: None,
        };
        assert_eq!(on_fill.apply(&mut doc), Err(EditError::NotRaster(fill_id)));
        let no_mask = Edit::SetMaskPaint {
            id,
            painted: Some(image(size, true, 0)),
        };
        assert_eq!(no_mask.apply(&mut doc), Err(EditError::NoMask(id)));
    }

    #[test]
    fn masks_are_painted_apart_and_deleted_with_the_pixels() {
        let size = Size::new(8, 8);
        let mut doc = Document::new(size);
        let id = raster_layer(&mut doc, image(size, false, 10));
        let mask = image(size, true, 255);
        Edit::SetLayerMask {
            id,
            mask: Some(LayerMask {
                image: Arc::clone(&mask),
                enabled: true,
                replaces_alpha: false,
                original: None,
            }),
        }
        .apply(&mut doc)
        .unwrap();
        let color = Edit::SetMaskPaint {
            id,
            painted: Some(image(size, false, 0)),
        };
        assert_eq!(color.apply(&mut doc), Err(EditError::InvalidPaint));
        let painted = image(size, true, 0);
        assert_round_trip(
            &mut doc,
            Edit::SetMaskPaint {
                id,
                painted: Some(Arc::clone(&painted)),
            },
        );
        let original = Arc::clone(doc.layer(id).unwrap().content.original().unwrap());
        Edit::SetLayerStack {
            id,
            stack: painted_stack(&original, 1.0),
            shown: None,
        }
        .apply(&mut doc)
        .unwrap();
        let layer = doc.layer(id).unwrap();
        assert!(Arc::ptr_eq(&layer.mask.as_ref().unwrap().image, &painted));
        // Both come off in one edit.
        let delete = Edit::delete_paint(&doc, &[id]).unwrap();
        assert_round_trip(&mut doc.clone(), delete);
        Edit::delete_paint(&doc, &[id])
            .unwrap()
            .apply(&mut doc)
            .unwrap();
        let layer = doc.layer(id).unwrap();
        assert!(!layer.is_painted());
        assert!(Arc::ptr_eq(&layer.mask.as_ref().unwrap().image, &mask));
    }
}
