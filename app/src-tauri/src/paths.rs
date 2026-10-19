//! The Paths panel (ADR 0041): which path a command acts on, the edits of the document's paths
//! (Work Path, saved paths), Load Path as Selection and Make Work Path from Selection. Fill
//! Path and Stroke Path paint through `paint` (Edit > Fill's and the Brush's code).

use slopshop_core::paths::{DocumentPath, PathId};
use slopshop_core::selection::{self, Combine, EdgeOptions};
use slopshop_core::shape::Geometry;
use slopshop_core::{Document, Edit, HistoryLabel, LayerContent, LayerId, Projective, Session};

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::AppState;
use crate::ipc::DocumentView;
use crate::selection::{combine, on_worker, set_selection};
use crate::shape::GeometryDto;

/// A path the Paths panel lists: one of the document's, or the active layer's shape path or
/// vector mask path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum PathRef {
    Path { id: u64 },
    Shape { layer: u64 },
    VectorMask { layer: u64 },
}

impl PathRef {
    /// Its geometry and what places it in the document (its layer's transform and its groups').
    pub(crate) fn placed(self, doc: &Document) -> Result<(Geometry, Projective), String> {
        match self {
            PathRef::Path { id } => {
                let path = doc.path(PathId::from_raw(id)).ok_or("unknown path")?;
                Ok((path.geometry.clone(), Projective::IDENTITY))
            }
            PathRef::Shape { layer } | PathRef::VectorMask { layer } => {
                let id = LayerId::from_raw(layer);
                let found = doc.layer(id).ok_or("unknown layer")?;
                let geometry = match (self, &found.content) {
                    (PathRef::VectorMask { .. }, _) => found
                        .vector_mask
                        .as_ref()
                        .ok_or("the layer has no vector mask")?
                        .geometry()
                        .clone(),
                    (_, LayerContent::Vector { source, .. }) => source.shape().geometry.clone(),
                    _ => return Err("not a vector layer".to_owned()),
                };
                Ok((geometry, found.transform.then(doc.parent_transform(id))))
            }
        }
    }

    /// Its geometry as a path in document pixels (a live shape as its regular path); refused
    /// for a layer in perspective, under which a Bézier is no longer one (ADR 0038).
    pub(crate) fn document_path(self, doc: &Document) -> Result<Geometry, String> {
        let (geometry, placement) = self.placed(doc)?;
        let path = slopshop_core::shape::path::as_path(&geometry);
        slopshop_core::shape::mask::placed_path(&path, placement, Projective::IDENTITY)
            .ok_or_else(|| "the path cannot be placed in perspective".to_owned())
    }

    /// What it selects: the area it encloses, placed where it is.
    pub(crate) fn selection_shape(self, doc: &Document) -> Result<selection::Shape, String> {
        let (geometry, placement) = self.placed(doc)?;
        Ok(selection::path_shape(&geometry, placement))
    }

    /// Its subpaths as polylines in the document, each with whether it is closed: what Stroke
    /// Path paints along.
    pub(crate) fn polylines(
        self,
        doc: &Document,
    ) -> Result<Vec<slopshop_core::shape::path::Polyline>, String> {
        let (geometry, placement) = self.placed(doc)?;
        Ok(slopshop_core::shape::path::stroke_polylines(
            &geometry, placement,
        ))
    }
}

/// A document path as the panel shows it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PathView {
    pub id: u64,
    /// Empty for the Work Path (the UI names it).
    pub name: String,
    pub work: bool,
    /// In document pixels.
    pub geometry: GeometryDto,
}

impl PathView {
    pub(crate) fn of(path: &DocumentPath) -> Self {
        Self {
            id: path.id.get(),
            name: path.name.clone(),
            work: path.work,
            geometry: GeometryDto::of(&path.geometry),
        }
    }
}

/// `geometry` (the UI's) as a document path's: a valid path.
fn path_geometry(geometry: &GeometryDto) -> Result<Geometry, String> {
    let geometry = geometry.geometry()?;
    if !matches!(geometry, Geometry::Path { .. }) || !geometry.is_valid() {
        return Err("invalid path".to_owned());
    }
    Ok(geometry)
}

fn document_path(doc: &Document, id: u64) -> Result<DocumentPath, String> {
    doc.path(PathId::from_raw(id))
        .cloned()
        .ok_or_else(|| "unknown path".to_owned())
}

/// The Pen in Path mode with no path chosen in the panel: a new Work Path, in place of the one
/// there was (Photoshop's), in its place in the list (else last).
pub(crate) fn set_work_path(session: &mut Session, geometry: &GeometryDto) -> Result<Edit, String> {
    let geometry = path_geometry(geometry)?;
    let doc = session.document();
    let mut edits = Vec::new();
    let mut index = doc.paths().len();
    if let Some(old) = doc.work_path() {
        index = doc
            .paths()
            .iter()
            .position(|p| p.id == old.id)
            .unwrap_or(index);
        edits.push(Edit::RemovePath { id: old.id });
    }
    let id = session.allocate_path_id();
    edits.push(Edit::InsertPath {
        index,
        path: DocumentPath {
            id,
            name: String::new(),
            geometry,
            work: true,
        },
    });
    Ok(Edit::Batch(edits))
}

