//! Layer > Bake to Pixels (ADR 0031): Rasterize, Merge Layers (Merge Down for one layer),
//! Merge Visible, Flatten Image, and New Layer from Visible. What can show at once does: a merge
//! moves its layers into a group in the place of the result (the same pixels on screen), New
//! Layer from Visible puts copies of the visible layers there. The pixels are then composited
//! on a worker (on the GPU when there is one) and replace that group, in the same undo entry
//! when nothing happened meanwhile; the document is then sent again (`document-updated`).

use std::sync::Arc;

use serde::Deserialize;
use slopshop_core::bake::{self, BakePlan, Merge};
use slopshop_core::copy::{MERGED_FORMAT, Rows};
use slopshop_core::{BlendMode, Edit, Layer, LayerContent, LayerId, RasterImage, Rect, Session};
use slopshop_render::Renderer;
use tauri::{AppHandle, Manager};

use crate::clipboard::composite_bands;
use crate::ipc::DocumentView;
use crate::selection::on_worker;
use crate::{AppState, emit};

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
    /// New Layer from Visible: the visible layers composited into a new layer `name` on top,
    /// the layers kept.
    Visible { name: String },
}

/// What a request leaves to do once its instant part is applied: the composites, and the
/// revision right after that part when the pixels should join its undo entry.
struct Pending {
    plans: Vec<BakePlan>,
    joins: Option<u64>,
}

/// The instant part of `request` on `session` (one undo entry), and what is left to composite.
/// `baked`: what Rasterize does to the pixel layers whose stack it bakes, evaluated beforehand.
fn start(session: &mut Session, request: BakeRequest, baked: Vec<Edit>) -> Result<Pending, String> {
    let err = |e: slopshop_core::EditError| e.to_string();
    let ids = |ids: Vec<u64>| -> Vec<LayerId> { ids.into_iter().map(LayerId::from_raw).collect() };
    let merge = match request {
        BakeRequest::Rasterize { ids: raw } => {
            // The stacks baked first; the fills and groups join that entry.
            let joins = if baked.is_empty() {
                None
            } else {
                session.perform(Edit::Batch(baked)).map_err(err)?;
                Some(session.document().revision())
            };
            let plans = bake::rasterize_plans(session.document(), &ids(raw)).map_err(err)?;
            return Ok(Pending { plans, joins });
        }
        BakeRequest::Merge { ids: raw } => {
            let ids = session.document().outermost(&ids(raw));
            match ids.as_slice() {
                [one] => Merge::Down(*one),
                _ => Merge::Layers(ids),
            }
        }
        BakeRequest::MergeVisible => Merge::Visible,
        BakeRequest::Flatten { name } => Merge::Flatten(name),
        BakeRequest::Visible { name } => {
            // Copies of the visible layers (their pixels shared) in a group on top.
            let children: Vec<Layer> = session
                .document()
                .layers()
                .iter()
                .filter(|l| l.visible)
                .cloned()
                .collect();
            let group = Layer {
                id: LayerId::from_raw(1),
                name,
                visible: true,
                opacity: 1.0,
                blend_mode: BlendMode::Normal,
                mask: None,
                clipped: false,
                transform: slopshop_core::Affine::IDENTITY,
                content: LayerContent::Group {
                    children,
                    pass_through: false,
                },
            };
            let copies = session.insert_layer_copies(&[group], None).map_err(err)?;
            let group = *copies.ids.first().ok_or("nothing copied")?;
            let joins = Some(session.document().revision());
            let plans = bake::rasterize_plans(session.document(), &[group]).map_err(err)?;
            return Ok(Pending { plans, joins });
        }
    };
    let group = session.allocate_layer_id();
    let Some(preview) = bake::merge_preview(session.document(), merge, group).map_err(err)? else {
        return Ok(Pending {
            plans: Vec::new(),
            joins: None,
        });
    };
    session.perform(preview).map_err(err)?;
    let joins = Some(session.document().revision());
    let plans = bake::rasterize_plans(session.document(), &[group]).map_err(err)?;
    Ok(Pending { plans, joins })
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

/// The composited pixels put in place of what they bake, as one edit: joined to the instant
/// part's undo entry when nothing happened since, a new entry otherwise; nothing for layers
/// undone or changed meanwhile.
fn land(
    session: &mut Session,
    composites: Vec<(BakePlan, Arc<RasterImage>)>,
    joins: Option<u64>,
) -> Result<(), String> {
    let mut edits = Vec::new();
    for (plan, image) in composites {
        if let Some(edit) = plan
            .finish(session.document(), image)
            .map_err(|e| e.to_string())?
        {
            edits.push(edit);
        }
    }
    let edit = Edit::Batch(edits);
    match joins {
        Some(revision) => session.perform_after(edit, revision),
        None => session.perform(edit),
    }
    .map_err(|e| e.to_string())
}

/// Layer > Bake to Pixels and New Layer from Visible: `request` on document `document_id`. What
/// shows at once is applied before this resolves (the document as it is then); the pixels
/// follow, sent with `document-updated`.
#[tauri::command]
pub async fn bake_layers(
    app: AppHandle,
    document_id: u64,
    request: BakeRequest,
) -> Result<DocumentView, String> {
    // Rasterize evaluates the stacks it bakes: on a worker, from a copy.
    let baked = match &request {
        BakeRequest::Rasterize { ids } => {
            let document = {
                let state = app.state::<AppState>();
                let mut documents = state.documents()?;
                documents.get_mut(document_id)?.session.document().clone()
            };
            let ids: Vec<LayerId> = ids.iter().copied().map(LayerId::from_raw).collect();
            on_worker(move || bake::rasterize_in_place(&document, &ids).map_err(|e| e.to_string()))
                .await?
        }
        _ => Vec::new(),
    };
    let (pending, view) = {
        let state = app.state::<AppState>();
        let mut documents = state.documents()?;
        let open = documents.get_mut(document_id)?;
        let pending = start(&mut open.session, request, baked)?;
        open.baking
            .extend(pending.plans.iter().map(BakePlan::layer));
        (pending, open.view())
    };
    if !pending.plans.is_empty() {
        let worker = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = finish(worker, document_id, pending).await {
                eprintln!("bake to pixels: {e}");
            }
        });
    }
    Ok(view)
}

