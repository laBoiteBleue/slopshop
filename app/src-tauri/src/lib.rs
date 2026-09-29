//! SlopShop desktop shell.
//!
//! A thin layer between the Svelte UI and the engine crates: it owns the open documents (one per
//! tab) and their views, translates IPC requests into engine calls and sends display frames
//! back. No image or document logic belongs here.
//!
//! Threading: every command is `async` (so it never runs on the main/UI thread) and heavy work
//! (GPU, decoding) runs in `spawn_blocking` or a worker thread.

mod export;
mod ipc;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use slopshop_core::view::Viewport;
use slopshop_core::{
    Document, Edit, Layer, LayerContent, LayerId, LinearRgba, RasterImage, Rect, Session, Size,
};
use slopshop_io::slop::SlopFile;
use slopshop_render::Renderer;
use slopshop_render::present::{Presented, Presenter};
use tauri::ipc::Response;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_opener::OpenerExt;

use crate::export::ExportJobs;
use crate::ipc::{
    DocumentMeta, DocumentView, EditRequest, FRAME_HEADER_LEN, FrameHeader, GpuInfo, PresentInfo,
    SaveFailed, ViewInfo, ViewRequest,
};

/// How the viewport reaches the screen (ADR 0002):
/// - `window` (default on Windows): the engine presents to the window surface, drawn under a
///   transparent webview; the UI leaves the canvas area transparent;
/// - `frames` (default elsewhere, and the fallback): frames over IPC, drawn by the UI in a
///   canvas.
///
/// `SLOPSHOP_PRESENTER=frames|window` overrides the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PresenterMode {
    Frames,
    Window,
}

impl PresenterMode {
    fn from_env() -> Self {
        Self::requested(std::env::var("SLOPSHOP_PRESENTER").ok().as_deref())
    }

    fn requested(value: Option<&str>) -> Self {
        match value {
            Some("frames") => Self::Frames,
            Some("window") => Self::Window,
            _ if cfg!(windows) => Self::Window,
            _ => Self::Frames,
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Frames => "frames",
            Self::Window => "window",
        }
    }
}

/// The main window's label (the only window).
const MAIN_WINDOW: &str = "main";
/// Surface clear color around the canvas area: the pasteboard, sRGB-encoded (`--pasteboard`).
const PASTEBOARD_SRGB: [f64; 4] = [32.0 / 255.0, 32.0 / 255.0, 35.0 / 255.0, 1.0];

/// Events about opens, whoever started them (the UI, or the startup file).
const EVENT_OPEN_STARTED: &str = "open-started";
const EVENT_OPEN_FINISHED: &str = "open-finished";
const EVENT_OPEN_FAILED: &str = "open-failed";
/// The user asked to close the window while some documents have unsaved changes: the UI asks
/// them, then calls `quit`.
const EVENT_CLOSE_REQUESTED: &str = "close-requested";

/// Error returned when a request targets a document that is not open (closed tab). The UI
/// recognizes it and resyncs silently.
const DOCUMENT_CLOSED: &str = "document-closed";

/// How many recent open failures are kept for a UI that missed their events.
const KEPT_FAILURES: usize = 8;

/// How long closing the app waits for cancelled exports to end (and remove their temporary
/// files) before quitting anyway.
const QUIT_EXPORT_TIMEOUT: Duration = Duration::from_secs(5);

/// Default size of a new blank document.
const NEW_DOCUMENT_SIZE: Size = Size::new(6000, 4000);

/// An open document (one tab): content, history, identity and view.
struct OpenDocument {
    session: Session,
    meta: DocumentMeta,
    /// View state (zoom, pan, fit). Not document state: never part of the undo history.
    viewport: Viewport,
    /// How each imported layer's source file was interpreted. Keyed by layer so that the
    /// document's warnings follow its layers through undo, redo and deletion.
    layer_warnings: HashMap<LayerId, Vec<&'static str>>,
    /// The `.slop` file of the document, if it was opened from or saved to one. Taken out while
    /// a save runs.
    file: Option<SlopFile>,
    /// Path of that file (kept while the file is taken out for a save).
    path: Option<PathBuf>,
    /// Revision of the document when it was opened, created or last saved.
    saved_revision: u64,
    /// A save of this document is running.
    saving: bool,
}

impl OpenDocument {
    fn new(session: Session, meta: DocumentMeta, output: Size) -> Self {
        let viewport = Viewport::new(session.document().size(), output);
        let saved_revision = session.document().revision();
        Self {
            session,
            meta,
            viewport,
            layer_warnings: HashMap::new(),
            file: None,
            path: None,
            saved_revision,
            saving: false,
        }
    }

    /// Changed since it was opened, created or last saved.
    fn dirty(&self) -> bool {
        self.session.document().revision() != self.saved_revision
    }

    /// Warnings of the layers currently in the document, without duplicates.
    fn warnings(&self) -> Vec<&'static str> {
        let mut warnings = Vec::new();
        for layer in self.session.document().layers() {
            for &warning in self.layer_warnings.get(&layer.id).into_iter().flatten() {
                if !warnings.contains(&warning) {
                    warnings.push(warning);
                }
            }
        }
        warnings
    }

    fn view(&self) -> DocumentView {
        let mut view = DocumentView::new(&self.session, &self.meta, self.warnings());
        view.path = self.path.as_ref().map(|p| p.display().to_string());
        view.dirty = self.dirty();
        view
    }
}

/// Open documents in tab order.
#[derive(Default)]
struct Documents {
    tabs: Vec<OpenDocument>,
    /// Size of the viewport last rendered, used to fit newly opened documents.
    output: Option<Size>,
}

impl Documents {
    fn contains(&self, id: u64) -> bool {
        self.tabs.iter().any(|d| d.meta.id == id)
    }

    fn get_mut(&mut self, id: u64) -> Result<&mut OpenDocument, String> {
        self.tabs
            .iter_mut()
            .find(|d| d.meta.id == id)
            .ok_or_else(|| DOCUMENT_CLOSED.to_owned())
    }

    /// Move a tab to `index` among the others (clamped): the tab order is the document order.
    fn move_tab(&mut self, id: u64, index: usize) -> Result<(), String> {
        let from = self
            .tabs
            .iter()
            .position(|d| d.meta.id == id)
            .ok_or_else(|| DOCUMENT_CLOSED.to_owned())?;
        let document = self.tabs.remove(from);
        let index = index.min(self.tabs.len());
        self.tabs.insert(index, document);
        Ok(())
    }

