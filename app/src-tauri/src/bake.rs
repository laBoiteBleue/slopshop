//! Layer > Bake to Pixels (ADR 0030): Rasterize, Merge Layers (Merge Down for one layer),
//! Merge Visible and Flatten Image. The engine plans what to composite
//! (`slopshop_core::bake`); here it is composited, on the GPU when there is one, on a worker,
//! then applied as one undo entry.

use std::sync::Arc;

use serde::Deserialize;
use slopshop_core::bake::{self, BakePlan};
use slopshop_core::copy::{MERGED_FORMAT, Rows};
use slopshop_core::{Document, Edit, LayerId, RasterImage, Rect};
use slopshop_render::Renderer;
use tauri::{AppHandle, Manager};

use crate::AppState;
use crate::clipboard::composite_bands;
use crate::ipc::DocumentView;
use crate::selection::on_worker;

#[derive(Debug, Clone, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum BakeRequest {
    /// The selected layers' content baked, each layer kept.
    Rasterize { ids: Vec<u64> },
    /// The selected layers merged into one; one layer: merged with the layer below it.
    Merge { ids: Vec<u64> },
    /// Every visible layer merged into one; hidden ones stay.
    MergeVisible,
    /// Every layer into one named `name`, hidden ones dropped.
    Flatten { name: String },
}

/// What `request` changes without compositing (stacks baked in place), and the composites it
/// needs. Blocking: evaluates stacks.
fn plan(document: &Document, request: BakeRequest) -> Result<(Vec<Edit>, Vec<BakePlan>), String> {
    let ids = |ids: Vec<u64>| -> Vec<LayerId> { ids.into_iter().map(LayerId::from_raw).collect() };
    let err = |e: slopshop_core::EditError| e.to_string();
    Ok(match request {
        BakeRequest::Rasterize { ids: raw } => {
            let ids = ids(raw);
            (
                bake::rasterize_in_place(document, &ids).map_err(err)?,
                bake::rasterize_plans(document, &ids).map_err(err)?,
            )
        }
        BakeRequest::Merge { ids: raw } => {
            let ids = document.outermost(&ids(raw));
            let plan = match ids.as_slice() {
                [one] => bake::merge_down_plan(document, *one),
                _ => bake::merge_plan(document, &ids),
            }
            .map_err(err)?;
            (Vec::new(), plan.into_iter().collect())
        }
        BakeRequest::MergeVisible => (
            Vec::new(),
            bake::merge_visible_plan(document)
                .map_err(err)?
                .into_iter()
                .collect(),
        ),
        BakeRequest::Flatten { name } => (
            Vec::new(),
            bake::flatten_plan(document, name)
                .map_err(err)?
                .into_iter()
                .collect(),
        ),
    })
}

/// A plan's scratch document composited whole, as an image in [`MERGED_FORMAT`]. Blocking.
fn composited(renderer: Option<&Renderer>, plan: &BakePlan) -> Result<Arc<RasterImage>, String> {
    let scratch = &plan.scratch;
    let size = scratch.size();
    let rows = Rows::merged(size.width, scratch.blend_space()).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    composite_bands(
        renderer,
        scratch,
        Rect::new(0, 0, size.width, size.height),
        |_, pixels, first_row| {
            rows.convert(pixels, first_row, &mut bytes)
                .map_err(|e| e.to_string())
        },
    )?;
    RasterImage::from_pixels(size, MERGED_FORMAT, &bytes)
        .map(Arc::new)
        .map_err(|e| e.to_string())
}

