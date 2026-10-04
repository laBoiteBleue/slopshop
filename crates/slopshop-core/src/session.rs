//! A document together with its undo/redo history.

use std::sync::Arc;

use crate::blend::BlendMode;
use crate::document::{Document, Layer, LayerContent, LayerId, LayerMask};
use crate::edit::{Edit, EditError};
use crate::selection::Selection;

/// What [`Session::insert_layer_copies`] made.
#[derive(Debug, Clone, PartialEq)]
pub struct Copies {
    /// The group holding the copies, when one was asked for.
    pub group: Option<LayerId>,
    /// The id of each copy, in the order the copied layers and their subtrees are walked depth
    /// first (as [`Document::all_layers`] does).
    pub ids: Vec<LayerId>,
}

/// What a history entry did, for the History panel: an identifier the UI translates (`kind`,
/// camelCase), and what was applied when that is part of it (an adjustment's or a filter's id).
/// Display text is the UI's business.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryLabel {
    pub kind: &'static str,
    pub detail: Option<&'static str>,
}

impl HistoryLabel {
    pub const fn new(kind: &'static str) -> Self {
        Self { kind, detail: None }
    }

    pub const fn with(kind: &'static str, detail: &'static str) -> Self {
        Self {
            kind,
            detail: Some(detail),
        }
    }

    /// What `edit` does, said generically: callers that know better (a tool, a menu command)
    /// name their entries with [`Session::with_label`].
    pub fn of(edit: &Edit) -> Self {
        let kind = match edit {
            Edit::InsertLayer { .. } => "newLayer",
            Edit::RemoveLayer { .. } => "deleteLayer",
            Edit::SetLayerVisible { .. } => "visibility",
            Edit::SetLayerOpacity { .. } => "opacity",
            Edit::RenameLayer { .. } => "rename",
            Edit::SetLayerBlendMode { .. } => "blendMode",
            Edit::SetBlendSpace { .. } => "blendSpace",
            Edit::SetResolution { .. } => "resolution",
            Edit::SetCanvasSize { .. } => "canvasSize",
            Edit::SetLayerMask { .. } => "mask",
            Edit::SetLayerMaskEnabled { .. } => "maskEnabled",
            Edit::MoveLayer { .. } => "arrange",
            Edit::SetLayerTransform { .. } => "transform",
            Edit::SetLayerStack { .. } => "pixels",
            Edit::SetMaskPaint { .. } => "maskPixels",
            Edit::SetFillColor { .. } => "fillColor",
            Edit::SetAdjustment { adjustment, .. } => {
                return Self::with("adjustmentSettings", adjustment.id());
            }
            Edit::SetFilter { filter, .. } => return Self::with("filterSettings", filter.id()),
            Edit::SetLayerStyle { .. } => "layerStyle",
            Edit::SetLayerClipped { .. } => "clipping",
            Edit::SetGroupPassThrough { .. } => "passThrough",
            Edit::SetSelection { .. } => "selection",
            Edit::SetQuickMask { .. } => "quickMask",
            Edit::SetGuides { .. } => "guides",
            Edit::InsertSavedSelection { .. } => "saveSelection",
            Edit::RemoveSavedSelection { .. } => "deleteSavedSelection",
            Edit::RenameSavedSelection { .. } => "renameSavedSelection",
            Edit::SetSavedSelection { .. } => "replaceSavedSelection",
            Edit::Batch(edits) => return edits.first().map_or(Self::new("edit"), Self::of),
        };
        Self::new(kind)
    }
}

/// One step of the history: the edit that goes back (or forward) over it, and what it did.
#[derive(Debug)]
struct Entry {
    edit: Edit,
    label: HistoryLabel,
}

/// An editing session: the document and the inverse edits needed to undo/redo.
///
/// History is linear: performing a new edit discards the redo stack. Continuous interactions
/// (e.g. a slider drag) are *gestures*: applied live, recorded as one entry. Each entry has a
/// [`HistoryLabel`]. Bounding history memory is not needed yet.
#[derive(Debug)]
pub struct Session {
    document: Document,
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    /// Inverses of the edits applied by the gesture in progress, in application order.
    gesture: Vec<Edit>,
    /// What the gesture in progress does (set by its first edit).
    gesture_label: HistoryLabel,
    /// The label entries recorded now get (see [`Self::with_label`]); `None`: their edit's.
    label: Option<HistoryLabel>,
    /// The selection before the gesture in progress, if it has begun.
    gesture_selection: Option<Option<Selection>>,
    /// The selection a committed change last removed or replaced: Select > Reselect brings it
    /// back (the maintainer's choice, 2026-10-04: replaced too, not only deselected). Undo and
    /// redo do not count, nor a gesture's steps.
    last_selection: Option<Selection>,
}