/// Path `id` drawn in or edited with the Pen: its new geometry.
pub(crate) fn set_path_geometry(
    doc: &Document,
    id: u64,
    geometry: &GeometryDto,
) -> Result<Edit, String> {
    Ok(Edit::SetPath {
        path: DocumentPath {
            geometry: path_geometry(geometry)?,
            ..document_path(doc, id)?
        },
    })
}

/// New Path: an empty path named `name`, last.
pub(crate) fn new_path(session: &mut Session, name: String) -> Edit {
    let index = session.document().paths().len();
    let id = session.allocate_path_id();
    Edit::InsertPath {
        index,
        path: DocumentPath::empty(id, name),
    }
}

/// Path `id` renamed.
pub(crate) fn rename_path(doc: &Document, id: u64, name: String) -> Result<Edit, String> {
    let path = document_path(doc, id)?;
    if path.work {
        return Err("the Work Path is saved, not renamed".to_owned());
    }
    Ok(Edit::SetPath {
        path: DocumentPath { name, ..path },
    })
}

/// Save Path: the Work Path kept under `name`.
pub(crate) fn save_path(doc: &Document, id: u64, name: String) -> Result<Edit, String> {
    let path = document_path(doc, id)?;
    if !path.work {
        return Err("only the Work Path is saved".to_owned());
    }
    Ok(Edit::SetPath {
        path: DocumentPath {
            name,
            work: false,
            ..path
        },
    })
}

/// Duplicate Path: a saved path named `name` with `from`'s subpaths, after it when it is one
/// of the document's, else last (a layer's shape path or vector mask, placed in the document).
pub(crate) fn duplicate_path(
    session: &mut Session,
    from: PathRef,
    name: String,
) -> Result<Edit, String> {
    let doc = session.document();
    let geometry = from.document_path(doc)?;
    let index = match from {
        PathRef::Path { id } => doc
            .paths()
            .iter()
            .position(|p| p.id.get() == id)
            .map_or(doc.paths().len(), |i| i + 1),
        _ => doc.paths().len(),
    };
    let id = session.allocate_path_id();
    Ok(Edit::InsertPath {
        index,
        path: DocumentPath {
            id,
            name,
            geometry,
            work: false,
        },
    })
}

/// Paths panel > Load Path as Selection (its button, Ctrl+click on a path): the area `path`
/// encloses, anti-aliased, replacing the selection or combined with it by `mode` (Shift adds,
/// Alt subtracts, both intersect, as Photoshop's); one undo entry, "Make Selection".
#[tauri::command]
pub async fn load_path_selection(
    state: State<'_, AppState>,
    document_id: u64,
    path: PathRef,
    mode: String,
) -> Result<DocumentView, String> {
    let how = combine(&mode)?;
    // A copy (its pixels shared): the documents are not held while the path is covered.
    let doc = state
        .documents()?
        .get_mut(document_id)?
        .session
        .document()
        .clone();
    let image = on_worker(move || path_selection(&doc, path, how)).await?;
    set_selection(
        &state,
        document_id,
        image,
        HistoryLabel::new("makeSelection"),
    )
}

/// Paths panel > Make Work Path from Selection: the selection's outline as the new Work Path
/// (in place of the one there was), fitted within `tolerance` pixels (0.5 to 10, Photoshop's);
/// one undo entry, "Make Work Path".
#[tauri::command]
pub async fn work_path_from_selection(
    state: State<'_, AppState>,
    document_id: u64,
    tolerance: f64,
) -> Result<DocumentView, String> {
    let selected = {
        let mut documents = state.documents()?;
        let doc = documents.get_mut(document_id)?.session.document();
        doc.selection()
            .map(|s| std::sync::Arc::clone(s.image()))
            .ok_or("nothing is selected")?
    };
    let geometry = on_worker(move || {
        slopshop_core::shape::trace::path_of_selection(&selected, tolerance)
            .ok_or_else(|| "nothing is selected".to_owned())
    })
    .await?;
    let mut documents = state.documents()?;
    let document = documents.get_mut(document_id)?;
    let edit = set_work_path(&mut document.session, &GeometryDto::of(&geometry))?;
    document
        .session
        .with_label(Some(HistoryLabel::new("makeWorkPath")), |s| s.perform(edit))
        .map_err(|e| e.to_string())?;
    Ok(document.view())
}

