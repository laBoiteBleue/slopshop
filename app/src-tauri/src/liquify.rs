//! Filter > Liquify (ADR 0037): the workspace's session. Opening it evaluates what the layer
//! shows (or what a Liquify entry is applied to, to edit it again) and starts from the entry's
//! field or an empty one; strokes edit the field in the engine (`slopshop_core::liquify`), the
//! workspace asks for frames of the layer seen through it, and OK makes the field one entry of the
//! layer's stack, one undo entry. Nothing of the document changes before OK.

use std::sync::{Arc, Mutex, PoisonError};

use serde::{Deserialize, Serialize};
use slopshop_core::liquify::{Brush, Field, Stroke, Tool, View, frame};
use slopshop_core::stack::Entry;
use slopshop_core::{BlendSpace, Edit, HistoryLabel, LayerContent, LayerId, RasterImage};
use tauri::ipc::Response;
use tauri::{AppHandle, Manager, State};

use crate::AppState;
use crate::ipc::DocumentView;
use crate::selection::on_worker;

/// The most pixels a frame has (a 16 megapixel view is far beyond any screen).
const MAX_FRAME_PIXELS: u64 = 16_000_000;

/// What an open Liquify workspace works on.
#[derive(Debug)]
pub(crate) struct LiquifySession {
    layer: LayerId,
    /// The entry edited again, or none for a new one.
    index: Option<usize>,
    /// What the entry is applied to: the layer's pixels below it.
    input: Arc<RasterImage>,
    space: BlendSpace,
    field: Field,
    /// The field as the workspace opened: OK without a change adds nothing.
    start: Field,
    /// The fields before each stroke, and those undone.
    undo: Vec<Field>,
    redo: Vec<Field>,
    stroke: Option<Stroke>,
}

/// The session as the workspace needs to know it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiquifyState {
    /// The layer's size in pixels.
    pub width: u32,
    pub height: u32,
    pub can_undo: bool,
    pub can_redo: bool,
    /// The field is not what the workspace opened with.
    pub changed: bool,
    /// Something is displaced (Restore All does something).
    pub displaced: bool,
}

impl LiquifySession {
    fn state(&self) -> LiquifyState {
        let size = self.input.size();
        LiquifyState {
            width: size.width,
            height: size.height,
            can_undo: !self.undo.is_empty(),
            can_redo: !self.redo.is_empty(),
            changed: !same_field(&self.field, &self.start),
            displaced: !self.field.is_identity(),
        }
    }

    /// Remember the field before a change, so that Undo gives it back.
    fn remember(&mut self) {
        self.undo.push(self.field.clone());
        self.redo.clear();
    }
}

/// Whether two fields are the same: the same tiles (a stroke changes at least one), as far as
/// what the workspace shows and OK stores.
fn same_field(a: &Field, b: &Field) -> bool {
    a.shares_tiles_with(b)
}

/// A brush's settings, from the workspace.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrushRequest {
    size: f32,
    density: f32,
    pressure: f32,
    rate: f32,
}

impl BrushRequest {
    fn brush(self) -> Result<Brush, String> {
        let brush = Brush {
            size: self.size,
            density: self.density,
            pressure: self.pressure,
            rate: self.rate,
        };
        brush
            .is_valid()
            .then_some(brush)
            .ok_or_else(|| "invalid brush settings".to_owned())
    }
}

/// A piece of a stroke: the pointer's samples in layer pixels, and how long it stayed at the
/// last one.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StrokeRequest {
    /// A tool's identifier (`Tool::id`).
    tool: String,
    brush: BrushRequest,
    /// The pointer went down: a new stroke.
    begin: bool,
    /// Where the pointer went since the last piece.
    points: Vec<[f64; 2]>,
    /// Seconds the pointer has been held still since.
    hold: f64,
    /// The pointer went up: the stroke is done.
    end: bool,
}

/// What the workspace shows: layer point (`x`, `y`) at the top left, `zoom` pixels per layer
/// pixel, the frame's size in pixels, and whether the frozen area is tinted.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewRequest {
    x: f64,
    y: f64,
    zoom: f64,
    width: u32,
    height: u32,
    overlay: bool,
}