    /// Copy every layer of `source` on top of `target`, as one undoable edit of `target`, with
    /// their import warnings. Pixels are shared, never copied.
    fn copy_layers(&mut self, source: u64, target: u64) -> Result<DocumentView, String> {
        if source == target {
            return Err("a document cannot be copied into itself".to_owned());
        }
        let from = self.get_mut(source)?;
        let layers = from.session.document().layers().to_vec();
        let warnings: Vec<Option<Vec<&'static str>>> = layers
            .iter()
            .map(|layer| from.layer_warnings.get(&layer.id).cloned())
            .collect();
        let to = self.get_mut(target)?;
        let ids = to
            .session
            .insert_layer_copies(&layers)
            .map_err(|e| e.to_string())?;
        for (id, warnings) in ids.into_iter().zip(warnings) {
            if let Some(warnings) = warnings {
                to.layer_warnings.insert(id, warnings);
            }
        }
        Ok(to.view())
    }
}

/// An open in progress (decoding a large file takes seconds).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Opening {
    id: u64,
    name: String,
    target: OpenTarget,
}

/// Where an opened image goes.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum OpenTarget {
    /// A new document, in a new tab.
    NewTab,
    /// A new top layer of an existing document (undoable).
    Layer { document_id: u64 },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OpenFinished {
    id: u64,
    target: OpenTarget,
    document: DocumentView,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OpenFailed {
    id: u64,
    name: String,
    /// Stable identifier translated by the UI (`open.error.<code>`), e.g. `notYetSupported`.
    code: &'static str,
    /// Technical detail (format name, decoder message), shown inside the translated message.
    detail: String,
}

/// Code of an open whose target tab was closed during the decode (not a user-facing error).
const CODE_DOCUMENT_CLOSED: &str = "documentClosed";

struct AppState {
    documents: Mutex<Documents>,
    /// The user agreed to quit despite unsaved changes.
    quit_confirmed: AtomicBool,
    /// Created lazily, off the UI thread (adapter/device creation can take a while).
    renderer: OnceLock<Result<Renderer, String>>,
    next_document_id: AtomicU64,
    next_open_id: AtomicU64,
    openings: Mutex<Vec<Opening>>,
    /// Recent failures (newest last), for a UI that subscribed to events too late.
    failures: Mutex<Vec<OpenFailed>>,
    /// Presentation asked for at startup.
    requested_presenter: PresenterMode,
    /// Presentation in use, decided once (see [`AppState::presenter_mode`]).
    presenter_mode: OnceLock<PresenterMode>,
    /// Window surface (native presentation only).
    presenter: Mutex<Option<Presenter>>,
    /// Client area of the main window in physical pixels, kept by window events.
    surface_size: Mutex<Option<Size>>,
    /// Exports running (see the `export` module).
    exports: ExportJobs,
}

impl AppState {
    fn new() -> Self {
        Self {
            documents: Mutex::new(Documents::default()),
            quit_confirmed: AtomicBool::new(false),
            renderer: OnceLock::new(),
            next_document_id: AtomicU64::new(1),
            next_open_id: AtomicU64::new(1),
            openings: Mutex::new(Vec::new()),
            failures: Mutex::new(Vec::new()),
            requested_presenter: PresenterMode::from_env(),
            presenter_mode: OnceLock::new(),
            presenter: Mutex::new(None),
            surface_size: Mutex::new(None),
            exports: ExportJobs::default(),
        }
    }

    /// Some open document has changes that are not saved, or is being saved.
    fn has_unsaved(&self) -> bool {
        // A poisoned lock: nothing reliable to save, let the window close.
        self.documents()
            .is_ok_and(|documents| documents.tabs.iter().any(|d| d.dirty() || d.saving))
    }

    fn documents(&self) -> Result<MutexGuard<'_, Documents>, String> {
        self.documents
            .lock()
            .map_err(|_| "document state is poisoned".to_owned())
    }

    /// Blocking: call from a worker thread.
    fn renderer(&self) -> Result<&Renderer, String> {
        self.renderer
            .get_or_init(|| Renderer::new().map_err(|e| e.to_string()))
            .as_ref()
            .map_err(Clone::clone)
    }

    /// Add a document in a new tab (fitted in the current viewport size). `warnings` apply to
    /// its initial layers.
    fn add_document(
        &self,
        session: Session,
        name: Option<String>,
        warnings: Vec<&'static str>,
    ) -> Result<DocumentView, String> {
        self.add_document_from(session, name, warnings, None)
    }

    /// [`Self::add_document`], for a document read from `file`.
    fn add_document_from(
        &self,
        session: Session,
        name: Option<String>,
        warnings: Vec<&'static str>,
        file: Option<SlopFile>,
    ) -> Result<DocumentView, String> {
        let meta = DocumentMeta {
            id: self.next_document_id.fetch_add(1, Ordering::Relaxed),
            name,
        };
        let mut documents = self.documents()?;
        let output = documents.output.unwrap_or(Size::new(1, 1));
        let mut document = OpenDocument::new(session, meta, output);
        document.path = file.as_ref().map(|f| f.path().to_owned());
        document.file = file;
        if !warnings.is_empty() {
            let ids: Vec<LayerId> = document
                .session
                .document()
                .layers()
                .iter()
                .map(|l| l.id)
                .collect();
            for id in ids {
                document.layer_warnings.insert(id, warnings.clone());
            }
        }
        let view = document.view();
        documents.tabs.push(document);
        Ok(view)
    }
}

/// Initial content applied directly to the document, so that it is not part of the history.
fn session_with_layer(size: Size, name: &str, content: LayerContent) -> Session {
    let mut doc = Document::new(size);
    let id = doc.allocate_layer_id();
    let layer = Edit::InsertLayer {
        index: 0,
        layer: Layer {
            id,
            name: name.to_owned(),
            visible: true,
            opacity: 1.0,
            content,
        },
    };
    // Invariant: a fresh id at index 0 of an empty document with valid content is accepted.
    layer.apply(&mut doc).expect("initial layer is valid");
    Session::new(doc)
}