/// What `path` selects in `doc`, combined with its selection by `how`: see
/// [`load_path_selection`].
pub(crate) fn path_selection(
    doc: &Document,
    path: PathRef,
    how: Combine,
) -> Result<Option<slopshop_core::RasterImage>, String> {
    selection::select_shape(
        doc.size(),
        doc.selection().map(|s| s.image().as_ref()),
        &path.selection_shape(doc)?,
        EdgeOptions::default(),
        how,
    )
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use slopshop_core::{Rect, Size};

    fn triangle() -> GeometryDto {
        serde_json::from_str(
            r#"{"kind":"path","rule":"nonzero","subpaths":[
                {"start":[10,10],"segments":[[110,10],[10,60]],"closed":true}]}"#,
        )
        .unwrap()
    }

    fn perform(session: &mut Session, edit: Edit) {
        session.perform(edit).unwrap();
    }

    #[test]
    fn the_pen_replaces_the_work_path_and_save_keeps_it() {
        let mut s = Session::new(Document::new(Size::new(200, 100)));
        let edit = set_work_path(&mut s, &triangle()).unwrap();
        perform(&mut s, edit);
        let first = s.document().work_path().unwrap().id;
        // A saved path before it, then a new Work Path: in the old one's place.
        let saved = new_path(&mut s, "Path 1".into());
        perform(&mut s, saved);
        let edit = set_work_path(&mut s, &triangle()).unwrap();
        perform(&mut s, edit);
        let paths = s.document().paths();
        assert_eq!(paths.len(), 2);
        assert!(paths[0].work && paths[0].id != first);
        assert_eq!(paths[1].name, "Path 1");
        // One undo brings the first Work Path back.
        s.undo().unwrap();
        assert_eq!(s.document().work_path().unwrap().id, first);
        // Saved: named, no longer the Work Path; renamed; a live shape refused.
        let id = first.get();
        let edit = save_path(s.document(), id, "Outline".into()).unwrap();
        perform(&mut s, edit);
        assert_eq!(s.document().work_path(), None);
        assert!(save_path(s.document(), id, "Again".into()).is_err());
        let edit = rename_path(s.document(), id, "Edge".into()).unwrap();
        perform(&mut s, edit);
        assert_eq!(
            s.document().path(PathId::from_raw(id)).unwrap().name,
            "Edge"
        );
        let ellipse: GeometryDto =
            serde_json::from_str(r#"{"kind":"ellipse","center":[5,5],"radii":[2,2]}"#).unwrap();
        assert!(set_path_geometry(s.document(), id, &ellipse).is_err());
        assert!(set_work_path(&mut s, &ellipse).is_err());
    }

    #[test]
    fn duplicate_path_takes_a_layers_path_where_it_is() {
        let mut s = Session::new(Document::new(Size::new(200, 100)));
        let id = s.allocate_layer_id();
        let shape: crate::shape::ShapeDto = serde_json::from_value(serde_json::json!({
            "geometry": {"kind": "rectangle", "rect": [0, 0, 20, 10], "radii": [0, 0, 0, 0]},
            "fill": null, "stroke": null
        }))
        .unwrap();
        s.perform(Edit::InsertLayer {
            parent: None,
            index: 0,
            layer: slopshop_core::Layer {
                locks: Default::default(),
                style: None,
                transform: slopshop_core::Affine::translation(50.0, 5.0).into(),
                clipped: false,
                id,
                name: "Shape 1".into(),
                visible: true,
                opacity: 1.0,
                blend_mode: slopshop_core::BlendMode::Normal,
                mask: None,
                content: LayerContent::vector(slopshop_core::shape::ShapeSource::new(
                    shape.shape().unwrap(),
                    "Shape 1",
                )),
                warp: None,
                vector_mask: None,
            },
        })
        .unwrap();
        let edit =
            duplicate_path(&mut s, PathRef::Shape { layer: id.get() }, "Copy".into()).unwrap();
        perform(&mut s, edit);
        let copy = &s.document().paths()[0];
        assert!(!copy.work);
        // The rectangle as a path, moved where the layer shows it.
        let shape = selection::path_shape(&copy.geometry, Projective::IDENTITY);
        let image = selection::select_shape(
            Size::new(200, 100),
            None,
            &shape,
            EdgeOptions::default(),
            Combine::Replace,
        )
        .unwrap()
        .unwrap();
        assert_eq!(selection::bounds(&image), Some(Rect::new(50, 5, 20, 10)));
        // A copy of a document path goes after it.
        let original = copy.id.get();
        let edit = duplicate_path(&mut s, PathRef::Path { id: original }, "Copy 2".into()).unwrap();
        perform(&mut s, edit);
        assert_eq!(s.document().paths()[1].name, "Copy 2");
        assert!(
            duplicate_path(&mut s, PathRef::VectorMask { layer: id.get() }, "x".into()).is_err()
        );
    }

    #[test]
    fn a_path_selects_what_it_encloses_combined_by_mode() {
        let mut s = Session::new(Document::new(Size::new(200, 100)));
        let edit = set_work_path(&mut s, &triangle()).unwrap();
        perform(&mut s, edit);
        let path = PathRef::Path {
            id: s.document().work_path().unwrap().id.get(),
        };
        let image = path_selection(s.document(), path, Combine::Replace)
            .unwrap()
            .unwrap();
        assert_eq!(selection::bounds(&image), Some(Rect::new(10, 10, 100, 50)));
        // Intersected with nothing selected: nothing.
        assert!(
            path_selection(s.document(), path, Combine::Intersect)
                .unwrap()
                .is_none()
        );
        assert!(path_selection(s.document(), PathRef::Path { id: 99 }, Combine::Add).is_err());
    }
}