fn session_of(state: &AppState, document_id: u64) -> Result<Arc<Mutex<LiquifySession>>, String> {
    let mut documents = state.documents()?;
    documents
        .get_mut(document_id)?
        .liquify
        .clone()
        .ok_or_else(|| "Liquify is not open".to_owned())
}

fn locked(session: &Mutex<LiquifySession>) -> std::sync::MutexGuard<'_, LiquifySession> {
    // A poisoned lock only means a stroke panicked: the field it left is still a field.
    session.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Filter > Liquify…: the workspace opens on raster layer `layer_id`, with Liquify entry `index`
/// of its stack to edit again, or a new one.
#[tauri::command]
pub async fn liquify_open(
    app: AppHandle,
    document_id: u64,
    layer_id: u64,
    index: Option<usize>,
) -> Result<LiquifyState, String> {
    on_worker(move || open(&app.state::<AppState>(), document_id, layer_id, index)).await
}

pub(crate) fn open(
    state: &AppState,
    document_id: u64,
    layer_id: u64,
    index: Option<usize>,
) -> Result<LiquifyState, String> {
    let id = LayerId::from_raw(layer_id);
    let (stack, shown, space) = {
        let mut documents = state.documents()?;
        let document = documents.get_mut(document_id)?;
        let doc = document.session.document();
        let layer = doc.layer(id).ok_or("unknown layer")?;
        let LayerContent::Raster { image, .. } = &layer.content else {
            return Err("Liquify needs a pixel layer".to_owned());
        };
        if !layer.visible {
            return Err("the layer is hidden".to_owned());
        }
        (
            layer.content.stack().ok_or("Liquify needs a pixel layer")?,
            image.clone(),
            doc.blend_space(),
        )
    };
    // The pixels are evaluated here, on a worker: the lock is not held meanwhile.
    let (input, field, space) = match index {
        None => (shown.get(), None, space),
        Some(index) => {
            let Some(Entry::Liquify(entry)) = stack.entries().get(index) else {
                return Err("not a Liquify entry".to_owned());
            };
            (
                stack.evaluate_below(index).map_err(|e| e.to_string())?,
                Some(Field::clone(entry.field())),
                entry.space(),
            )
        }
    };
    let field = field.unwrap_or_else(|| Field::new(input.size()));
    let session = LiquifySession {
        layer: id,
        index,
        input,
        space,
        start: field.clone(),
        field,
        undo: Vec::new(),
        redo: Vec::new(),
        stroke: None,
    };
    let opened = session.state();
    let mut documents = state.documents()?;
    documents.get_mut(document_id)?.liquify = Some(Arc::new(Mutex::new(session)));
    Ok(opened)
}

/// A piece of a stroke of a tool (see [`StrokeRequest`]).
#[tauri::command]
pub async fn liquify_stroke(
    app: AppHandle,
    document_id: u64,
    request: StrokeRequest,
) -> Result<LiquifyState, String> {
    on_worker(move || stroke(&app.state::<AppState>(), document_id, &request)).await
}

pub(crate) fn stroke(
    state: &AppState,
    document_id: u64,
    request: &StrokeRequest,
) -> Result<LiquifyState, String> {
    let session = session_of(state, document_id)?;
    let mut session = locked(&session);
    if request.begin {
        let tool = Tool::from_id(&request.tool).ok_or("unknown Liquify tool")?;
        session.remember();
        session.stroke = Some(Stroke::new(tool, request.brush.brush()?));
    }
    let session = &mut *session;
    let stroke = session.stroke.as_mut().ok_or("no stroke is under way")?;
    for point in &request.points {
        stroke.move_to(&mut session.field, *point);
    }
    if request.hold > 0.0 {
        stroke.hold(&mut session.field, request.hold);
    }
    if request.end {
        stroke.finish(&mut session.field);
        session.stroke = None;
    }
    Ok(session.state())
}

/// Undo the last stroke or Restore All of the workspace (`redo`: bring it back).
#[tauri::command]
pub async fn liquify_undo(
    app: AppHandle,
    document_id: u64,
    redo: bool,
) -> Result<LiquifyState, String> {
    on_worker(move || undo(&app.state::<AppState>(), document_id, redo)).await
}

