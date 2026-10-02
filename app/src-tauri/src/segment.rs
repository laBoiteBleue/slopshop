//! AI selection with SAM 2.1 (ADR 0025), through the `slopshop-ai` helper: Object Selection
//! (the object under the pointer lights up, a click or a box selects it) and Quick Selection
//! (brush strokes). The helper is started on first use and kept (the model stays loaded); the
//! image SAM sees is encoded once and reused until the document, the region or the sampled
//! layer changes, so hovering and clicking only decode (milliseconds).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use serde::Deserialize;
use slopshop_ai::{Client, Launch, MASK_SIDE, Point};
use slopshop_core::selection::{self as core_selection, Combine};
use slopshop_core::view::ViewTransform;
use slopshop_core::{Document, RasterImage, Rect, Size};
use tauri::ipc::Response;
use tauri::{AppHandle, Manager};

use crate::AppState;
use crate::ai::{self, AiFailure, Feature, Runtime};
use crate::ipc::DocumentView;
use crate::selection;

/// The longest side of the image SAM is given (it sees 1024² whatever it gets).
const SAM_SIDE: f64 = 1024.0;

/// Refine Edge: the side of the windows ViTMatte mattes, and how many at most (a longer
/// outline is matted coarser).
const MATTE_SIDE: u32 = 1024;
const MAX_MATTE_WINDOWS: usize = 48;

/// The helper, the image it has encoded, and the Quick Selection session under way.
#[derive(Default)]
pub(crate) struct SegmentState {
    session: Mutex<Session>,
}

#[derive(Default)]
struct Session {
    helper: Option<Client>,
    encoded: Option<Encoded>,
    quick: Option<QuickSession>,
    next_key: u64,
}

/// The image SAM has encoded: a region of a document as it was at a revision.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Encoded {
    document_id: u64,
    revision: u64,
    region: Rect,
    layer_id: Option<u64>,
    /// Document pixels per pixel SAM was given.
    scale: f64,
    /// The image's key in the helper.
    key: u64,
}

/// Strokes of one Quick Selection session refine one mask, combined with the selection there
/// was before the session.
struct QuickSession {
    /// The UI's session id.
    id: u64,
    base: Option<Arc<RasterImage>>,
    combine: Combine,
    /// The document's revision after the session's last result: another change ends it.
    revision: u64,
}

fn internal(e: impl ToString) -> AiFailure {
    AiFailure::new("internal", e)
}

/// The `slopshop-ai` executable: `SLOPSHOP_AI_HELPER`, else next to the app's (or one folder
/// up, as in development builds).
fn helper_executable() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("SLOPSHOP_AI_HELPER") {
        return Some(PathBuf::from(path));
    }
    let name = format!("slopshop-ai{}", std::env::consts::EXE_SUFFIX);
    let exe = std::env::current_exe().ok()?;
    let folder = exe.parent()?;
    [Some(folder), folder.parent()]
        .into_iter()
        .flatten()
        .map(|dir| dir.join(&name))
        .find(|path| path.is_file())
}

/// Starts the helper on this machine's runtime, if its components are installed.
fn start_helper(root: &Path) -> Result<Client, AiFailure> {
    let runtime = Runtime::detect().ok_or_else(|| AiFailure::new("unsupported", ""))?;
    let ids = runtime.components(Feature::Segmentation);
    for id in ids {
        let component = slopshop_ai::install::component(id).ok_or_else(|| internal(id))?;
        if !component.is_installed(root) {
            return Err(AiFailure::new("notInstalled", id));
        }
    }
    let name = format!(
        "/{}onnxruntime{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    let library = slopshop_ai::install::component(ids[0])
        .and_then(|c| c.files.iter().find(|f| f.path.ends_with(&name)))
        .map(|f| root.join(f.path))
        .ok_or_else(|| internal("no ONNX Runtime library in the runtime"))?;
    let folder = library.parent().ok_or_else(|| internal("runtime folder"))?;
    let executable = helper_executable()
        .ok_or_else(|| AiFailure::new("start", "the slopshop-ai helper was not found"))?;
    Client::start(&Launch {
        executable: &executable,
        runtime: &library,
        models: &root.join("models"),
        provider: runtime.provider(),
        library_paths: &[folder],
    })
    .map_err(|e| AiFailure::new("start", e))
}

/// Stops the helper (it frees its memory and its libraries), forgetting what it encoded.
pub(crate) fn stop(state: &AppState) {
    if let Ok(mut session) = state.segment.session.lock() {
        *session = Session::default();
    }
}

fn lock(state: &AppState) -> Result<MutexGuard<'_, Session>, AiFailure> {
    state.segment.session.lock().map_err(internal)
}

