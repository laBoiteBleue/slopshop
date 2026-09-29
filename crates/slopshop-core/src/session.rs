//! A document together with its undo/redo history.

use crate::document::{Document, LayerId};
use crate::edit::{Edit, EditError};

/// An editing session: the document and the inverse edits needed to undo/redo.
///
/// History is linear: performing a new edit discards the redo stack. Grouping edits (e.g. a
/// slider drag) and bounding history memory are not needed yet.
#[derive(Debug)]
pub struct Session {
    document: Document,
    undo: Vec<Edit>,
    redo: Vec<Edit>,
}

impl Session {
    pub fn new(document: Document) -> Self {
        Self {
            document,
            undo: Vec::new(),
            redo: Vec::new(),
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
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Apply an edit and record it. On error nothing changes, history included.
    pub fn perform(&mut self, edit: Edit) -> Result<(), EditError> {
        let inverse = edit.apply(&mut self.document)?;
        self.undo.push(inverse);
        self.redo.clear();
        Ok(())
    }

    /// Undo the last edit. Returns `Ok(false)` if there was nothing to undo.
    pub fn undo(&mut self) -> Result<bool, EditError> {
        Self::step(&mut self.document, &mut self.undo, &mut self.redo)
    }

    /// Redo the last undone edit. Returns `Ok(false)` if there was nothing to redo.
    pub fn redo(&mut self) -> Result<bool, EditError> {
        Self::step(&mut self.document, &mut self.redo, &mut self.undo)
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