/// A white canvas.
fn blank_session() -> Session {
    let white = LayerContent::Fill {
        color: LinearRgba::new(1.0, 1.0, 1.0, 1.0),
    };
    session_with_layer(NEW_DOCUMENT_SIZE, "Background", white)
}

fn image_session(image: RasterImage, name: &str) -> Session {
    let size = image.size();
    let content = LayerContent::Raster {
        image: Arc::new(image),
    };
    session_with_layer(size, name, content)
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn layer_name(path: &Path) -> String {
    path.file_stem()
        .map_or_else(|| file_name(path), |s| s.to_string_lossy().into_owned())
}

/// Removes an open from the "in progress" list when dropped: explicitly before its outcome is
/// announced, or on unwinding if the open panics.
struct OpeningGuard<'a> {
    state: &'a AppState,
    id: u64,
}

impl Drop for OpeningGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut openings) = self.state.openings.lock() {
            openings.retain(|o| o.id != self.id);
        }
    }
}

/// Puts the results of opens decoded in parallel into their target in the order they were
/// asked for (the order of the files chosen or dropped), whatever order the decodes end in.
#[derive(Debug, Default)]
struct InsertionOrder {
    /// Index of the next open allowed to insert its result.
    next: Mutex<usize>,
    turn_changed: Condvar,
}

impl InsertionOrder {
    fn wait_for(&self, index: usize) -> MutexGuard<'_, usize> {
        // The counter stays consistent even if a holder panicked: keep going.
        let mut next = self.next.lock().unwrap_or_else(|e| e.into_inner());
        while *next < index {
            next = self
                .turn_changed
                .wait(next)
                .unwrap_or_else(|e| e.into_inner());
        }
        next
    }
}

/// The place of one open in an [`InsertionOrder`]. Dropping it (after the insertion, or after a
/// failure, even while unwinding) waits for the earlier opens and lets the next one go, so that
/// a failed open never holds up the others.
struct Turn<'a> {
    order: &'a InsertionOrder,
    index: usize,
}

impl Turn<'_> {
    /// Block until every earlier open has inserted its result or failed.
    fn wait(&self) {
        drop(self.order.wait_for(self.index));
    }
}

impl Drop for Turn<'_> {
    fn drop(&mut self) {
        let mut next = self.order.wait_for(self.index);
        *next = self.index + 1;
        drop(next);
        self.order.turn_changed.notify_all();
    }
}

/// Decode `path` and put it into `target`, after the earlier opens of `turn`'s batch if any.
/// Blocking and heavy: worker threads only. Reports progress and outcome through events,
/// whoever started the open.
fn open_path(
    app: &AppHandle,
    path: &Path,
    target: OpenTarget,
    turn: Option<&Turn<'_>>,
) -> Result<DocumentView, String> {
    let state = app.state::<AppState>();
    let id = state.next_open_id.fetch_add(1, Ordering::Relaxed);
    let name = file_name(path);
    // SlopShop documents are recognized by their content and always open in a new tab.
    let is_document = slopshop_io::slop::is_slop_file(path).unwrap_or(false);
    let target = if is_document {
        OpenTarget::NewTab
    } else {
        target
    };
    let opening = Opening {
        id,
        name: name.clone(),
        target,
    };
    if let Ok(mut openings) = state.openings.lock() {
        openings.push(opening.clone());
    }
    let guard = OpeningGuard { state: &state, id };
    emit(app, EVENT_OPEN_STARTED, &opening);

    // Don't spend seconds decoding for a tab that is already closed.
    let target_gone = match target {
        OpenTarget::Layer { document_id } => !state.documents()?.contains(document_id),
        OpenTarget::NewTab => false,
    };
    // Decoders parse untrusted files: a panic in one must still end this open with a failure
    // event (the UI shows a pending tab until then), and must not stop other opens.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if is_document {
            let (document, file) = SlopFile::open(path).map_err(|e| (e.code(), e.to_string()))?;
            if let Some(turn) = turn {
                turn.wait();
            }
            return state
                .add_document_from(
                    Session::new(document),
                    Some(name.clone()),
                    Vec::new(),
                    Some(file),
                )
                .map_err(|e| ("internal", e));
        }
        let decoded = if target_gone {
            Err((CODE_DOCUMENT_CLOSED, DOCUMENT_CLOSED.to_owned()))
        } else {
            slopshop_io::open_image(path).map_err(|e| (e.code(), e.to_string()))
        };
        decoded.and_then(|imported| {
            if let Some(turn) = turn {
                turn.wait();
            }
            insert_imported(&state, path, &name, imported, target).map_err(|e| {
                let code = if e == DOCUMENT_CLOSED {
                    CODE_DOCUMENT_CLOSED
                } else {
                    "internal"
                };
                (code, e)
            })
        })
    }))
    .unwrap_or_else(|panic| Err(("internal", panic_detail(panic.as_ref(), "decoder panicked"))));

    // Leave the "in progress" list before announcing the outcome, so that a UI catching up
    // after the event cannot see this open as still running.
    drop(guard);
    match &result {
        Ok(document) => emit(
            app,
            EVENT_OPEN_FINISHED,
            &OpenFinished {
                id,
                target,
                document: document.clone(),
            },
        ),
        Err((code, detail)) => {
            let failed = OpenFailed {
                id,
                name,
                code,
                detail: detail.clone(),
            };
            // A target tab closed by the user is not a failure worth replaying later.
            let closed = *code == CODE_DOCUMENT_CLOSED;
            if let (false, Ok(mut failures)) = (closed, state.failures.lock()) {
                failures.push(failed.clone());
                let excess = failures.len().saturating_sub(KEPT_FAILURES);
                failures.drain(..excess);
            }
            emit(app, EVENT_OPEN_FAILED, &failed);
        }
    }
    result.map_err(|(code, detail)| {
        if code == CODE_DOCUMENT_CLOSED {
            DOCUMENT_CLOSED.to_owned()
        } else {
            detail
        }
    })
}

