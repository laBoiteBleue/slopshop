//! Selection commands (ADR 0024): thin wrappers over `slopshop_core::selection`. The masks are
//! built on a worker thread, then set on the document as one undoable edit; the UI only ever
//! receives the outline, sized to the view.

use std::sync::Arc;

use serde::Deserialize;
use slopshop_core::selection::{self, Combine, EdgeOptions, Selection, Shape};
use slopshop_core::{Edit, LayerContent, LayerId, LayerMask, RasterImage, Rect, Size};
use tauri::State;
use tauri::ipc::Response;

use crate::AppState;
use crate::ipc::DocumentView;

/// A shape to select, in document pixels.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ShapeRequest {
    Rectangle {
        left: f64,
        top: f64,
        right: f64,
        bottom: f64,
    },
    Ellipse {
        left: f64,
        top: f64,
        right: f64,
        bottom: f64,
    },
    Polygon {
        points: Vec<[f64; 2]>,
    },
}

impl From<ShapeRequest> for Shape {
    fn from(request: ShapeRequest) -> Self {
        match request {
            ShapeRequest::Rectangle {
                left,
                top,
                right,
                bottom,
            } => Shape::Rectangle {
                left,
                top,
                right,
                bottom,
            },
            ShapeRequest::Ellipse {
                left,
                top,
                right,
                bottom,
            } => Shape::Ellipse {
                left,
                top,
                right,
                bottom,
            },
            ShapeRequest::Polygon { points } => Shape::Polygon { points },
        }
    }
}

fn combine(id: &str) -> Result<Combine, String> {
    match id {
        "replace" => Ok(Combine::Replace),
        "add" => Ok(Combine::Add),
        "subtract" => Ok(Combine::Subtract),
        "intersect" => Ok(Combine::Intersect),
        other => Err(format!("unknown selection mode {other}")),
    }
}

/// The canvas size and the current selection of a document.
fn snapshot(
    state: &AppState,
    document_id: u64,
) -> Result<(Size, Option<Arc<RasterImage>>), String> {
    let mut documents = state.documents()?;
    let doc = documents.get_mut(document_id)?.session.document();
    Ok((doc.size(), doc.selection().map(|s| Arc::clone(s.image()))))
}

/// Make `image` (or nothing) the selection, as one undo entry; nothing is recorded when it is
/// already so.
fn set_selection(
    state: &AppState,
    document_id: u64,
    image: Option<RasterImage>,
) -> Result<DocumentView, String> {
    let selection = match image {
        Some(image) => Some(Selection::new(Arc::new(image)).ok_or("a selection is gray")?),
        None => None,
    };
    let mut documents = state.documents()?;
    let document = documents.get_mut(document_id)?;
    if selection.is_some() || document.session.document().selection().is_some() {
        document
            .session
            .perform(Edit::SetSelection { selection })
            .map_err(|e| e.to_string())?;
    }
    Ok(document.view())
}

async fn on_worker<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|e| e.to_string())?
}

/// Select `shape`, combined with the current selection (`replace`, `add`, `subtract` or
/// `intersect`), with anti-aliased edges or not and a feather radius in pixels.
#[tauri::command]
pub async fn select_shape(
    state: State<'_, AppState>,
    document_id: u64,
    shape: ShapeRequest,
    mode: String,
    anti_alias: bool,
    feather: f64,
) -> Result<DocumentView, String> {
    let combine = combine(&mode)?;
    let (canvas, current) = snapshot(&state, document_id)?;
    let shape = Shape::from(shape);
    let edges = EdgeOptions {
        anti_alias,
        feather,
    };
    let image = on_worker(move || {
        selection::select_shape(canvas, current.as_deref(), &shape, edges, combine)
            .map_err(|e| e.to_string())
    })
    .await?;
    set_selection(&state, document_id, image)
}

/// Select > All.
#[tauri::command]
pub async fn select_all(
    state: State<'_, AppState>,
    document_id: u64,
) -> Result<DocumentView, String> {
    let (canvas, _) = snapshot(&state, document_id)?;
    let image = on_worker(move || selection::select_all(canvas).map_err(|e| e.to_string())).await?;
    set_selection(&state, document_id, Some(image))
}

/// Select > Inverse.
#[tauri::command]
pub async fn invert_selection(
    state: State<'_, AppState>,
    document_id: u64,
) -> Result<DocumentView, String> {
    let (canvas, current) = snapshot(&state, document_id)?;
    let image =
        on_worker(move || selection::invert(canvas, current.as_deref()).map_err(|e| e.to_string()))
            .await?;
    set_selection(&state, document_id, image)
}