/// The document as it is now.
fn document(state: &AppState, document_id: u64) -> Result<Document, AiFailure> {
    let mut documents = state.documents().map_err(internal)?;
    Ok(documents
        .get_mut(document_id)
        .map_err(internal)?
        .session
        .document()
        .clone())
}

/// `[x, y, width, height]` within the canvas.
fn clamp_region(region: [u32; 4], canvas: Size) -> Result<Rect, AiFailure> {
    let [x, y, width, height] = region;
    Rect::new(x, y, width, height)
        .intersection(Rect::new(0, 0, canvas.width, canvas.height))
        .ok_or_else(|| internal("the region is outside the document"))
}

impl Session {
    /// The helper, started if needed.
    fn client(&mut self, root: &Path) -> Result<&mut Client, AiFailure> {
        match self.helper {
            Some(ref mut client) => Ok(client),
            None => Ok(self.helper.insert(start_helper(root)?)),
        }
    }

    /// A failed call leaves the helper in doubt: it is started again next time.
    fn failed(&mut self, e: impl ToString) -> AiFailure {
        *self = Session {
            next_key: self.next_key,
            ..Session::default()
        };
        AiFailure::new("model", e)
    }

    /// `region` of `doc` (only `layer_id`, if given) encoded by SAM: at once if it already is.
    fn encode(
        &mut self,
        state: &AppState,
        root: &Path,
        document_id: u64,
        doc: &Document,
        region: Rect,
        layer_id: Option<u64>,
    ) -> Result<Encoded, AiFailure> {
        let wanted = |e: &Encoded| {
            e.document_id == document_id
                && e.revision == doc.revision()
                && e.region == region
                && e.layer_id == layer_id
        };
        if let Some(encoded) = self.encoded.filter(wanted) {
            return Ok(encoded);
        }
        let source = selection::sampled_document(doc, layer_id).map_err(internal)?;
        let scale = (f64::from(region.width.max(region.height)) / SAM_SIDE).max(1.0);
        let output = Size::new(
            (f64::from(region.width) / scale).ceil().max(1.0) as u32,
            (f64::from(region.height) / scale).ceil().max(1.0) as u32,
        );
        let view = ViewTransform {
            origin: [f64::from(region.x), f64::from(region.y)],
            scale,
        };
        let frame = state
            .renderer()
            .map_err(internal)?
            .render_view(&source, view, output)
            .map_err(internal)?;
        let rgb: Vec<u8> = frame
            .data
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2]])
            .collect();
        let key = self.next_key;
        self.next_key += 1;
        self.encoded = None;
        let client = self.client(root)?;
        if let Err(e) = client.sam_encode(key, output.width, output.height, rgb) {
            return Err(self.failed(e));
        }
        let encoded = Encoded {
            document_id,
            revision: doc.revision(),
            region,
            layer_id,
            scale,
            key,
        };
        self.encoded = Some(encoded);
        Ok(encoded)
    }

    /// SAM's 256² logits over the encoded region for points and a box (document pixels).
    fn decode(
        &mut self,
        root: &Path,
        encoded: Encoded,
        points: &[(f64, f64, bool)],
        boxed: Option<[f64; 4]>,
    ) -> Result<Vec<f32>, AiFailure> {
        let to_input = |x: f64, y: f64| {
            (
                ((x - f64::from(encoded.region.x)) / encoded.scale) as f32,
                ((y - f64::from(encoded.region.y)) / encoded.scale) as f32,
            )
        };
        let points = points
            .iter()
            .map(|&(x, y, positive)| {
                let (x, y) = to_input(x, y);
                Point { x, y, positive }
            })
            .collect();
        let boxed = boxed.map(|[x0, y0, x1, y1]| {
            let (a, b) = to_input(x0, y0);
            let (c, d) = to_input(x1, y1);
            [a, b, c, d]
        });
        let client = self.client(root)?;
        match client.sam_decode(encoded.key, points, boxed) {
            Ok((logits, _score)) => Ok(logits),
            Err(e) => Err(self.failed(e)),
        }
    }
}

