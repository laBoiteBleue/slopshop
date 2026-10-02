//! Selection commands (ADR 0024): thin wrappers over `slopshop_core::selection`. The masks are
//! built on a worker thread, then set on the document as one undoable edit; the UI only ever
//! receives the outline, sized to the view.

use std::sync::{Arc, Mutex};

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

pub(crate) fn combine(id: &str) -> Result<Combine, String> {
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
pub(crate) fn set_selection(
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

pub(crate) async fn on_worker<T: Send + 'static>(
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

/// The Magic Wand at document pixel (`x`, `y`): similar colors, within `tolerance` (0–255),
/// connected or not, combined by `mode`. It samples the composited document, or with
/// `layer_id` only that layer (placed as in the document).
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn magic_wand(
    state: State<'_, AppState>,
    document_id: u64,
    x: u32,
    y: u32,
    tolerance: f32,
    contiguous: bool,
    anti_alias: bool,
    layer_id: Option<u64>,
    mode: String,
) -> Result<DocumentView, String> {
    let combine = combine(&mode)?;
    let (source, current) = {
        let mut documents = state.documents()?;
        let doc = documents.get_mut(document_id)?.session.document();
        let current = doc.selection().map(|s| Arc::clone(s.image()));
        (sampled_document(doc, layer_id)?, current)
    };
    let options = selection::WandOptions {
        tolerance,
        contiguous,
        anti_alias,
    };
    let image = on_worker(move || {
        selection::magic_wand(&source, current.as_deref(), (x, y), options, combine)
            .map_err(|e| e.to_string())
    })
    .await?;
    set_selection(&state, document_id, image)
}

/// Quick Selection works on the region in view at most this many pixels on a side (the colors
/// as shown, rendered by the GPU): the cut costs time in proportion to its pixels.
const QUICK_SIDE: f64 = 1600.0;

/// A Quick Selection stroke (ADR 0026), sent again while it is painted.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickRequest {
    /// The UI's stroke: requests of one stroke share its image and the selection before it.
    stroke: u64,
    /// The brush's path so far and its radius, document pixels.
    points: Vec<[f64; 2]>,
    radius: f64,
    /// The region to work on, document pixels `[x, y, width, height]`: the view when zoomed in
    /// (finer), else the whole document. Read at the stroke's first request.
    region: [u32; 4],
    /// Only that layer (placed as in the document), else the composited document.
    layer_id: Option<u64>,
    /// `replace` (a new selection), `add` or `subtract`.
    mode: String,
    /// The stroke goes on: its selection is shown, and replaced by the next request. Otherwise
    /// it is done: one undo entry.
    live: bool,
}

/// What the requests of the stroke under way share: the image the cut works on, rendered once,
/// and the selection before the stroke (at full resolution and on that image's grid).
#[derive(Default)]
pub(crate) struct QuickState {
    stroke: Mutex<Option<Arc<QuickStroke>>>,
}

struct QuickStroke {
    id: u64,
    document_id: u64,
    canvas: Size,
    region: Rect,
    scale: f64,
    width: usize,
    height: usize,
    rgb: Vec<u8>,
    selected: Vec<bool>,
    base: Option<Arc<RasterImage>>,
}

impl QuickStroke {
    /// Renders `region` of the document (only `layer_id`, if given) at most [`QUICK_SIDE`] on a
    /// side, and the selection on that grid.
    fn new(
        state: &AppState,
        id: u64,
        document_id: u64,
        region: [u32; 4],
        layer_id: Option<u64>,
    ) -> Result<Self, String> {
        let (doc, base) = {
            let mut documents = state.documents()?;
            let doc = documents.get_mut(document_id)?.session.document().clone();
            let base = doc.selection().map(|s| Arc::clone(s.image()));
            (doc, base)
        };
        let canvas = doc.size();
        let [x, y, w, h] = region;
        let region = Rect::new(x, y, w, h)
            .intersection(canvas.bounds())
            .ok_or("empty region")?;
        let source = sampled_document(&doc, layer_id)?;
        let scale = (f64::from(region.width.max(region.height)) / QUICK_SIDE).max(1.0);
        let output = Size::new(
            (f64::from(region.width) / scale).ceil().max(1.0) as u32,
            (f64::from(region.height) / scale).ceil().max(1.0) as u32,
        );
        let view = slopshop_core::view::ViewTransform {
            origin: [f64::from(region.x), f64::from(region.y)],
            scale,
        };
        let frame = state
            .renderer()?
            .render_view(&source, view, output)
            .map_err(|e| e.to_string())?;
        let rgb: Vec<u8> = frame
            .data
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2]])
            .collect();
        let (width, height) = (output.width as usize, output.height as usize);
        // Inside where the selection's coverage reaches one half.
        let selected: Vec<bool> = match &base {
            Some(mask) => (0..width * height)
                .map(|p| {
                    let sx = f64::from(region.x) + ((p % width) as f64 + 0.5) * scale;
                    let sy = f64::from(region.y) + ((p / width) as f64 + 0.5) * scale;
                    mask.gray_at(sx as u32, sy as u32) >= 0.5
                })
                .collect(),
            None => vec![false; width * height],
        };
        Ok(Self {
            id,
            document_id,
            canvas,
            region,
            scale,
            width,
            height,
            rgb,
            selected,
            base,
        })
    }

    /// The selection after the stroke along `points` (document pixels), combined by `mode`.
    fn select(
        &self,
        points: &[[f64; 2]],
        radius: f64,
        mode: &str,
    ) -> Result<Option<RasterImage>, String> {
        use slopshop_core::quick_select::{self as quick, QuickImage, QuickMode};
        let (combine, mode) = match mode {
            "add" => (Combine::Add, QuickMode::Add),
            "subtract" => (Combine::Subtract, QuickMode::Subtract),
            _ => (Combine::Replace, QuickMode::New),
        };
        let (w, h, region, scale) = (self.width, self.height, self.region, self.scale);
        let points: Vec<[f64; 2]> = points
            .iter()
            .map(|[px, py]| {
                [
                    (px - f64::from(region.x)) / scale,
                    (py - f64::from(region.y)) / scale,
                ]
            })
            .collect();
        let stroke = quick::brush(w, h, &points, radius / scale);
        let image = QuickImage {
            width: w,
            height: h,
            rgb: &self.rgb,
        };
        let changed = quick::quick_select(&image, &self.selected, &stroke, mode);
        let scores = quick::change_scores(w, h, &changed, &self.selected, mode);
        let base = self.base.as_deref();
        selection::select_scores(self.canvas, base, &scores, w, h, region, combine)
            .map_err(|e| e.to_string())
    }
}