pub(crate) fn undo(state: &AppState, document_id: u64, redo: bool) -> Result<LiquifyState, String> {
    let session = session_of(state, document_id)?;
    let mut session = locked(&session);
    let session = &mut *session;
    session.stroke = None;
    let (from, to) = if redo {
        (&mut session.redo, &mut session.undo)
    } else {
        (&mut session.undo, &mut session.redo)
    };
    if let Some(field) = from.pop() {
        to.push(std::mem::replace(&mut session.field, field));
    }
    Ok(session.state())
}

/// Restore All: every displacement taken back (the freeze mask stays), one step of the
/// workspace's own undo.
#[tauri::command]
pub async fn liquify_restore_all(app: AppHandle, document_id: u64) -> Result<LiquifyState, String> {
    on_worker(move || restore_all(&app.state::<AppState>(), document_id)).await
}

pub(crate) fn restore_all(state: &AppState, document_id: u64) -> Result<LiquifyState, String> {
    let session = session_of(state, document_id)?;
    let mut session = locked(&session);
    session.stroke = None;
    if !session.field.is_identity() {
        session.remember();
        session.field = session.field.restored();
    }
    Ok(session.state())
}

/// The layer seen through the field, as 8-bit sRGB RGBA with straight alpha, `width × height`
/// pixels, row-major (see [`ViewRequest`]).
#[tauri::command]
pub async fn liquify_frame(
    app: AppHandle,
    document_id: u64,
    view: ViewRequest,
) -> Result<Response, String> {
    on_worker(move || frame_of(&app.state::<AppState>(), document_id, view).map(Response::new))
        .await
}

pub(crate) fn frame_of(
    state: &AppState,
    document_id: u64,
    view: ViewRequest,
) -> Result<Vec<u8>, String> {
    if u64::from(view.width) * u64::from(view.height) > MAX_FRAME_PIXELS {
        return Err("the view is too large".to_owned());
    }
    let session = session_of(state, document_id)?;
    // Cheap: the field's tiles and the pixels are shared. The lock is not held while drawing.
    let (input, space, field) = {
        let session = locked(&session);
        (
            Arc::clone(&session.input),
            session.space,
            session.field.clone(),
        )
    };
    frame(
        &input,
        space,
        &field,
        View {
            origin: [view.x, view.y],
            zoom: view.zoom,
            width: view.width,
            height: view.height,
        },
        view.overlay,
    )
    .map_err(|e| e.to_string())
}

/// OK: the field becomes an entry of the layer's stack, one undo entry (none when nothing
/// changed), and the workspace closes.
#[tauri::command]
pub async fn liquify_commit(app: AppHandle, document_id: u64) -> Result<DocumentView, String> {
    on_worker(move || commit(&app.state::<AppState>(), document_id)).await
}

pub(crate) fn commit(state: &AppState, document_id: u64) -> Result<DocumentView, String> {
    let session = session_of(state, document_id)?;
    let (layer, index, input, field, changed) = {
        let session = locked(&session);
        (
            session.layer,
            session.index,
            Arc::clone(&session.input),
            Arc::new(session.field.clone().pruned()),
            session.state().changed,
        )
    };
    let mut documents = state.documents()?;
    let document = documents.get_mut(document_id)?;
    document.liquify = None;
    if !changed {
        return Ok(document.view());
    }
    let edit = match index {
        None if field.is_identity() => return Ok(document.view()),
        None => Edit::apply_liquify(document.session.document(), layer, field, Some(input)),
        Some(index) => Edit::set_liquify(
            document.session.document(),
            layer,
            index,
            field,
            Some(input),
        ),
    }
    .map_err(|e| e.to_string())?;
    let label = HistoryLabel::new(if index.is_some() {
        "editEntry"
    } else {
        "liquify"
    });
    document
        .session
        .with_label(Some(label), |s| s.perform(edit))
        .map_err(|e| e.to_string())?;
    Ok(document.view())
}

/// Cancel: the workspace closes, the document is as it was.
#[tauri::command]
pub async fn liquify_close(state: State<'_, AppState>, document_id: u64) -> Result<(), String> {
    let mut documents = state.documents()?;
    documents.get_mut(document_id)?.liquify = None;
    Ok(())
}