/// Layer > Bake to Pixels: `request` on document `document_id`, one undo entry. The
/// compositing runs on a worker; nothing changes when there is nothing to bake.
#[tauri::command]
pub async fn bake_layers(
    app: AppHandle,
    document_id: u64,
    request: BakeRequest,
) -> Result<DocumentView, String> {
    let document = {
        let state = app.state::<AppState>();
        let mut documents = state.documents()?;
        documents.get_mut(document_id)?.session.document().clone()
    };
    let worker = app.clone();
    let (mut edits, composites) = on_worker(move || {
        let state = worker.state::<AppState>();
        // Without a GPU, the CPU compositor gives the same pixels, only slower.
        let renderer = state.renderer().ok();
        let (edits, plans) = plan(&document, request)?;
        let composites = plans
            .into_iter()
            .map(|plan| composited(renderer, &plan).map(|image| (plan, image)))
            .collect::<Result<Vec<_>, String>>()?;
        Ok((edits, composites))
    })
    .await?;
    let state = app.state::<AppState>();
    let mut documents = state.documents()?;
    let open = documents.get_mut(document_id)?;
    for (plan, image) in composites {
        let id = open.session.allocate_layer_id();
        edits.push(
            plan.finish(open.session.document(), image, id)
                .map_err(|e| e.to_string())?,
        );
    }
    open.session
        .perform(Edit::Batch(edits))
        .map_err(|e| e.to_string())?;
    Ok(open.view())
}

#[cfg(test)]
mod tests {
    use super::*;
    use slopshop_core::{Session, Size};

    /// A document of two fill layers and a hidden one.
    fn session() -> (Session, Vec<u64>) {
        let mut s = Session::new(Document::new(Size::new(8, 8)));
        for (name, visible) in [("a", true), ("hidden", false), ("b", true)] {
            let json = format!(r#"{{"kind":"addFillLayer","name":"{name}","color":[1,0,0,1]}}"#);
            let request: crate::ipc::EditRequest = serde_json::from_str(&json).unwrap();
            let edit = request.into_edit(&mut s).unwrap();
            s.perform(edit).unwrap();
            if !visible {
                let id = s.document().layers().last().unwrap().id;
                s.perform(Edit::SetLayerVisible { id, visible }).unwrap();
            }
        }
        let ids = s.document().layers().iter().map(|l| l.id.get()).collect();
        (s, ids)
    }

    fn apply(s: &mut Session, json: &str) {
        let request: BakeRequest = serde_json::from_str(json).unwrap();
        let (mut edits, plans) = plan(s.document(), request).unwrap();
        for plan in plans {
            let image = composited(None, &plan).unwrap();
            let id = s.allocate_layer_id();
            edits.push(plan.finish(s.document(), image, id).unwrap());
        }
        s.perform(Edit::Batch(edits)).unwrap();
    }

    fn names(s: &Session) -> Vec<&str> {
        s.document()
            .layers()
            .iter()
            .map(|l| l.name.as_str())
            .collect()
    }

    #[test]
    fn requests_bake_as_asked_in_one_undo_entry() {
        let (mut s, ids) = session();
        // One layer: merged down onto the layer below it (hidden: nothing to do).
        apply(&mut s, &format!(r#"{{"kind":"merge","ids":[{}]}}"#, ids[2]));
        assert_eq!(names(&s), ["a", "hidden", "b"]);
        apply(
            &mut s,
            &format!(r#"{{"kind":"merge","ids":[{},{}]}}"#, ids[0], ids[2]),
        );
        assert_eq!(names(&s), ["hidden", "b"]);
        assert!(s.undo().unwrap());
        assert_eq!(names(&s), ["a", "hidden", "b"]);

        apply(&mut s, r#"{"kind":"mergeVisible"}"#);
        assert_eq!(names(&s), ["hidden", "b"]);
        assert!(s.undo().unwrap());
        apply(&mut s, r#"{"kind":"flatten","name":"Flat"}"#);
        assert_eq!(names(&s), ["Flat"]);
        assert!(s.undo().unwrap());

        // A fill becomes pixels, in its place.
        apply(
            &mut s,
            &format!(r#"{{"kind":"rasterize","ids":[{}]}}"#, ids[0]),
        );
        let layer = &s.document().layers()[0];
        assert_eq!((layer.name.as_str(), layer.id.get()), ("a", ids[0]));
        assert!(matches!(
            layer.content,
            slopshop_core::LayerContent::Raster { .. }
        ));
    }
}
