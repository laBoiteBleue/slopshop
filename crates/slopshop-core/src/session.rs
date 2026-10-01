//! A document together with its undo/redo history.

use crate::blend::BlendMode;
use crate::document::{Document, Layer, LayerContent, LayerId};
use crate::edit::{Edit, EditError};

/// What [`Session::insert_layer_copies`] made.
#[derive(Debug, Clone, PartialEq)]
pub struct Copies {
    /// The group holding the copies, when one was asked for.
    pub group: Option<LayerId>,
    /// The id of each copy, in the order the copied layers and their subtrees are walked depth
    /// first (as [`Document::all_layers`] does).
    pub ids: Vec<LayerId>,
}

/// An editing session: the document and the inverse edits needed to undo/redo.
///
/// History is linear: performing a new edit discards the redo stack. Continuous interactions
/// (e.g. a slider drag) are *gestures*: applied live, recorded as one entry. Bounding history
/// memory is not needed yet.
#[derive(Debug)]
pub struct Session {
    document: Document,
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    /// Inverses of the edits applied by the gesture in progress, in application order.
    gesture: Vec<Edit>,
}

impl Session {
    pub fn new(document: Document) -> Self {
        Self {
            document,
            undo: Vec::new(),
            redo: Vec::new(),
            gesture: Vec::new(),
        }
    }

    pub fn document(&self) -> &Document {
        &self.document
    }

