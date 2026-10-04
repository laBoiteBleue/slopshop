//! SlopShop desktop shell.
//!
//! A thin layer between the Svelte UI and the engine crates: it owns the open documents (one per
//! tab) and their views, translates IPC requests into engine calls and sends display frames
//! back. No image or document logic belongs here.
//!
//! Threading: every command is `async` (so it never runs on the main/UI thread) and heavy work
//! (GPU, decoding) runs in `spawn_blocking` or a worker thread.

mod acquire;
mod ai;
mod bake;
mod clipboard;
mod export;
mod info;
mod ipc;
mod move_pixels;
mod paint;
mod print;
mod recent;
mod refine;
mod segment;
mod selection;
mod vector;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use slopshop_core::color::PixelFormat;
use slopshop_core::view::Viewport;
use slopshop_core::{
    BlendMode, BlendSpace, Document, Edit, Layer, LayerContent, LayerId, LayerMask, LinearRgba,
    RasterImage, Rect, Session, Size,
};
use slopshop_io::slop::SlopFile;
use slopshop_render::present::{Presented, Presenter};
use slopshop_render::{Renderer, ViewOverlays};
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

/// Largest side of a new document, in pixels: Photoshop's largest (PSB), as Image Size accepts.
pub(crate) const NEW_DOCUMENT_MAX_SIDE: u32 = 300_000;

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
    /// The file the document was opened from (an image or a `.slop`), for Document Info.
    source: Option<PathBuf>,
    /// Revision of the document when it was opened, created or last saved.
    saved_revision: u64,
    /// A save of this document is running.
    saving: bool,
    /// The selection Select > Deselect removed last, for Select > Reselect.
    last_selection: Option<slopshop_core::selection::Selection>,
    /// What the view shows over the image (Quick Mask): view state, like the viewport.
    overlays: ViewOverlays,
    /// Select and Mask is open: the selection it refines (`refine`).
    refine_base: Option<Arc<slopshop_core::RasterImage>>,
    /// A paint stroke under way, shown in place of its layer's pixels (view state too).
    paint_preview: Option<paint::PaintPreview>,
    /// Selected pixels the Move tool moved, floating until something else happens.
    floating: Option<move_pixels::Floating>,
    /// A drag of selected pixels under way, shown floating (view state too).
    move_preview: Option<move_pixels::MovePreview>,
    /// Layers whose pixels are being composited (Layer > Bake to Pixels, ADR 0031): a merge's
    /// group, shown as the layer it becomes until its pixels replace it.
    baking: std::collections::HashSet<LayerId>,
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
            source: None,
            saved_revision,
            saving: false,
            last_selection: None,
            overlays: ViewOverlays::default(),
            refine_base: None,
            paint_preview: None,
            floating: None,
            move_preview: None,
            baking: std::collections::HashSet::new(),
        }
    }

    /// The document as the view shows it, with the paint stroke or the pixels moved under way
    /// if any, and the
    /// document's revision (the preview is not a revision). Cheap: raster pixels are shared,
    /// never copied.
    fn snapshot(&self) -> (slopshop_core::Document, u64) {
        let mut doc = self.session.document().clone();
        let revision = doc.revision();
        if let Some(preview) = &self.paint_preview {
            preview.apply_to(&mut doc);
        }
        if let Some(preview) = &self.move_preview {
            preview.apply_to(&mut doc);
        }
        (doc, revision)
    }

    /// The selection Reselect would bring back: the last one deselected, while nothing is
    /// selected and it still fits the canvas.
    fn reselectable(&self) -> Option<&slopshop_core::selection::Selection> {
        let doc = self.session.document();
        self.last_selection
            .as_ref()
            .filter(|s| doc.selection().is_none() && s.image().size() == doc.size())
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
        view.can_reselect = self.reselectable().is_some();
        view.quick_mask = self.overlays.quick_mask;
        view.quick_mask_opacity = self.overlays.quick_mask_opacity;
        if !self.baking.is_empty() {
            mark_baking(&mut view.layers, &self.baking);
        }
        view
    }
}

