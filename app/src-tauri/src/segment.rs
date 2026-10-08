//! AI selection with SAM 2.1 (ADR 0025), through the `slopshop-ai` helper: Object Selection
//! (the object under the pointer lights up, a click or a box selects it) and Quick Selection
//! (brush strokes). The helper is started on first use and kept (the model stays loaded); the
//! image SAM sees is encoded once and reused until the document, the region or the sampled
//! layer changes, so hovering and clicking only decode (milliseconds).

use slopshop_core::HistoryLabel;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use slopshop_ai::{Client, Launch, MASK_SIDE, Point};
use slopshop_core::selection::{self as core_selection, Combine};
use slopshop_core::view::ViewTransform;
use slopshop_core::{CancelToken, Document, RasterImage, Rect, Size};
use tauri::ipc::Response;
use tauri::{AppHandle, Manager};

use crate::AppState;
use crate::ai::{self, AiFailure, Feature, Runtime};
use crate::ipc::DocumentView;
use crate::selection;

/// The progress of an AI request the UI follows (`AiProgress` in engine.ts).
const EVENT_AI_PROGRESS: &str = "ai-progress";

#[derive(Debug, Clone, Serialize)]
struct AiProgress {
    task: u64,
    /// `select` (the model), then `refine` (Refine Edge's windows), for the UI's wording.
    stage: &'static str,
    done: u64,
    total: u64,
}

/// An AI request the UI follows and can cancel (`ai_cancel`): its steps (a model run, a window
/// of Refine Edge) are reported as they are done, and cancellation is checked between them.
/// A model run under way finishes first. Other long requests (Color Range) report the same way.
pub(crate) struct Task<'a> {
    app: &'a AppHandle,
    id: u64,
    stage: &'static str,
    cancel: CancelToken,
    done: u64,
    total: u64,
}

impl<'a> Task<'a> {
    pub(crate) fn start(app: &'a AppHandle, id: u64) -> Self {
        let cancel = CancelToken::new();
        if let Ok(mut tasks) = app.state::<AppState>().segment.tasks.lock() {
            tasks.insert(id, cancel.clone());
        }
        Self {
            app,
            id,
            stage: "select",
            cancel,
            done: 0,
            total: 0,
        }
    }

    /// `steps` more steps to come.
    fn expect(&mut self, steps: u64) {
        self.total += steps;
        self.report();
    }

    /// A step is done; stops here if the request was cancelled.
    fn step(&mut self) -> Result<(), AiFailure> {
        self.done = (self.done + 1).min(self.total);
        self.report();
        self.check()
    }

    fn check(&self) -> Result<(), AiFailure> {
        if self.cancel.is_cancelled() {
            Err(AiFailure::new("cancelled", ""))
        } else {
            Ok(())
        }
    }

    /// `done` of `total` steps reported from any thread, at most every 50 ms (or the last).
    pub(crate) fn shared(&self, stage: &'static str) -> SharedProgress<'a> {
        SharedProgress {
            app: self.app,
            id: self.id,
            stage,
            last: Mutex::new(None),
        }
    }

    /// Whether the UI cancelled the request.
    pub(crate) fn cancel_token(&self) -> &CancelToken {
        &self.cancel
    }

    fn report(&self) {
        let progress = AiProgress {
            task: self.id,
            stage: self.stage,
            done: self.done,
            total: self.total,
        };
        crate::emit(self.app, EVENT_AI_PROGRESS, &progress);
    }
}

/// [`Task::shared`].
pub(crate) struct SharedProgress<'a> {
    app: &'a AppHandle,
    id: u64,
    stage: &'static str,
    last: Mutex<Option<std::time::Instant>>,
}

impl SharedProgress<'_> {
    pub(crate) fn report(&self, done: usize, total: usize) {
        let now = std::time::Instant::now();
        let Ok(mut last) = self.last.lock() else {
            return;
        };
        let due = last.is_none_or(|t| now.duration_since(t).as_millis() >= 50);
        if !due && done < total {
            return;
        }
        *last = Some(now);
        let progress = AiProgress {
            task: self.id,
            stage: self.stage,
            done: done as u64,
            total: total as u64,
        };
        crate::emit(self.app, EVENT_AI_PROGRESS, &progress);
    }
}