/// Composite `pending` on a worker, then put it in place and send the document again.
async fn finish(app: AppHandle, document_id: u64, pending: Pending) -> Result<(), String> {
    let worker = app.clone();
    let Pending { plans, joins } = pending;
    let layers: Vec<LayerId> = plans.iter().map(BakePlan::layer).collect();
    let composites = on_worker(move || {
        let state = worker.state::<AppState>();
        // Without a GPU, the CPU compositor gives the same pixels, only slower.
        let renderer = state.renderer().ok();
        plans
            .into_iter()
            .map(|plan| composited(renderer, &plan).map(|image| (plan, image)))
            .collect::<Result<Vec<_>, String>>()
    })
    .await;
    let state = app.state::<AppState>();
    let mut documents = state.documents()?;
    // Closed meanwhile: nothing to do.
    let Ok(open) = documents.get_mut(document_id) else {
        return Ok(());
    };
    for id in &layers {
        open.baking.remove(id);
    }
    let landed = composites.and_then(|composites| land(&mut open.session, composites, joins));
    emit(&app, "document-updated", &open.view());
    landed
}

#[cfg(test)]
mod tests {
    use super::*;
    use slopshop_core::{Document, Size};

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

    /// `pending`'s composites made on the CPU.
    fn composites(pending: Pending) -> (Vec<(BakePlan, Arc<RasterImage>)>, Option<u64>) {
        let composites = pending
            .plans
            .into_iter()
            .map(|plan| {
                let image = composited(None, &plan).unwrap();
                (plan, image)
            })
            .collect();
        (composites, pending.joins)
    }

