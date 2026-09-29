//! Reversible document edits.
//!
//! An [`Edit`] is the only way to mutate a [`Document`]. Applying an edit validates it first
//! (a failed edit leaves the document untouched) and returns its exact inverse, which is what
//! the undo/redo history stores.
//!
//! On error the document content is unchanged. (A failing [`Edit::Batch`] rolls back what it
//! applied; its revision still advances, since revisions must never be reused.)

use std::fmt;

use crate::document::{Document, Layer, LayerContent, LayerId};

#[derive(Debug, Clone, PartialEq)]
pub enum Edit {
    /// Insert `layer` at `index` in the stack (0 = bottom, `len` = top). Its id must come from
    /// [`Document::allocate_layer_id`] and not be in use.
    InsertLayer {
        index: usize,
        layer: Layer,
    },
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
    /// Move a layer so that it ends up at `index` in the stack (0 = bottom).
    MoveLayer {
        id: LayerId,
        index: usize,
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
        }
    }
}

impl std::error::Error for EditError {}

impl Edit {
    /// Apply the edit and return its inverse. On error the document content is unchanged.
    pub fn apply(self, doc: &mut Document) -> Result<Edit, EditError> {
        let inverse = match self {
            Edit::InsertLayer { index, layer } => {
                validate_new_layer(doc, index, &layer)?;
                let id = layer.id;
                doc.layers_mut().insert(index, layer);
                Edit::RemoveLayer { id }
            }
            Edit::RemoveLayer { id } => {
                let index = doc.layer_index(id).ok_or(EditError::UnknownLayer(id))?;
                let layer = doc.layers_mut().remove(index);
                Edit::InsertLayer { index, layer }
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
            Edit::MoveLayer { id, index } => {
                let from = doc.layer_index(id).ok_or(EditError::UnknownLayer(id))?;
                let len = doc.layers().len();
                if index >= len {
                    return Err(EditError::IndexOutOfRange { index, len });
                }
                let layer = doc.layers_mut().remove(from);
                doc.layers_mut().insert(index, layer);
                Edit::MoveLayer { id, index: from }
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

fn validate_new_layer(doc: &Document, index: usize, layer: &Layer) -> Result<(), EditError> {
    let len = doc.layers().len();
    if index > len {
        return Err(EditError::IndexOutOfRange { index, len });
    }
    if !doc.is_allocated(layer.id) {
        return Err(EditError::LayerIdNotAllocated(layer.id));
    }
    if doc.layer(layer.id).is_some() {
        return Err(EditError::LayerIdInUse(layer.id));
    }
    validate_opacity(layer.opacity)?;
    match &layer.content {
        LayerContent::Fill { color } if !color.is_finite() => Err(EditError::InvalidColor),
        LayerContent::Fill { .. } => Ok(()),
    }
}

fn validate_opacity(opacity: f32) -> Result<(), EditError> {
    if (0.0..=1.0).contains(&opacity) {
        Ok(())
    } else {
        Err(EditError::InvalidOpacity(opacity))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::LinearRgba;
    use crate::geom::Size;

    fn fill_layer(doc: &mut Document, name: &str) -> Layer {
        Layer {
            id: doc.allocate_layer_id(),
            name: name.to_owned(),
            visible: true,
            opacity: 1.0,
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
        Edit::InsertLayer { index: 0, layer: a }
            .apply(&mut doc)
            .unwrap();
        let inverse = Edit::InsertLayer { index: 0, layer: b }
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
        Edit::InsertLayer { index: 0, layer }
            .apply(&mut doc)
            .unwrap();

        for edit in [
            Edit::SetLayerVisible { id, visible: false },
            Edit::SetLayerOpacity { id, opacity: 0.25 },
            Edit::RenameLayer {
                id,
                name: "renamed".into(),
            },
            Edit::RemoveLayer { id },
        ] {
            assert_round_trip(&mut doc, edit);
        }
    }

    fn stack(doc: &mut Document, names: &[&str]) -> Vec<LayerId> {
        names
            .iter()
            .map(|name| {
                let layer = fill_layer(doc, name);
                let id = layer.id;
                let index = doc.layers().len();
                Edit::InsertLayer { index, layer }.apply(doc).unwrap();
                id
            })
            .collect()
    }

    #[test]
    fn move_layer_and_inverse() {
        let mut doc = Document::new(Size::new(8, 8));
        let ids = stack(&mut doc, &["a", "b", "c", "d"]);

        let inverse = Edit::MoveLayer {
            id: ids[0],
            index: 3,
        }
        .apply(&mut doc)
        .unwrap();
        assert_eq!(names(&doc), ["b", "c", "d", "a"]);
        inverse.apply(&mut doc).unwrap();
        assert_eq!(names(&doc), ["a", "b", "c", "d"]);

        Edit::MoveLayer {
            id: ids[3],
            index: 1,
        }
        .apply(&mut doc)
        .unwrap();
        assert_eq!(names(&doc), ["a", "d", "b", "c"]);
        assert_round_trip(
            &mut doc,
            Edit::MoveLayer {
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
                Edit::InsertLayer { index: 0, layer },
                EditError::LayerIdInUse(id),
            ),
            (
                Edit::InsertLayer {
                    index: 0,
                    layer: unallocated,
                },
                EditError::LayerIdNotAllocated(ghost),
            ),
            (
                Edit::InsertLayer {
                    index: 5,
                    layer: nan_color.clone(),
                },
                EditError::IndexOutOfRange { index: 5, len: 1 },
            ),
            (
                Edit::InsertLayer {
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
            Edit::InsertLayer {
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
        let inverse = Edit::InsertLayer { index: 0, layer }
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
        let undo = Edit::InsertLayer { index: 0, layer }
            .apply(&mut doc)
            .unwrap();
        undo.apply(&mut doc).unwrap();
        assert_ne!(doc.allocate_layer_id(), first);
    }
}