impl Drop for Task<'_> {
    fn drop(&mut self) {
        if let Ok(mut tasks) = self.app.state::<AppState>().segment.tasks.lock() {
            tasks.remove(&self.id);
        }
    }
}

/// Cancels the AI request `task`: it stops at its next step, changing nothing.
#[tauri::command]
pub(crate) fn ai_cancel(app: AppHandle, task: u64) {
    if let Ok(tasks) = app.state::<AppState>().segment.tasks.lock()
        && let Some(cancel) = tasks.get(&task)
    {
        cancel.cancel();
    }
}

/// The longest side of the image SAM is given (it sees 1024² whatever it gets).
const SAM_SIDE: f64 = 1024.0;

/// Refine Edge: the side of the windows ViTMatte mattes, and how many at most (a longer
/// outline is matted coarser). Its time follows the pixels it is given (about a megapixel a
/// second for ViTMatte-B on an RTX 4090 Laptop with DirectML): on a 24 MP portrait, 34 windows
/// of 512 seen at half resolution took 8.4 s against 21.7 s for 34 windows of 1024 at full
/// resolution, with hair strands alike at 100 %.
const MATTE_SIDE: u32 = 512;
const MAX_MATTE_WINDOWS: usize = 40;

/// The helper and the image it has encoded.
#[derive(Default)]
pub(crate) struct SegmentState {
    session: Mutex<Session>,
    /// The AI requests under way, by the UI's task id.
    tasks: Mutex<HashMap<u64, CancelToken>>,
}

#[derive(Default)]
pub(crate) struct Session {
    helper: Option<Client>,
    encoded: Option<Encoded>,
    next_key: u64,
    /// When the helper was last asked for something.
    last_used: Option<std::time::Instant>,
}

/// The helper stops after this long unused, freeing its memory (several GB of RAM and VRAM with
/// the models loaded); the next request starts it again, loading the models (a few seconds).
const HELPER_IDLE: std::time::Duration = std::time::Duration::from_secs(120);

