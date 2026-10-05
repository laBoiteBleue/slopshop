//! Select > Select and Mask (ADR 0024): a panel beside the image refines the selection it was
//! opened with: ViTMatte can matte its edge (`segment::ai_refine_base`), within a band of the
//! outline and wherever the refine-edge brush painted (`refine_brush`), which gives the *base*;
//! the edge settings are shown live on the base as a gesture each new value replaces; the
//! result goes to the selection, the active layer's mask, or a copy of the layer with that
//! mask, as one undo entry. The view of the selection (ants, overlay, on black or white, the
//! mask) is view state.

use slopshop_core::HistoryLabel;
use std::sync::Arc;

use serde::Deserialize;
use slopshop_core::paint::{Brush, Paint, PointerSample, Stroke};
use slopshop_core::selection::{self, EdgeSettings, Selection};
use slopshop_core::{Edit, LayerId, LayerMask, Projective, RasterImage};
use slopshop_render::SelectionView;
use tauri::{AppHandle, Manager, State};

use crate::AppState;
use crate::ipc::DocumentView;
use crate::selection::on_worker;

/// What an open Select and Mask works on.
#[derive(Debug, Clone)]
pub(crate) struct RefineSession {
    /// The selection it was opened with: what edge detection mattes, each time from scratch.
    pub original: Arc<RasterImage>,
    /// What the settings apply to: the original, or its matted edge.
    pub base: Arc<RasterImage>,
    /// Where the refine-edge brush painted (coverage), `None` before it paints.
    pub region: Option<Arc<RasterImage>>,
}

impl RefineSession {
    pub fn new(selection: Arc<RasterImage>) -> Self {
        Self {
            original: Arc::clone(&selection),
            base: selection,
            region: None,
        }
    }
}

/// The edge settings, from the panel (document pixels, Contrast in percent).
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EdgeRequest {
    smooth: f64,
    feather: f64,
    contrast: f64,
    shift: f64,
}

impl From<EdgeRequest> for EdgeSettings {
    fn from(r: EdgeRequest) -> Self {
        EdgeSettings {
            smooth: r.smooth,
            shift: r.shift,
            feather: r.feather,
            contrast: r.contrast,
        }
    }
}

/// Select and Mask opens on the current selection: it becomes the base the settings apply to.
#[tauri::command]
pub async fn refine_open(
    state: State<'_, AppState>,
    document_id: u64,
) -> Result<DocumentView, String> {
    let mut documents = state.documents()?;
    let document = documents.get_mut(document_id)?;
    let base = document
        .session
        .document()
        .selection()
        .map(|s| Arc::clone(s.image()))
        .ok_or("nothing is selected")?;
    document.refine = Some(RefineSession::new(base));
    Ok(document.view())
}

/// How the selection is shown while the panel is open: `ants`, `overlay`, `onBlack`, `onWhite`
/// or `mask`.
#[tauri::command]
pub async fn refine_view(
    state: State<'_, AppState>,
    document_id: u64,
    view: String,
) -> Result<DocumentView, String> {
    let view = match view.as_str() {
        "ants" => SelectionView::Off,
        "overlay" => SelectionView::Overlay,
        "onBlack" => SelectionView::OnBlack,
        "onWhite" => SelectionView::OnWhite,
        "mask" => SelectionView::Mask,
        other => return Err(format!("unknown selection view {other}")),
    };
    let mut documents = state.documents()?;
    let document = documents.get_mut(document_id)?;
    document.overlays.selection_view = view;
    Ok(document.view())
}

/// The base with `settings` applied becomes the selection: shown while `live` (the next one
/// replaces it), else one undo entry and the panel's session ends.
#[tauri::command]
pub async fn refine_preview(
    app: AppHandle,
    document_id: u64,
    settings: EdgeRequest,
    live: bool,
) -> Result<DocumentView, String> {
    on_worker(move || preview(&app.state::<AppState>(), document_id, settings.into(), live)).await
}

/// [`refine_preview`]'s work, on a worker thread.
pub(crate) fn preview(
    state: &AppState,
    document_id: u64,
    settings: EdgeSettings,
    live: bool,
) -> Result<DocumentView, String> {
    let (canvas, base) = base_of(state, document_id)?;
    let image = selection::refine_edges(canvas, &base, settings).map_err(|e| e.to_string())?;
    let selection = match image {
        Some(image) => Some(Selection::new(Arc::new(image)).ok_or("a selection is gray")?),
        None => None,
    };
    let mut documents = state.documents()?;
    let document = documents.get_mut(document_id)?;
    let session = &mut document.session;
    session.cancel_gesture().map_err(|e| e.to_string())?;
    session
        .perform_in_gesture(Edit::SetSelection { selection })
        .map_err(|e| e.to_string())?;
    if !live {
        session.end_gesture();
        close_session(document);
    }
    Ok(document.view())
}

/// The canvas and the panel's base.
fn base_of(
    state: &AppState,
    document_id: u64,
) -> Result<(slopshop_core::Size, Arc<RasterImage>), String> {
    let mut documents = state.documents()?;
    let document = documents.get_mut(document_id)?;
    let base = document
        .refine
        .as_ref()
        .map(|r| Arc::clone(&r.base))
        .ok_or("Select and Mask is not open")?;
    Ok((document.session.document().size(), base))
}

fn close_session(document: &mut crate::OpenDocument) {
    document.refine = None;
    document.overlays.selection_view = SelectionView::Off;
}

