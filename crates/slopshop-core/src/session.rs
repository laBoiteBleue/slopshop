//! A document together with its undo/redo history.

use crate::document::{Document, Layer, LayerId};
use crate::edit::{Edit, EditError};

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
    /// current stack, with fresh ids, as one undoable entry. Raster pixels are shared, never
    /// copied. Returns the new ids, bottom to top; on error nothing changes.
    pub fn insert_layer_copies(&mut self, layers: &[Layer]) -> Result<Vec<LayerId>, EditError> {
        let base = self.document.layers().len();
        let mut ids = Vec::with_capacity(layers.len());
        let mut edits = Vec::with_capacity(layers.len());
        for (offset, layer) in layers.iter().enumerate() {
            let id = self.document.allocate_layer_id();
            ids.push(id);
            edits.push(Edit::InsertLayer {
                index: base + offset,
                layer: Layer {
                    id,
                    ..layer.clone()
                },
            });
        }
        if !edits.is_empty() {
            self.perform(Edit::Batch(edits))?;
        }
        Ok(ids)
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
    use crate::color::LinearRgba;
    use crate::document::{Layer, LayerContent};
    use crate::geom::Size;

    fn add_layer(session: &mut Session, name: &str) -> LayerId {
        let id = session.allocate_layer_id();
        let index = session.document().layers().len();
        session
            .perform(Edit::InsertLayer {
                index,
                layer: Layer {
                    id,
                    name: name.into(),
                    visible: true,
                    opacity: 1.0,
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
                index: 1,
                layer: Layer {
                    id,
                    name: "photo".into(),
                    visible: false,
                    opacity: 0.5,
                    content: LayerContent::Raster {
                        image: image.clone(),
                    },
                },
            })
            .unwrap();

        let mut target = Session::new(Document::new(Size::new(16, 16)));
        let existing = add_layer(&mut target, "background");
        let ids = target
            .insert_layer_copies(source.document().layers())
            .unwrap();
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
        assert!(target.insert_layer_copies(&[]).unwrap().is_empty());
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
}