/// Stops the helper if it has been unused for [`HELPER_IDLE`]; never while a request holds it.
/// Called periodically by the app.
pub(crate) fn stop_if_idle(state: &AppState) {
    let Ok(mut session) = state.segment.session.try_lock() else {
        return;
    };
    let idle = session
        .last_used
        .is_some_and(|used| used.elapsed() >= HELPER_IDLE);
    if session.helper.is_some() && idle {
        *session = Session::default();
    }
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

/// Fails with `notInstalled` (naming the feature, for the UI to offer the download) unless
/// every component `feature` needs is installed; `unsupported` where it is not offered.
pub(crate) fn require(root: &Path, feature: Feature) -> Result<(), AiFailure> {
    let runtime = Runtime::detect().ok_or_else(|| AiFailure::new("unsupported", ""))?;
    let ids = runtime.components(feature);
    if ids.is_empty() {
        return Err(AiFailure::new("unsupported", ""));
    }
    for id in ids {
        let component = slopshop_ai::install::component(id).ok_or_else(|| internal(id))?;
        if !component.is_installed(root) {
            let name = match feature {
                Feature::Segmentation => "segmentation",
                Feature::Subject => "subject",
                Feature::Erase => "erase",
            };
            return Err(AiFailure::new("notInstalled", name));
        }
    }
    Ok(())
}

/// Stops the helper (it frees its memory and its libraries), forgetting what it encoded.
pub(crate) fn stop(state: &AppState) {
    if let Ok(mut session) = state.segment.session.lock() {
        *session = Session::default();
    }
}

pub(crate) fn lock(state: &AppState) -> Result<MutexGuard<'_, Session>, AiFailure> {
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

/// What the models see where the image is transparent: a flat mid gray (sRGB 128), not the
/// view's checkerboard, whose squares they would take for texture against the subject.
const MODEL_BACKGROUND: u8 = 128;

/// `source` seen through `view` as `output`-sized 8-bit sRGB pixels for a model, without alpha:
/// transparent parts over [`MODEL_BACKGROUND`].
pub(crate) fn model_rgb(
    state: &AppState,
    source: &Document,
    view: ViewTransform,
    output: Size,
) -> Result<Vec<u8>, AiFailure> {
    let frame = state
        .renderer()
        .map_err(internal)?
        .render_view_transparent(source, view, output)
        .map_err(internal)?;
    Ok(over_background(&frame.data))
}

/// Straight-alpha 8-bit sRGB pixels flattened over [`MODEL_BACKGROUND`] in linear light (as the
/// view composites), to RGB.
fn over_background(rgba: &[u8]) -> Vec<u8> {
    let decode = |c: u8| {
        let c = f32::from(c) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let encode = |l: f32| {
        let c = if l <= 0.003_130_8 {
            l * 12.92
        } else {
            1.055 * l.powf(1.0 / 2.4) - 0.055
        };
        (c.clamp(0.0, 1.0) * 255.0).round() as u8
    };
    let linear: Vec<f32> = (0..=255).map(decode).collect();
    let background = linear[usize::from(MODEL_BACKGROUND)];
    rgba.as_chunks::<4>()
        .0
        .iter()
        .flat_map(|&[r, g, b, a]| match a {
            255 => [r, g, b],
            0 => [MODEL_BACKGROUND; 3],
            _ => {
                let a = f32::from(a) / 255.0;
                let over = |c: u8| encode(linear[usize::from(c)] * a + background * (1.0 - a));
                [over(r), over(g), over(b)]
            }
        })
        .collect()
}

/// Where Select > Subject looks: where `source` (the sampled layer, or every layer) can show
/// on the canvas, or the whole canvas when nothing can.
fn subject_region(source: &Document) -> Rect {
    slopshop_core::pick::canvas_extent(source).unwrap_or_else(|| source.size().bounds())
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
    pub(crate) fn client(&mut self, root: &Path) -> Result<&mut Client, AiFailure> {
        self.last_used = Some(std::time::Instant::now());
        match self.helper {
            Some(ref mut client) => Ok(client),
            None => Ok(self.helper.insert(start_helper(root)?)),
        }
    }

    /// A failed call leaves the helper in doubt: it is started again next time.
    pub(crate) fn failed(&mut self, e: impl ToString) -> AiFailure {
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
        let rgb = model_rgb(state, &source, view, output)?;
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
    /// `soft`: the mask keeps the model's probabilities (BiRefNet's thin strands) instead of
    /// being cut at one half (SAM's masks).
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
        soft: bool,
        task: &mut Task<'_>,
    ) -> Result<Option<RasterImage>, AiFailure> {
        // The model's square mask: SAM's 256², BiRefNet's 1024².
        let side = (logits.len() as f64).sqrt() as usize;
        let canvas = doc.size();
        let select = |current: Option<&RasterImage>, combine: Combine| {
            if soft {
                core_selection::select_logits_soft(canvas, current, logits, side, region, combine)
            } else {
                core_selection::select_logits(canvas, current, logits, side, region, combine)
            }
            .map_err(internal)
        };
        if !refine {
            return select(current, combine);
        }
        let Some(mask) = select(None, Combine::Replace)? else {
            return select(current, combine);
        };
        // The band to decide, in model cells around the coarse outline: narrow inside it; outside,
        // narrow too for a soft mask (its partly covered pixels, BiRefNet's strands, are
        // decided anyway), wider for a hard one (SAM's), where hair and fur reach past it. A
        // wide band over plain background lets the matting model leave it half selected.
        let longest = f64::from(region.width.max(region.height));
        let cell = longest / side as f64;
        let outward = if soft { cell * 2.0 } else { cell * 6.0 };
        let band = core_selection::RefineBand {
            inward: (cell * 2.0).clamp(16.0, 64.0) as u32,
            outward: outward.clamp(16.0, 128.0) as u32,
        };
        let mut plan =
            core_selection::plan_refinement(canvas, &mask, band, MATTE_SIDE, MAX_MATTE_WINDOWS)
                .map_err(internal)?;
        self.matte(state, root, doc, layer_id, &mut plan, task)?;
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
        task: &mut Task<'_>,
    ) -> Result<(), AiFailure> {
        task.stage = "refine";
        task.expect(plan.windows().len() as u64);
        let source = selection::sampled_document(doc, layer_id).map_err(internal)?;
        for window in plan.windows().to_vec() {
            let size = window.input_size();
            let view = ViewTransform {
                origin: [f64::from(window.rect.x), f64::from(window.rect.y)],
                scale: f64::from(window.scale),
            };
            let rgb = model_rgb(state, &source, view, size)?;
            let trimap = plan.trimap(&window);
            let client = self.client(root)?;
            let alpha = match client.matte(size.width, size.height, rgb, trimap.clone()) {
                Ok(alpha) => alpha,
                Err(e) => return Err(self.failed(e)),
            };
            plan.apply(&window, &trimap, &alpha);
            task.step()?;
        }
        Ok(())
    }
}

/// The AI folder, and the document.
pub(crate) fn prepare(app: &AppHandle, document_id: u64) -> Result<(PathBuf, Document), AiFailure> {
    let state = app.state::<AppState>();
    Ok((ai::folder(app)?, document(&state, document_id)?))
}

/// A prompt, in document pixels.
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
        require(&root, Feature::Segmentation)?;
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
    /// The UI's task, to follow and cancel (`ai_cancel`).
    task: u64,
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
        require(&root, Feature::Segmentation)?;
        let region = clamp_region(request.region, doc.size())?;
        let mut task = Task::start(&app, request.task);
        task.expect(1);
        let mut session = lock(&state)?;
        let encoded = session.encode(&state, &root, document_id, &doc, region, request.layer_id)?;
        task.step()?;
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
            false,
            &mut task,
        )?;
        task.check()?;
        let view = selection::set_selection(
            &state,
            document_id,
            image,
            HistoryLabel::new("objectSelection"),
        )
        .map_err(internal)?;
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

/// `current`'s edge matted at full resolution by ViTMatte within `radius` document pixels of
/// its outline and wherever `region` covers at least half (Select and Mask's edge detection and
/// refine-edge brush).
#[allow(clippy::too_many_arguments)]
fn matted(
    state: &AppState,
    root: &std::path::Path,
    session: &mut Session,
    doc: &slopshop_core::Document,
    current: &RasterImage,
    radius: u32,
    region: Option<&RasterImage>,
    layer_id: Option<u64>,
    task: &mut Task,
) -> Result<Option<RasterImage>, AiFailure> {
    let radius = radius.clamp(1, MAX_REFINE_RADIUS);
    let mut plan = core_selection::plan_refinement_with(
        doc.size(),
        current,
        core_selection::RefineBand::both(radius),
        region,
        MATTE_SIDE,
        MAX_MATTE_WINDOWS,
    )
    .map_err(internal)?;
    session.matte(state, root, doc, layer_id, &mut plan, task)?;
    task.check()?;
    plan.finish(None, Combine::Replace).map_err(internal)
}

/// Select and Mask's edge detection: the selection it was opened with matted by ViTMatte within
/// `radius` pixels of its outline and wherever the refine-edge brush painted, which becomes the
/// base its settings apply to. The panel then shows them on it again (`refine_preview`).
#[tauri::command]
pub(crate) async fn ai_refine_base(
    app: AppHandle,
    document_id: u64,
    radius: u32,
    layer_id: Option<u64>,
    task: u64,
) -> Result<(), AiFailure> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let (root, doc) = prepare(&app, document_id)?;
        require(&root, Feature::Segmentation)?;
        let (original, region) = {
            let mut documents = state.documents().map_err(internal)?;
            let document = documents.get_mut(document_id).map_err(internal)?;
            let session = document
                .refine
                .as_ref()
                .ok_or_else(|| internal("Select and Mask is not open"))?;
            (Arc::clone(&session.original), session.region.clone())
        };
        let mut task = Task::start(&app, task);
        let mut session = lock(&state)?;
        let image = matted(
            &state,
            &root,
            &mut session,
            &doc,
            &original,
            radius,
            region.as_deref(),
            layer_id,
            &mut task,
        )?;
        let image = match image {
            Some(image) => image,
            None => core_selection::uniform_mask(doc.size(), false).map_err(internal)?,
        };
        let mut documents = state.documents().map_err(internal)?;
        let document = documents.get_mut(document_id).map_err(internal)?;
        // The panel may have closed meanwhile.
        if let Some(session) = document.refine.as_mut() {
            session.base = Arc::new(image);
        }
        Ok(())
    })
    .await
    .map_err(internal)?
}