/// Select > Modify (`kind`: `feather`, `expand`, `contract`, `border`, `smooth`) by `amount`
/// pixels, as one undo entry; nothing left selected deselects.
#[tauri::command]
pub async fn modify_selection(
    state: State<'_, AppState>,
    document_id: u64,
    kind: String,
    amount: f64,
) -> Result<DocumentView, String> {
    let how = match kind.as_str() {
        "feather" => selection::Modify::Feather(amount),
        "expand" => selection::Modify::Expand(amount),
        "contract" => selection::Modify::Contract(amount),
        "border" => selection::Modify::Border(amount),
        "smooth" => selection::Modify::Smooth(amount),
        other => return Err(format!("unknown selection change {other}")),
    };
    let (canvas, current) = snapshot(&state, document_id)?;
    let current = current.ok_or("nothing is selected")?;
    let image =
        on_worker(move || selection::modify(canvas, &current, how).map_err(|e| e.to_string()))
            .await?;
    set_selection(&state, document_id, image)
}

/// Select > Deselect: the selection is kept for Reselect.
#[tauri::command]
pub async fn deselect(
    state: State<'_, AppState>,
    document_id: u64,
) -> Result<DocumentView, String> {
    let mut documents = state.documents()?;
    let document = documents.get_mut(document_id)?;
    let Some(current) = document.session.document().selection().cloned() else {
        return Ok(document.view());
    };
    document
        .session
        .perform(Edit::SetSelection { selection: None })
        .map_err(|e| e.to_string())?;
    document.last_selection = Some(current);
    Ok(document.view())
}

/// Select > Reselect: the selection last removed by Deselect, if it still fits the canvas.
#[tauri::command]
pub async fn reselect(
    state: State<'_, AppState>,
    document_id: u64,
) -> Result<DocumentView, String> {
    let mut documents = state.documents()?;
    let document = documents.get_mut(document_id)?;
    let Some(last) = document.reselectable().cloned() else {
        return Ok(document.view());
    };
    document
        .session
        .perform(Edit::SetSelection {
            selection: Some(last),
        })
        .map_err(|e| e.to_string())?;
    Ok(document.view())
}

/// Select > Edit in Quick Mask Mode (Q): the view tints what the selection leaves out. View
/// state: not an edit, not in the history.
#[tauri::command]
pub async fn set_quick_mask(
    state: State<'_, AppState>,
    document_id: u64,
    on: bool,
) -> Result<DocumentView, String> {
    let mut documents = state.documents()?;
    let document = documents.get_mut(document_id)?;
    document.overlays.quick_mask = on;
    Ok(document.view())
}