    /// See [`Document::allocate_layer_id`].
    pub fn allocate_layer_id(&mut self) -> LayerId {
        self.document.allocate_layer_id()
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty() || !self.gesture.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Apply an edit and record it. Ends any gesture in progress first. On error nothing
    /// changes, history included.
    pub fn perform(&mut self, edit: Edit) -> Result<(), EditError> {
        // An empty batch changes nothing: no history entry, no revision.
        if matches!(&edit, Edit::Batch(edits) if edits.is_empty()) {
            return Ok(());
        }
        self.end_gesture();
        let inverse = edit.apply(&mut self.document)?;
        self.push_undo(inverse);
        Ok(())
    }

    /// Apply an edit as part of a continuous gesture (e.g. dragging a slider): the document
    /// changes immediately, and all edits of the gesture become a single history entry when
    /// [`Self::end_gesture`] is called.
    pub fn perform_in_gesture(&mut self, edit: Edit) -> Result<(), EditError> {
        let inverse = edit.apply(&mut self.document)?;
        self.gesture.push(inverse);
        Ok(())
    }

    /// Insert copies of `layers` (bottom to top, e.g. another document's stack) above the
    /// current stack, with fresh ids (groups with everything inside them), as one undoable
    /// entry; inside a new pass-through group named `group` when given. Raster pixels are
    /// shared, never copied. On error nothing changes.
    pub fn insert_layer_copies(
        &mut self,
        layers: &[Layer],
        group: Option<String>,
    ) -> Result<Copies, EditError> {
        let mut ids = Vec::new();
        let copies: Vec<Layer> = layers
            .iter()
            .map(|layer| self.fresh_copy(layer, &mut ids))
            .collect();
        let base = self.document.layers().len();
        let (group, edit) = match group {
            Some(name) => {
                let id = self.document.allocate_layer_id();
                let edit = Edit::InsertLayer {
                    parent: None,
                    index: base,
                    layer: Layer {
                        transform: crate::transform::Affine::IDENTITY,
                        clipped: false,
                        id,
                        name,
                        visible: true,
                        opacity: 1.0,
                        blend_mode: BlendMode::Normal,
                        mask: None,
                        // Isolated: the copied document's adjustments stay within it.
                        content: LayerContent::Group {
                            children: copies,
                            pass_through: false,
                        },
                    },
                };
                (Some(id), edit)
            }
            None => (
                None,
                Edit::Batch(
                    copies
                        .into_iter()
                        .enumerate()
                        .map(|(offset, layer)| Edit::InsertLayer {
                            parent: None,
                            index: base + offset,
                            layer,
                        })
                        .collect(),
                ),
            ),
        };
        self.perform(edit)?;
        Ok(Copies { group, ids })
    }

    /// The edit that duplicates `ids` (Layer > Duplicate Layer): each copy, with fresh ids (a
    /// group with everything inside it), goes right above its original, named by `name` from
    /// the original's name. A layer inside another of `ids` is copied with it. Not performed:
    /// the caller performs it (one undo entry).
    pub fn duplicate_layers_edit(
        &mut self,
        ids: &[LayerId],
        name: impl Fn(&str) -> String,
    ) -> Result<Edit, EditError> {
        let originals = crate::edit::outermost_in_order(&self.document, ids)?;
        if originals.is_empty() {
            return Err(EditError::NoLayers);
        }
        let mut copies = Vec::with_capacity(originals.len());
        for &id in &originals {
            let original = self
                .document
                .layer(id)
                .ok_or(EditError::UnknownLayer(id))?
                .clone();
            let mut copy = self.fresh_copy(&original, &mut Vec::new());
            copy.name = name(&original.name);
            copies.push((id, copy));
        }
        // Positions are worked out on a copy of the document as the copies go in.
        let mut plan = self.document.clone();
        let mut edits = Vec::with_capacity(copies.len());
        for (id, layer) in copies {
            let (parent, index) = plan.locate(id).ok_or(EditError::UnknownLayer(id))?;
            let edit = Edit::InsertLayer {
                parent,
                index: index + 1,
                layer,
            };
            edit.clone().apply(&mut plan)?;
            edits.push(edit);
        }
        Ok(Edit::Batch(edits))
    }

    /// A copy of `layer`, and for a group of everything inside it, with fresh ids, pushed to
    /// `ids` depth first.
    fn fresh_copy(&mut self, layer: &Layer, ids: &mut Vec<LayerId>) -> Layer {
        let mut copy = layer.clone();
        copy.id = self.document.allocate_layer_id();
        ids.push(copy.id);
        if let LayerContent::Group { children, .. } = &mut copy.content {
            for child in children.iter_mut() {
                *child = self.fresh_copy(child, ids);
            }
        }
        copy
    }

    /// Record the gesture in progress as one undoable entry. No-op if there is none.
    pub fn end_gesture(&mut self) {
        let mut inverses = std::mem::take(&mut self.gesture);
        let entry = match inverses.len() {
            0 => return,
            1 => inverses.remove(0),
            _ => {
                inverses.reverse();
                Edit::Batch(inverses)
            }
        };
        self.push_undo(entry);
    }

    /// Revert the gesture in progress, leaving no history entry (a transform cancelled with
    /// Esc). Returns `Ok(false)` if there was none.
    pub fn cancel_gesture(&mut self) -> Result<bool, EditError> {
        let inverses = std::mem::take(&mut self.gesture);
        if inverses.is_empty() {
            return Ok(false);
        }
        for inverse in inverses.into_iter().rev() {
            inverse.apply(&mut self.document)?;
        }
        Ok(true)
    }

    /// Undo the last edit (ending any gesture first). Returns `Ok(false)` if there was nothing
    /// to undo.
    pub fn undo(&mut self) -> Result<bool, EditError> {
        self.end_gesture();
        Self::step(&mut self.document, &mut self.undo, &mut self.redo)
    }

    /// Redo the last undone edit. Returns `Ok(false)` if there was nothing to redo.
    pub fn redo(&mut self) -> Result<bool, EditError> {
        self.end_gesture();
        Self::step(&mut self.document, &mut self.redo, &mut self.undo)
    }

    fn push_undo(&mut self, inverse: Edit) {
        self.undo.push(inverse);
        self.redo.clear();
    }

    fn step(
        doc: &mut Document,
        from: &mut Vec<Edit>,
        to: &mut Vec<Edit>,
    ) -> Result<bool, EditError> {
        let Some(edit) = from.pop() else {
            return Ok(false);
        };
        // Inverses are exact, so this can only fail on a bug. Keep the entry in that case so
        // history is not silently lost.
        match edit.clone().apply(doc) {
            Ok(inverse) => {
                to.push(inverse);
                Ok(true)
            }
            Err(err) => {
                from.push(edit);
                Err(err)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blend::BlendMode;
    use crate::color::LinearRgba;
    use crate::document::{Layer, LayerContent};
    use crate::geom::Size;

    fn add_layer(session: &mut Session, name: &str) -> LayerId {
        let id = session.allocate_layer_id();
        let index = session.document().layers().len();
        session
            .perform(Edit::InsertLayer {
                parent: None,
                index,
                layer: Layer {
                    transform: crate::transform::Affine::IDENTITY,
                    clipped: false,
                    id,
                    name: name.into(),
                    visible: true,
                    opacity: 1.0,
                    blend_mode: BlendMode::Normal,
                    mask: None,
                    content: LayerContent::Fill {
                        color: LinearRgba::new(0.0, 0.0, 1.0, 1.0),
                    },
                },
            })
            .unwrap();
        id
    }

    fn names(session: &Session) -> Vec<String> {
        session
            .document()
            .layers()
            .iter()
            .map(|l| l.name.clone())
            .collect()
    }

    #[test]
    fn undo_redo_sequence() {
        let mut s = Session::new(Document::new(Size::new(16, 16)));
        assert!(!s.can_undo() && !s.can_redo());

        let a = add_layer(&mut s, "a");
        add_layer(&mut s, "b");
        s.perform(Edit::SetLayerVisible {
            id: a,
            visible: false,
        })
        .unwrap();

        assert!(s.undo().unwrap());
        assert!(s.document().layer(a).unwrap().visible);
        assert!(s.undo().unwrap());
        assert_eq!(names(&s), ["a"]);
        assert!(s.redo().unwrap());
        assert_eq!(names(&s), ["a", "b"]);
        assert!(s.redo().unwrap());
        assert!(!s.document().layer(a).unwrap().visible);
        assert!(!s.redo().unwrap());

        while s.undo().unwrap() {}
        assert!(s.document().layers().is_empty());
        assert!(s.can_redo());
    }

    #[test]
    fn gesture_is_one_history_entry() {
        let mut s = Session::new(Document::new(Size::new(16, 16)));
        let a = add_layer(&mut s, "a");
        let entries_before = s.undo.len();

        for opacity in [0.9, 0.6, 0.3] {
            s.perform_in_gesture(Edit::SetLayerOpacity { id: a, opacity })
                .unwrap();
            // Live: the document reflects the gesture immediately.
            assert_eq!(s.document().layer(a).unwrap().opacity, opacity);
        }
        assert!(s.can_undo());
        s.end_gesture();
        assert_eq!(s.undo.len(), entries_before + 1);

        assert!(s.undo().unwrap());
        assert_eq!(s.document().layer(a).unwrap().opacity, 1.0);
        assert!(s.redo().unwrap());
        assert_eq!(s.document().layer(a).unwrap().opacity, 0.3);
    }

    #[test]
    fn a_cancelled_gesture_leaves_no_trace() {
        let mut s = Session::new(Document::new(Size::new(16, 16)));
        let a = add_layer(&mut s, "a");
        let entries_before = s.undo.len();
        for opacity in [0.9, 0.6, 0.3] {
            s.perform_in_gesture(Edit::SetLayerOpacity { id: a, opacity })
                .unwrap();
        }
        assert!(s.cancel_gesture().unwrap());
        assert_eq!(s.document().layer(a).unwrap().opacity, 1.0);
        assert_eq!(s.undo.len(), entries_before);
        assert!(!s.cancel_gesture().unwrap());
    }

    #[test]
    fn undo_or_perform_ends_pending_gesture() {
        let mut s = Session::new(Document::new(Size::new(16, 16)));
        let a = add_layer(&mut s, "a");
        s.perform_in_gesture(Edit::SetLayerOpacity {
            id: a,
            opacity: 0.5,
        })
        .unwrap();
        // Undo without an explicit end: the gesture is recorded, then undone.
        assert!(s.undo().unwrap());
        assert_eq!(s.document().layer(a).unwrap().opacity, 1.0);

        s.perform_in_gesture(Edit::SetLayerVisible {
            id: a,
            visible: false,
        })
        .unwrap();
        s.perform(Edit::RenameLayer {
            id: a,
            name: "b".into(),
        })
        .unwrap();
        s.undo().unwrap();
        assert!(
            !s.document().layer(a).unwrap().visible,
            "rename undone alone"
        );
        s.undo().unwrap();
        assert!(s.document().layer(a).unwrap().visible, "then the gesture");
    }

    #[test]
    fn end_gesture_without_gesture_is_noop() {
        let mut s = Session::new(Document::new(Size::new(16, 16)));
        add_layer(&mut s, "a");
        s.undo().unwrap();
        s.end_gesture();
        assert!(s.can_redo(), "an empty gesture must not clear redo");
    }

    #[test]
    fn new_edit_clears_redo() {
        let mut s = Session::new(Document::new(Size::new(16, 16)));
        add_layer(&mut s, "a");
        s.undo().unwrap();
        assert!(s.can_redo());
        add_layer(&mut s, "b");
        assert!(!s.can_redo());
        assert_eq!(names(&s), ["b"]);
    }

    #[test]
    fn layer_copies_are_one_undo_entry_with_fresh_ids_and_shared_pixels() {
        use crate::raster::RasterImage;
        use std::sync::Arc;

        let mut source = Session::new(Document::new(Size::new(16, 16)));
        add_layer(&mut source, "fill");
        let image = Arc::new(
            RasterImage::from_pixels(
                Size::new(2, 2),
                crate::color::PixelFormat::RGBA8_SRGB,
                &[255; 16],
            )
            .unwrap(),
        );
        let id = source.allocate_layer_id();
        source
            .perform(Edit::InsertLayer {
                parent: None,
                index: 1,
                layer: Layer {
                    transform: crate::transform::Affine::IDENTITY,
                    clipped: false,
                    id,
                    name: "photo".into(),
                    visible: false,
                    opacity: 0.5,
                    blend_mode: BlendMode::Normal,
                    mask: None,
                    content: LayerContent::Raster {
                        image: image.clone(),
                    },
                },
            })
            .unwrap();

        let mut target = Session::new(Document::new(Size::new(16, 16)));
        let existing = add_layer(&mut target, "background");
        let ids = target
            .insert_layer_copies(source.document().layers(), None)
            .unwrap()
            .ids;
        assert_eq!(names(&target), ["background", "fill", "photo"]);
        let layers = target.document().layers();
        assert_eq!(ids, [layers[1].id, layers[2].id]);
        assert!(!ids.contains(&existing) && ids[0] != ids[1]);
        // Properties kept, pixels shared.
        assert!(!layers[2].visible && layers[2].opacity == 0.5);
        let LayerContent::Raster { image: copied } = &layers[2].content else {
            panic!("raster expected");
        };
        assert!(Arc::ptr_eq(copied, &image));
        // One undo removes both copies.
        target.undo().unwrap();
        assert_eq!(names(&target), ["background"]);
        assert!(
            target
                .insert_layer_copies(&[], None)
                .unwrap()
                .ids
                .is_empty()
        );

        // Into a group: one layer at the top, holding the copies.
        let copies = target
            .insert_layer_copies(source.document().layers(), Some("import".into()))
            .unwrap();
        let group = copies.group.unwrap();
        assert_eq!(names(&target), ["background", "import"]);
        let inside: Vec<LayerId> = target
            .document()
            .layer(group)
            .unwrap()
            .children()
            .unwrap()
            .iter()
            .map(|l| l.id)
            .collect();
        assert_eq!(inside, copies.ids);
        target.undo().unwrap();
        assert_eq!(names(&target), ["background"]);
    }

    #[test]
    fn failed_edit_does_not_touch_history() {
        let mut s = Session::new(Document::new(Size::new(16, 16)));
        add_layer(&mut s, "a");
        s.undo().unwrap();
        let err = s.perform(Edit::RemoveLayer {
            id: LayerId::from_raw(42),
        });
        assert!(err.is_err());
        assert!(s.can_redo(), "a failed edit must not clear redo");
    }

    #[test]
    fn copied_groups_get_fresh_ids_for_their_whole_subtree() {
        let mut source = Session::new(Document::new(Size::new(4, 4)));
        let child = add_layer(&mut source, "child");
        let group = source.allocate_layer_id();
        source
            .perform(Edit::InsertLayer {
                parent: None,
                index: 1,
                layer: Layer {
                    transform: crate::transform::Affine::IDENTITY,
                    clipped: false,
                    id: group,
                    name: "group".into(),
                    visible: true,
                    opacity: 1.0,
                    blend_mode: BlendMode::Normal,
                    mask: None,
                    content: LayerContent::Group {
                        children: Vec::new(),
                        pass_through: true,
                    },
                },
            })
            .unwrap();
        source
            .perform(Edit::MoveLayer {
                id: child,
                parent: Some(group),
                index: 0,
            })
            .unwrap();

        // The target already uses the source's ids.
        let mut target = Session::new(Document::new(Size::new(4, 4)));
        add_layer(&mut target, "a");
        add_layer(&mut target, "b");
        let ids = target
            .insert_layer_copies(source.document().layers(), None)
            .unwrap()
            .ids;
        let copy = target.document().layer(ids[0]).unwrap();
        let copied_child = copy.children().unwrap()[0].id;
        let all: Vec<LayerId> = target.document().all_layers().map(|l| l.id).collect();
        let unique: std::collections::HashSet<_> = all.iter().collect();
        assert_eq!(unique.len(), all.len(), "every id is unique");
        assert_eq!(
            target.document().locate(copied_child),
            Some((Some(ids[0]), 0))
        );
        // One undo entry removes the copy with everything inside it.
        target.undo().unwrap();
        assert_eq!(target.document().all_layers().count(), 2);
    }

    #[test]
    fn duplicates_go_above_their_originals_with_fresh_ids() {
        let mut s = Session::new(Document::new(Size::new(4, 4)));
        let a = add_layer(&mut s, "a");
        add_layer(&mut s, "b");
        let c = add_layer(&mut s, "c");
        let edit = s
            .duplicate_layers_edit(&[c, a], |name| format!("{name} copy"))
            .unwrap();
        s.perform(edit).unwrap();
        assert_eq!(names(&s), ["a", "a copy", "b", "c", "c copy"]);
        let all: Vec<LayerId> = s.document().all_layers().map(|l| l.id).collect();
        let unique: std::collections::HashSet<_> = all.iter().collect();
        assert_eq!(unique.len(), all.len());
        // One undo entry removes both copies.
        s.undo().unwrap();
        assert_eq!(names(&s), ["a", "b", "c"]);
        assert_eq!(
            s.duplicate_layers_edit(&[], |n| n.to_owned()),
            Err(EditError::NoLayers)
        );
    }
}