/// Select > Subject: BiRefNet's mask of the image's main subject (where the sampled layer, or
/// every layer, shows on the canvas, at most 1024 pixels on a side), refined at full resolution
/// when `refine` is set, combined with the selection by `mode`, as one undo entry.
#[tauri::command]
pub(crate) async fn ai_select_subject(
    app: AppHandle,
    document_id: u64,
    layer_id: Option<u64>,
    mode: String,
    refine: bool,
    task: u64,
) -> Result<DocumentView, AiFailure> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let combine = selection::combine(&mode).map_err(internal)?;
        let (root, doc) = prepare(&app, document_id)?;
        require(&root, Feature::Subject)?;
        let mut task = Task::start(&app, task);
        task.expect(1);
        let source = selection::sampled_document(&doc, layer_id).map_err(internal)?;
        // Where the layer (or the layers) can show, at its own resolution up to the model's:
        // a small layer on a large canvas is not seen as a thumbnail.
        let region = subject_region(&source);
        let scale = (f64::from(region.width.max(region.height)) / SAM_SIDE).max(1.0);
        let output = Size::new(
            (f64::from(region.width) / scale).ceil().max(1.0) as u32,
            (f64::from(region.height) / scale).ceil().max(1.0) as u32,
        );
        let view = ViewTransform {
            origin: [f64::from(region.x), f64::from(region.y)],
            scale,
        };
        let rgb = model_rgb(&state, &source, view, output)?;
        let mut session = lock(&state)?;
        let client = session.client(&root)?;
        let logits = match client.subject(output.width, output.height, rgb) {
            Ok(logits) => logits,
            Err(e) => return Err(session.failed(e)),
        };
        task.step()?;
        let current = doc.selection().map(|s| Arc::clone(s.image()));
        let image = session.selection(
            &state,
            &root,
            &doc,
            layer_id,
            &logits,
            region,
            current.as_deref(),
            combine,
            refine,
            true,
            &mut task,
        )?;
        task.check()?;
        let view = selection::set_selection(
            &state,
            document_id,
            image,
            HistoryLabel::new("selectSubject"),
        )
        .map_err(internal)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use slopshop_core::{Affine, BlendMode, Edit, Layer, LayerContent};

    #[test]
    fn transparency_is_seen_over_flat_gray_not_the_checkerboard() {
        let rgba = [
            10, 20, 30, 255, // opaque: as it is
            10, 20, 30, 0, // transparent: the background
            255, 255, 255, 128, // half white: between, in linear light
        ];
        let rgb = over_background(&rgba);
        assert_eq!(&rgb[..6], &[10, 20, 30, 128, 128, 128]);
        // Alpha 128 of white (1.0) over sRGB 128 (0.216) is 0.610 in linear light: sRGB 205.
        assert_eq!(&rgb[6..], &[205, 205, 205]);
    }

    #[test]
    fn select_subject_looks_where_the_layer_is() {
        let mut doc = Document::new(Size::new(100, 80));
        let image = RasterImage::from_pixels(
            Size::new(20, 10),
            slopshop_core::color::PixelFormat::RGBA8_SRGB,
            &[255; 20 * 10 * 4],
        )
        .expect("a 20 x 10 image");
        let layer = Layer {
            style: None,
            id: doc.allocate_layer_id(),
            name: "small".into(),
            visible: true,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            content: LayerContent::raster(Arc::new(image)),
            mask: None,
            clipped: false,
            transform: Affine::translation(30.0, 40.0).into(),
        };
        Edit::InsertLayer {
            parent: None,
            index: 0,
            layer,
        }
        .apply(&mut doc)
        .expect("a new layer");
        assert_eq!(subject_region(&doc), Rect::new(30, 40, 20, 10));
        // Nothing on the canvas: the whole canvas.
        assert_eq!(
            subject_region(&Document::new(Size::new(100, 80))),
            Rect::new(0, 0, 100, 80)
        );
    }
}