/// Quick Selection: the region of similar colors the stroke paints over, bounded by the
/// image's edges, combined with the selection by `mode`. While the stroke goes on (`live`),
/// each request replaces the previous one's selection; the last one makes a single undo entry.
#[tauri::command]
pub async fn quick_select(
    app: tauri::AppHandle,
    document_id: u64,
    request: QuickRequest,
) -> Result<DocumentView, String> {
    use tauri::Manager;
    on_worker(move || {
        let state = app.state::<AppState>();
        let started = std::time::Instant::now();
        let cached = {
            let stroke = state.quick.stroke.lock().map_err(|e| e.to_string())?;
            stroke
                .as_ref()
                .filter(|s| s.id == request.stroke && s.document_id == document_id)
                .map(Arc::clone)
        };
        let stroke = match cached {
            Some(stroke) => stroke,
            None => {
                let stroke = Arc::new(QuickStroke::new(
                    &state,
                    request.stroke,
                    document_id,
                    request.region,
                    request.layer_id,
                )?);
                *state.quick.stroke.lock().map_err(|e| e.to_string())? = Some(Arc::clone(&stroke));
                stroke
            }
        };
        let prepared = started.elapsed();
        let image = stroke.select(&request.points, request.radius, &request.mode)?;
        let selected = started.elapsed();
        if !request.live {
            *state.quick.stroke.lock().map_err(|e| e.to_string())? = None;
        }
        let selection = match image {
            Some(image) => Some(Selection::new(Arc::new(image)).ok_or("a selection is gray")?),
            None => None,
        };
        let mut documents = state.documents()?;
        let document = documents.get_mut(document_id)?;
        let session = &mut document.session;
        // The stroke so far replaces what its previous requests showed.
        session.cancel_gesture().map_err(|e| e.to_string())?;
        if selection.is_some() || session.document().selection().is_some() {
            session
                .perform_in_gesture(Edit::SetSelection { selection })
                .map_err(|e| e.to_string())?;
        }
        if !request.live {
            session.end_gesture();
        }
        if cfg!(debug_assertions) {
            eprintln!(
                "quick select {}x{}: image {prepared:?}, cut and combine {:?}, applied {:?}",
                stroke.width,
                stroke.height,
                selected - prepared,
                started.elapsed() - selected,
            );
        }
        Ok(document.view())
    })
    .await
}