/// Flag the layers of `views` (at any depth) being baked.
fn mark_baking(views: &mut [ipc::LayerView], baking: &std::collections::HashSet<LayerId>) {
    for view in views {
        view.baking = baking.contains(&LayerId::from_raw(view.id));
        mark_baking(&mut view.children, baking);
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

    /// Copy every layer of `source` on top of `target` (grouped when there are several), or only
    /// the layers `ids` (as they are, each group with its layers), as one undoable edit of
    /// `target`, with their import warnings. Pixels are shared, never copied.
    fn copy_layers(
        &mut self,
        source: u64,
        target: u64,
        ids: Option<&[u64]>,
    ) -> Result<DocumentView, String> {
        if source == target {
            return Err("a document cannot be copied into itself".to_owned());
        }
        let from = self.get_mut(source)?;
        if let Some(ids) = ids {
            let document = from.session.document();
            let ids: Vec<LayerId> = ids.iter().copied().map(LayerId::from_raw).collect();
            let layers: Vec<Layer> = document
                .outermost(&ids)
                .into_iter()
                .filter_map(|id| document.layer(id).cloned())
                .collect();
            if layers.is_empty() {
                return Err("no layers to copy".to_owned());
            }
            let warnings: Vec<Vec<&'static str>> = layers
                .iter()
                .flat_map(|layer| layer.subtree())
                .map(|layer| {
                    from.layer_warnings
                        .get(&layer.id)
                        .cloned()
                        .unwrap_or_default()
                })
                .collect();
            let space = document.blend_space();
            return insert_layer_copies(self.get_mut(target)?, &layers, space, warnings, None);
        }
        let document = from.session.document().clone();
        let warnings: Vec<Vec<&'static str>> = document
            .all_layers()
            .map(|layer| {
                from.layer_warnings
                    .get(&layer.id)
                    .cloned()
                    .unwrap_or_default()
            })
            .collect();
        let name = from.meta.name.clone();
        insert_document_layers(self.get_mut(target)?, &document, warnings, name)
    }
}

/// Warning of layers that come from a document blending in another space: they now blend in
/// the space of the document they joined (ADR 0012).
const WARNING_BLEND_SPACE: &str = "blendSpaceDiffers";

/// Add copies of every layer of `source` on top of `target` (one undo entry), with the import
/// warnings of each (`warnings`, in the order of `Document::all_layers`). Several top-level
/// layers arrive inside a group named `name` (the source's file or tab name), so that an
/// imported document stays one item of the stack; a single one arrives as it is.
fn insert_document_layers(
    target: &mut OpenDocument,
    source: &Document,
    warnings: Vec<Vec<&'static str>>,
    name: Option<String>,
) -> Result<DocumentView, String> {
    let group = (source.layers().len() > 1).then(|| name.unwrap_or_else(|| "Layers".to_owned()));
    insert_layer_copies(
        target,
        source.layers(),
        source.blend_space(),
        warnings,
        group,
    )
}

/// Add copies of `layers` (from a document blending in `space`) on top of `target` (one undo
/// entry), inside a new group named `group` if given, with the warnings of each copied layer
/// (`warnings`, depth first as `Layer::subtree` walks them).
fn insert_layer_copies(
    target: &mut OpenDocument,
    layers: &[Layer],
    space: BlendSpace,
    warnings: Vec<Vec<&'static str>>,
    group: Option<String>,
) -> Result<DocumentView, String> {
    let other_space = space != target.session.document().blend_space();
    let copies = target
        .session
        .insert_layer_copies(layers, group)
        .map_err(|e| e.to_string())?;
    for (id, mut warnings) in copies.ids.into_iter().zip(warnings) {
        if other_space {
            warnings.push(WARNING_BLEND_SPACE);
        }
        if !warnings.is_empty() {
            target.layer_warnings.insert(id, warnings);
        }
    }
    Ok(target.view())
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
    /// What Edit > Copy took (see the `clipboard` module).
    layer_clipboard: Mutex<Option<clipboard::Copied>>,
    /// The file the Import PDF / SVG dialog shows (see the `vector` module).
    vector: Arc<vector::VectorCache>,
    /// AI components being installed (see the `ai` module).
    ai: ai::AiState,
    /// The AI helper and what it has encoded (see the `segment` module).
    segment: segment::SegmentState,
    /// The Quick Selection stroke under way (see the `selection` module).
    quick: selection::QuickState,
    /// The paint stroke under way (see the `paint` module).
    paint: paint::PaintState,
    recent: recent::RecentFiles,
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
            layer_clipboard: Mutex::new(None),
            vector: Arc::default(),
            ai: ai::AiState::default(),
            segment: segment::SegmentState::default(),
            quick: selection::QuickState::default(),
            paint: paint::PaintState::default(),
            recent: recent::RecentFiles::default(),
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
        let per_layer = vec![warnings; session.document().layers().len()];
        self.add_document_with(session, name, per_layer, file)
    }

    /// [`Self::add_document_from`], with the warnings of each layer (in the order of
    /// `Document::all_layers`).
    fn add_document_with(
        &self,
        session: Session,
        name: Option<String>,
        warnings: Vec<Vec<&'static str>>,
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
        let ids: Vec<LayerId> = document
            .session
            .document()
            .all_layers()
            .map(|l| l.id)
            .collect();
        for (id, warnings) in ids.into_iter().zip(warnings) {
            if !warnings.is_empty() {
                document.layer_warnings.insert(id, warnings);
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
        parent: None,
        index: 0,
        layer: Layer {
            style: None,
            transform: slopshop_core::Affine::IDENTITY,
            clipped: false,
            id,
            name: name.to_owned(),
            visible: true,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            mask: None,
            content,
        },
    };
    // Invariant: a fresh id at index 0 of an empty document with valid content is accepted.
    layer.apply(&mut doc).expect("initial layer is valid");
    Session::new(doc)
}

/// A new document of `size`, as Photoshop's New dialog makes it: one layer named `layer_name`,
/// filled with `background` (sRGB-encoded) or transparent. A fill layer whatever the size, or
/// one shared transparent tile: nothing is allocated per pixel.
fn blank_session(
    size: Size,
    background: Option<[f32; 3]>,
    layer_name: &str,
) -> Result<Session, String> {
    let valid = |side: u32| (1..=NEW_DOCUMENT_MAX_SIDE).contains(&side);
    if !valid(size.width) || !valid(size.height) {
        return Err(format!(
            "a new document is 1 to {NEW_DOCUMENT_MAX_SIDE} pixels a side, not {}×{}",
            size.width, size.height
        ));
    }
    let content = match background {
        Some([r, g, b]) => LayerContent::Fill {
            color: LinearRgba::from_srgb_encoded_to_working(r, g, b, 1.0),
        },
        None => {
            let image = RasterImage::from_placed(
                size,
                PixelFormat::RGBA8_SRGB,
                Rect::new(0, 0, 0, 0),
                &[],
                &[0, 0, 0, 0],
            )
            .map_err(|e| e.to_string())?;
            LayerContent::raster(Arc::new(image))
        }
    };
    Ok(session_with_layer(size, layer_name, content))
}

fn image_session(image: RasterImage, name: &str) -> Session {
    let size = image.size();
    let content = LayerContent::Raster {
        stack: None,
        image: slopshop_core::stack::Pixels::ready(Arc::new(image)),
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

/// What an open reads, besides its path.
#[derive(Clone, Copy)]
enum Source<'a> {
    /// The file, whatever it holds.
    File,
    /// Pages of a PDF or an SVG, at a resolution: one document.
    Pages(&'a vector::VectorPages),
    /// DICOM files opened together (a series, the path being the first): one document.
    Series(&'a [PathBuf]),
}

/// Decode `path` (as `source` says) and put it into `target`, after the earlier opens of
/// `turn`'s batch if any. Blocking and heavy: worker threads only. Reports progress and outcome
/// through events, whoever started the open.
fn open_path(
    app: &AppHandle,
    path: &Path,
    source: Source<'_>,
    target: OpenTarget,
    turn: Option<&Turn<'_>>,
) -> Result<DocumentView, String> {
    let state = app.state::<AppState>();
    let id = state.next_open_id.fetch_add(1, Ordering::Relaxed);
    let (name, layer) = match source {
        Source::File => (file_name(path), layer_name(path)),
        Source::Pages(_) => (file_name(path), layer_name(path)),
        // A series is named after its folder.
        Source::Series(_) => {
            let folder = path.parent().map_or_else(|| file_name(path), file_name);
            (folder.clone(), folder)
        }
    };
    // Files of a series that could not be read, reported once the series is open.
    let mut skipped = Vec::new();
    // SlopShop documents are recognized by their content: in a new tab, or their layers added
    // on top of the target document.
    let is_document =
        matches!(source, Source::File) && slopshop_io::slop::is_slop_file(path).unwrap_or(false);
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
            if target_gone {
                return Err((CODE_DOCUMENT_CLOSED, DOCUMENT_CLOSED.to_owned()));
            }
            let (document, file) = SlopFile::open(path).map_err(|e| (e.code(), e.to_string()))?;
            if let Some(turn) = turn {
                turn.wait();
            }
            let added = match target {
                OpenTarget::NewTab => state.add_document_from(
                    Session::new(document),
                    Some(name.clone()),
                    Vec::new(),
                    Some(file),
                ),
                OpenTarget::Layer { document_id } => {
                    let warnings = vec![Vec::new(); document.all_layers().count()];
                    let group = Some(layer_name(path));
                    state.documents().and_then(|mut documents| {
                        let target = documents.get_mut(document_id)?;
                        insert_document_layers(target, &document, warnings, group)
                    })
                }
            };
            return added.map_err(|e| {
                let code = if e == DOCUMENT_CLOSED {
                    CODE_DOCUMENT_CLOSED
                } else {
                    "internal"
                };
                (code, e)
            });
        }
        let decoded = if target_gone {
            Err((CODE_DOCUMENT_CLOSED, DOCUMENT_CLOSED.to_owned()))
        } else {
            match source {
                Source::File => slopshop_io::open_file(path),
                Source::Pages(pages) => {
                    pages
                        .file
                        .open_pages(&layer, &pages.pages, pages.dpi, &pages.background)
                }
                Source::Series(paths) => {
                    slopshop_io::open_dicom_series(paths).map(|(opened, failures)| {
                        skipped = failures;
                        opened
                    })
                }
            }
            .map_err(|e| (e.code(), e.to_string()))
        };
        decoded.and_then(|opened| {
            if let Some(turn) = turn {
                turn.wait();
            }
            let inserted = match opened {
                slopshop_io::Opened::Image(imported) => {
                    insert_imported(&state, &name, &layer, imported, target)
                }
                slopshop_io::Opened::Layers(layers) => {
                    insert_layers(&state, path, &name, layers, target)
                }
            };
            inserted.map_err(|e| {
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
    for (path, error) in skipped {
        report_failure(
            app,
            &state,
            OpenFailed {
                id: state.next_open_id.fetch_add(1, Ordering::Relaxed),
                name: file_name(&path),
                code: error.code(),
                detail: error.to_string(),
            },
        );
    }
    // Document Info names the file a new tab came from.
    if let (Ok(view), OpenTarget::NewTab) = (&result, target)
        && let Ok(mut documents) = state.documents()
        && let Ok(document) = documents.get_mut(view.id)
    {
        document.source = Some(path.to_owned());
    }
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
        Err((code, detail)) => report_failure(
            app,
            &state,
            OpenFailed {
                id,
                name,
                code,
                detail: detail.clone(),
            },
        ),
    }
    result.map_err(|(code, detail)| {
        if code == CODE_DOCUMENT_CLOSED {
            DOCUMENT_CLOSED.to_owned()
        } else {
            detail
        }
    })
}

/// Announce a failed open, and keep it for a UI that subscribes late.
fn report_failure(app: &AppHandle, state: &AppState, failed: OpenFailed) {
    // A target tab closed by the user is not a failure worth replaying later.
    let closed = failed.code == CODE_DOCUMENT_CLOSED;
    if let (false, Ok(mut failures)) = (closed, state.failures.lock()) {
        failures.push(failed.clone());
        let excess = failures.len().saturating_sub(KEPT_FAILURES);
        failures.drain(..excess);
    }
    emit(app, EVENT_OPEN_FAILED, &failed);
}

/// Put a decoded image into its target: a new tab, or a new top layer (undoable).
fn insert_imported(
    state: &AppState,
    name: &str,
    layer_name: &str,
    imported: slopshop_io::Imported,
    target: OpenTarget,
) -> Result<DocumentView, String> {
    let warnings: Vec<_> = imported.warnings.iter().map(|w| w.id()).collect();
    match (target, imported.resolution) {
        // A new document takes the file's resolution (ADR 0028); out of range, the default.
        (OpenTarget::NewTab, Some(ppi)) => {
            let session = image_session(imported.image, layer_name);
            let resolved = session.document().clone().with_resolution(ppi);
            let session = resolved.map_or(session, Session::new);
            state.add_document(session, Some(name.to_owned()), warnings)
        }
        _ => insert_image(state, name, layer_name, imported.image, warnings, target),
    }
}

/// Put a layered file (Photoshop) into its target: a new tab, or its layers on top of the
/// document (undoable). Each layer keeps the file's warnings and its own.
fn insert_layers(
    state: &AppState,
    path: &Path,
    name: &str,
    imported: slopshop_io::ImportedLayers,
    target: OpenTarget,
) -> Result<DocumentView, String> {
    let group_name = layer_name(path);
    let common: Vec<&'static str> = imported.warnings.iter().map(|w| w.id()).collect();
    let warnings: Vec<Vec<&'static str>> = imported
        .layer_warnings
        .iter()
        .map(|own| {
            let mut all = common.clone();
            for id in own.iter().map(|w| w.id()) {
                if !all.contains(&id) {
                    all.push(id);
                }
            }
            all
        })
        .collect();
    match target {
        OpenTarget::NewTab => state.add_document_with(
            Session::new(imported.document),
            Some(name.to_owned()),
            warnings,
            None,
        ),
        OpenTarget::Layer { document_id } => state.documents().and_then(|mut documents| {
            let target = documents.get_mut(document_id)?;
            insert_document_layers(target, &imported.document, warnings, Some(group_name))
        }),
    }
}

/// An image just scanned (File > Import from Device) in a new tab, untitled: its temporary
/// file is no name for it, and no source to show. False when it could not be opened (the
/// open's events said why).
#[cfg(windows)]
fn open_scanned(app: &AppHandle, path: &Path) -> bool {
    let Ok(view) = open_path(app, path, Source::File, OpenTarget::NewTab, None) else {
        return false;
    };
    let state = app.state::<AppState>();
    if let Ok(mut documents) = state.documents()
        && let Ok(document) = documents.get_mut(view.id)
    {
        document.meta.name = None;
        document.source = None;
    }
    true
}

/// Put an image into its target: a new tab named `tab_name`, or a new top layer (undoable);
/// the layer is named `layer_name` either way.
fn insert_image(
    state: &AppState,
    tab_name: &str,
    layer_name: &str,
    image: RasterImage,
    warnings: Vec<&'static str>,
    target: OpenTarget,
) -> Result<DocumentView, String> {
    match target {
        OpenTarget::NewTab => state.add_document(
            image_session(image, layer_name),
            Some(tab_name.to_owned()),
            warnings,
        ),
        OpenTarget::Layer { document_id } => {
            let mut documents = state.documents()?;
            // The tab may have been closed during the decode.
            let document = documents.get_mut(document_id)?;
            let session = &mut document.session;
            let layer_id = session.allocate_layer_id();
            let edit = Edit::InsertLayer {
                parent: None,
                index: session.document().layers().len(),
                layer: Layer {
                    style: None,
                    transform: slopshop_core::Affine::IDENTITY,
                    clipped: false,
                    id: layer_id,
                    name: layer_name.to_owned(),
                    visible: true,
                    opacity: 1.0,
                    blend_mode: BlendMode::Normal,
                    mask: None,
                    content: LayerContent::Raster {
                        stack: None,
                        image: slopshop_core::stack::Pixels::ready(Arc::new(image)),
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

pub(crate) fn emit<T: Serialize + Clone>(app: &AppHandle, event: &str, payload: &T) {
    if let Err(e) = app.emit(event, payload.clone()) {
        eprintln!("cannot emit {event}: {e}");
    }
}

/// Files to open at startup: paths given on the command line, or in dev builds the test file
/// (`SLOPSHOP_OPEN`, else `out/default.slop`, else `out/default.jpg` at the repository root) so
/// that every restart shows real content without clicking through a dialog.
fn startup_files() -> Vec<PathBuf> {
    let from_args: Vec<PathBuf> = std::env::args_os()
        .skip(1)
        .map(PathBuf::from)
        .filter(|p| p.is_file())
        .collect();
    if !from_args.is_empty() || !cfg!(debug_assertions) {
        return from_args;
    }
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out");
    std::env::var_os("SLOPSHOP_OPEN")
        .map(PathBuf::from)
        .into_iter()
        .chain(["default.slop", "default.jpg"].map(|name| out.join(name)))
        .find(|p| p.is_file())
        .into_iter()
        .collect()
}

/// Another launch of the app while it runs (a file opened from the file manager, the app
/// started again): show and focus this window, and open the files it was given in new tabs.
fn second_launch(app: &AppHandle, argv: Vec<String>, cwd: String) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        // Best effort: the files still open if the window cannot be raised.
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
    let paths = launch_files(&argv, Path::new(&cwd));
    if !paths.is_empty() {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = open_images(app, paths, None).await {
                eprintln!("cannot open the files of a second launch: {e}");
            }
        });
    }
}

/// The existing files among the arguments of a launch (the first one is the program),
/// relative paths resolved from the launch's working directory.
fn launch_files(argv: &[String], cwd: &Path) -> Vec<PathBuf> {
    argv.iter()
        .skip(1)
        .map(|arg| cwd.join(arg))
        .filter(|path| path.is_file())
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

/// A new blank document in a new tab (File > New): `width × height` pixels, one layer named
/// `layer_name` (translated by the UI) filled with `background` (sRGB-encoded), or transparent
/// without one. `name` names the tab; none keeps it untitled.
#[tauri::command]
async fn new_document(
    state: State<'_, AppState>,
    name: Option<String>,
    width: u32,
    height: u32,
    background: Option<[f32; 3]>,
    layer_name: String,
    resolution: Option<f64>,
) -> Result<DocumentView, String> {
    let mut session = blank_session(Size::new(width, height), background, &layer_name)?;
    if let Some(ppi) = resolution {
        // Cheap: the layer's pixels are shared.
        let doc = session.document().clone().with_resolution(ppi);
        session = Session::new(doc.map_err(|e| e.to_string())?);
    }
    let name = name.map(|n| n.trim().to_owned()).filter(|n| !n.is_empty());
    state.add_document(session, name, Vec::new())
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

/// Copy every layer of `source_id` on top of `target_id` (a tab dropped on the canvas), or only
/// `layer_ids` (layers dragged to another tab), as one undoable edit of the target.
#[tauri::command]
async fn copy_layers(
    state: State<'_, AppState>,
    source_id: u64,
    target_id: u64,
    layer_ids: Option<Vec<u64>>,
) -> Result<DocumentView, String> {
    state
        .documents()?
        .copy_layers(source_id, target_id, layer_ids.as_deref())
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

/// What `open_images` found in the folders and archives it was given.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct OpenSummary {
    /// Files of folders and archives that SlopShop does not open.
    skipped: usize,
    /// Archives that could not be read: name and technical detail.
    failed_archives: Vec<(String, String)>,
}

/// Replace folders by their openable files and zip archives by their extracted ones (kept in
/// `archives`, whose temporary folders live until the opens are done). Blocking.
fn expand_paths(
    paths: Vec<PathBuf>,
    summary: &mut OpenSummary,
    archives: &mut Vec<slopshop_io::collection::ExtractedArchive>,
) -> Vec<PathBuf> {
    use slopshop_io::collection;
    let mut files = Vec::new();
    for path in paths {
        if path.is_dir() {
            match collection::folder_files(&path) {
                Ok((found, skipped)) => {
                    files.extend(found);
                    summary.skipped += skipped;
                }
                Err(e) => summary
                    .failed_archives
                    .push((file_name(&path), e.to_string())),
            }
        } else if collection::is_archive(&path) {
            match collection::extract_archive(&path) {
                Ok(archive) => {
                    files.extend(archive.files.iter().cloned());
                    summary.skipped += archive.skipped;
                    archives.push(archive);
                }
                Err(e) => summary
                    .failed_archives
                    .push((file_name(&path), e.to_string())),
            }
        } else {
            files.push(path);
        }
    }
    files
}

/// Open images, decoded in parallel: each in a new tab, or each as a new top layer of
/// `document_id` (undoable, one edit per image). Folders and zip archives open their files
/// like several files (not their subfolders). Tabs and layers come in the order of `paths`.
/// Every outcome arrives as an `open-*` event; this resolves once all are done, with what was
/// skipped in folders and archives.
#[tauri::command]
async fn open_images(
    app: AppHandle,
    paths: Vec<PathBuf>,
    document_id: Option<u64>,
) -> Result<OpenSummary, String> {
    let requested = paths.clone();
    let (paths, series, summary, archives) = tauri::async_runtime::spawn_blocking(move || {
        let mut summary = OpenSummary::default();
        let mut archives = Vec::new();
        let paths = expand_paths(paths, &mut summary, &mut archives);
        // Several DICOM files together are a series: one document (the maintainer's choice).
        let (series, others): (Vec<PathBuf>, Vec<PathBuf>) = paths
            .into_iter()
            .partition(|path| slopshop_io::is_dicom_file(path).unwrap_or(false));
        let (series, paths) = if series.len() > 1 {
            (series, others)
        } else {
            (Vec::new(), series.into_iter().chain(others).collect())
        };
        (paths, series, summary, archives)
    })
    .await
    .map_err(|e| e.to_string())?;
    let target = match document_id {
        Some(document_id) => OpenTarget::Layer { document_id },
        None => OpenTarget::NewTab,
    };
    let order = Arc::new(InsertionOrder::default());
    let count = paths.len();
    let mut opens: Vec<_> = paths
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
                let opened = open_path(&app, &path, Source::File, target, Some(&turn));
                opened.map(|view| vec![(path, view.id)]).unwrap_or_default()
            })
        })
        .collect();
    if let Some(first) = series.first().cloned() {
        let (app, order) = (app.clone(), order.clone());
        opens.push(tauri::async_runtime::spawn_blocking(move || {
            let turn = Turn {
                order: &order,
                index: count,
            };
            // A series is remembered by its first file.
            let opened = open_path(&app, &first, Source::Series(&series), target, Some(&turn));
            opened
                .map(|view| vec![(first, view.id)])
                .unwrap_or_default()
        }));
    }
    let mut opened = Vec::new();
    for open in opens {
        opened.extend(open.await.map_err(|e| e.to_string())?);
    }
    // Every file is decoded (into memory): the extracted copies can go.
    drop(archives);
    if document_id.is_none() {
        // File > Open Recent: what was asked for, as asked (folders and archives whole), when
        // it opened.
        tauri::async_runtime::spawn_blocking(move || {
            for (path, document_id) in &opened {
                recent::remember_document(&app, path, *document_id);
            }
            let recorded: Vec<PathBuf> = requested
                .into_iter()
                .filter(|path| {
                    path.is_dir()
                        || slopshop_io::collection::is_archive(path)
                        || opened.iter().any(|(opened, _)| opened == path)
                })
                .collect();
            recent::record(&app, &recorded);
        })
        .await
        .map_err(|e| e.to_string())?;
    }
    Ok(summary)
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
    let view = document.view();
    drop(documents);
    if let Some(path) = &view.path {
        let path = PathBuf::from(path);
        recent::remember_thumbnail(app, &path, &snapshot);
        recent::record(app, &[path]);
    }
    Ok(view)
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

/// The layer showing a pixel at document pixel (`x`, `y`): the Move tool's Auto-Select.
#[tauri::command]
async fn layer_at(
    state: State<'_, AppState>,
    document_id: u64,
    x: i64,
    y: i64,
) -> Result<Option<u64>, String> {
    let document = state
        .documents()?
        .get_mut(document_id)?
        .session
        .document()
        .clone();
    tauri::async_runtime::spawn_blocking(move || {
        slopshop_core::pick::layer_at(&document, x, y).map(LayerId::get)
    })
    .await
    .map_err(|e| e.to_string())
}

/// A rectangle in document pixels (right and bottom exclusive).
#[derive(Debug, Clone, Copy, Serialize)]
struct BoundsDto {
    left: i64,
    top: i64,
    right: i64,
    bottom: i64,
}

impl From<slopshop_core::pick::Bounds> for BoundsDto {
    fn from(b: slopshop_core::pick::Bounds) -> Self {
        Self {
            left: b.left,
            top: b.top,
            right: b.right,
            bottom: b.bottom,
        }
    }
}

/// What moving layers can snap to (ADR 0017): the bounds of what moves, and of the other
/// visible layers' pixels (the canvas is known to the UI).
#[derive(Debug, Clone, Serialize)]
struct SnapTargets {
    moving: Option<BoundsDto>,
    others: Vec<BoundsDto>,
}

/// Snap targets for moving `ids`. Layer bounds are computed once per image (a scan of its
/// pixels the first time): on a worker.
#[tauri::command]
async fn move_snap_targets(
    state: State<'_, AppState>,
    document_id: u64,
    ids: Vec<u64>,
) -> Result<SnapTargets, String> {
    let document = state
        .documents()?
        .get_mut(document_id)?
        .session
        .document()
        .clone();
    tauri::async_runtime::spawn_blocking(move || {
        let ids: Vec<LayerId> = ids.into_iter().map(LayerId::from_raw).collect();
        // Everything that moves: the layers and what is inside those that are groups.
        let moving: std::collections::HashSet<LayerId> = document
            .outermost(&ids)
            .into_iter()
            .filter_map(|id| document.layer(id))
            .flat_map(|layer| layer.subtree().map(|l| l.id))
            .collect();
        SnapTargets {
            moving: slopshop_core::pick::bounds_of(&document, &ids).map(BoundsDto::from),
            others: slopshop_core::pick::visible_layer_bounds(&document)
                .into_iter()
                .filter(|(id, _)| !moving.contains(id))
                .map(|(_, b)| b.into())
                .collect(),
        }
    })
    .await
    .map_err(|e| e.to_string())
}

/// Add a mask made from the transparency of a raster layer (ADR 0014), undoable. Copying the
/// alpha channel of a large image takes a while: done on a worker, outside the document lock.
#[tauri::command]
async fn add_mask_from_transparency(
    state: State<'_, AppState>,
    document_id: u64,
    layer_id: u64,
) -> Result<DocumentView, String> {
    let id = LayerId::from_raw(layer_id);
    let image = {
        let mut documents = state.documents()?;
        let layer = documents
            .get_mut(document_id)?
            .session
            .document()
            .layer(id)
            .ok_or("unknown layer")?;
        match &layer.content {
            LayerContent::Raster { image, .. } => image.clone(),
            LayerContent::Fill { .. } => return Err("a fill layer has no transparency".to_owned()),
            LayerContent::Group { .. } => return Err("a group has no transparency".to_owned()),
            LayerContent::Adjustment { .. } => {
                return Err("an adjustment layer has no transparency".to_owned());
            }
        }
    };
    let mask =
        tauri::async_runtime::spawn_blocking(move || LayerMask::from_transparency(&image.get()))
            .await
            .map_err(|e| e.to_string())?
            .ok_or("the layer has no transparency")?;
    let mut documents = state.documents()?;
    let document = documents.get_mut(document_id)?;
    document
        .session
        .perform(Edit::SetLayerMask {
            id,
            mask: Some(mask),
        })
        .map_err(|e| e.to_string())?;
    Ok(document.view())
}

/// Largest thumbnail side a request may ask for, in pixels.
const MAX_THUMBNAIL_SIDE: u32 = 512;

/// Thumbnail of a raster layer, or of a layer's mask with `mask`, at most `max_side` pixels on
/// its longer side (capped at [`MAX_THUMBNAIL_SIDE`]): raw binary, a header of width and
/// height (`u32` little-endian), then RGBA8 pixels with straight alpha (sRGB; mask coverage
/// values as they are). Fill layers have no layer thumbnail (the UI shows their color).
#[tauri::command]
async fn layer_thumbnail(
    state: State<'_, AppState>,
    document_id: u64,
    layer_id: u64,
    max_side: u32,
    mask: bool,
) -> Result<Response, String> {
    let image = {
        let mut documents = state.documents()?;
        let document = documents.get_mut(document_id)?;
        let layer = document
            .session
            .document()
            .layer(LayerId::from_raw(layer_id))
            .ok_or("unknown layer")?;
        match (&layer.content, &layer.mask, mask) {
            (_, Some(layer_mask), true) => {
                slopshop_core::stack::Pixels::ready(layer_mask.image.clone())
            }
            (_, None, true) => return Err("the layer has no mask".to_owned()),
            // A stack's pixels are evaluated below, off the async runtime (ADR 0029).
            (LayerContent::Raster { image, .. }, _, false) => image.clone(),
            (LayerContent::Fill { .. }, _, false) => {
                return Err("fill layers have no thumbnail".to_owned());
            }
            (LayerContent::Group { .. }, _, false) => {
                return Err("groups have no thumbnail".to_owned());
            }
            (LayerContent::Adjustment { .. }, _, false) => {
                return Err("adjustment layers have no thumbnail".to_owned());
            }
        }
    };
    let max_side = max_side.min(MAX_THUMBNAIL_SIDE);
    let thumbnail = tauri::async_runtime::spawn_blocking(move || {
        let image = image.get();
        if mask {
            slopshop_core::thumbnail::mask_thumbnail(&image, max_side)
        } else {
            slopshop_core::thumbnail::raster_thumbnail(&image, max_side)
        }
    })
    .await
    .map_err(|e| e.to_string())?;
    let mut bytes = Vec::with_capacity(8 + thumbnail.pixels.len());
    bytes.extend(thumbnail.size.width.to_le_bytes());
    bytes.extend(thumbnail.size.height.to_le_bytes());
    bytes.extend(thumbnail.pixels);
    Ok(Response::new(bytes))
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
    let edit = edit.into_edit(&mut document.session)?;
    document.session.perform(edit).map_err(|e| e.to_string())?;
    Ok(document.view())
}

/// Apply an edit live, as part of a continuous gesture (slider drag, …). The gesture becomes a
/// single undo entry when `end_gesture` is called (or when any other edit/undo happens).
/// `replace`: the edit is the whole gesture so far (a move or transform since the drag began),
/// so what the gesture applied before is reverted first: nothing drifts, and live edits merged
/// while the engine was busy lose nothing.
#[tauri::command]
async fn perform_live(
    state: State<'_, AppState>,
    document_id: u64,
    edit: EditRequest,
    replace: Option<bool>,
) -> Result<DocumentView, String> {
    let mut documents = state.documents()?;
    let document = documents.get_mut(document_id)?;
    if replace == Some(true) {
        document
            .session
            .cancel_gesture()
            .map_err(|e| e.to_string())?;
    }
    let edit = edit.into_edit(&mut document.session)?;
    document
        .session
        .perform_in_gesture(edit)
        .map_err(|e| e.to_string())?;
    Ok(document.view())
}

/// Revert the gesture in progress without a history entry (a transform cancelled with Esc).
#[tauri::command]
async fn cancel_gesture(
    state: State<'_, AppState>,
    document_id: u64,
) -> Result<DocumentView, String> {
    let mut documents = state.documents()?;
    let document = documents.get_mut(document_id)?;
    document
        .session
        .cancel_gesture()
        .map_err(|e| e.to_string())?;
    Ok(document.view())
}

/// Revert the gesture in progress without a history entry, then apply `edit` as one undo entry,
/// in one go: the view goes from what the gesture showed straight to the edit's result (Image >
/// Adjustments: its preview, then the effect).
#[tauri::command]
async fn replace_gesture(
    state: State<'_, AppState>,
    document_id: u64,
    edit: EditRequest,
) -> Result<DocumentView, String> {
    let mut documents = state.documents()?;
    let document = documents.get_mut(document_id)?;
    document
        .session
        .cancel_gesture()
        .map_err(|e| e.to_string())?;
    let edit = edit.into_edit(&mut document.session)?;
    document.session.perform(edit).map_err(|e| e.to_string())?;
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
        let (doc, revision, viewport, overlays) = {
            let mut documents = state.documents()?;
            documents.output = Some(output);
            let document = documents.get_mut(document_id)?;
            let (doc, revision) = document.snapshot();
            if document.viewport.output() != output {
                document.viewport.resize(doc.size(), output);
            }
            (doc, revision, document.viewport, document.overlays)
        };

        let mut bytes = Vec::with_capacity(FRAME_HEADER_LEN + pixel_bytes);
        // Reserve the header, append the pixels in place (no extra frame copy), then fill the
        // header once the render time is known.
        bytes.resize(FRAME_HEADER_LEN, 0);
        let start = Instant::now();
        let stats = renderer
            .render_view_into(&doc, viewport.transform(), overlays, output, &mut bytes)
            .map_err(|e| e.to_string())?;
        let header = FrameHeader {
            size: output,
            fit: viewport.is_fit(),
            incomplete: stats.incomplete,
            revision,
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
        let (doc, revision, viewport, overlays) = {
            let mut documents = state.documents()?;
            documents.output = Some(output);
            let document = documents.get_mut(document_id)?;
            let (doc, revision) = document.snapshot();
            if !output.is_empty() && document.viewport.output() != output {
                document.viewport.resize(doc.size(), output);
            }
            (doc, revision, document.viewport, document.overlays)
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
                    overlays,
                    Rect::new(x, y, width, height),
                    surface_size,
                    PASTEBOARD_SRGB,
                )
                .map_err(|e| e.to_string())?
        };
        Ok(PresentInfo {
            presented: presented != Presented::Skipped,
            complete: presented != Presented::Partial,
            revision,
            zoom: viewport.zoom(),
            origin: viewport.transform().origin,
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
        // First, so that a second launch hands over before anything else starts.
        .plugin(tauri_plugin_single_instance::init(second_launch))
        .plugin(tauri_plugin_window_state::Builder::default().build())
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
            // The AI helper stops when unused for a while, freeing its memory.
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(15));
                    segment::stop_if_idle(&handle.state::<AppState>());
                }
            });
            // Warm up the GPU in the background so the first frame is fast, then open the
            // startup files, if any (decoding a large image takes seconds).
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                if let Err(e) = handle.state::<AppState>().renderer() {
                    eprintln!("GPU initialization failed: {e}");
                }
                for path in startup_files() {
                    let start = Instant::now();
                    match open_path(&handle, &path, Source::File, OpenTarget::NewTab, None) {
                        Ok(view) => {
                            // Files the app was launched with, not the dev builds' test file.
                            if std::env::args_os().skip(1).any(|a| Path::new(&a) == path) {
                                recent::remember_document(&handle, &path, view.id);
                                recent::record(&handle, std::slice::from_ref(&path));
                            }
                            eprintln!(
                                "opened {} in {:.1} s",
                                path.display(),
                                start.elapsed().as_secs_f32()
                            )
                        }
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
            vector::vector_info,
            vector::vector_thumbnail,
            vector::close_vector,
            vector::open_vector_pages,
            perform,
            perform_live,
            end_gesture,
            cancel_gesture,
            replace_gesture,
            undo,
            redo,
            gpu_info,
            view,
            render_view,
            presenter_mode,
            present_view,
            reveal_in_folder,
            ai::ai_components,
            ai::ai_runtime,
            ai::ai_install,
            ai::ai_cancel_install,
            ai::ai_remove,
            ai::ai_open_license,
            segment::ai_object_hover,
            segment::ai_object_select,
            segment::ai_select_subject,
            segment::ai_cancel,
            layer_thumbnail,
            add_mask_from_transparency,
            selection::select_shape,
            selection::select_all,
            selection::invert_selection,
            selection::translate_selection,
            selection::deselect,
            selection::reselect,
            selection::set_quick_mask,
            selection::add_layer_masks,
            selection::crop_to_selection,
            selection::modify_selection,
            selection::magic_wand,
            selection::grow_selection,
            selection::selection_bounds,
            selection::transform_selection,
            selection::save_selection,
            selection::load_selection,
            selection::rename_saved_selection,
            selection::delete_saved_selection,
            refine::refine_open,
            refine::refine_view,
            refine::refine_preview,
            refine::refine_close,
            refine::refine_output,
            segment::ai_refine_base,
            selection::quick_select,
            paint::paint_stroke,
            paint::fill,
            move_pixels::float_pixels,
            move_pixels::layer_via,
            move_pixels::move_selected_pixels,
            move_pixels::selection_bounds_at,
            recent::recent_files,
            recent::clear_recent_files,
            recent::recent_thumbnail,
            info::document_info,
            print::print_page,
            acquire::acquire_image,
            paint::sample_color,
            selection::color_range_preview,
            selection::color_range,
            selection::selection_outline,
            layer_at,
            move_snap_targets,
            clipboard::paste,
            clipboard::copy,
            bake::bake_layers,
            clipboard::clipboard_size,
            clipboard::clipboard_contents,
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

    /// A white 6000 × 4000 document, as File > New makes by default.
    fn blank_session() -> Session {
        super::blank_session(Size::new(6000, 4000), Some([1.0, 1.0, 1.0]), "Background")
            .expect("a valid size")
    }

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
    fn folders_expand_to_their_openable_files() {
        let dir = std::env::temp_dir().join(format!("slopshop-expand-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["b10.png", "b9.png", "notes.txt"] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        let single = dir.join("single.jpg");
        let broken = dir.join("broken.zip");
        std::fs::write(&broken, b"not a zip").unwrap();
        let mut summary = OpenSummary::default();
        let mut archives = Vec::new();
        let files = expand_paths(
            vec![single.clone(), dir.clone(), broken],
            &mut summary,
            &mut archives,
        );
        // Plain files pass through (the importer reports what it cannot open).
        assert_eq!(files, [single, dir.join("b9.png"), dir.join("b10.png")]);
        // notes.txt, and the broken archive listed inside the folder is not an image.
        assert_eq!(summary.skipped, 2);
        assert_eq!(summary.failed_archives.len(), 1);
        assert_eq!(summary.failed_archives[0].0, "broken.zip");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn second_launches_open_their_existing_files() {
        let dir = std::env::temp_dir().join(format!("slopshop-launch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("photo.png");
        std::fs::write(&file, b"not really a png").unwrap();
        let argv = [
            "slopshop.exe".to_owned(),
            "photo.png".to_owned(),
            "missing.png".to_owned(),
            file.display().to_string(),
            "--flag".to_owned(),
        ];
        // Relative from the launch's directory, absolute as is; missing files and options skipped.
        assert_eq!(launch_files(&argv, &dir), [file.clone(), file]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn mask_requests_and_views() {
        let mut s = blank_session();
        let pixels = [10, 20, 30, 128, 40, 50, 60, 255];
        let image =
            RasterImage::from_pixels(Size::new(2, 1), PixelFormat::RGBA8_SRGB, &pixels).unwrap();
        let mask = LayerMask::from_transparency(&image).unwrap();
        let id = s.allocate_layer_id();
        let index = s.document().layers().len();
        s.perform(Edit::InsertLayer {
            parent: None,
            index,
            layer: Layer {
                style: None,
                transform: slopshop_core::Affine::IDENTITY,
                clipped: false,
                id,
                name: "masked".to_owned(),
                visible: true,
                opacity: 1.0,
                blend_mode: BlendMode::Normal,
                content: LayerContent::Raster {
                    stack: None,
                    image: slopshop_core::stack::Pixels::ready(Arc::new(image)),
                },
                mask: Some(mask),
            },
        })
        .unwrap();
        let view = DocumentView::new(&s, &meta(), Vec::new());
        let layer = view.layers.last().unwrap();
        assert!(layer.has_alpha);
        assert!(layer.mask.as_ref().is_some_and(|m| m.enabled));
        let request = |json: String| {
            serde_json::from_str::<EditRequest>(&json)
                .unwrap()
                .into_edit(&mut blank_session())
                .unwrap()
        };
        let raw = id.get();
        s.perform(request(format!(
            r#"{{"kind":"setLayerMaskEnabled","id":{raw},"enabled":false}}"#
        )))
        .unwrap();
        let view = DocumentView::new(&s, &meta(), Vec::new());
        assert!(
            view.layers
                .last()
                .unwrap()
                .mask
                .as_ref()
                .is_some_and(|m| !m.enabled)
        );
        s.perform(request(format!(
            r#"{{"kind":"removeLayerMask","id":{raw}}}"#
        )))
        .unwrap();
        let view = DocumentView::new(&s, &meta(), Vec::new());
        assert!(view.layers.last().unwrap().mask.is_none());
    }

    #[test]
    fn a_paint_stroke_shows_while_painted_and_commits_once() {
        let state = AppState::new();
        let doc = state
            .add_document(blank_session(), None, Vec::new())
            .unwrap();
        {
            let mut documents = state.documents().unwrap();
            let document = documents.get_mut(doc.id).unwrap();
            let edit: crate::ipc::EditRequest =
                serde_json::from_str(r#"{"kind":"addEmptyLayer","name":"Layer 1","index":0}"#)
                    .unwrap();
            let edit = edit.into_edit(&mut document.session).unwrap();
            document.session.perform(edit).unwrap();
        }
        let (layer, revision) = {
            let mut documents = state.documents().unwrap();
            let document = documents.get_mut(doc.id).unwrap();
            let layer = document.session.document().layers()[0].id.get();
            (layer, document.session.document().revision())
        };
        let batch = |samples: Vec<[f64; 3]>, end: bool| paint::PaintRequest {
            restore: false,
            stroke: 7,
            target: paint::PaintTarget::Layer,
            layer_id: layer,
            brush: paint::BrushRequest {
                size: 40.0,
                hardness: 1.0,
                spacing: 0.25,
                flow: 1.0,
                opacity: 1.0,
                pressure_size: true,
                pressure_opacity: false,
            },
            color: Some([1.0, 0.0, 0.0]),
            samples,
            end,
        };
        // While painted: shown in the view, not in the document.
        let live = paint::paint(&state, doc.id, batch(vec![[50.0, 50.0, 1.0]], false)).unwrap();
        assert!(live.is_none());
        {
            let mut documents = state.documents().unwrap();
            let document = documents.get_mut(doc.id).unwrap();
            assert_eq!(document.session.document().revision(), revision);
            let (shown, shown_revision) = document.snapshot();
            assert_eq!(shown_revision, revision);
            assert!(shown.layers()[0].is_painted());
            assert!(!document.session.document().layers()[0].is_painted());
        }
        // The end commits the stroke: one undo entry, the preview gone.
        let view = paint::paint(&state, doc.id, batch(vec![[120.0, 80.0, 0.5]], true))
            .unwrap()
            .unwrap();
        assert!(view.layers[0].painted);
        let mut documents = state.documents().unwrap();
        let document = documents.get_mut(doc.id).unwrap();
        assert!(document.paint_preview.is_none());
        assert!(document.session.document().layers()[0].is_painted());
        document.session.undo().unwrap();
        assert!(!document.session.document().layers()[0].is_painted());
    }

    #[test]
    fn a_stroke_outside_a_small_layer_grows_it_and_undo_restores_it() {
        let state = AppState::new();
        let doc = state
            .add_document(blank_session(), None, Vec::new())
            .unwrap();
        // A 100 × 100 opaque layer placed at (1000, 1000), on top.
        let small = Arc::new(
            RasterImage::from_pixels(
                Size::new(100, 100),
                PixelFormat::RGBA8_SRGB,
                &[200; 100 * 100 * 4],
            )
            .unwrap(),
        );
        let id = {
            let mut documents = state.documents().unwrap();
            let document = documents.get_mut(doc.id).unwrap();
            let id = document.session.allocate_layer_id();
            let index = document.session.document().layers().len();
            let layer = Layer {
                style: None,
                id,
                name: "small".into(),
                visible: true,
                opacity: 1.0,
                blend_mode: BlendMode::Normal,
                mask: None,
                clipped: false,
                transform: slopshop_core::Affine::translation(1000.0, 1000.0),
                content: LayerContent::raster(Arc::clone(&small)),
            };
            document
                .session
                .perform(Edit::InsertLayer {
                    parent: None,
                    index,
                    layer,
                })
                .unwrap();
            id
        };
        let request = paint::PaintRequest {
            restore: false,
            stroke: 1,
            target: paint::PaintTarget::Layer,
            layer_id: id.get(),
            brush: paint::BrushRequest {
                size: 20.0,
                hardness: 1.0,
                spacing: 0.25,
                flow: 1.0,
                opacity: 1.0,
                pressure_size: false,
                pressure_opacity: false,
            },
            color: Some([1.0, 0.0, 0.0]),
            samples: vec![[100.0, 100.0, 1.0]],
            end: true,
        };
        let view = paint::paint(&state, doc.id, request).unwrap().unwrap();
        assert!(view.layers.last().unwrap().painted);
        let mut documents = state.documents().unwrap();
        let document = documents.get_mut(doc.id).unwrap();
        let layer = document.session.document().layer(id).unwrap().clone();
        let LayerContent::Raster { image, stack } = &layer.content else {
            panic!("a raster layer");
        };
        let image = image.get();
        // Grown to the canvas by whole tiles; its old pixels where they were.
        let doc_point = |x: f64, y: f64| layer.transform.inverse().unwrap().apply(x, y);
        let (x, y) = doc_point(100.5, 100.5);
        assert!(
            image.alpha_at(x as u32, y as u32) > 0.99,
            "painted at (100, 100)"
        );
        let (x, y) = doc_point(1050.5, 1050.5);
        let alpha = image.alpha_at(x as u32, y as u32);
        // Its alpha: 200 / 255.
        assert!(
            (alpha - 200.0 / 255.0).abs() < 1e-3,
            "the old pixels kept: {alpha} at {x}, {y}"
        );
        assert_eq!(stack.as_ref().unwrap().original().size(), image.size());
        // One undo entry brings the small layer back as it was.
        document.session.undo().unwrap();
        let layer = document.session.document().layer(id).unwrap();
        assert_eq!(
            layer.transform,
            slopshop_core::Affine::translation(1000.0, 1000.0)
        );
        assert!(
            matches!(&layer.content, LayerContent::Raster { image, stack: None }
            if Arc::ptr_eq(&image.get(), &small))
        );
    }

    /// A one-batch stroke of a hard 20-pixel brush at (50, 50) on `target`.
    fn dab(
        layer: LayerId,
        target: paint::PaintTarget,
        color: Option<[f32; 3]>,
    ) -> paint::PaintRequest {
        paint::PaintRequest {
            restore: false,
            stroke: 1,
            target,
            layer_id: layer.get(),
            brush: paint::BrushRequest {
                size: 20.0,
                hardness: 1.0,
                spacing: 0.25,
                flow: 1.0,
                opacity: 1.0,
                pressure_size: false,
                pressure_opacity: false,
            },
            color,
            samples: vec![[50.5, 50.5, 1.0]],
            end: true,
        }
    }

    #[test]
    fn the_restore_eraser_brings_back_the_original_pixels() {
        let state = AppState::new();
        let doc = state
            .add_document(blank_session(), None, Vec::new())
            .unwrap();
        let size = Size::new(200, 200);
        let pixels = Arc::new(
            RasterImage::from_pixels(size, PixelFormat::RGBA8_SRGB, &[200; 200 * 200 * 4]).unwrap(),
        );
        let id = {
            let mut documents = state.documents().unwrap();
            let document = documents.get_mut(doc.id).unwrap();
            let id = document.session.allocate_layer_id();
            let index = document.session.document().layers().len();
            document
                .session
                .perform(Edit::InsertLayer {
                    parent: None,
                    index,
                    layer: Layer {
                        style: None,
                        transform: slopshop_core::Affine::IDENTITY,
                        clipped: false,
                        id,
                        name: "painted".to_owned(),
                        visible: true,
                        opacity: 1.0,
                        blend_mode: BlendMode::Normal,
                        content: LayerContent::raster(Arc::clone(&pixels)),
                        mask: None,
                    },
                })
                .unwrap();
            id
        };
        let view = paint::paint(
            &state,
            doc.id,
            dab(id, paint::PaintTarget::Layer, Some([1.0, 0.0, 0.0])),
        )
        .unwrap()
        .unwrap();
        assert_eq!(view.layers.last().unwrap().entries.len(), 1);
        // A larger restore over the dab: the paint is gone, the original shows again.
        let mut restore = dab(id, paint::PaintTarget::Layer, None);
        restore.stroke = 2;
        restore.restore = true;
        restore.brush.size = 40.0;
        let view = paint::paint(&state, doc.id, restore).unwrap().unwrap();
        assert!(view.layers.last().unwrap().entries.is_empty());
        let mut documents = state.documents().unwrap();
        let document = documents.get_mut(doc.id).unwrap();
        let layer = document.session.document().layer(id).unwrap();
        // The layer grew to the canvas at the first stroke: its pixels where they were.
        let LayerContent::Raster { image, stack: None } = &layer.content else {
            panic!("a raster layer without paint");
        };
        let image = image.get();
        let (x, y) = layer.transform.inverse().unwrap().apply(50.5, 50.5);
        let tile = image.levels()[0]
            .tile(slopshop_core::tile::TileCoord {
                col: x as u32 / 256,
                row: y as u32 / 256,
            })
            .unwrap();
        let at = (((y as usize) % 256) * 256 + (x as usize) % 256) * 4;
        assert_eq!(&tile[at..at + 4], &[200, 200, 200, 200]);
        // On a mask, it is refused.
        drop(documents);
        let mut on_mask = dab(id, paint::PaintTarget::Mask, None);
        on_mask.stroke = 3;
        on_mask.restore = true;
        assert!(paint::paint(&state, doc.id, on_mask).is_err());
    }

    #[test]
    fn a_stroke_paints_a_mask_and_keeps_the_pixels() {
        let state = AppState::new();
        let doc = state
            .add_document(blank_session(), None, Vec::new())
            .unwrap();
        let size = Size::new(200, 200);
        let pixels = Arc::new(
            RasterImage::from_pixels(size, PixelFormat::RGBA8_SRGB, &[200; 200 * 200 * 4]).unwrap(),
        );
        let id = {
            let mut documents = state.documents().unwrap();
            let document = documents.get_mut(doc.id).unwrap();
            let id = document.session.allocate_layer_id();
            let index = document.session.document().layers().len();
            let mask = LayerMask {
                image: Arc::new(slopshop_core::selection::select_all(size).unwrap()),
                enabled: true,
                replaces_alpha: false,
                original: None,
            };
            document
                .session
                .perform(Edit::InsertLayer {
                    parent: None,
                    index,
                    layer: Layer {
                        style: None,
                        transform: slopshop_core::Affine::IDENTITY,
                        clipped: false,
                        id,
                        name: "masked".to_owned(),
                        visible: true,
                        opacity: 1.0,
                        blend_mode: BlendMode::Normal,
                        content: LayerContent::Raster {
                            stack: None,
                            image: slopshop_core::stack::Pixels::ready(Arc::clone(&pixels)),
                        },
                        mask: Some(mask),
                    },
                })
                .unwrap();
            id
        };
        // Black (the foreground) hides where it paints; the pixels stay as they were.
        let request = dab(id, paint::PaintTarget::Mask, Some([0.0; 3]));
        let view = paint::paint(&state, doc.id, request).unwrap().unwrap();
        assert!(view.layers.last().unwrap().painted);
        let mut documents = state.documents().unwrap();
        let document = documents.get_mut(doc.id).unwrap();
        let layer = document.session.document().layer(id).unwrap();
        let mask = layer.mask.as_ref().unwrap();
        assert_eq!(mask.image.gray_at(50, 50), 0.0);
        assert_eq!(mask.image.gray_at(150, 150), 1.0);
        assert!(mask.original.is_some());
        assert!(
            matches!(&layer.content, LayerContent::Raster { image, stack: None }
            if Arc::ptr_eq(&image.get(), &pixels))
        );
        // One undo entry.
        document.session.undo().unwrap();
        let mask = document.session.document().layer(id).unwrap().mask.as_ref();
        assert!(mask.is_some_and(|m| m.original.is_none() && m.image.gray_at(50, 50) == 1.0));
    }

    #[test]
    fn a_stroke_in_quick_mask_paints_the_selection() {
        let state = AppState::new();
        let doc = state
            .add_document(blank_session(), None, Vec::new())
            .unwrap();
        let background = {
            let mut documents = state.documents().unwrap();
            documents
                .get_mut(doc.id)
                .unwrap()
                .session
                .document()
                .layers()[0]
                .id
        };
        let selection = |state: &AppState| {
            let mut documents = state.documents().unwrap();
            let document = documents.get_mut(doc.id).unwrap();
            document.session.document().selection().cloned()
        };
        // Nothing selected is everything selected: black unselects where it paints.
        let request = dab(background, paint::PaintTarget::Selection, Some([0.0; 3]));
        paint::paint(&state, doc.id, request).unwrap().unwrap();
        let selected = selection(&state).expect("a selection");
        assert_eq!(selected.image().gray_at(50, 50), 0.0);
        assert_eq!(selected.image().gray_at(150, 150), 1.0);
        // White selects again; the background layer is never painted.
        let request = dab(background, paint::PaintTarget::Selection, Some([1.0; 3]));
        paint::paint(&state, doc.id, request).unwrap().unwrap();
        assert_eq!(selection(&state).unwrap().image().gray_at(50, 50), 1.0);
        let mut documents = state.documents().unwrap();
        let document = documents.get_mut(doc.id).unwrap();
        assert!(!document.session.document().layers()[0].is_painted());
        document.session.undo().unwrap();
        document.session.undo().unwrap();
        assert!(document.session.document().selection().is_none());
    }

    #[test]
    fn select_modify_shows_each_amount_then_applies_once() {
        use slopshop_core::selection::{self as sel, Modify};
        let state = AppState::new();
        let doc = state
            .add_document(blank_session(), None, Vec::new())
            .unwrap();
        let square = sel::select_shape(
            Size::new(6000, 4000),
            None,
            &sel::Shape::Rectangle {
                left: 100.0,
                top: 100.0,
                right: 300.0,
                bottom: 300.0,
            },
            sel::EdgeOptions::default(),
            sel::Combine::Replace,
        )
        .unwrap();
        selection::set_selection(&state, doc.id, square).unwrap();
        let at = |state: &AppState, x: u32| {
            let mut documents = state.documents().unwrap();
            let document = documents.get_mut(doc.id).unwrap();
            let doc = document.session.document();
            doc.selection().map_or(0.0, |s| s.image().gray_at(x, 200))
        };
        // Each amount shown replaces the previous one: both from the selection before.
        selection::modify(&state, doc.id, Modify::Expand(20.0), true).unwrap();
        assert_eq!(at(&state, 85), 1.0);
        selection::modify(&state, doc.id, Modify::Expand(5.0), true).unwrap();
        assert_eq!(at(&state, 85), 0.0);
        assert_eq!(at(&state, 97), 1.0);
        // Cancel: as it was.
        {
            let mut documents = state.documents().unwrap();
            documents
                .get_mut(doc.id)
                .unwrap()
                .session
                .cancel_gesture()
                .unwrap();
        }
        assert_eq!(at(&state, 97), 0.0);
        // Shown then applied: one undo entry, back to the square.
        selection::modify(&state, doc.id, Modify::Contract(50.0), true).unwrap();
        selection::modify(&state, doc.id, Modify::Contract(10.0), false).unwrap();
        assert_eq!(at(&state, 105), 0.0);
        assert_eq!(at(&state, 115), 1.0);
        {
            let mut documents = state.documents().unwrap();
            documents.get_mut(doc.id).unwrap().session.undo().unwrap();
        }
        assert_eq!(at(&state, 105), 1.0);
    }

    #[test]
    fn select_and_mask_previews_then_outputs_to_a_new_layer_with_a_mask() {
        use slopshop_core::selection::{self as sel, EdgeSettings};
        let state = AppState::new();
        let doc = state
            .add_document(blank_session(), None, Vec::new())
            .unwrap();
        let square = sel::select_shape(
            Size::new(6000, 4000),
            None,
            &sel::Shape::Rectangle {
                left: 100.0,
                top: 100.0,
                right: 300.0,
                bottom: 300.0,
            },
            sel::EdgeOptions::default(),
            sel::Combine::Replace,
        )
        .unwrap();
        selection::set_selection(&state, doc.id, square).unwrap();
        let with = |f: &mut dyn FnMut(&mut OpenDocument)| {
            let mut documents = state.documents().unwrap();
            f(documents.get_mut(doc.id).unwrap());
        };
        let mut background = None;
        with(&mut |d| {
            d.refine_base = d
                .session
                .document()
                .selection()
                .map(|s| Arc::clone(s.image()));
            background = Some(d.session.document().layers()[0].id);
        });
        let background = background.unwrap();
        let at = |x: u32| {
            let mut value = 0.0;
            with(&mut |d| {
                value = d
                    .session
                    .document()
                    .selection()
                    .map_or(0.0, |s| s.image().gray_at(x, 200));
            });
            value
        };
        // Each setting shown from the base: shifting out by 20, then by 5.
        let shift = |px| EdgeSettings {
            shift: px,
            ..EdgeSettings::default()
        };
        refine::preview(&state, doc.id, shift(20.0), true).unwrap();
        assert_eq!(at(85), 1.0);
        refine::preview(&state, doc.id, shift(5.0), true).unwrap();
        assert_eq!((at(85), at(97)), (0.0, 1.0));
        // Out to a copy of the layer with that mask: the layer hidden, nothing selected, one
        // undo entry back to the selection as it was.
        let view =
            refine::output(&state, doc.id, shift(5.0), background, true, "{name} copy").unwrap();
        assert_eq!(view.layers.len(), 2);
        assert_eq!(view.layers[1].name, "Background copy");
        with(&mut |d| {
            let document = d.session.document();
            assert!(document.selection().is_none());
            assert!(!document.layers()[0].visible);
            let mask = document.layers()[1].mask.as_ref().expect("a mask");
            assert_eq!(mask.image.gray_at(97, 200), 1.0);
            assert_eq!(mask.image.gray_at(90, 200), 0.0);
            assert!(d.refine_base.is_none());
            d.session.undo().unwrap();
            assert_eq!(d.session.document().layers().len(), 1);
            assert!(d.session.document().layers()[0].visible);
        });
        assert_eq!((at(97), at(100)), (0.0, 1.0));
    }

    #[test]
    fn the_eyedropper_samples_the_colors_shown() {
        let state = AppState::new();
        let doc = state
            .add_document(blank_session(), None, Vec::new())
            .unwrap();
        let layer = {
            let mut documents = state.documents().unwrap();
            let document = documents.get_mut(doc.id).unwrap();
            let edit: crate::ipc::EditRequest =
                serde_json::from_str(r#"{"kind":"addEmptyLayer","name":"Layer 1","index":1}"#)
                    .unwrap();
            let edit = edit.into_edit(&mut document.session).unwrap();
            document.session.perform(edit).unwrap();
            document.session.document().layers()[1].id
        };
        paint::paint(
            &state,
            doc.id,
            dab(layer, paint::PaintTarget::Layer, Some([1.0, 0.0, 0.0])),
        )
        .unwrap();
        let mut documents = state.documents().unwrap();
        let shown = documents.get_mut(doc.id).unwrap().session.document();
        // The red paint over the white background, the background elsewhere.
        assert_eq!(paint::sample_color_at(shown, 50.5, 50.5), Some([255, 0, 0]));
        assert_eq!(paint::sample_color_at(shown, 150.0, 150.0), Some([255; 3]));
        assert_eq!(paint::sample_color_at(shown, -1.0, 10.0), None);
        assert_eq!(paint::sample_color_at(shown, 1e12, 10.0), None);
        assert_eq!(paint::sample_color_at(shown, f64::NAN, 10.0), None);
        // Nothing shown: no color.
        let clear =
            RasterImage::from_pixels(Size::new(4, 4), PixelFormat::RGBA8_SRGB, &[0; 4 * 4 * 4])
                .unwrap();
        let session = image_session(clear, "clear");
        assert_eq!(paint::sample_color_at(session.document(), 1.0, 1.0), None);
    }

    #[test]
    fn pasted_images_become_a_layer_or_a_new_tab() {
        let state = AppState::new();
        let doc = state
            .add_document(blank_session(), None, Vec::new())
            .unwrap();
        let image = || {
            RasterImage::from_pixels(Size::new(3, 2), PixelFormat::RGBA8_SRGB, &[200; 3 * 2 * 4])
                .unwrap()
        };
        let layers = doc.layers.len();
        let target = OpenTarget::Layer {
            document_id: doc.id,
        };
        let view = insert_image(&state, "Pasted", "Pasted", image(), Vec::new(), target).unwrap();
        assert_eq!(view.id, doc.id);
        assert_eq!(view.layers.len(), layers + 1);
        assert_eq!(view.layers.last().unwrap().name, "Pasted");
        let tab = insert_image(
            &state,
            "Pasted",
            "Pasted",
            image(),
            Vec::new(),
            OpenTarget::NewTab,
        )
        .unwrap();
        assert_ne!(tab.id, doc.id);
        assert_eq!((tab.width, tab.height), (3, 2));
        assert_eq!(tab.name.as_deref(), Some("Pasted"));
    }

    #[test]
    fn imported_layers_are_one_undo_step_and_warn_about_another_blend_space() {
        let mut documents = documents_with(2);
        let source = documents.get_mut(1).unwrap().session.document().clone();
        let target = documents.get_mut(2).unwrap();
        let before = target.session.document().layers().len();
        let view = insert_document_layers(target, &source, vec![Vec::new(); 1], None).unwrap();
        assert_eq!(view.layers.len(), before + source.layers().len());
        assert!(
            view.warnings.is_empty(),
            "same blend space: {:?}",
            view.warnings
        );
        target.session.undo().unwrap();
        assert_eq!(target.session.document().layers().len(), before);

        let mut linear = source.clone();
        Edit::SetBlendSpace {
            space: BlendSpace::Linear,
        }
        .apply(&mut linear)
        .unwrap();
        let view = insert_document_layers(target, &linear, vec![Vec::new(); 1], None).unwrap();
        assert_eq!(view.warnings, [WARNING_BLEND_SPACE]);
        // The warning follows the imported layers.
        target.session.undo().unwrap();
        assert!(target.view().warnings.is_empty());
    }

    #[test]
    fn documents_of_several_layers_arrive_as_a_group() {
        let mut documents = documents_with(2);
        // A second layer in the source, with a warning of its own.
        let source_doc = documents.get_mut(1).unwrap();
        let second = source_doc.session.allocate_layer_id();
        source_doc
            .session
            .perform(Edit::InsertLayer {
                parent: None,
                index: 1,
                layer: Layer {
                    style: None,
                    transform: slopshop_core::Affine::IDENTITY,
                    clipped: false,
                    id: second,
                    name: "second".into(),
                    visible: true,
                    opacity: 1.0,
                    blend_mode: BlendMode::Normal,
                    mask: None,
                    content: LayerContent::Fill {
                        color: slopshop_core::LinearRgba::new(0.0, 1.0, 0.0, 1.0),
                    },
                },
            })
            .unwrap();
        source_doc
            .layer_warnings
            .insert(second, vec![WARNING_BLEND_SPACE]);
        // Only the chosen layer, as it is (no group), with its own warning.
        documents.copy_layers(1, 2, Some(&[second.get()])).unwrap();
        let target = documents.get_mut(2).unwrap();
        let top = target.session.document().layers().last().unwrap().clone();
        assert_eq!(top.name, "second");
        assert_eq!(
            target.layer_warnings.get(&top.id),
            Some(&vec![WARNING_BLEND_SPACE])
        );
        target.session.undo().unwrap();
        assert!(documents.copy_layers(1, 2, Some(&[999])).is_err());

        documents.copy_layers(1, 2, None).unwrap();

        let target = documents.get_mut(2).unwrap();
        let top = target.session.document().layers().last().unwrap().clone();
        assert_eq!(top.name, "doc 1");
        let children = top.children().unwrap();
        assert_eq!(children.len(), 2);
        // The warning follows the copy of its layer.
        assert_eq!(
            target.layer_warnings.get(&children[1].id),
            Some(&vec![WARNING_BLEND_SPACE])
        );
        assert!(!target.layer_warnings.contains_key(&children[0].id));
        target.session.undo().unwrap();
        assert!(
            target
                .session
                .document()
                .layers()
                .iter()
                .all(|l| !l.is_group())
        );
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
            style: None,
            transform: slopshop_core::Affine::IDENTITY,
            clipped: false,
            id,
            name: "fill".to_owned(),
            visible: true,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            mask: None,
            content: LayerContent::Fill {
                color: LinearRgba::new(1.0, 0.0, 0.0, 1.0),
            },
        };
        let index = document.session.document().layers().len();
        document
            .session
            .perform(Edit::InsertLayer {
                parent: None,
                index,
                layer,
            })
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
        let view = documents.copy_layers(1, 2, None).unwrap();
        assert_eq!(view.layers.len(), before + source_layers);
        documents.tabs[1].session.undo().unwrap();
        assert_eq!(documents.tabs[1].session.document().layers().len(), before);
        assert!(documents.copy_layers(2, 2, None).is_err());
        assert_eq!(
            documents.copy_layers(9, 2, None).unwrap_err(),
            DOCUMENT_CLOSED
        );
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
        let size = Size::new(1920, 1080);
        let s = super::blank_session(size, Some([1.0, 1.0, 1.0]), "Arrière-plan").unwrap();
        assert_eq!(s.document().size(), size);
        assert_eq!(s.document().layers().len(), 1);
        assert!(!s.can_undo());
        let view = DocumentView::new(&s, &meta(), Vec::new());
        assert_eq!(view.layers[0].kind, "fill");
        assert_eq!(view.layers[0].name, "Arrière-plan");
    }

    #[test]
    fn a_transparent_blank_session_has_one_empty_raster_layer() {
        let size = Size::new(300_000, 2);
        let s = super::blank_session(size, None, "Layer 1").unwrap();
        assert_eq!(s.document().size(), size);
        let view = DocumentView::new(&s, &meta(), Vec::new());
        assert_eq!(view.layers.len(), 1);
        assert_eq!(view.layers[0].kind, "raster");
    }

    #[test]
    fn a_blank_session_is_1_to_300000_pixels_a_side() {
        for size in [Size::new(0, 10), Size::new(10, 0), Size::new(300_001, 10)] {
            assert!(
                super::blank_session(size, None, "Layer 1").is_err(),
                "{size:?}"
            );
        }
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
    fn batches_apply_together_and_undo_together() {
        let mut s = blank_session();
        let id = s.document().layers()[0].id.get();
        let json = format!(
            r#"{{"kind":"batch","edits":[
                {{"kind":"setLayerOpacity","id":{id},"opacity":0.5}},
                {{"kind":"renameLayer","id":{id},"name":"both"}}]}}"#
        );
        let edit = serde_json::from_str::<EditRequest>(&json)
            .unwrap()
            .into_edit(&mut s)
            .unwrap();
        s.perform(edit).unwrap();
        let layer = &s.document().layers()[0];
        assert_eq!((layer.opacity, layer.name.as_str()), (0.5, "both"));
        assert!(s.undo().unwrap());
        let layer = &s.document().layers()[0];
        assert_eq!(layer.opacity, 1.0);
        assert_ne!(layer.name, "both");
        assert!(!s.can_undo(), "one undo entry for the whole batch");

        // One invalid edit refuses the whole batch.
        let json = format!(
            r#"{{"kind":"batch","edits":[
                {{"kind":"setLayerOpacity","id":{id},"opacity":0.5}},
                {{"kind":"setLayerBlendMode","id":{id},"mode":"pinkify"}}]}}"#
        );
        assert!(
            serde_json::from_str::<EditRequest>(&json)
                .unwrap()
                .into_edit(&mut s)
                .is_err()
        );
    }

    #[test]
    fn group_requests_build_the_layer_tree() {
        let mut s = blank_session();
        let apply = |s: &mut Session, json: String| {
            let edit = serde_json::from_str::<EditRequest>(&json)
                .unwrap()
                .into_edit(s)
                .unwrap();
            s.perform(edit).unwrap();
        };
        for name in ["a", "b"] {
            apply(
                &mut s,
                format!(r#"{{"kind":"addFillLayer","name":"{name}","color":[1,0,0,1]}}"#),
            );
        }
        let ids: Vec<u64> = s.document().layers().iter().map(|l| l.id.get()).collect();
        apply(
            &mut s,
            format!(
                r#"{{"kind":"groupLayers","ids":[{},{}],"name":"G"}}"#,
                ids[1], ids[2]
            ),
        );
        let view = DocumentView::new(&s, &meta(), Vec::new());
        assert_eq!(view.layers.len(), 2);
        let group = &view.layers[1];
        // New groups are isolated.
        assert_eq!((group.kind, group.pass_through), ("group", false));
        assert_eq!(group.children.len(), 2);
        let g = group.id;

        // A new group inside, then the bottom layer moved into it; pass-through on.
        apply(
            &mut s,
            format!(r#"{{"kind":"addGroup","name":"inner","parent":{g},"index":2}}"#),
        );
        let inner = s
            .document()
            .layer(LayerId::from_raw(g))
            .unwrap()
            .children()
            .unwrap()[2]
            .id
            .get();
        apply(
            &mut s,
            format!(
                r#"{{"kind":"moveLayers","ids":[{}],"parent":{inner},"index":0}}"#,
                ids[0]
            ),
        );
        apply(
            &mut s,
            format!(r#"{{"kind":"setGroupPassThrough","id":{g},"passThrough":true}}"#),
        );
        apply(
            &mut s,
            format!(
                r#"{{"kind":"setLayerClipped","id":{},"clipped":true}}"#,
                ids[2]
            ),
        );
        assert!(
            s.document()
                .layer(LayerId::from_raw(ids[2]))
                .unwrap()
                .clipped
        );
        let view = DocumentView::new(&s, &meta(), Vec::new());
        assert_eq!(view.layers.len(), 1);
        assert!(view.layers[0].pass_through);
        assert_eq!(view.layers[0].children[2].children[0].id, ids[0]);

        apply(&mut s, format!(r#"{{"kind":"ungroup","ids":[{g}]}}"#));
        let names: Vec<&str> = s
            .document()
            .layers()
            .iter()
            .map(|l| l.name.as_str())
            .collect();
        assert_eq!(names, ["a", "b", "inner"]);

        // Fill layers have no edges: Align and Distribute move nothing, without an error.
        apply(
            &mut s,
            format!(
                r#"{{"kind":"alignLayers","ids":[{},{}],"align":"left"}}"#,
                ids[1], ids[2]
            ),
        );
        apply(
            &mut s,
            format!(
                r#"{{"kind":"distributeLayers","ids":[{},{}],"distribute":"verticalSpacing"}}"#,
                ids[1], ids[2]
            ),
        );

        for (arrange, expected) in [
            ("front", ["b", "inner", "a"]),
            ("back", ["a", "b", "inner"]),
        ] {
            apply(
                &mut s,
                format!(
                    r#"{{"kind":"arrangeLayers","ids":[{}],"arrange":"{arrange}"}}"#,
                    ids[1]
                ),
            );
            let names: Vec<&str> = s
                .document()
                .layers()
                .iter()
                .map(|l| l.name.as_str())
                .collect();
            assert_eq!(names, expected);
        }

        apply(
            &mut s,
            format!(
                r#"{{"kind":"duplicateLayers","ids":[{}],"nameFormat":"{{name}} copie"}}"#,
                ids[1]
            ),
        );
        let names: Vec<&str> = s
            .document()
            .layers()
            .iter()
            .map(|l| l.name.as_str())
            .collect();
        assert_eq!(names, ["a", "a copie", "b", "inner"]);
    }

    #[test]
    fn blend_requests_become_edits_and_unknown_ids_are_errors() {
        let mut s = blank_session();
        let id = s.document().layers()[0].id.get();
        let request = |json: String| {
            serde_json::from_str::<EditRequest>(&json)
                .unwrap()
                .into_edit(&mut blank_session())
        };
        let json = format!(r#"{{"kind":"setLayerBlendMode","id":{id},"mode":"softLight"}}"#);
        let edit = serde_json::from_str::<EditRequest>(&json)
            .unwrap()
            .into_edit(&mut s)
            .unwrap();
        s.perform(edit).unwrap();
        let space = r#"{"kind":"setBlendSpace","space":"linear"}"#;
        let edit = serde_json::from_str::<EditRequest>(space)
            .unwrap()
            .into_edit(&mut s)
            .unwrap();
        s.perform(edit).unwrap();
        let view = DocumentView::new(&s, &meta(), Vec::new());
        assert_eq!(view.layers[0].blend_mode, "softLight");
        assert_eq!(view.blend_space, "linear");
        assert!(
            request(format!(
                r#"{{"kind":"setLayerBlendMode","id":{id},"mode":"pinkify"}}"#
            ))
            .is_err()
        );
        assert!(request(r#"{"kind":"setBlendSpace","space":"cmyk"}"#.to_owned()).is_err());
    }

    #[test]
    fn image_requests_resize_rotate_and_reframe_the_canvas() {
        let mut s = blank_session();
        let size = s.document().size();
        let mut perform = |json: &str| {
            let edit = serde_json::from_str::<EditRequest>(json)
                .unwrap()
                .into_edit(&mut s)
                .unwrap();
            s.perform(edit).unwrap();
            s.document().size()
        };
        let turned = perform(r#"{"kind":"rotateImage","turn":"clockwise"}"#);
        assert_eq!(turned, Size::new(size.height, size.width));
        let resized = perform(r#"{"kind":"resizeImage","width":40,"height":30}"#);
        assert_eq!(resized, Size::new(40, 30));
        let canvas = perform(r#"{"kind":"canvasSize","width":50,"height":30,"anchor":[0.5,0.5]}"#);
        assert_eq!(canvas, Size::new(50, 30));
        let cropped = perform(r#"{"kind":"crop","x":-5,"y":2,"width":20,"height":10}"#);
        assert_eq!(cropped, Size::new(20, 10));
        let unknown =
            serde_json::from_str::<EditRequest>(r#"{"kind":"rotateImage","turn":"sideways"}"#)
                .unwrap()
                .into_edit(&mut blank_session());
        assert!(unknown.is_err());
        while s.undo().unwrap() {}
        assert_eq!(s.document().size(), size);
        // Image Size with a resolution (ADR 0028): one undo entry; without resampling, the
        // resolution alone.
        let resize = |s: &mut Session, json: &str| {
            let edit = serde_json::from_str::<EditRequest>(json)
                .unwrap()
                .into_edit(s)
                .unwrap();
            s.perform(edit).unwrap();
        };
        resize(
            &mut s,
            r#"{"kind":"resizeImage","width":40,"height":30,"resolution":300}"#,
        );
        assert_eq!(
            (s.document().size(), s.document().resolution()),
            (Size::new(40, 30), 300.0)
        );
        let (w, h) = (40, 30);
        resize(
            &mut s,
            &format!(r#"{{"kind":"resizeImage","width":{w},"height":{h},"resolution":150}}"#),
        );
        assert_eq!(
            (s.document().size(), s.document().resolution()),
            (Size::new(40, 30), 150.0)
        );
        s.undo().unwrap();
        assert_eq!(s.document().resolution(), 300.0);
        s.undo().unwrap();
        assert_eq!(
            (s.document().size(), s.document().resolution()),
            (size, slopshop_core::document::DEFAULT_RESOLUTION)
        );
    }

    #[test]
    fn adjustment_requests_add_and_change_adjustment_layers() {
        let mut s = blank_session();
        let edit = serde_json::from_str::<EditRequest>(
            r#"{"kind":"addAdjustmentLayer","name":"Levels 1","adjustment":"levels","parent":null,"index":1}"#,
        )
        .unwrap()
        .into_edit(&mut s)
        .unwrap();
        s.perform(edit).unwrap();
        let id = s.document().layers()[1].id;
        let json = format!(
            r#"{{"kind":"setAdjustment","id":{},"adjustment":"levels","values":[0.1,0.9,1.5,0,1]}}"#,
            id.get()
        );
        let edit = serde_json::from_str::<EditRequest>(&json)
            .unwrap()
            .into_edit(&mut s)
            .unwrap();
        s.perform(edit).unwrap();
        let view = DocumentView::new(&s, &meta(), Vec::new());
        assert_eq!(view.layers[1].kind, "adjustment");
        let adjustment = view.layers[1].adjustment.as_ref().unwrap();
        assert_eq!(adjustment.id, "levels");
        assert_eq!(adjustment.values[..6], [0.1, 0.9, 1.5, 0.0, 1.0, 0.0]);
        // More than five values, for the adjustments that have them.
        let json = format!(
            r#"{{"kind":"setAdjustment","id":{},"adjustment":"channelMixer","values":[0,0,100,0,0,100,0,0,100,0,0,0,1]}}"#,
            id.get()
        );
        let edit = serde_json::from_str::<EditRequest>(&json)
            .unwrap()
            .into_edit(&mut s)
            .unwrap();
        s.perform(edit).unwrap();
        let view = DocumentView::new(&s, &meta(), Vec::new());
        let adjustment = view.layers[1].adjustment.as_ref().unwrap();
        assert_eq!(adjustment.id, "channelMixer");
        assert_eq!(adjustment.values[12], 1.0);
        // Curves: their points.
        let json = format!(
            r#"{{"kind":"setAdjustment","id":{},"adjustment":"curves","values":[],"curves":[[[0,0],[128,160],[255,255]],[[0,0],[255,255]],[[0,0],[255,255]],[[30,0],[255,255]]]}}"#,
            id.get()
        );
        let edit = serde_json::from_str::<EditRequest>(&json)
            .unwrap()
            .into_edit(&mut s)
            .unwrap();
        s.perform(edit).unwrap();
        let view = DocumentView::new(&s, &meta(), Vec::new());
        let adjustment = view.layers[1].adjustment.as_ref().unwrap();
        assert_eq!(adjustment.id, "curves");
        let curves = adjustment.curves.as_ref().unwrap();
        assert_eq!(curves[0], [[0, 0], [128, 160], [255, 255]]);
        assert_eq!(curves[3], [[30, 0], [255, 255]]);
        let bad = format!(
            r#"{{"kind":"setAdjustment","id":{},"adjustment":"curves","values":[],"curves":[[[9,0],[3,255]],[],[],[]]}}"#,
            id.get()
        );
        let edit = serde_json::from_str::<EditRequest>(&bad)
            .unwrap()
            .into_edit(&mut s);
        assert!(edit.is_err());
        let too_many = format!(
            r#"{{"kind":"setAdjustment","id":{},"adjustment":"invert","values":[{}]}}"#,
            id.get(),
            ["0"; 38].join(",")
        );
        let edit = serde_json::from_str::<EditRequest>(&too_many)
            .unwrap()
            .into_edit(&mut s);
        assert!(edit.is_err());
        let unknown = serde_json::from_str::<EditRequest>(
            r#"{"kind":"addAdjustmentLayer","name":"x","adjustment":"colorLookup","parent":null,"index":0}"#,
        )
        .unwrap()
        .into_edit(&mut s);
        assert!(unknown.is_err());
    }

    #[test]
    fn auto_levels_requests_do_nothing_on_a_flat_image_and_know_their_corrections() {
        let mut s = blank_session();
        let ids: Vec<u64> = s.document().layers().iter().map(|l| l.id.get()).collect();
        let request = |correction: &str| {
            serde_json::from_str::<EditRequest>(&format!(
                r#"{{"kind":"autoLevels","ids":{ids:?},"correction":"{correction}"}}"#
            ))
            .unwrap()
        };
        // A white canvas: nothing to stretch, an empty edit (no undo entry).
        for correction in ["tone", "contrast", "color"] {
            let edit = request(correction).into_edit(&mut s).unwrap();
            assert!(
                matches!(edit, Edit::Batch(ref e) if e.is_empty()),
                "{correction}"
            );
        }
        assert!(request("equalize").into_edit(&mut s).is_err());
    }

    #[test]
    fn transform_requests_replace_the_gesture_so_far() {
        let mut s = blank_session();
        let id = s.document().layers()[0].id;
        // Live steps of one Free Transform, each the whole transform so far (`replace`).
        for matrix in ["[2,0,0,2,0,0]", "[0,1,-1,0,10,0]"] {
            let json = format!(
                r#"{{"kind":"transformLayers","ids":[{}],"matrix":{matrix}}}"#,
                id.get()
            );
            // As `perform_live` does: revert the gesture, then build the edit.
            s.cancel_gesture().unwrap();
            let edit = serde_json::from_str::<EditRequest>(&json)
                .unwrap()
                .into_edit(&mut s)
                .unwrap();
            s.perform_in_gesture(edit).unwrap();
        }
        // Only the last step applies: a quarter turn, exact.
        let transform = s.document().layer(id).unwrap().transform;
        assert_eq!(transform.to_array(), [0.0, 1.0, -1.0, 0.0, 10.0, 0.0]);
        s.end_gesture();
        assert!(s.undo().unwrap());
        assert!(s.document().layer(id).unwrap().transform.is_identity());
    }

    #[test]
    fn edit_requests_round_trip_through_undo() {
        let mut s = blank_session();
        let json = r#"{"kind":"addFillLayer","name":"Pink","color":[1.0,0.5,0.8,1.0]}"#;
        let request: EditRequest = serde_json::from_str(json).unwrap();
        let edit = request.into_edit(&mut s).unwrap();
        s.perform(edit).unwrap();
        let view = DocumentView::new(&s, &meta(), Vec::new());
        assert_eq!(view.layers.len(), 2);
        assert_eq!(view.layers[1].name, "Pink");
        // The swatch is converted back to sRGB for display: it must match what was sent.
        let swatch = view.layers[1].swatch;
        assert!((swatch[1] - 0.5).abs() < 1e-5, "swatch {swatch:?}");

        // Its color changes, as sent; undo brings the pink back.
        let id = view.layers[1].id;
        let json = format!(r#"{{"kind":"setFillColor","id":{id},"color":[0.0,0.25,1.0,1.0]}}"#);
        let request: EditRequest = serde_json::from_str(&json).unwrap();
        let edit = request.into_edit(&mut s).unwrap();
        s.perform(edit).unwrap();
        let swatch = DocumentView::new(&s, &meta(), Vec::new()).layers[1].swatch;
        assert!((swatch[1] - 0.25).abs() < 1e-5, "swatch {swatch:?}");
        s.undo().unwrap();

        // Placed below the pink layer.
        let json = r#"{"kind":"addFillLayer","name":"Below","color":[0,0,0,1],"index":1}"#;
        let request: EditRequest = serde_json::from_str(json).unwrap();
        let edit = request.into_edit(&mut s).unwrap();
        s.perform(edit).unwrap();
        let view = DocumentView::new(&s, &meta(), Vec::new());
        assert_eq!(view.layers[1].name, "Below");
        s.undo().unwrap();

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
            incomplete: true,
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
        assert_eq!(u32_at(12), 0b11);
        assert_eq!(u64::from_le_bytes(bytes[16..24].try_into().unwrap()), 7);
        assert_eq!(f64_at(24), 0.5);
        assert_eq!(f32::from_le_bytes(bytes[32..36].try_into().unwrap()), 1.5);
        assert_eq!(u32_at(36), 3);
        assert_eq!(f64_at(40), -12.5);
        assert_eq!(f64_at(48), 40.25);
    }

    #[test]
    fn selected_pixels_move_float_and_undo_by_drag() {
        use slopshop_core::selection::{Combine, EdgeOptions, Selection, Shape, select_shape};
        let size = Size::new(300, 200);
        let session = super::blank_session(size, None, "Layer 1").unwrap();
        let state = AppState::new();
        let doc = state.add_document(session, None, Vec::new()).unwrap();
        // Pixel (x, y) of the layer is [x, y, 9, 255]; columns 10 to 20 of rows 5 to 8 selected.
        let mut pixels = Vec::new();
        for y in 0..size.height {
            for x in 0..size.width {
                pixels.extend_from_slice(&[x as u8, y as u8, 9, 255]);
            }
        }
        let image =
            Arc::new(RasterImage::from_pixels(size, PixelFormat::RGBA8_SRGB, &pixels).unwrap());
        let shape = Shape::Rectangle {
            left: 10.0,
            top: 5.0,
            right: 20.0,
            bottom: 8.0,
        };
        let selected = select_shape(size, None, &shape, EdgeOptions::default(), Combine::Replace)
            .unwrap()
            .unwrap();
        let id = {
            let mut documents = state.documents().unwrap();
            let document = documents.get_mut(doc.id).unwrap();
            let id = document.session.allocate_layer_id();
            let layer = Layer {
                style: None,
                id,
                name: "photo".into(),
                visible: true,
                opacity: 1.0,
                blend_mode: BlendMode::Normal,
                mask: None,
                clipped: false,
                transform: slopshop_core::Affine::IDENTITY,
                content: LayerContent::raster(image),
            };
            document
                .session
                .perform(Edit::InsertLayer {
                    parent: None,
                    index: 1,
                    layer,
                })
                .unwrap();
            document
                .session
                .perform(Edit::SetSelection {
                    selection: Selection::new(Arc::new(selected)),
                })
                .unwrap();
            id
        };
        let request = |drag: u64, dx: i64, end: bool| move_pixels::MovePixelsRequest {
            drag,
            target: paint::PaintTarget::Layer,
            layer_id: id.get(),
            dx,
            dy: 0,
            copy: false,
            end,
        };
        let pixel = |state: &AppState, x: u32, y: u32| -> Vec<u8> {
            let mut documents = state.documents().unwrap();
            let document = documents.get_mut(doc.id).unwrap();
            let layer = document.session.document().layer(id).unwrap();
            let LayerContent::Raster { image, .. } = &layer.content else {
                panic!("a raster layer");
            };
            let image = image.get();
            let tile = image.levels()[0]
                .tile(slopshop_core::tile::TileCoord { col: 0, row: 0 })
                .unwrap();
            let at = ((y * 256 + x) * 4) as usize;
            tile[at..at + 4].to_vec()
        };
        let selection_left = |state: &AppState| {
            let mut documents = state.documents().unwrap();
            let document = documents.get_mut(doc.id).unwrap();
            let selection = document.session.document().selection().unwrap();
            slopshop_core::selection::bounds(selection.image())
                .unwrap()
                .x
        };
        // While dragged, the pixels only float in the view: the document does not change.
        let revision = {
            let mut documents = state.documents().unwrap();
            documents
                .get_mut(doc.id)
                .unwrap()
                .session
                .document()
                .revision()
        };
        let live = move_pixels::move_pixels(&state, doc.id, &request(1, 10, false)).unwrap();
        assert!(live.is_none());
        {
            let mut documents = state.documents().unwrap();
            let document = documents.get_mut(doc.id).unwrap();
            assert_eq!(document.session.document().revision(), revision);
            assert!(!document.session.document().layer(id).unwrap().is_painted());
            let (shown, shown_revision) = document.snapshot();
            assert_eq!(shown_revision, revision);
            let floating = shown.layer(id).unwrap();
            let LayerContent::Group { children, .. } = &floating.content else {
                panic!("the layer floats as a group");
            };
            assert_eq!(children.len(), 2);
            assert_eq!(children[1].transform.e, 10.0);
        }
        // The end moves them: the pixels and the selection, a hole left, the view as it was.
        let view = move_pixels::move_pixels(&state, doc.id, &request(1, 20, true))
            .unwrap()
            .unwrap();
        assert!(view.layers.last().unwrap().painted);
        {
            let mut documents = state.documents().unwrap();
            assert!(documents.get_mut(doc.id).unwrap().move_preview.is_none());
        }
        assert_eq!(pixel(&state, 30, 5), [10, 5, 9, 255]);
        assert_eq!(pixel(&state, 12, 6), [0, 0, 0, 0]);
        assert_eq!(selection_left(&state), 30);
        // The next drag moves them from where they started: what they covered comes back.
        move_pixels::move_pixels(&state, doc.id, &request(2, 5, true)).unwrap();
        assert_eq!(pixel(&state, 35, 5), [10, 5, 9, 255]);
        assert_eq!(pixel(&state, 32, 5), [32, 5, 9, 255]);
        assert_eq!(pixel(&state, 12, 6), [0, 0, 0, 0]);
        assert_eq!(selection_left(&state), 35);
        // One undo entry per drag.
        {
            let mut documents = state.documents().unwrap();
            documents.get_mut(doc.id).unwrap().session.undo().unwrap();
        }
        assert_eq!(pixel(&state, 30, 5), [10, 5, 9, 255]);
        assert_eq!(selection_left(&state), 30);
        {
            let mut documents = state.documents().unwrap();
            documents.get_mut(doc.id).unwrap().session.undo().unwrap();
        }
        assert_eq!(pixel(&state, 12, 6), [12, 6, 9, 255]);
        assert_eq!(selection_left(&state), 10);
        // A drag back where it started leaves no undo entry.
        move_pixels::move_pixels(&state, doc.id, &request(3, 4, false)).unwrap();
        move_pixels::move_pixels(&state, doc.id, &request(3, 0, true)).unwrap();
        let mut documents = state.documents().unwrap();
        let document = documents.get_mut(doc.id).unwrap();
        assert!(!document.session.document().layer(id).unwrap().is_painted());
        assert!(document.session.can_redo());
    }

    #[test]
    fn selected_pixels_moved_off_the_canvas_are_kept() {
        use slopshop_core::selection::{Combine, EdgeOptions, Selection, Shape, select_shape};
        use slopshop_core::tile::TileCoord;
        let size = Size::new(300, 200);
        let session = super::blank_session(size, None, "Layer 1").unwrap();
        let state = AppState::new();
        let doc = state.add_document(session, None, Vec::new()).unwrap();
        // Pixel (x, y) of the layer is [x, y, 9, 255] (x below 256, as bytes).
        let mut pixels = Vec::new();
        for y in 0..size.height {
            for x in 0..size.width {
                pixels.extend_from_slice(&[x as u8, y as u8, 9, 255]);
            }
        }
        let image =
            Arc::new(RasterImage::from_pixels(size, PixelFormat::RGBA8_SRGB, &pixels).unwrap());
        let select = |left: f64, right: f64| {
            let shape = Shape::Rectangle {
                left,
                top: 0.0,
                right,
                bottom: 4.0,
            };
            let image = select_shape(size, None, &shape, EdgeOptions::default(), Combine::Replace)
                .unwrap()
                .unwrap();
            Edit::SetSelection {
                selection: Selection::new(Arc::new(image)),
            }
        };
        let id = {
            let mut documents = state.documents().unwrap();
            let document = documents.get_mut(doc.id).unwrap();
            let id = document.session.allocate_layer_id();
            let layer = Layer {
                style: None,
                id,
                name: "photo".into(),
                visible: true,
                opacity: 1.0,
                blend_mode: BlendMode::Normal,
                mask: None,
                clipped: false,
                transform: slopshop_core::Affine::IDENTITY,
                content: LayerContent::raster(image),
            };
            let insert = Edit::InsertLayer {
                parent: None,
                index: 1,
                layer,
            };
            document.session.perform(insert).unwrap();
            document.session.perform(select(10.0, 20.0)).unwrap();
            id
        };
        let drag = |drag: u64, dx: i64| {
            let request = move_pixels::MovePixelsRequest {
                drag,
                target: paint::PaintTarget::Layer,
                layer_id: id.get(),
                dx,
                dy: 0,
                copy: false,
                end: true,
            };
            move_pixels::move_pixels(&state, doc.id, &request).unwrap();
        };
        // The layer's image, its original's size and its transform's x offset.
        let layer = |state: &AppState| {
            let mut documents = state.documents().unwrap();
            let document = documents.get_mut(doc.id).unwrap();
            let layer = document.session.document().layer(id).unwrap();
            let LayerContent::Raster { image, stack } = &layer.content else {
                panic!("a raster layer");
            };
            let original = stack.as_ref().unwrap().original().size();
            (image.get(), original, layer.transform.e)
        };
        let pixel = |image: &RasterImage, x: u32, y: u32| -> Vec<u8> {
            let coord = TileCoord {
                col: x / 256,
                row: y / 256,
            };
            let tile = image.levels()[0].tile(coord).unwrap();
            let at = (((y % 256) * 256 + x % 256) * 4) as usize;
            tile[at..at + 4].to_vec()
        };
        // Off the left edge, then something else (a new selection) ends the float.
        drag(1, -30);
        {
            let mut documents = state.documents().unwrap();
            let document = documents.get_mut(doc.id).unwrap();
            document.session.perform(select(0.0, 300.0)).unwrap();
        }
        let (image, original, x) = layer(&state);
        // One tile was added before the pixels; the layer still lies where it did.
        assert_eq!(
            (image.size(), original, x),
            (Size::new(556, 200), Size::new(556, 200), -256.0)
        );
        assert_eq!(pixel(&image, 256 - 20, 2), [10, 2, 9, 255]);
        assert_eq!(pixel(&image, 256 + 10, 2), [0, 0, 0, 0]);
        assert_eq!(pixel(&image, 256 + 25, 2), [25, 2, 9, 255]);
        // Off the right edge: the layer grows after its pixels, by a whole tile (a new float,
        // the canvas's width: what lies off the canvas is not selected, so it stays).
        drag(2, 40);
        let (image, original, _) = layer(&state);
        assert_eq!(
            (image.size(), original),
            (Size::new(812, 200), Size::new(812, 200))
        );
        assert_eq!(pixel(&image, 256 - 20, 2), [10, 2, 9, 255]);
        assert_eq!(pixel(&image, 256 + 5, 2), [0, 0, 0, 0]);
        assert_eq!(pixel(&image, 256 + 299 + 40, 2), [43, 2, 9, 255]);
        // Undo brings the layer back as it was, size and place.
        let mut documents = state.documents().unwrap();
        let document = documents.get_mut(doc.id).unwrap();
        for _ in 0..3 {
            document.session.undo().unwrap();
        }
        let layer = document.session.document().layer(id).unwrap();
        let LayerContent::Raster { image, stack } = &layer.content else {
            panic!("a raster layer");
        };
        assert_eq!((image.size(), stack.is_none()), (size, true));
        assert_eq!(layer.transform, slopshop_core::Affine::IDENTITY);
    }
}