impl Session {
    /// The model's mask (`logits` over `region`) as a selection, its edge refined at full
    /// resolution by ViTMatte when `refine` is set, combined with `current` by `combine`.
    #[allow(clippy::too_many_arguments)]
    fn selection(
        &mut self,
        state: &AppState,
        root: &Path,
        doc: &Document,
        layer_id: Option<u64>,
        logits: &[f32],
        region: Rect,
        current: Option<&RasterImage>,
        combine: Combine,
        refine: bool,
    ) -> Result<Option<RasterImage>, AiFailure> {
        let canvas = doc.size();
        if !refine {
            return core_selection::select_logits(
                canvas, current, logits, MASK_SIDE, region, combine,
            )
            .map_err(internal);
        }
        let Some(mask) = core_selection::select_logits(
            canvas,
            None,
            logits,
            MASK_SIDE,
            region,
            Combine::Replace,
        )
        .map_err(internal)?
        else {
            return core_selection::select_logits(
                canvas, current, logits, MASK_SIDE, region, combine,
            )
            .map_err(internal);
        };
        // The band to decide: a model cell and a half on each side of the coarse outline.
        let cell = f64::from(region.width.max(region.height)) / MASK_SIDE as f64;
        let band = (cell * 1.5).clamp(8.0, 256.0) as u32;
        let mut plan =
            core_selection::plan_refinement(canvas, &mask, band, MATTE_SIDE, MAX_MATTE_WINDOWS)
                .map_err(internal)?;
        self.matte(state, root, doc, layer_id, &mut plan)?;
        plan.finish(current, combine).map_err(internal)
    }

    /// Mattes every window of `plan` with ViTMatte, on `doc` (only `layer_id`, if given).
    fn matte(
        &mut self,
        state: &AppState,
        root: &Path,
        doc: &Document,
        layer_id: Option<u64>,
        plan: &mut core_selection::EdgeRefinement,
    ) -> Result<(), AiFailure> {
        let source = selection::sampled_document(doc, layer_id).map_err(internal)?;
        let renderer = state.renderer().map_err(internal)?;
        for window in plan.windows().to_vec() {
            let size = window.input_size();
            let view = ViewTransform {
                origin: [f64::from(window.rect.x), f64::from(window.rect.y)],
                scale: f64::from(window.scale),
            };
            let frame = renderer
                .render_view(&source, view, size)
                .map_err(internal)?;
            let rgb: Vec<u8> = frame
                .data
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|p| [p[0], p[1], p[2]])
                .collect();
            let trimap = plan.trimap(&window);
            let client = self.client(root)?;
            let alpha = match client.matte(size.width, size.height, rgb, trimap.clone()) {
                Ok(alpha) => alpha,
                Err(e) => return Err(self.failed(e)),
            };
            plan.apply(&window, &trimap, &alpha);
        }
        Ok(())
    }
}

/// The AI folder, and the document.
fn prepare(app: &AppHandle, document_id: u64) -> Result<(PathBuf, Document), AiFailure> {
    let state = app.state::<AppState>();
    Ok((ai::folder(app)?, document(&state, document_id)?))
}