impl Session {
    pub fn new(document: Document) -> Self {
        Self {
            document,
            undo: Vec::new(),
            redo: Vec::new(),
            gesture: Vec::new(),
            gesture_label: HistoryLabel::new("edit"),
            label: None,
            gesture_selection: None,
            last_selection: None,
        }
    }

    pub fn document(&self) -> &Document {
        &self.document
    }

    /// See [`Document::allocate_layer_id`].
    pub fn allocate_layer_id(&mut self) -> LayerId {
        self.document.allocate_layer_id()
    }

    /// See [`Document::allocate_saved_selection_id`].
    pub fn allocate_saved_selection_id(&mut self) -> crate::document::SavedSelectionId {
        self.document.allocate_saved_selection_id()
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty() || !self.gesture.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Run `f`, the entries it records named `label` (when given; else after their edit). A
    /// gesture started within keeps the label until it ends.
    pub fn with_label<R>(
        &mut self,
        label: Option<HistoryLabel>,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let inner = label.or(self.label);
        let outer = std::mem::replace(&mut self.label, inner);
        let result = f(self);
        self.label = outer;
        result
    }

    /// The history, oldest first: the entries done (the gesture in progress last), then those
    /// undone that redo would bring back, nearest first; and how many are done.
    pub fn history(&self) -> (Vec<HistoryLabel>, usize) {
        let mut labels: Vec<HistoryLabel> = self.undo.iter().map(|e| e.label).collect();
        if !self.gesture.is_empty() {
            labels.push(self.gesture_label);
        }
        let done = labels.len();
        labels.extend(self.redo.iter().rev().map(|e| e.label));
        (labels, done)
    }

    /// Undo or redo until `done` entries of [`Self::history`] are done (0: the document as it
    /// was opened), ending any gesture first. Stops early at the end of the history. Returns
    /// whether anything changed.
    pub fn go_to(&mut self, done: usize) -> Result<bool, EditError> {
        self.end_gesture();
        let mut changed = false;
        while self.undo.len() > done && self.undo()? {
            changed = true;
        }
        while self.undo.len() < done && self.redo()? {
            changed = true;
        }
        Ok(changed)
    }

    /// Apply an edit and record it. Ends any gesture in progress first. On error nothing
    /// changes, history included.
    pub fn perform(&mut self, edit: Edit) -> Result<(), EditError> {
        // An empty batch changes nothing: no history entry, no revision.
        if matches!(&edit, Edit::Batch(edits) if edits.is_empty()) {
            return Ok(());
        }
        self.end_gesture();
        let before = self.document.selection().cloned();
        let label = self.label.unwrap_or_else(|| HistoryLabel::of(&edit));
        let inverse = edit.apply(&mut self.document)?;
        self.push_undo(inverse, label);
        self.remember_selection(before);
        Ok(())
    }

    /// `before` replaced or removed by a committed change: what Reselect brings back.
    fn remember_selection(&mut self, before: Option<Selection>) {
        if let Some(before) = before
            && self.document.selection() != Some(&before)
        {
            self.last_selection = Some(before);
        }
    }

    /// Select > Reselect: the selection a committed change last removed or replaced, while it
    /// is not the selection and still fits the canvas.
    pub fn reselectable(&self) -> Option<&Selection> {
        let doc = &self.document;
        self.last_selection
            .as_ref()
            .filter(|s| doc.selection() != Some(*s) && s.image().size() == doc.size())
    }

    /// Apply `edit` as the end of the last history entry, made when the document was at
    /// `revision`: one undo entry covers both when nothing happened since (Layer > Bake to
    /// Pixels: the pixels replacing their preview, ADR 0031); otherwise a new entry, as
    /// [`Self::perform`].
    pub fn perform_after(&mut self, edit: Edit, revision: u64) -> Result<(), EditError> {
        if matches!(&edit, Edit::Batch(edits) if edits.is_empty()) {
            return Ok(());
        }
        let continues = self.gesture.is_empty() && self.document.revision() == revision;
        match self.undo.pop() {
            Some(last) if continues => match edit.apply(&mut self.document) {
                Ok(inverse) => {
                    self.undo.push(Entry {
                        edit: Edit::Batch(vec![inverse, last.edit]),
                        label: self.label.unwrap_or(last.label),
                    });
                    Ok(())
                }
                Err(e) => {
                    self.undo.push(last);
                    Err(e)
                }
            },
            last => {
                self.undo.extend(last);
                self.perform(edit)
            }
        }
    }

    /// Apply an edit as part of a continuous gesture (e.g. dragging a slider): the document
    /// changes immediately, and all edits of the gesture become a single history entry when
    /// [`Self::end_gesture`] is called.
    pub fn perform_in_gesture(&mut self, edit: Edit) -> Result<(), EditError> {
        let before = self.document.selection().cloned();
        let label = self.label.unwrap_or_else(|| HistoryLabel::of(&edit));
        let inverse = edit.apply(&mut self.document)?;
        if self.gesture.is_empty() {
            self.gesture_selection = Some(before);
            self.gesture_label = label;
        }
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
        self.insert_copies(layers, group.map(|name| (name, None)))
    }

    /// Edit > Paste Into: [`Self::insert_layer_copies`] inside a new group named `name` whose
    /// mask is the selection, the copies free to move under it; the selection is deselected (it
    /// became the mask, as in Photoshop), in the same undo entry. `None` without a selection.
    pub fn insert_into_selection(
        &mut self,
        layers: &[Layer],
        name: String,
    ) -> Result<Option<Copies>, EditError> {
        let Some(selection) = self.document.selection() else {
            return Ok(None);
        };
        let mask = LayerMask {
            image: Arc::clone(selection.image()),
            enabled: true,
            replaces_alpha: false,
            original: None,
        };
        self.insert_copies(layers, Some((name, Some(mask))))
            .map(Some)
    }

    fn insert_copies(
        &mut self,
        layers: &[Layer],
        group: Option<(String, Option<LayerMask>)>,
    ) -> Result<Copies, EditError> {
        let mut ids = Vec::new();
        let copies: Vec<Layer> = layers
            .iter()
            .map(|layer| self.fresh_copy(layer, &mut ids))
            .collect();
        let base = self.document.layers().len();
        let (group, edit) = match group {
            Some((name, mask)) => {
                let id = self.document.allocate_layer_id();
                let edit = Edit::InsertLayer {
                    parent: None,
                    index: base,
                    layer: Layer {
                        style: None,
                        transform: crate::transform::Affine::IDENTITY,
                        clipped: false,
                        id,
                        name,
                        visible: true,
                        opacity: 1.0,
                        blend_mode: BlendMode::Normal,
                        mask,
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
        // A mask made from the selection deselects (Paste Into).
        let deselect = matches!(&edit, Edit::InsertLayer { layer, .. } if layer.mask.is_some());
        let edit = if deselect {
            Edit::Batch(vec![edit, Edit::SetSelection { selection: None }])
        } else {
            edit
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
        let before = self.gesture_selection.take().flatten();
        let entry = match inverses.len() {
            0 => return,
            1 => inverses.remove(0),
            _ => {
                inverses.reverse();
                Edit::Batch(inverses)
            }
        };
        self.push_undo(entry, self.gesture_label);
        self.remember_selection(before);
    }

    /// Revert the gesture in progress, leaving no history entry (a transform cancelled with
    /// Esc). Returns `Ok(false)` if there was none.
    pub fn cancel_gesture(&mut self) -> Result<bool, EditError> {
        let inverses = std::mem::take(&mut self.gesture);
        self.gesture_selection = None;
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

    fn push_undo(&mut self, inverse: Edit, label: HistoryLabel) {
        self.undo.push(Entry {
            edit: inverse,
            label,
        });
        self.redo.clear();
    }

    fn step(
        doc: &mut Document,
        from: &mut Vec<Entry>,
        to: &mut Vec<Entry>,
    ) -> Result<bool, EditError> {
        let Some(entry) = from.pop() else {
            return Ok(false);
        };
        // Inverses are exact, so this can only fail on a bug. Keep the entry in that case so
        // history is not silently lost.
        match entry.edit.clone().apply(doc) {
            Ok(inverse) => {
                to.push(Entry {
                    edit: inverse,
                    label: entry.label,
                });
                Ok(true)
            }
            Err(err) => {
                from.push(entry);
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

    /// A selection of `rect` (left, top, right, bottom) on a 100 × 80 canvas.
    fn rect_selection(rect: [f64; 4]) -> Selection {
        use crate::selection::{Combine, EdgeOptions, Shape, select_shape};
        let [left, top, right, bottom] = rect;
        let shape = Shape::Rectangle {
            left,
            top,
            right,
            bottom,
        };
        let size = crate::geom::Size::new(100, 80);
        let image = select_shape(size, None, &shape, EdgeOptions::default(), Combine::Replace)
            .unwrap()
            .unwrap();
        Selection::new(Arc::new(image)).unwrap()
    }

    #[test]
    fn reselect_brings_back_the_selection_last_removed_or_replaced() {
        let mut s = Session::new(Document::new(crate::geom::Size::new(100, 80)));
        let (a, b) = (
            rect_selection([0.0, 0.0, 10.0, 10.0]),
            rect_selection([20.0, 20.0, 40.0, 40.0]),
        );
        let select = |s: &mut Session, sel: Option<&Selection>| {
            s.perform(Edit::SetSelection {
                selection: sel.cloned(),
            })
            .unwrap();
        };
        assert!(s.reselectable().is_none());
        select(&mut s, Some(&a));
        assert!(s.reselectable().is_none());
        // Replaced: A comes back; twice, they swap.
        select(&mut s, Some(&b));
        assert_eq!(s.reselectable(), Some(&a));
        select(&mut s, Some(&a));
        assert_eq!(s.reselectable(), Some(&b));
        // Deselected: the one removed.
        select(&mut s, None);
        assert_eq!(s.reselectable(), Some(&a));
        // Undo does not count; nor what is the selection already.
        s.undo().unwrap();
        assert_eq!(s.document().selection(), Some(&a));
        assert!(s.reselectable().is_none());
        // A gesture counts once, from the selection before it; a cancelled one not at all.
        s.perform_in_gesture(Edit::SetSelection {
            selection: Some(b.clone()),
        })
        .unwrap();
        s.perform_in_gesture(Edit::SetSelection { selection: None })
            .unwrap();
        s.cancel_gesture().unwrap();
        assert!(s.reselectable().is_none());
        s.perform_in_gesture(Edit::SetSelection {
            selection: Some(b.clone()),
        })
        .unwrap();
        s.perform_in_gesture(Edit::SetSelection {
            selection: Some(b.clone()),
        })
        .unwrap();
        s.end_gesture();
        assert_eq!(s.reselectable(), Some(&a));
    }
    use crate::geom::Size;

    fn add_layer(session: &mut Session, name: &str) -> LayerId {
        let id = session.allocate_layer_id();
        let index = session.document().layers().len();
        session
            .perform(Edit::InsertLayer {
                parent: None,
                index,
                layer: Layer {
                    style: None,
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

    /// The kinds of `session`'s history, and how many are done.
    fn kinds(session: &Session) -> (Vec<&'static str>, usize) {
        let (labels, done) = session.history();
        (labels.iter().map(|l| l.kind).collect(), done)
    }

    #[test]
    fn entries_are_named_after_their_edit_unless_their_caller_names_them() {
        let mut s = Session::new(Document::new(Size::new(16, 16)));
        let a = add_layer(&mut s, "a");
        s.with_label(Some(HistoryLabel::with("filter", "gaussianBlur")), |s| {
            s.perform(Edit::SetLayerVisible {
                id: a,
                visible: false,
            })
        })
        .unwrap();
        // Unnamed within: the edit's own label; a batch is named after its first edit.
        s.with_label(None, |s| {
            s.perform(Edit::Batch(vec![Edit::SetLayerOpacity {
                id: a,
                opacity: 0.5,
            }]))
        })
        .unwrap();
        let (labels, done) = s.history();
        assert_eq!(done, 3);
        assert_eq!(labels[0], HistoryLabel::new("newLayer"));
        assert_eq!(labels[1], HistoryLabel::with("filter", "gaussianBlur"));
        assert_eq!(labels[2], HistoryLabel::new("opacity"));
    }

    #[test]
    fn a_gesture_keeps_the_label_it_started_with() {
        let mut s = Session::new(Document::new(Size::new(16, 16)));
        let a = add_layer(&mut s, "a");
        s.with_label(Some(HistoryLabel::new("brush")), |s| {
            s.perform_in_gesture(Edit::SetLayerOpacity {
                id: a,
                opacity: 0.5,
            })
        })
        .unwrap();
        // In progress: listed last.
        assert_eq!(kinds(&s), (vec!["newLayer", "brush"], 2));
        s.perform_in_gesture(Edit::SetLayerOpacity {
            id: a,
            opacity: 0.25,
        })
        .unwrap();
        s.end_gesture();
        assert_eq!(kinds(&s), (vec!["newLayer", "brush"], 2));
    }

    #[test]
    fn going_to_an_entry_undoes_or_redoes_up_to_it_and_keeps_the_labels() {
        let mut s = Session::new(Document::new(Size::new(16, 16)));
        let a = add_layer(&mut s, "a");
        add_layer(&mut s, "b");
        s.perform(Edit::SetLayerVisible {
            id: a,
            visible: false,
        })
        .unwrap();
        assert!(s.go_to(1).unwrap());
        assert_eq!(names(&s), ["a"]);
        assert!(s.document().layer(a).unwrap().visible);
        // Undone entries stay listed after the current one, until a new edit.
        assert_eq!(kinds(&s), (vec!["newLayer", "newLayer", "visibility"], 1));
        assert!(s.go_to(3).unwrap());
        assert!(!s.document().layer(a).unwrap().visible);
        assert!(!s.go_to(3).unwrap());
        // Past the end: as far as it goes.
        assert!(s.go_to(0).unwrap());
        assert!(s.document().layers().is_empty());
        assert!(s.go_to(99).unwrap());
        assert_eq!(kinds(&s).1, 3);
        s.go_to(1).unwrap();
        s.perform(Edit::RenameLayer {
            id: a,
            name: "c".into(),
        })
        .unwrap();
        assert_eq!(kinds(&s), (vec!["newLayer", "rename"], 2));
    }

    #[test]
    fn an_edit_after_continues_the_last_entry_while_nothing_happened_since() {
        let mut s = Session::new(Document::new(Size::new(16, 16)));
        let a = add_layer(&mut s, "a");
        let revision = s.document().revision();
        s.perform_after(
            Edit::RenameLayer {
                id: a,
                name: "b".into(),
            },
            revision,
        )
        .unwrap();
        assert_eq!(names(&s), ["b"]);
        // One undo takes both back.
        assert!(s.undo().unwrap());
        assert_eq!(names(&s), Vec::<String>::new());
        assert!(s.redo().unwrap());
        assert_eq!(names(&s), ["b"]);

        // Something happened since: an entry of its own.
        let revision = s.document().revision();
        add_layer(&mut s, "c");
        s.perform_after(
            Edit::RenameLayer {
                id: a,
                name: "d".into(),
            },
            revision,
        )
        .unwrap();
        assert!(s.undo().unwrap());
        assert_eq!(names(&s), ["b", "c"]);
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
                    style: None,
                    transform: crate::transform::Affine::IDENTITY,
                    clipped: false,
                    id,
                    name: "photo".into(),
                    visible: false,
                    opacity: 0.5,
                    blend_mode: BlendMode::Normal,
                    mask: None,
                    content: LayerContent::Raster {
                        stack: None,
                        image: crate::stack::Pixels::ready(image.clone()),
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
        let LayerContent::Raster { image: copied, .. } = &layers[2].content else {
            panic!("raster expected");
        };
        assert!(Arc::ptr_eq(&copied.get(), &image));
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

        // Into the selection (Paste Into): a group masked by it, deselected, one undo entry.
        assert!(
            target
                .insert_into_selection(source.document().layers(), "pasted".into())
                .unwrap()
                .is_none()
        );
        let coverage = Arc::new(
            crate::RasterImage::from_pixels(
                Size::new(16, 16),
                crate::selection::SELECTION_FORMAT,
                &[0xffu8; 16 * 16 * 2],
            )
            .unwrap(),
        );
        let selection = crate::selection::Selection::new(Arc::clone(&coverage));
        target.perform(Edit::SetSelection { selection }).unwrap();
        let copies = target
            .insert_into_selection(source.document().layers(), "pasted".into())
            .unwrap()
            .unwrap();
        assert!(target.document().selection().is_none());
        let group = target.document().layer(copies.group.unwrap()).unwrap();
        assert!(Arc::ptr_eq(&group.mask.as_ref().unwrap().image, &coverage));
        assert_eq!(group.children().unwrap().len(), 2);
        target.undo().unwrap();
        assert_eq!(names(&target), ["background"]);
        assert!(target.document().selection().is_some());
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
                    style: None,
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