/// Layer > Layer Mask > Reveal All, Hide All, Reveal Selection or Hide Selection (`kind`:
/// `revealAll`, `hideAll`, `revealSelection`, `hideSelection`) on each of `layer_ids` that has no
/// mask yet, as one undo entry. A mask from the selection drops the selection, as Photoshop does.
#[tauri::command]
pub async fn add_layer_masks(
    state: State<'_, AppState>,
    document_id: u64,
    layer_ids: Vec<u64>,
    kind: String,
) -> Result<DocumentView, String> {
    let (from_selection, hide) = match kind.as_str() {
        "revealAll" => (false, false),
        "hideAll" => (false, true),
        "revealSelection" => (true, false),
        "hideSelection" => (true, true),
        other => return Err(format!("unknown layer mask {other}")),
    };
    // Each layer's mask size and placement, from a snapshot of the document.
    let (targets, selection) = {
        let mut documents = state.documents()?;
        let doc = documents.get_mut(document_id)?.session.document();
        let canvas = doc.size();
        let mut targets = Vec::new();
        for raw in layer_ids {
            let id = LayerId::from_raw(raw);
            let layer = doc.layer(id).ok_or("unknown layer")?;
            if layer.mask.is_some() {
                continue;
            }
            let to_document = layer.transform.then(doc.parent_transform(id));
            let size = match &layer.content {
                LayerContent::Raster { image } => image.size(),
                // Fills, groups and adjustments: the canvas, seen from the layer.
                _ => {
                    let inverse = to_document
                        .inverse()
                        .ok_or("a layer transform is not invertible")?;
                    let [_, _, x1, y1] = inverse.map_rect([
                        0.0,
                        0.0,
                        f64::from(canvas.width),
                        f64::from(canvas.height),
                    ]);
                    let side = |v: f64| v.ceil().clamp(1.0, f64::from(u32::MAX)) as u32;
                    Size::new(side(x1), side(y1))
                }
            };
            targets.push((id, size, to_document));
        }
        let selection = doc.selection().map(|s| Arc::clone(s.image()));
        (targets, selection)
    };
    if from_selection && selection.is_none() {
        return Err("nothing is selected".to_owned());
    }
    let masks = on_worker(move || {
        targets
            .into_iter()
            .map(|(id, size, to_document)| {
                let image = match &selection {
                    Some(selection) if from_selection => {
                        selection::layer_mask(selection, size, to_document, hide)
                    }
                    _ => selection::uniform_mask(size, !hide),
                }
                .map_err(|e| e.to_string())?;
                Ok((id, image))
            })
            .collect::<Result<Vec<_>, String>>()
    })
    .await?;
    let mut documents = state.documents()?;
    let document = documents.get_mut(document_id)?;
    let mut edits: Vec<Edit> = masks
        .into_iter()
        .map(|(id, image)| Edit::SetLayerMask {
            id,
            mask: Some(LayerMask {
                image: Arc::new(image),
                enabled: true,
                replaces_alpha: false,
            }),
        })
        .collect();
    if edits.is_empty() {
        return Ok(document.view());
    }
    if from_selection {
        edits.push(Edit::SetSelection { selection: None });
    }
    document
        .session
        .perform(Edit::Batch(edits))
        .map_err(|e| e.to_string())?;
    Ok(document.view())
}

/// Image > Crop with a selection: the canvas becomes the selection's bounds (nothing is deleted,
/// ADR 0017), which drops the selection.
#[tauri::command]
pub async fn crop_to_selection(
    state: State<'_, AppState>,
    document_id: u64,
) -> Result<DocumentView, String> {
    let (_, current) = snapshot(&state, document_id)?;
    let image = current.ok_or("nothing is selected")?;
    let bounds = on_worker(move || Ok(selection::bounds(&image))).await?;
    let Some(bounds) = bounds else {
        return Err("nothing is selected".to_owned());
    };
    let mut documents = state.documents()?;
    let document = documents.get_mut(document_id)?;
    let area = [
        i64::from(bounds.x),
        i64::from(bounds.y),
        i64::from(bounds.width),
        i64::from(bounds.height),
    ];
    let crop = Edit::crop(document.session.document(), area).map_err(|e| e.to_string())?;
    document.session.perform(crop).map_err(|e| e.to_string())?;
    Ok(document.view())
}

/// Most points an outline sends: beyond it, a coarser level is used.
const MAX_OUTLINE_POINTS: usize = 60_000;

/// The marching ants of the selection over `x, y, width, height` (document pixels) at `zoom`
/// (screen pixels per document pixel): raw binary, little-endian `u32`s: the number of lines,
/// then for each its number of points and their `x, y` in document pixels. No lines when
/// nothing is selected.
#[tauri::command]
pub async fn selection_outline(
    state: State<'_, AppState>,
    document_id: u64,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    zoom: f64,
) -> Result<Response, String> {
    let (_, current) = snapshot(&state, document_id)?;
    let Some(image) = current else {
        return Ok(Response::new(0u32.to_le_bytes().to_vec()));
    };
    let region = Rect::new(x, y, width, height);
    on_worker(move || {
        // The level whose pixels are at least about one screen pixel.
        let mut level = if zoom > 0.0 && zoom.is_finite() {
            (1.0 / zoom).log2().floor().max(0.0) as usize
        } else {
            0
        };
        let levels = image.levels().len();
        let lines = loop {
            match selection::outline(&image, level, region, MAX_OUTLINE_POINTS) {
                Some(lines) => break lines,
                None if level + 1 < levels => level += 1,
                None => break Vec::new(),
            }
        };
        let mut bytes = (lines.len() as u32).to_le_bytes().to_vec();
        for line in &lines {
            bytes.extend_from_slice(&(line.len() as u32).to_le_bytes());
            for [px, py] in line {
                bytes.extend_from_slice(&px.to_le_bytes());
                bytes.extend_from_slice(&py.to_le_bytes());
            }
        }
        Ok(Response::new(bytes))
    })
    .await
}