/// A prompt, in document pixels.
#[derive(Debug, Clone, Copy, Deserialize)]
pub(crate) struct PromptPoint {
    x: f64,
    y: f64,
    /// Part of the object (else: not part of it).
    positive: bool,
}

/// One Quick Selection request: every prompt of the session so far.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SegmentRequest {
    /// The UI's session: a new id starts over from the current selection.
    session: u64,
    points: Vec<PromptPoint>,
    /// What SAM sees, document pixels `[x, y, width, height]`: the view when zoomed in (finer),
    /// else the whole document.
    region: [u32; 4],
    /// Only that layer (placed as in the document), else the composited document.
    layer_id: Option<u64>,
    /// How the mask combines with the selection before the session (`replace`, `add`,
    /// `subtract`); read when the session starts.
    mode: String,
    /// Refine the edge at full resolution (ViTMatte).
    refine: bool,
}

/// Quick Selection: SAM 2.1's mask for the session's prompts, combined with the selection
/// before the session, as one undo entry.
#[tauri::command]
pub(crate) async fn ai_segment(
    app: AppHandle,
    document_id: u64,
    request: SegmentRequest,
) -> Result<DocumentView, AiFailure> {
    tauri::async_runtime::spawn_blocking(move || quick_selection(&app, document_id, &request))
        .await
        .map_err(internal)?
}

fn quick_selection(
    app: &AppHandle,
    document_id: u64,
    request: &SegmentRequest,
) -> Result<DocumentView, AiFailure> {
    let state = app.state::<AppState>();
    let (root, doc) = prepare(app, document_id)?;
    let region = clamp_region(request.region, doc.size())?;
    let mut session = lock(&state)?;
    let continuing = session
        .quick
        .as_ref()
        .is_some_and(|q| q.id == request.session && q.revision == doc.revision())
        && session.encoded.is_some_and(|e| {
            e.document_id == document_id && e.region == region && e.layer_id == request.layer_id
        });
    let encoded = match (continuing, session.encoded) {
        (true, Some(encoded)) => encoded,
        _ => {
            let combine = selection::combine(&request.mode).map_err(internal)?;
            session.quick = Some(QuickSession {
                id: request.session,
                base: doc.selection().map(|s| Arc::clone(s.image())),
                combine,
                revision: doc.revision(),
            });
            session.encode(&state, &root, document_id, &doc, region, request.layer_id)?
        }
    };
    let points: Vec<_> = request
        .points
        .iter()
        .map(|p| (p.x, p.y, p.positive))
        .collect();
    let logits = session.decode(&root, encoded, &points, None)?;
    let (base, combine) = {
        let quick = session
            .quick
            .as_ref()
            .ok_or_else(|| internal("no session"))?;
        (quick.base.clone(), quick.combine)
    };
    let image = session.selection(
        &state,
        &root,
        &doc,
        request.layer_id,
        &logits,
        region,
        base.as_deref(),
        combine,
        request.refine,
    )?;
    let view = selection::set_selection(&state, document_id, image).map_err(internal)?;
    if let Some(quick) = session.quick.as_mut() {
        quick.revision = view.revision;
    }
    // The encoded image stays valid: only the selection changed.
    if let Some(encoded) = session.encoded.as_mut() {
        encoded.revision = view.revision;
    }
    Ok(view)
}

/// Object Selection's hover: the object under document point (`x`, `y`), as SAM's mask over
/// `region`, raw binary: its side (`u32` little-endian, 0 when nothing is found) then one byte
/// per cell (0 or 255), rows from the top.
#[tauri::command]
pub(crate) async fn ai_object_hover(
    app: AppHandle,
    document_id: u64,
    x: f64,
    y: f64,
    region: [u32; 4],
    layer_id: Option<u64>,
) -> Result<Response, AiFailure> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let (root, doc) = prepare(&app, document_id)?;
        let region = clamp_region(region, doc.size())?;
        let mut session = lock(&state)?;
        let encoded = session.encode(&state, &root, document_id, &doc, region, layer_id)?;
        let logits = session.decode(&root, encoded, &[(x, y, true)], None)?;
        let mask = core_selection::logits_mask(&logits, MASK_SIDE);
        let mut bytes = Vec::with_capacity(4 + mask.len());
        if mask.iter().any(|&inside| inside) {
            bytes.extend_from_slice(&(MASK_SIDE as u32).to_le_bytes());
            bytes.extend(mask.iter().map(|&inside| if inside { 255u8 } else { 0 }));
        } else {
            bytes.extend_from_slice(&0u32.to_le_bytes());
        }
        Ok(Response::new(bytes))
    })
    .await
    .map_err(internal)?
}