/// The document to sample: the composited document, or with `layer_id` a document holding
/// only that layer, placed as in the document.
pub(crate) fn sampled_document(
    doc: &slopshop_core::Document,
    layer_id: Option<u64>,
) -> Result<slopshop_core::Document, String> {
    let Some(raw) = layer_id else {
        return Ok(doc.clone());
    };
    let id = LayerId::from_raw(raw);
    let mut layer = doc.layer(id).ok_or("unknown layer")?.clone();
    // Alone, at the top level: placed by its own transform and its groups'.
    layer.transform = layer.transform.then(doc.parent_transform(id));
    layer.clipped = false;
    slopshop_core::Document::restore(
        doc.size(),
        doc.working_space(),
        doc.blend_space(),
        vec![layer],
        doc.next_layer_id(),
    )
    .map_err(|e| e.to_string())
}

/// Select > Color Range's samples and settings, from the dialog.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColorRangeRequest {
    /// Document pixels whose colors are selected, and those whose colors are taken away.
    included: Vec<[u32; 2]>,
    excluded: Vec<[u32; 2]>,
    fuzziness: f32,
    invert: bool,
    layer_id: Option<u64>,
}

impl ColorRangeRequest {
    /// The source to sample and the range, its sampled colors read from the source.
    fn resolve(
        &self,
        doc: &slopshop_core::Document,
    ) -> Result<(slopshop_core::Document, selection::ColorRange), String> {
        let source = sampled_document(doc, self.layer_id)?;
        let points = |p: &[[u32; 2]]| p.iter().map(|[x, y]| (*x, *y)).collect::<Vec<_>>();
        let range = selection::ColorRange {
            included: selection::sample_colors(&source, &points(&self.included)),
            excluded: selection::sample_colors(&source, &points(&self.excluded)),
            fuzziness: self.fuzziness,
            invert: self.invert,
        };
        Ok((source, range))
    }
}

/// Select > Color Range's preview: the selection it would make, computed on the document as
/// shown fitted in `max_side` pixels (rendered by the GPU, so instant whatever the size): raw
/// binary, width and height (`u32` little-endian), then one 8-bit gray value per pixel.
#[tauri::command]
pub async fn color_range_preview(
    app: tauri::AppHandle,
    document_id: u64,
    request: ColorRangeRequest,
    max_side: u32,
) -> Result<Response, String> {
    use tauri::Manager;
    let (doc, current) = {
        let state = app.state::<AppState>();
        let mut documents = state.documents()?;
        let doc = documents.get_mut(document_id)?.session.document().clone();
        let current = doc.selection().map(|s| Arc::clone(s.image()));
        (doc, current)
    };
    on_worker(move || {
        let state = app.state::<AppState>();
        let (source, range) = request.resolve(&doc)?;
        let size = source.size();
        let side = max_side.clamp(16, 1024);
        let scale = (f64::from(size.width.max(size.height)) / f64::from(side)).max(1.0);
        let output = Size::new(
            (f64::from(size.width) / scale).ceil().max(1.0) as u32,
            (f64::from(size.height) / scale).ceil().max(1.0) as u32,
        );
        let view = slopshop_core::view::ViewTransform {
            origin: [0.0, 0.0],
            scale,
        };
        let frame = state
            .renderer()?
            .render_view(&source, view, output)
            .map_err(|e| e.to_string())?;
        let mut bytes = Vec::with_capacity(8 + (output.width * output.height) as usize);
        bytes.extend_from_slice(&output.width.to_le_bytes());
        bytes.extend_from_slice(&output.height.to_le_bytes());
        for (i, px) in frame.data.as_chunks::<4>().0.iter().enumerate() {
            let color = [f32::from(px[0]), f32::from(px[1]), f32::from(px[2]), 255.0];
            let mut c = range.coverage(color);
            if let Some(selection) = &current {
                let x = ((i as u32 % output.width) as f64 * scale) as u32;
                let y = ((i as u32 / output.width) as f64 * scale) as u32;
                c *= selection.gray_at(x, y);
            }
            bytes.push((c * 255.0).round() as u8);
        }
        Ok(Response::new(bytes))
    })
    .await
}

/// Select > Color Range: the colors of the samples, within the current selection if any.
#[tauri::command]
pub async fn color_range(
    state: State<'_, AppState>,
    document_id: u64,
    request: ColorRangeRequest,
) -> Result<DocumentView, String> {
    let (doc, current) = {
        let mut documents = state.documents()?;
        let doc = documents.get_mut(document_id)?.session.document().clone();
        let current = doc.selection().map(|s| Arc::clone(s.image()));
        (doc, current)
    };
    let image = on_worker(move || {
        let (source, range) = request.resolve(&doc)?;
        selection::color_range(&source, current.as_deref(), &range).map_err(|e| e.to_string())
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