    /// `json` started, then its pixels landed, as the command does.
    fn apply(s: &mut Session, json: &str) {
        let request: BakeRequest = serde_json::from_str(json).unwrap();
        let baked = match &request {
            BakeRequest::Rasterize { ids } => {
                let ids: Vec<LayerId> = ids.iter().copied().map(LayerId::from_raw).collect();
                bake::rasterize_in_place(s.document(), &ids).unwrap()
            }
            _ => Vec::new(),
        };
        let (made, joins) = composites(start(s, request, baked).unwrap());
        land(s, made, joins).unwrap();
    }

    fn names(s: &Session) -> Vec<&str> {
        s.document()
            .layers()
            .iter()
            .map(|l| l.name.as_str())
            .collect()
    }

    fn is_raster(layer: &Layer) -> bool {
        matches!(layer.content, LayerContent::Raster { .. })
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
        assert!(is_raster(&s.document().layers()[1]));
        // The preview and the pixels: one undo.
        assert!(s.undo().unwrap());
        assert_eq!(names(&s), ["a", "hidden", "b"]);

        apply(&mut s, r#"{"kind":"mergeVisible"}"#);
        assert_eq!(names(&s), ["hidden", "b"]);
        assert!(s.undo().unwrap());
        apply(&mut s, r#"{"kind":"flatten","name":"Flat"}"#);
        assert_eq!(names(&s), ["Flat"]);
        assert!(s.undo().unwrap());

        // New Layer from Visible: on top, the layers kept, one undo.
        apply(&mut s, r#"{"kind":"visible","name":"Stamp"}"#);
        assert_eq!(names(&s), ["a", "hidden", "b", "Stamp"]);
        assert!(is_raster(&s.document().layers()[3]));
        assert!(s.undo().unwrap());
        assert_eq!(names(&s), ["a", "hidden", "b"]);

        // A fill becomes pixels, in its place.
        apply(
            &mut s,
            &format!(r#"{{"kind":"rasterize","ids":[{}]}}"#, ids[0]),
        );
        let layer = &s.document().layers()[0];
        assert_eq!((layer.name.as_str(), layer.id.get()), ("a", ids[0]));
        assert!(is_raster(layer));
    }

    #[test]
    fn pixels_after_another_edit_make_an_entry_of_their_own() {
        let (mut s, ids) = session();
        let json = format!(r#"{{"kind":"merge","ids":[{},{}]}}"#, ids[0], ids[2]);
        let request: BakeRequest = serde_json::from_str(&json).unwrap();
        let pending = start(&mut s, request, Vec::new()).unwrap();
        // The preview shows at once: the group, named as the result.
        assert!(s.document().layers()[1].is_group());
        // Something else happens while the pixels are composited.
        s.perform(Edit::RenameLayer {
            id: LayerId::from_raw(ids[1]),
            name: "renamed".into(),
        })
        .unwrap();
        let (made, joins) = composites(pending);
        land(&mut s, made, joins).unwrap();
        assert!(is_raster(&s.document().layers()[1]));
        // Undo: the group of the preview comes back, then the rename goes, then the merge.
        assert!(s.undo().unwrap());
        assert!(s.document().layers()[1].is_group());
        assert!(s.undo().unwrap());
        assert!(s.undo().unwrap());
        assert_eq!(names(&s), ["a", "hidden", "b"]);
    }

    #[test]
    fn pixels_of_an_undone_merge_bake_nothing() {
        let (mut s, ids) = session();
        let json = format!(r#"{{"kind":"merge","ids":[{},{}]}}"#, ids[0], ids[2]);
        let request: BakeRequest = serde_json::from_str(&json).unwrap();
        let pending = start(&mut s, request, Vec::new()).unwrap();
        assert!(s.undo().unwrap());
        let (made, joins) = composites(pending);
        land(&mut s, made, joins).unwrap();
        assert_eq!(names(&s), ["a", "hidden", "b"]);
        assert!(s.can_redo());
    }
}
