//! SlopShop desktop shell.
//!
//! A thin layer between the Svelte UI and the engine crates: it owns the editing session,
//! translates IPC requests into core edits and sends display frames back. No image or document
//! logic belongs here.
//!
//! Threading: every command is `async` (so it never runs on the main/UI thread) and GPU work
//! runs in `spawn_blocking`.

mod ipc;

use std::sync::{Mutex, MutexGuard, OnceLock};

use slopshop_core::view::ViewTransform;
use slopshop_core::{Document, Edit, Layer, LayerContent, LinearRgba, Session, Size};
use slopshop_render::Renderer;
use tauri::ipc::Response;
use tauri::{AppHandle, Manager, State};

use crate::ipc::{DocumentView, EditRequest, GpuInfo};

/// Margin, in output pixels, kept around the document when fitting it in the viewport.
const VIEW_MARGIN: u32 = 24;

struct AppState {
    session: Mutex<Session>,
    /// Created lazily, off the UI thread (adapter/device creation can take a while).
    renderer: OnceLock<Result<Renderer, String>>,
}

impl AppState {
    fn session(&self) -> Result<MutexGuard<'_, Session>, String> {
        self.session
            .lock()
            .map_err(|_| "session state is poisoned".to_owned())
    }

    /// Blocking: call from a worker thread.
    fn renderer(&self) -> Result<&Renderer, String> {
        self.renderer
            .get_or_init(|| Renderer::new().map_err(|e| e.to_string()))
            .as_ref()
            .map_err(Clone::clone)
    }
}

/// The document shown at startup. Built by applying edits directly to the document, so that
/// the initial content is not part of the undo history.
fn initial_session() -> Session {
    let mut doc = Document::new(Size::new(6000, 4000));
    let id = doc.allocate_layer_id();
    let background = Edit::InsertLayer {
        index: 0,
        layer: Layer {
            id,
            name: "Background".into(),
            visible: true,
            opacity: 1.0,
            content: LayerContent::Fill {
                color: LinearRgba::new(1.0, 1.0, 1.0, 1.0),
            },
        },
    };
    // Invariant: a fresh id at index 0 of an empty document with a valid color is accepted.
    background
        .apply(&mut doc)
        .expect("initial background layer is valid");
    Session::new(doc)
}

#[tauri::command]
async fn document(state: State<'_, AppState>) -> Result<DocumentView, String> {
    Ok(DocumentView::new(&*state.session()?))
}

#[tauri::command]
async fn perform(state: State<'_, AppState>, edit: EditRequest) -> Result<DocumentView, String> {
    let mut session = state.session()?;
    let edit = edit.into_edit(&mut session);
    session.perform(edit).map_err(|e| e.to_string())?;
    Ok(DocumentView::new(&session))
}

/// Apply an edit live, as part of a continuous gesture (slider drag, …). The gesture becomes a
/// single undo entry when `end_gesture` is called (or when any other edit/undo happens).
#[tauri::command]
async fn perform_live(
    state: State<'_, AppState>,
    edit: EditRequest,
) -> Result<DocumentView, String> {
    let mut session = state.session()?;
    let edit = edit.into_edit(&mut session);
    session
        .perform_in_gesture(edit)
        .map_err(|e| e.to_string())?;
    Ok(DocumentView::new(&session))
}

#[tauri::command]
async fn end_gesture(state: State<'_, AppState>) -> Result<DocumentView, String> {
    let mut session = state.session()?;
    session.end_gesture();
    Ok(DocumentView::new(&session))
}

#[tauri::command]
async fn undo(state: State<'_, AppState>) -> Result<DocumentView, String> {
    let mut session = state.session()?;
    session.undo().map_err(|e| e.to_string())?;
    Ok(DocumentView::new(&session))
}

#[tauri::command]
async fn redo(state: State<'_, AppState>) -> Result<DocumentView, String> {
    let mut session = state.session()?;
    session.redo().map_err(|e| e.to_string())?;
    Ok(DocumentView::new(&session))
}

#[tauri::command]
async fn gpu_info(app: AppHandle) -> Result<GpuInfo, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let a = app.state::<AppState>().renderer()?.adapter_summary();
        Ok(GpuInfo {
            name: a.name,
            backend: a.backend,
            device_type: a.device_type,
            driver: a.driver,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Render the document fitted into a `width × height` viewport. Returns raw RGBA8 sRGB bytes
/// (an `ArrayBuffer` on the JS side): viewport-sized, never document-sized, and never JSON.
#[tauri::command]
async fn render_view(app: AppHandle, width: u32, height: u32) -> Result<Response, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        // Snapshot so the session lock is not held while the GPU works. Cheap today (no
        // pixel data); pixel layers will share their tiles instead of copying them.
        let doc = state.session()?.document().clone();
        let output = Size::new(width, height);
        let view = ViewTransform::fit(doc.size(), output, VIEW_MARGIN);
        let frame = state
            .renderer()?
            .render_view(&doc, view, output)
            .map_err(|e| e.to_string())?;
        Ok(Response::new(frame.data))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState {
            session: Mutex::new(initial_session()),
            renderer: OnceLock::new(),
        })
        .setup(|app| {
            // Warm up the GPU in the background so the first frame is fast.
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                if let Err(e) = handle.state::<AppState>().renderer() {
                    eprintln!("GPU initialization failed: {e}");
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            document,
            perform,
            perform_live,
            end_gesture,
            undo,
            redo,
            gpu_info,
            render_view
        ])
        .run(tauri::generate_context!())
        .expect("error while running SlopShop");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_session_has_background_and_empty_history() {
        let s = initial_session();
        assert_eq!(s.document().layers().len(), 1);
        assert!(!s.can_undo());
    }

    #[test]
    fn edit_requests_round_trip_through_undo() {
        let mut s = initial_session();
        let json = r#"{"kind":"addFillLayer","name":"Pink","color":[1.0,0.5,0.8,1.0]}"#;
        let request: EditRequest = serde_json::from_str(json).unwrap();
        let edit = request.into_edit(&mut s);
        s.perform(edit).unwrap();
        let view = DocumentView::new(&s);
        assert_eq!(view.layers.len(), 2);
        assert_eq!(view.layers[1].name, "Pink");
        // The swatch is converted back to sRGB for display: it must match what was sent.
        let swatch = view.layers[1].swatch;
        assert!((swatch[1] - 0.5).abs() < 1e-5, "swatch {swatch:?}");

        s.undo().unwrap();
        assert_eq!(DocumentView::new(&s).layers.len(), 1);
    }
}