/// The refine-edge brush: a stroke (`samples`: `[x, y, pressure]`, document pixels) of a round
/// brush `size` pixels wide painting where edge detection decides (`erase`: no longer). The
/// panel then runs edge detection again.
#[tauri::command]
pub async fn refine_brush(
    app: AppHandle,
    document_id: u64,
    samples: Vec<[f64; 3]>,
    size: f32,
    erase: bool,
) -> Result<(), String> {
    on_worker(move || brush(&app.state::<AppState>(), document_id, &samples, size, erase)).await
}

pub(crate) fn brush(
    state: &AppState,
    document_id: u64,
    samples: &[[f64; 3]],
    size: f32,
    erase: bool,
) -> Result<(), String> {
    let (canvas, blend_space, region) = {
        let mut documents = state.documents()?;
        let document = documents.get_mut(document_id)?;
        let session = document
            .refine
            .as_ref()
            .ok_or("Select and Mask is not open")?;
        let doc = document.session.document();
        (doc.size(), doc.blend_space(), session.region.clone())
    };
    let region = match region {
        Some(region) => region,
        None => Arc::new(selection::uniform_mask(canvas, false).map_err(|e| e.to_string())?),
    };
    // The Brush's engine, painting a gray coverage (ADR 0027): white marks, the eraser clears.
    let brush = Brush {
        diameter: size.clamp(1.0, slopshop_core::paint::MAX_DIAMETER),
        hardness: 1.0,
        pressure_size: false,
        ..Brush::default()
    };
    let paint = Paint::Gray(if erase { 0.0 } else { 1.0 });
    let mut stroke = Stroke::new(
        region,
        Projective::IDENTITY,
        None,
        blend_space,
        brush,
        paint,
    )
    .map_err(|e| e.to_string())?;
    let samples: Vec<PointerSample> = samples
        .iter()
        .map(|&[x, y, pressure]| PointerSample {
            x,
            y,
            pressure: pressure as f32,
        })
        .collect();
    stroke.add(&samples);
    let Some(painted) = stroke.finish().map_err(|e| e.to_string())? else {
        return Ok(());
    };
    let mut documents = state.documents()?;
    let document = documents.get_mut(document_id)?;
    if let Some(session) = document.refine.as_mut() {
        session.region = Some(painted);
    }
    Ok(())
}

/// Cancel: the selection as it was when the panel opened.
#[tauri::command]
pub async fn refine_close(
    state: State<'_, AppState>,
    document_id: u64,
) -> Result<DocumentView, String> {
    let mut documents = state.documents()?;
    let document = documents.get_mut(document_id)?;
    document
        .session
        .cancel_gesture()
        .map_err(|e| e.to_string())?;
    close_session(document);
    Ok(document.view())
}

/// The refined selection as a layer mask, as one undo entry that also deselects (as Photoshop's
/// Output To): on layer `layer_id` (`newLayer` false, its mask replaced), or on a copy of it
/// named by `name_format` (`{name}`: the layer's name) above it, the layer hidden.
#[tauri::command]
pub async fn refine_output(
    app: AppHandle,
    document_id: u64,
    settings: EdgeRequest,
    layer_id: u64,
    new_layer: bool,
    name_format: String,
) -> Result<DocumentView, String> {
    on_worker(move || {
        let state = app.state::<AppState>();
        output(
            &state,
            document_id,
            settings.into(),
            LayerId::from_raw(layer_id),
            new_layer,
            &name_format,
        )
    })
    .await
}

pub(crate) fn output(
    state: &AppState,
    document_id: u64,
    settings: EdgeSettings,
    id: LayerId,
    new_layer: bool,
    name_format: &str,
) -> Result<DocumentView, String> {
    let (canvas, base) = base_of(state, document_id)?;
    let (size, to_document) = {
        let mut documents = state.documents()?;
        let doc = documents.get_mut(document_id)?.session.document();
        let layer = doc.layer(id).ok_or("unknown layer")?;
        (
            crate::selection::mask_size(doc, layer)?,
            layer.transform.then(doc.parent_transform(id)),
        )
    };
    let refined = selection::refine_edges(canvas, &base, settings).map_err(|e| e.to_string())?;
    let image = match &refined {
        Some(refined) => selection::layer_mask(refined, size, to_document, false),
        None => selection::uniform_mask(size, false),
    }
    .map_err(|e| e.to_string())?;
    let mask = LayerMask {
        original: None,
        image: Arc::new(image),
        enabled: true,
        replaces_alpha: false,
    };
    let mut documents = state.documents()?;
    let document = documents.get_mut(document_id)?;
    let session = &mut document.session;
    session.cancel_gesture().map_err(|e| e.to_string())?;
    let mut edits = Vec::new();
    if new_layer {
        let mut copy = session
            .duplicate_layers_edit(&[id], |name| name_format.replace("{name}", name))
            .map_err(|e| e.to_string())?;
        // The copy is the only layer it inserts: it takes the mask.
        if let Edit::Batch(inserts) = &mut copy {
            for insert in inserts {
                if let Edit::InsertLayer { layer, .. } = insert {
                    layer.mask = Some(mask.clone());
                }
            }
        }
        edits.push(copy);
        edits.push(Edit::SetLayerVisible { id, visible: false });
    } else {
        edits.push(Edit::SetLayerMask {
            id,
            mask: Some(mask),
        });
    }
    if session.document().selection().is_some() {
        edits.push(Edit::SetSelection { selection: None });
    }
    session
        .with_label(Some(HistoryLabel::new("selectAndMask")), |s| {
            s.perform(Edit::Batch(edits))
        })
        .map_err(|e| e.to_string())?;
    close_session(document);
    Ok(document.view())
}