/// Put a decoded image into its target: a new tab, or a new top layer (undoable).
fn insert_imported(
    state: &AppState,
    path: &Path,
    name: &str,
    imported: slopshop_io::Imported,
    target: OpenTarget,
) -> Result<DocumentView, String> {
    let warnings: Vec<_> = imported.warnings.iter().map(|w| w.id()).collect();
    match target {
        OpenTarget::NewTab => state.add_document(
            image_session(imported.image, &layer_name(path)),
            Some(name.to_owned()),
            warnings,
        ),
        OpenTarget::Layer { document_id } => {
            let mut documents = state.documents()?;
            // The tab may have been closed during the decode.
            let document = documents.get_mut(document_id)?;
            let session = &mut document.session;
            let layer_id = session.allocate_layer_id();
            let edit = Edit::InsertLayer {
                index: session.document().layers().len(),
                layer: Layer {
                    id: layer_id,
                    name: layer_name(path),
                    visible: true,
                    opacity: 1.0,
                    content: LayerContent::Raster {
                        image: Arc::new(imported.image),
                    },
                },
            };
            session.perform(edit).map_err(|e| e.to_string())?;
            if !warnings.is_empty() {
                document.layer_warnings.insert(layer_id, warnings);
            }
            Ok(document.view())
        }
    }
}

/// The message of a caught panic, or `fallback` when it has none.
fn panic_detail(panic: &(dyn std::any::Any + Send), fallback: &str) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| fallback.to_owned())
}

fn emit<T: Serialize + Clone>(app: &AppHandle, event: &str, payload: &T) {
    if let Err(e) = app.emit(event, payload.clone()) {
        eprintln!("cannot emit {event}: {e}");
    }
}

/// Files to open at startup: paths given on the command line, or in dev builds the test image
/// (`SLOPSHOP_OPEN`, else `out/default.jpg` at the repository root) so that every restart
/// shows real pixels without clicking through a dialog.
fn startup_files() -> Vec<PathBuf> {
    let from_args: Vec<PathBuf> = std::env::args_os()
        .skip(1)
        .map(PathBuf::from)
        .filter(|p| p.is_file())
        .collect();
    if !from_args.is_empty() || !cfg!(debug_assertions) {
        return from_args;
    }
    std::env::var_os("SLOPSHOP_OPEN")
        .map(PathBuf::from)
        .or_else(|| Some(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/default.jpg")))
        .filter(|p| p.is_file())
        .into_iter()
        .collect()
}

/// All open documents, in tab order.
#[tauri::command]
async fn documents(state: State<'_, AppState>) -> Result<Vec<DocumentView>, String> {
    Ok(state
        .documents()?
        .tabs
        .iter()
        .map(OpenDocument::view)
        .collect())
}

#[tauri::command]
async fn document(state: State<'_, AppState>, document_id: u64) -> Result<DocumentView, String> {
    Ok(state.documents()?.get_mut(document_id)?.view())
}

/// A new blank document in a new tab.
#[tauri::command]
async fn new_document(state: State<'_, AppState>) -> Result<DocumentView, String> {
    state.add_document(blank_session(), None, Vec::new())
}

#[tauri::command]
async fn close_document(state: State<'_, AppState>, document_id: u64) -> Result<(), String> {
    state.documents()?.tabs.retain(|d| d.meta.id != document_id);
    Ok(())
}

/// Rename a document (its tab). The file on disk, if any, keeps its name.
#[tauri::command]
async fn rename_document(
    state: State<'_, AppState>,
    document_id: u64,
    name: String,
) -> Result<DocumentView, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("a document name cannot be empty".to_owned());
    }
    let mut documents = state.documents()?;
    let document = documents.get_mut(document_id)?;
    document.meta.name = Some(name.to_owned());
    Ok(document.view())
}

/// Move a tab to `index` among the other tabs.
#[tauri::command]
async fn move_document(
    state: State<'_, AppState>,
    document_id: u64,
    index: usize,
) -> Result<(), String> {
    state.documents()?.move_tab(document_id, index)
}

/// Copy every layer of `source_id` on top of `target_id` (e.g. a tab dropped on the canvas),
/// as one undoable edit of the target.
#[tauri::command]
async fn copy_layers(
    state: State<'_, AppState>,
    source_id: u64,
    target_id: u64,
) -> Result<DocumentView, String> {
    state.documents()?.copy_layers(source_id, target_id)
}

/// Opens in progress.
#[tauri::command]
async fn openings(state: State<'_, AppState>) -> Result<Vec<Opening>, String> {
    state
        .openings
        .lock()
        .map(|o| o.clone())
        .map_err(|_| "open state is poisoned".to_owned())
}

/// Recent open failures, for a UI that missed their events.
#[tauri::command]
async fn open_failures(state: State<'_, AppState>) -> Result<Vec<OpenFailed>, String> {
    state
        .failures
        .lock()
        .map(|f| f.clone())
        .map_err(|_| "open state is poisoned".to_owned())
}