/// Object Selection: the object at a point, or in a box (document pixels), combined with the
/// selection by `mode`, as one undo entry.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ObjectRequest {
    point: Option<[f64; 2]>,
    /// `[left, top, right, bottom]`.
    #[serde(rename = "box")]
    boxed: Option<[f64; 4]>,
    region: [u32; 4],
    layer_id: Option<u64>,
    mode: String,
    /// Refine the edge at full resolution (ViTMatte).
    refine: bool,
}

#[tauri::command]
pub(crate) async fn ai_object_select(
    app: AppHandle,
    document_id: u64,
    request: ObjectRequest,
) -> Result<DocumentView, AiFailure> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let combine = selection::combine(&request.mode).map_err(internal)?;
        let (root, doc) = prepare(&app, document_id)?;
        let region = clamp_region(request.region, doc.size())?;
        let mut session = lock(&state)?;
        let encoded = session.encode(&state, &root, document_id, &doc, region, request.layer_id)?;
        let points: Vec<_> = request
            .point
            .map(|[x, y]| (x, y, true))
            .into_iter()
            .collect();
        let logits = session.decode(&root, encoded, &points, request.boxed)?;
        let current = doc.selection().map(|s| Arc::clone(s.image()));
        let image = session.selection(
            &state,
            &root,
            &doc,
            request.layer_id,
            &logits,
            region,
            current.as_deref(),
            combine,
            request.refine,
        )?;
        let view = selection::set_selection(&state, document_id, image).map_err(internal)?;
        // Only the selection changed: the encoded image stays valid for the next hover.
        if let Some(encoded) = session.encoded.as_mut() {
            encoded.revision = view.revision;
        }
        Ok(view)
    })
    .await
    .map_err(internal)?
}

/// The largest Refine Edge radius, document pixels.
const MAX_REFINE_RADIUS: u32 = 256;

/// Select > Refine Edge: the current selection's edge matted at full resolution by ViTMatte,
/// within `radius` document pixels of its outline, as one undo entry.
#[tauri::command]
pub(crate) async fn ai_refine_selection(
    app: AppHandle,
    document_id: u64,
    radius: u32,
    layer_id: Option<u64>,
) -> Result<DocumentView, AiFailure> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let (root, doc) = prepare(&app, document_id)?;
        let Some(current) = doc.selection().map(|s| Arc::clone(s.image())) else {
            return Err(internal("nothing is selected"));
        };
        let radius = radius.clamp(1, MAX_REFINE_RADIUS);
        let mut plan = core_selection::plan_refinement(
            doc.size(),
            &current,
            radius,
            MATTE_SIDE,
            MAX_MATTE_WINDOWS,
        )
        .map_err(internal)?;
        let mut session = lock(&state)?;
        session.matte(&state, &root, &doc, layer_id, &mut plan)?;
        let image = plan.finish(None, Combine::Replace).map_err(internal)?;
        let view = selection::set_selection(&state, document_id, image).map_err(internal)?;
        // Only the selection changed: an image encoded from this revision stays valid.
        if let Some(encoded) = session.encoded.as_mut()
            && encoded.document_id == document_id
            && encoded.revision == doc.revision()
        {
            encoded.revision = view.revision;
        }
        Ok(view)
    })
    .await
    .map_err(internal)?
}
