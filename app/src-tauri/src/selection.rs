//! Selection commands (ADR 0024): thin wrappers over `slopshop_core::selection`. The masks are
//! built on a worker thread, then set on the document as one undoable edit; the UI only ever
//! receives the outline, sized to the view.

use std::sync::Arc;

use serde::Deserialize;
use slopshop_core::selection::{self, Combine, EdgeOptions, Selection, Shape};
use slopshop_core::{Edit, RasterImage, Rect, Size};
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