/// Open images, decoded in parallel: each in a new tab, or each as a new top layer of
/// `document_id` (undoable, one edit per image). Tabs and layers come in the order of `paths`.
/// Every outcome arrives as an `open-*` event; this resolves once all are done.
#[tauri::command]
async fn open_images(
    app: AppHandle,
    paths: Vec<PathBuf>,
    document_id: Option<u64>,
) -> Result<(), String> {
    let target = match document_id {
        Some(document_id) => OpenTarget::Layer { document_id },
        None => OpenTarget::NewTab,
    };
    let order = Arc::new(InsertionOrder::default());
    let opens: Vec<_> = paths
        .into_iter()
        .enumerate()
        .map(|(index, path)| {
            let (app, order) = (app.clone(), order.clone());
            tauri::async_runtime::spawn_blocking(move || {
                let turn = Turn {
                    order: &order,
                    index,
                };
                // The outcome is reported by the open's own events.
                let _ = open_path(&app, &path, target, Some(&turn));
            })
        })
        .collect();
    for open in opens {
        open.await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Save a document to its `.slop` file (incrementally), or to `path` (a compact new file, which
/// the document continues with). Runs on a worker; the document can be edited meanwhile: what
/// was saved is the document as it was when the save started.
#[tauri::command]
async fn save_document(
    app: AppHandle,
    document_id: u64,
    path: Option<PathBuf>,
) -> Result<DocumentView, SaveFailed> {
    tauri::async_runtime::spawn_blocking(move || save_to(&app, document_id, path))
        .await
        .map_err(|e| SaveFailed {
            code: "internal",
            detail: e.to_string(),
        })?
}

fn save_to(
    app: &AppHandle,
    document_id: u64,
    path: Option<PathBuf>,
) -> Result<DocumentView, SaveFailed> {
    let failed = |code: &'static str, detail: String| SaveFailed { code, detail };
    let closed = |e: String| {
        let code = if e == DOCUMENT_CLOSED {
            "documentClosed"
        } else {
            "internal"
        };
        failed(code, e)
    };
    let state = app.state::<AppState>();
    let (snapshot, file) = {
        let mut documents = state.documents().map_err(closed)?;
        let document = documents.get_mut(document_id).map_err(closed)?;
        if document.saving {
            return Err(failed("busy", String::new()));
        }
        if path.is_none() && document.file.is_none() {
            return Err(failed("internal", "no file to save to".to_owned()));
        }
        document.session.end_gesture();
        document.saving = true;
        (document.session.document().clone(), document.file.take())
    };

    // The file handle stays valid when a save fails: it describes the last committed save.
    let (outcome, file) = match (file, &path) {
        (Some(mut file), None) => (file.save(&snapshot).map(|_| ()), Some(file)),
        (Some(mut file), Some(path)) => (file.save_as(path, &snapshot).map(|_| ()), Some(file)),
        (None, Some(path)) => match SlopFile::create(path, &snapshot) {
            Ok(file) => (Ok(()), Some(file)),
            Err(e) => (Err(e), None),
        },
        (None, None) => unreachable!("checked above"),
    };

    let mut documents = state.documents().map_err(closed)?;
    let document = documents.get_mut(document_id).map_err(closed)?;
    document.saving = false;
    document.file = file;
    outcome.map_err(|e| failed(e.code(), e.to_string()))?;
    if let Some(file) = &document.file {
        document.path = Some(file.path().to_owned());
        document.meta.name = Some(file_name(file.path()));
    }
    document.saved_revision = snapshot.revision();
    Ok(document.view())
}

/// Quit even with unsaved changes (the UI asked the user after `close-requested`).
#[tauri::command]
async fn quit(app: AppHandle) -> Result<(), String> {
    app.state::<AppState>()
        .quit_confirmed
        .store(true, Ordering::Relaxed);
    let window = app
        .get_webview_window(MAIN_WINDOW)
        .ok_or("no main window")?;
    window.close().map_err(|e| e.to_string())
}

/// Show a file in the system's file manager, selected (e.g. an exported file).
#[tauri::command]
async fn reveal_in_folder(app: AppHandle, path: PathBuf) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.opener()
            .reveal_item_in_dir(&path)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn perform(
    state: State<'_, AppState>,
    document_id: u64,
    edit: EditRequest,
) -> Result<DocumentView, String> {
    let mut documents = state.documents()?;
    let document = documents.get_mut(document_id)?;
    let edit = edit.into_edit(&mut document.session);
    document.session.perform(edit).map_err(|e| e.to_string())?;
    Ok(document.view())
}

/// Apply an edit live, as part of a continuous gesture (slider drag, …). The gesture becomes a
/// single undo entry when `end_gesture` is called (or when any other edit/undo happens).
#[tauri::command]
async fn perform_live(
    state: State<'_, AppState>,
    document_id: u64,
    edit: EditRequest,
) -> Result<DocumentView, String> {
    let mut documents = state.documents()?;
    let document = documents.get_mut(document_id)?;
    let edit = edit.into_edit(&mut document.session);
    document
        .session
        .perform_in_gesture(edit)
        .map_err(|e| e.to_string())?;
    Ok(document.view())
}

#[tauri::command]
async fn end_gesture(state: State<'_, AppState>, document_id: u64) -> Result<DocumentView, String> {
    let mut documents = state.documents()?;
    let document = documents.get_mut(document_id)?;
    document.session.end_gesture();
    Ok(document.view())
}

#[tauri::command]
async fn undo(state: State<'_, AppState>, document_id: u64) -> Result<DocumentView, String> {
    let mut documents = state.documents()?;
    let document = documents.get_mut(document_id)?;
    document.session.undo().map_err(|e| e.to_string())?;
    Ok(document.view())
}

#[tauri::command]
async fn redo(state: State<'_, AppState>, document_id: u64) -> Result<DocumentView, String> {
    let mut documents = state.documents()?;
    let document = documents.get_mut(document_id)?;
    document.session.redo().map_err(|e| e.to_string())?;
    Ok(document.view())
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

/// Change a document's view (zoom, pan, fit). The UI requests a new frame afterwards.
#[tauri::command]
async fn view(
    state: State<'_, AppState>,
    document_id: u64,
    request: ViewRequest,
) -> Result<ViewInfo, String> {
    let mut documents = state.documents()?;
    let document = documents.get_mut(document_id)?;
    let size = document.session.document().size();
    request.apply(&mut document.viewport, size);
    Ok(ViewInfo::new(&document.viewport))
}

/// Render a document's view into a `width × height` (device pixels) viewport. Returns a binary
/// frame (see [`FrameHeader`]): an `ArrayBuffer` on the JS side, viewport-sized, never
/// document-sized, and never JSON.
#[tauri::command]
async fn render_view(
    app: AppHandle,
    document_id: u64,
    width: u32,
    height: u32,
) -> Result<Response, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let output = Size::new(width, height);
        let renderer = state.renderer()?;
        // Reject empty or oversized requests before allocating or touching the view state.
        let pixel_bytes = renderer
            .output_byte_len(output)
            .map_err(|e| e.to_string())
            .and_then(|len| usize::try_from(len).map_err(|e| e.to_string()))?;

        // Snapshot the document and its view together, then release the lock while the GPU
        // works. Cheap: raster pixels are shared, never copied.
        let (doc, viewport) = {
            let mut documents = state.documents()?;
            documents.output = Some(output);
            let document = documents.get_mut(document_id)?;
            let doc = document.session.document().clone();
            if document.viewport.output() != output {
                document.viewport.resize(doc.size(), output);
            }
            (doc, document.viewport)
        };

        let mut bytes = Vec::with_capacity(FRAME_HEADER_LEN + pixel_bytes);
        // Reserve the header, append the pixels in place (no extra frame copy), then fill the
        // header once the render time is known.
        bytes.resize(FRAME_HEADER_LEN, 0);
        let start = Instant::now();
        renderer
            .render_view_into(&doc, viewport.transform(), output, &mut bytes)
            .map_err(|e| e.to_string())?;
        let header = FrameHeader {
            size: output,
            fit: viewport.is_fit(),
            revision: doc.revision(),
            zoom: viewport.zoom(),
            render_ms: start.elapsed().as_secs_f32() * 1000.0,
            document_id,
            origin: viewport.transform().origin,
        };
        bytes[..FRAME_HEADER_LEN].copy_from_slice(&header.to_bytes());
        Ok(Response::new(bytes))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// How the viewport is presented: `frames` or `window` (see [`PresenterMode`]). The first call
/// decides, creating the window surface if needed (so it waits for the GPU).
#[tauri::command]
async fn presenter_mode(app: AppHandle) -> Result<&'static str, String> {
    tauri::async_runtime::spawn_blocking(move || app.state::<AppState>().presenter_mode(&app).id())
        .await
        .map_err(|e| e.to_string())
}

/// Present the document's view directly to the window (native presentation): into the
/// `width × height` canvas area at (`x`, `y`), in physical pixels of the window's client area.
/// Nothing crosses the IPC but this small request and its answer.
#[tauri::command]
async fn present_view(
    app: AppHandle,
    document_id: u64,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) -> Result<PresentInfo, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        if state.presenter_mode.get() != Some(&PresenterMode::Window) {
            return Err("native presentation is not enabled".to_owned());
        }
        let renderer = state.renderer()?;
        let output = Size::new(width, height);
        let (doc, viewport) = {
            let mut documents = state.documents()?;
            documents.output = Some(output);
            let document = documents.get_mut(document_id)?;
            let doc = document.session.document().clone();
            if !output.is_empty() && document.viewport.output() != output {
                document.viewport.resize(doc.size(), output);
            }
            (doc, document.viewport)
        };
        let surface_size = state.surface_size(&app)?;

        let mut guard = state
            .presenter
            .lock()
            .map_err(|_| "presenter lock poisoned".to_owned())?;
        let presenter = guard.as_mut().ok_or("no window surface")?;
        let start = Instant::now();
        let presented = if output.is_empty() {
            Presented::Skipped
        } else {
            renderer
                .present_view(
                    presenter,
                    &doc,
                    viewport.transform(),
                    Rect::new(x, y, width, height),
                    surface_size,
                    PASTEBOARD_SRGB,
                )
                .map_err(|e| e.to_string())?
        };
        Ok(PresentInfo {
            presented: presented == Presented::Frame,
            revision: doc.revision(),
            zoom: viewport.zoom(),
            fit: viewport.is_fit(),
            render_ms: start.elapsed().as_secs_f32() * 1000.0,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

impl AppState {
    /// The presentation in use. Decided on the first call: native presentation falls back to
    /// frames if the window surface cannot be created (reported on stderr).
    fn presenter_mode(&self, app: &AppHandle) -> PresenterMode {
        *self.presenter_mode.get_or_init(|| {
            if self.requested_presenter == PresenterMode::Frames {
                return PresenterMode::Frames;
            }
            match self.create_presenter(app) {
                Ok(()) => PresenterMode::Window,
                Err(e) => {
                    eprintln!("native presentation unavailable, using frames: {e}");
                    // The page draws the canvas itself again: no need to see through it.
                    if let Some(window) = app.get_webview_window(MAIN_WINDOW)
                        && let Err(e) = window.set_background_color(None)
                    {
                        eprintln!("cannot restore the webview background: {e}");
                    }
                    PresenterMode::Frames
                }
            }
        })
    }

    fn create_presenter(&self, app: &AppHandle) -> Result<(), String> {
        let window = app
            .get_webview_window(MAIN_WINDOW)
            .ok_or("main window not found")?;
        let presenter = self
            .renderer()?
            .create_presenter(window)
            .map_err(|e| e.to_string())?;
        *self
            .presenter
            .lock()
            .map_err(|_| "presenter lock poisoned".to_owned())? = Some(presenter);
        Ok(())
    }

    /// Client size of the main window, as last reported by window events.
    fn surface_size(&self, app: &AppHandle) -> Result<Size, String> {
        let mut size = self
            .surface_size
            .lock()
            .map_err(|_| "surface size lock poisoned".to_owned())?;
        if let Some(size) = *size {
            return Ok(size);
        }
        // No resize event yet: ask the window once.
        let window = app
            .get_webview_window(MAIN_WINDOW)
            .ok_or("main window not found")?;
        let inner = window.inner_size().map_err(|e| e.to_string())?;
        Ok(*size.insert(Size::new(inner.width, inner.height)))
    }

    fn set_surface_size(&self, width: u32, height: u32) {
        if let Ok(mut size) = self.surface_size.lock() {
            *size = Some(Size::new(width, height));
        }
    }
}

/// Handle a close request of the main window. While exports run, the close waits: they are
/// cancelled, and the window closes once they have ended (removing their temporary files), or
/// after [`QUIT_EXPORT_TIMEOUT`] anyway. The waiting happens on a worker thread, not on the main
/// thread, which keeps processing events meanwhile.
fn close_after_exports(window: &tauri::Window, api: &tauri::CloseRequestApi) {
    let exports = &window.state::<AppState>().exports;
    if exports.is_idle() {
        return;
    }
    api.prevent_close();
    if !exports.stop_all() {
        // Already closing: the worker below closes the window.
        return;
    }
    let window = window.clone();
    std::thread::spawn(move || {
        if !window
            .state::<AppState>()
            .exports
            .wait_idle(QUIT_EXPORT_TIMEOUT)
        {
            eprintln!("exports still running after {QUIT_EXPORT_TIMEOUT:?}: quitting anyway");
        }
        // Without a new close request, which would wait again.
        if let Err(e) = window.destroy() {
            eprintln!("cannot close the main window: {e}");
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::new())
        .setup(|app| {
            if app.state::<AppState>().requested_presenter == PresenterMode::Window {
                // The engine draws the canvas area under the webview: let it show through.
                if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
                    window.set_background_color(Some(tauri::window::Color(0, 0, 0, 0)))?;
                }
            }
            // Warm up the GPU in the background so the first frame is fast, then open the
            // startup files, if any (decoding a large image takes seconds).
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                if let Err(e) = handle.state::<AppState>().renderer() {
                    eprintln!("GPU initialization failed: {e}");
                }
                for path in startup_files() {
                    let start = Instant::now();
                    match open_path(&handle, &path, OpenTarget::NewTab, None) {
                        Ok(_) => eprintln!(
                            "opened {} in {:.1} s",
                            path.display(),
                            start.elapsed().as_secs_f32()
                        ),
                        Err(e) => eprintln!("cannot open {}: {e}", path.display()),
                    }
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() != MAIN_WINDOW {
                return;
            }
            let size = match event {
                tauri::WindowEvent::Resized(size) => *size,
                tauri::WindowEvent::ScaleFactorChanged { new_inner_size, .. } => *new_inner_size,
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    // Unsaved changes: the UI asks, then calls `quit` if the user agrees.
                    let state = window.state::<AppState>();
                    if !state.quit_confirmed.load(Ordering::Relaxed) && state.has_unsaved() {
                        api.prevent_close();
                        emit(window.app_handle(), EVENT_CLOSE_REQUESTED, &());
                        return;
                    }
                    close_after_exports(window, api);
                    return;
                }
                _ => return,
            };
            window
                .state::<AppState>()
                .set_surface_size(size.width, size.height);
        })
        .invoke_handler(tauri::generate_handler![
            documents,
            document,
            new_document,
            close_document,
            move_document,
            copy_layers,
            rename_document,
            openings,
            open_failures,
            open_images,
            perform,
            perform_live,
            end_gesture,
            undo,
            redo,
            gpu_info,
            view,
            render_view,
            presenter_mode,
            present_view,
            reveal_in_folder,
            save_document,
            quit,
            export::export_defaults,
            export::export_spaces,
            export::export_max_side,
            export::export_document,
            export::cancel_export
        ])
        .run(tauri::generate_context!())
        .expect("error while running SlopShop");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_insert_in_the_order_asked_whatever_order_they_end_in() {
        let order = InsertionOrder::default();
        let inserted = Mutex::new(Vec::new());
        std::thread::scope(|scope| {
            for index in 0..6 {
                let (order, inserted) = (&order, &inserted);
                scope.spawn(move || {
                    let turn = Turn { order, index };
                    // Later files decode faster; file 3 fails and inserts nothing.
                    std::thread::sleep(Duration::from_millis(60 - 10 * index as u64));
                    if index != 3 {
                        turn.wait();
                        inserted.lock().unwrap().push(index);
                    }
                });
            }
        });
        assert_eq!(inserted.into_inner().unwrap(), [0, 1, 2, 4, 5]);
    }

    #[test]
    fn a_panicking_open_lets_the_next_ones_insert() {
        let order = InsertionOrder::default();
        std::thread::scope(|scope| {
            let order = &order;
            let failed = scope.spawn(move || {
                let _turn = Turn { order, index: 0 };
                panic!("decoder panicked");
            });
            let next = scope.spawn(move || {
                let turn = Turn { order, index: 1 };
                turn.wait();
            });
            assert!(failed.join().is_err());
            next.join().unwrap();
        });
        assert_eq!(*order.next.lock().unwrap(), 2);
    }

    fn documents_with(count: u64) -> Documents {
        let mut documents = Documents::default();
        for id in 1..=count {
            let meta = DocumentMeta {
                id,
                name: Some(format!("doc {id}")),
            };
            documents
                .tabs
                .push(OpenDocument::new(blank_session(), meta, Size::new(8, 8)));
        }
        documents
    }

    fn tab_ids(documents: &Documents) -> Vec<u64> {
        documents.tabs.iter().map(|d| d.meta.id).collect()
    }

    #[test]
    fn a_document_is_dirty_until_saved_again() {
        let mut documents = documents_with(1);
        let document = documents.get_mut(1).unwrap();
        assert!(
            !document.dirty() && !document.view().dirty,
            "clean when created"
        );
        let id = document.session.allocate_layer_id();
        let layer = Layer {
            id,
            name: "fill".to_owned(),
            visible: true,
            opacity: 1.0,
            content: LayerContent::Fill {
                color: LinearRgba::new(1.0, 0.0, 0.0, 1.0),
            },
        };
        let index = document.session.document().layers().len();
        document
            .session
            .perform(Edit::InsertLayer { index, layer })
            .unwrap();
        assert!(document.dirty() && document.view().dirty);
        // What a save records.
        document.saved_revision = document.session.document().revision();
        assert!(!document.dirty());
        // Undoing past the save changes the document again.
        document.session.undo().unwrap();
        assert!(document.dirty());
    }

    #[test]
    fn tabs_move_to_an_index_among_the_others() {
        let mut documents = documents_with(3);
        documents.move_tab(1, 2).unwrap();
        assert_eq!(tab_ids(&documents), [2, 3, 1]);
        documents.move_tab(1, 0).unwrap();
        assert_eq!(tab_ids(&documents), [1, 2, 3]);
        documents.move_tab(2, 99).unwrap();
        assert_eq!(tab_ids(&documents), [1, 3, 2]);
        assert_eq!(documents.move_tab(9, 0).unwrap_err(), DOCUMENT_CLOSED);
    }

    #[test]
    fn copying_a_document_adds_its_layers_as_one_undo_entry() {
        let mut documents = documents_with(2);
        let source_layers = documents.tabs[0].session.document().layers().len();
        let before = documents.tabs[1].session.document().layers().len();
        let view = documents.copy_layers(1, 2).unwrap();
        assert_eq!(view.layers.len(), before + source_layers);
        documents.tabs[1].session.undo().unwrap();
        assert_eq!(documents.tabs[1].session.document().layers().len(), before);
        assert!(documents.copy_layers(2, 2).is_err());
        assert_eq!(documents.copy_layers(9, 2).unwrap_err(), DOCUMENT_CLOSED);
    }

    #[test]
    fn presenter_mode_defaults_to_the_native_surface_on_windows_only() {
        assert_eq!(
            PresenterMode::requested(Some("frames")),
            PresenterMode::Frames
        );
        assert_eq!(
            PresenterMode::requested(Some("window")),
            PresenterMode::Window
        );
        let default = if cfg!(windows) {
            PresenterMode::Window
        } else {
            PresenterMode::Frames
        };
        assert_eq!(PresenterMode::requested(None), default);
        assert_eq!(PresenterMode::requested(Some("bogus")), default);
    }

    fn meta() -> DocumentMeta {
        DocumentMeta { id: 1, name: None }
    }

    #[test]
    fn blank_session_has_background_and_empty_history() {
        let s = blank_session();
        assert_eq!(s.document().layers().len(), 1);
        assert!(!s.can_undo());
    }

    #[test]
    fn image_session_has_one_raster_layer_and_the_image_size() {
        use slopshop_core::color::PixelFormat;
        let size = Size::new(300, 200);
        let px = vec![128u8; 300 * 200 * 4];
        let image = RasterImage::from_pixels(size, PixelFormat::RGBA8_SRGB, &px).unwrap();
        let s = image_session(image, "photo");
        assert_eq!(s.document().size(), size);
        let view = DocumentView::new(&s, &meta(), Vec::new());
        assert_eq!(view.layers.len(), 1);
        assert_eq!(view.layers[0].kind, "raster");
        assert_eq!(view.layers[0].name, "photo");
        assert!(!s.can_undo());
    }

    #[test]
    fn documents_are_tabs_with_unique_ids() {
        let state = AppState::new();
        let a = state
            .add_document(blank_session(), None, Vec::new())
            .unwrap();
        let b = state
            .add_document(
                blank_session(),
                Some("b.png".into()),
                vec!["iccProfileIgnored"],
            )
            .unwrap();
        assert_ne!(a.id, b.id);
        assert_eq!(b.warnings, ["iccProfileIgnored"]);
        let mut documents = state.documents().unwrap();
        assert_eq!(documents.tabs.len(), 2);
        documents.tabs.retain(|d| d.meta.id != a.id);
        assert_eq!(
            documents.get_mut(a.id).err().as_deref(),
            Some(DOCUMENT_CLOSED)
        );
        assert!(documents.get_mut(b.id).is_ok());
    }

    #[test]
    fn warnings_follow_the_layers_through_undo() {
        let state = AppState::new();
        let doc = state
            .add_document(blank_session(), None, vec!["iccProfileIgnored"])
            .unwrap();
        assert_eq!(doc.warnings, ["iccProfileIgnored"]);
        let mut documents = state.documents().unwrap();
        let document = documents.get_mut(doc.id).unwrap();
        let background = document.session.document().layers()[0].id;
        document
            .session
            .perform(Edit::RemoveLayer { id: background })
            .unwrap();
        assert!(
            document.view().warnings.is_empty(),
            "layer gone, warning gone"
        );
        document.session.undo().unwrap();
        assert_eq!(document.view().warnings, ["iccProfileIgnored"]);
    }

    #[test]
    fn edit_requests_round_trip_through_undo() {
        let mut s = blank_session();
        let json = r#"{"kind":"addFillLayer","name":"Pink","color":[1.0,0.5,0.8,1.0]}"#;
        let request: EditRequest = serde_json::from_str(json).unwrap();
        let edit = request.into_edit(&mut s);
        s.perform(edit).unwrap();
        let view = DocumentView::new(&s, &meta(), Vec::new());
        assert_eq!(view.layers.len(), 2);
        assert_eq!(view.layers[1].name, "Pink");
        // The swatch is converted back to sRGB for display: it must match what was sent.
        let swatch = view.layers[1].swatch;
        assert!((swatch[1] - 0.5).abs() < 1e-5, "swatch {swatch:?}");

        s.undo().unwrap();
        assert_eq!(DocumentView::new(&s, &meta(), Vec::new()).layers.len(), 1);
    }

    #[test]
    fn view_requests_deserialize_and_apply() {
        let doc = Size::new(4000, 3000);
        let mut viewport = Viewport::new(doc, Size::new(800, 600));
        for json in [
            r#"{"kind":"setZoom","zoom":1.0}"#,
            r#"{"kind":"pan","dx":10.0,"dy":-5.0}"#,
            r#"{"kind":"zoomBy","factor":2.0,"x":100.0,"y":50.0}"#,
            r#"{"kind":"step","zoomIn":false,"x":null,"y":null}"#,
        ] {
            let request: ViewRequest = serde_json::from_str(json).unwrap();
            request.apply(&mut viewport, doc);
        }
        assert!(!viewport.is_fit());
        assert_eq!(viewport.zoom(), 1.0, "100% x2 = 200%, one step out = 100%");

        let fit: ViewRequest = serde_json::from_str(r#"{"kind":"fit"}"#).unwrap();
        fit.apply(&mut viewport, doc);
        let info = ViewInfo::new(&viewport);
        assert!(info.fit);
        assert_eq!(info.origin, viewport.transform().origin);
    }

    #[test]
    fn open_target_serializes_for_the_ui() {
        let json = serde_json::to_string(&OpenTarget::Layer { document_id: 4 }).unwrap();
        assert_eq!(json, r#"{"kind":"layer","documentId":4}"#);
        let json = serde_json::to_string(&OpenTarget::NewTab).unwrap();
        assert_eq!(json, r#"{"kind":"newTab"}"#);
    }

    #[test]
    fn frame_header_layout() {
        let bytes = FrameHeader {
            size: Size::new(640, 480),
            fit: true,
            revision: 7,
            zoom: 0.5,
            render_ms: 1.5,
            document_id: 3,
            origin: [-12.5, 40.25],
        }
        .to_bytes();
        let u32_at = |o: usize| u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
        let f64_at = |o: usize| f64::from_le_bytes(bytes[o..o + 8].try_into().unwrap());
        assert_eq!(u32_at(0), 2);
        assert_eq!(u32_at(4), 640);
        assert_eq!(u32_at(8), 480);
        assert_eq!(u32_at(12), 1);
        assert_eq!(u64::from_le_bytes(bytes[16..24].try_into().unwrap()), 7);
        assert_eq!(f64_at(24), 0.5);
        assert_eq!(f32::from_le_bytes(bytes[32..36].try_into().unwrap()), 1.5);
        assert_eq!(u32_at(36), 3);
        assert_eq!(f64_at(40), -12.5);
        assert_eq!(f64_at(48), 40.25);
    }
}
