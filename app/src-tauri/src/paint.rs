//! Painting (ADR 0027): the Brush and Eraser strokes, computed by the engine off the UI thread.
//!
//! A stroke is sent in batches of pointer samples while it is painted. Each batch extends the
//! engine's stroke and its result is shown as a preview in place of the layer's pixels (view
//! state, like the Quick Mask overlay: no history entry, no revision). The last batch commits
//! the stroke as one edit: on a layer, its paint on top of the layer's stack (ADR 0029,
//! `Edit::SetLayerStack`), continuing the top paint; one undo entry, the layer's original kept.
//! A layer's mask is painted into its painted image (`Edit::SetMaskPaint`), and the selection
//! in Quick Mask (`Edit::SetSelection`): both in gray, white showing or selecting, black hiding.
//!
//! A stroke reaches the whole canvas: a layer that does not cover it grows first, by whole
//! tiles before it (its stack and mask move with it, its transform compensates), so the layer
//! looks the same until painted; the growth is part of the stroke's undo entry.

use slopshop_core::HistoryLabel;
use std::sync::{Arc, Mutex};

use serde::Deserialize;
use slopshop_core::paint::{Brush, Paint, PointerSample, Stroke, canvas_growth, gray_of_srgb};
use slopshop_core::selection::{Selection, sample_region};
use slopshop_core::stack::LayerStack;
use slopshop_core::{
    Affine, Document, Edit, LayerContent, LayerId, LayerMask, LinearRgba, RasterImage, Size,
};
use tauri::Manager;
use tauri::ipc::Response;

use crate::AppState;
use crate::ipc::DocumentView;
use crate::selection::on_worker;

/// A batch of a stroke (see the module documentation).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaintRequest {
    /// Batches of one stroke share its id; a new id starts a new stroke.
    pub stroke: u64,
    /// What is painted: the layer's pixels (a raster layer), its mask, or the selection.
    #[serde(default)]
    pub target: PaintTarget,
    /// The layer painted, or whose mask is painted; unused for the selection.
    pub layer_id: u64,
    pub brush: BrushRequest,
    /// The Brush's color, sRGB-encoded RGB in `[0, 1]`; absent for the Eraser.
    #[serde(default)]
    pub color: Option<[f32; 3]>,
    /// The Restore Eraser (ADR 0029): the layer's paint brought back towards its original.
    #[serde(default)]
    pub restore: bool,
    /// Pointer samples since the last batch: document pixels and pressure (`[x, y, p]`).
    pub samples: Vec<[f64; 3]>,
    /// The last batch: the stroke is committed.
    pub end: bool,
}

/// What a stroke paints (ADR 0027).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PaintTarget {
    /// A raster layer's pixels.
    #[default]
    Layer,
    /// A layer's mask, in gray.
    Mask,
    /// Quick Mask's image, in gray (ADR 0024).
    QuickMask,
}

/// A stroke's target in a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Target {
    Layer(LayerId),
    Mask(LayerId),
    QuickMask,
}

impl PaintRequest {
    fn target(&self) -> Target {
        let id = LayerId::from_raw(self.layer_id);
        match self.target {
            PaintTarget::Layer => Target::Layer(id),
            PaintTarget::Mask => Target::Mask(id),
            PaintTarget::QuickMask => Target::QuickMask,
        }
    }
}

/// The options bar's brush. Shares between 0 and 1.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrushRequest {
    /// Diameter, document pixels.
    pub size: f32,
    pub hardness: f32,
    pub spacing: f32,
    pub flow: f32,
    pub opacity: f32,
    pub pressure_size: bool,
    pub pressure_opacity: bool,
}

impl BrushRequest {
    fn brush(self) -> Brush {
        Brush {
            diameter: self.size,
            hardness: self.hardness,
            spacing: self.spacing,
            flow: self.flow,
            opacity: self.opacity,
            pressure_size: self.pressure_size,
            pressure_opacity: self.pressure_opacity,
        }
    }
}

/// The stroke under way.
struct ActiveStroke {
    id: u64,
    document_id: u64,
    target: Target,
    stroke: Stroke,
    growth: Option<Growth>,
}

/// A layer as a stroke or a move found it, grown to cover the canvas if it needed to (see the
/// module documentation): its transform, its stack and its mask, if any.
#[derive(Debug, Clone)]
pub struct Growth {
    pub(crate) transform: Affine,
    pub(crate) stack: LayerStack,
    pub(crate) mask: Option<LayerMask>,
}

/// What a stroke or a move leaves on its target.
#[derive(Debug, Clone)]
pub(crate) enum Painted {
    /// A mask's or the selection's painted image (ADR 0027).
    Image(Arc<RasterImage>),
    /// A layer's stack and, when known, what the layer shows with it (ADR 0029).
    Stack(LayerStack, Option<Arc<RasterImage>>),
}

#[derive(Default)]
pub struct PaintState {
    stroke: Mutex<Option<ActiveStroke>>,
}

/// What a document shows of a stroke under way: its target as painted so far.
#[derive(Debug, Clone)]
pub struct PaintPreview {
    target: Target,
    painted: Painted,
    pub growth: Option<Growth>,
}

impl PaintPreview {
    /// `doc` (a snapshot being rendered) showing the stroke.
    pub fn apply_to(&self, doc: &mut Document) {
        // The layer may have gone meanwhile: then nothing to show.
        let _ = paint_edit(self.target, self.painted.clone(), self.growth.as_ref()).apply(doc);
    }
}

/// The edit that gives `target` what was `painted` on it, a layer grown first by `growth`.
pub(crate) fn paint_edit(target: Target, painted: Painted, growth: Option<&Growth>) -> Edit {
    let (layer, stack, shown) = match (target, painted) {
        (Target::Mask(id), Painted::Image(image)) => {
            return Edit::SetMaskPaint {
                id,
                painted: Some(image),
            };
        }
        // A stroke paints a gray image of the quick mask's: always a mask.
        (Target::QuickMask, Painted::Image(image)) => {
            return Edit::SetQuickMask {
                mask: Selection::new(image),
            };
        }
        (Target::Layer(id), Painted::Stack(stack, shown)) => (id, stack, shown),
        // Strokes and moves give layers stacks, masks and the selection images.
        _ => return Edit::Batch(Vec::new()),
    };
    // What the layer shows, when it is the stack's result as is (an Eraser that erased
    // nothing leaves no alpha channel to show): evaluated again otherwise.
    let shown =
        shown.filter(|s| s.format() == stack.format() && s.size() == stack.original().size());
    let mut edits = vec![Edit::SetLayerStack {
        id: layer,
        stack,
        shown,
    }];
    if let Some(growth) = growth {
        edits.push(Edit::SetLayerTransform {
            id: layer,
            transform: growth.transform,
        });
        if let Some(mask) = &growth.mask {
            edits.push(Edit::SetLayerMask {
                id: layer,
                mask: Some(mask.clone()),
            });
        }
    }
    Edit::Batch(edits)
}

/// What `stroke` painted on `target` so far: a layer's stack and what it shows, or an image.
fn painted_so_far(target: Target, stroke: &mut Stroke) -> Result<Painted, String> {
    let image = stroke.image().map_err(|e| e.to_string())?;
    Ok(match target {
        Target::Layer(_) => Painted::Stack(
            stroke
                .stack()
                .map_err(|e| e.to_string())?
                .ok_or("a layer's stroke paints its stack")?,
            Some(image),
        ),
        _ => Painted::Image(image),
    })
}

/// Raster layer `id` of `doc` as a stroke or a move works on it (see the module
/// documentation): what it shows, and its transform, stack and mask, grown to cover the canvas
/// if it needs to (`grow`).
pub(crate) fn grow(
    doc: &Document,
    id: LayerId,
    grow: bool,
) -> Result<(Arc<RasterImage>, Growth), String> {
    let layer = doc.layer(id).ok_or("the painted layer is gone")?;
    let LayerContent::Raster { image, .. } = &layer.content else {
        return Err("only raster layers can be painted".to_owned());
    };
    let stack = layer
        .content
        .stack()
        .ok_or("only raster layers can be painted")?;
    let parent = doc.parent_transform(id);
    let growth = grow
        .then(|| canvas_growth(image.size(), layer.transform.then(parent), doc.size()))
        .flatten();
    let image = image.get();
    let Some((offset, size)) = growth else {
        return Ok((
            image,
            Growth {
                transform: layer.transform,
                stack,
                mask: layer.mask.clone(),
            },
        ));
    };
    grown(
        &image,
        &Growth {
            transform: layer.transform,
            stack,
            mask: layer.mask.clone(),
        },
        offset,
        size,
    )
}

/// `layer` (showing `image`) grown by `offset` whole tiles before its pixels to `size`: what it
/// shows and its growth.
pub(crate) fn grown(
    image: &Arc<RasterImage>,
    layer: &Growth,
    offset: (u32, u32),
    size: Size,
) -> Result<(Arc<RasterImage>, Growth), String> {
    let shown = grown_pixels(image, offset, size)?;
    let mask = match &layer.mask {
        Some(mask) => Some(grown_mask(mask, offset, size)?),
        None => None,
    };
    Ok((
        shown,
        Growth {
            transform: grown_transform(layer.transform, offset),
            stack: layer.stack.grown(offset, size).map_err(|e| e.to_string())?,
            mask,
        },
    ))
}

/// A layer's pixels grown by `offset` whole tiles (columns, rows) before them to `size`:
/// transparent around them (an alpha channel added first, lossless).
pub(crate) fn grown_pixels(
    image: &Arc<RasterImage>,
    offset: (u32, u32),
    size: Size,
) -> Result<Arc<RasterImage>, String> {
    let with_alpha = match image.with_alpha() {
        Some(converted) => Arc::new(converted.map_err(|e| e.to_string())?),
        None => Arc::clone(image),
    };
    with_alpha
        .grown(offset, size)
        .ok_or("the layer cannot grow")?
        .map(Arc::new)
        .map_err(|e| e.to_string())
}

/// A layer's mask grown with its pixels (see [`grown_pixels`]): hiding around them.
pub(crate) fn grown_mask(
    mask: &LayerMask,
    offset: (u32, u32),
    size: Size,
) -> Result<LayerMask, String> {
    let grown = |image: &RasterImage| -> Result<Arc<RasterImage>, String> {
        image
            .grown(offset, size)
            .ok_or("the mask cannot grow")?
            .map(Arc::new)
            .map_err(|e| e.to_string())
    };
    Ok(LayerMask {
        image: grown(&mask.image)?,
        original: mask.original.as_deref().map(grown).transpose()?,
        ..mask.clone()
    })
}

/// A layer's transform once its pixels grew by `offset` whole tiles before them: they start
/// that much earlier, so the layer looks the same.
pub(crate) fn grown_transform(transform: Affine, (left, top): (u32, u32)) -> Affine {
    let tile = f64::from(slopshop_core::raster::TILE_SIZE);
    Affine::translation(-f64::from(left) * tile, -f64::from(top) * tile).then(transform)
}

/// A stroke on the request's target in `doc`, a layer grown to the canvas if it needs to
/// (`grow_layer`: strokes; not a clear, which only removes).
fn start(
    doc: &Document,
    request: &PaintRequest,
    grow_layer: bool,
) -> Result<(Stroke, Option<Growth>), String> {
    // In gray, a color paints its luminance and the Eraser hides (ADR 0027).
    let gray = Paint::Gray(request.color.map_or(0.0, gray_of_srgb));
    let selection = doc.selection().map(|s| Arc::clone(s.image()));
    let (image, to_document, growth, paint, selection) = match request.target() {
        Target::Layer(id) => {
            // Restoring only reaches paint: no need to grow.
            let (image, growth) = grow(doc, id, grow_layer && !request.restore)?;
            let paint = match request.color {
                _ if request.restore => Paint::Restore,
                Some([r, g, b]) => {
                    Paint::Color(LinearRgba::from_srgb_encoded_to_working(r, g, b, 1.0))
                }
                None => Paint::Erase,
            };
            // Into the parent, then into the document.
            let to_document = growth.transform.then(doc.parent_transform(id));
            let stroke = Stroke::on_stack(
                &growth.stack,
                image,
                to_document,
                selection,
                doc.blend_space(),
                request.brush.brush(),
                paint,
            )
            .map_err(|e| e.to_string())?;
            return Ok((stroke, Some(growth)));
        }
        _ if request.restore => {
            return Err("the Restore Eraser brings back a layer's pixels".to_owned());
        }
        // A mask lies in its layer's pixel grid and covers what the layer can show: it does not
        // grow.
        Target::Mask(id) => {
            let layer = doc.layer(id).ok_or("the painted layer is gone")?;
            let mask = layer.mask.as_ref().ok_or("the layer has no mask")?;
            let to_document = layer.transform.then(doc.parent_transform(id));
            (Arc::clone(&mask.image), to_document, None, gray, selection)
        }
        // Quick Mask's image, within the selection made meanwhile (as Photoshop's channel).
        Target::QuickMask => {
            let mask = doc.quick_mask().ok_or("Quick Mask is off")?;
            (
                Arc::clone(mask.image()),
                Affine::IDENTITY,
                None,
                gray,
                selection,
            )
        }
    };
    let stroke = Stroke::new(
        image,
        to_document,
        selection,
        doc.blend_space(),
        request.brush.brush(),
        paint,
    )
    .map_err(|e| e.to_string())?;
    Ok((stroke, growth))
}

/// Paint a batch of a stroke (see the module documentation). Returns the document once the
/// stroke is committed; `None` while it goes on (the view redraws to show the preview).
#[tauri::command]
pub async fn paint_stroke(
    app: tauri::AppHandle,
    document_id: u64,
    request: PaintRequest,
) -> Result<Option<DocumentView>, String> {
    on_worker(move || paint(&app.state::<AppState>(), document_id, request)).await
}

/// [`paint_stroke`]'s work, on the calling thread.
pub(crate) fn paint(
    state: &AppState,
    document_id: u64,
    request: PaintRequest,
) -> Result<Option<DocumentView>, String> {
    {
        let started = std::time::Instant::now();
        // The stroke is taken out while it computes: frames keep rendering meanwhile.
        let previous = state
            .paint
            .stroke
            .lock()
            .map_err(|e| e.to_string())?
            .take()
            .filter(|s| s.id == request.stroke && s.document_id == document_id);
        let mut active = match previous {
            Some(active) => active,
            None => {
                // A copy (its pixels shared): a layer whose pixels are still evaluated (a filter
                // just applied) is waited for without holding the documents meanwhile.
                let doc = {
                    let mut documents = state.documents()?;
                    documents.get_mut(document_id)?.session.document().clone()
                };
                let (stroke, growth) = start(&doc, &request, true)?;
                ActiveStroke {
                    id: request.stroke,
                    document_id,
                    target: request.target(),
                    stroke,
                    growth,
                }
            }
        };
        let samples: Vec<PointerSample> = request
            .samples
            .iter()
            .map(|&[x, y, pressure]| PointerSample {
                x,
                y,
                pressure: pressure as f32,
            })
            .collect();
        active.stroke.add(&samples);
        let has_paint = active.stroke.has_paint();
        let painted = painted_so_far(active.target, &mut active.stroke)?;
        let computed = started.elapsed();

        let mut documents = state.documents()?;
        let document = documents.get_mut(document_id)?;
        let view = if request.end {
            document.set_paint_preview(None);
            if has_paint {
                let label = HistoryLabel::new(if request.restore {
                    "restoreEraser"
                } else if request.color.is_none() {
                    "eraser"
                } else {
                    "brush"
                });
                let edit = paint_edit(active.target, painted, active.growth.as_ref());
                document
                    .session
                    .with_label(Some(label), |s| s.perform(edit))
                    .map_err(|e| e.to_string())?;
            }
            Some(document.view())
        } else {
            document.set_paint_preview(Some(PaintPreview {
                target: active.target,
                painted,
                growth: active.growth.clone(),
            }));
            *state.paint.stroke.lock().map_err(|e| e.to_string())? = Some(active);
            None
        };
        if cfg!(debug_assertions) {
            eprintln!(
                "paint {} samples{}: {computed:?}",
                samples.len(),
                if request.end { " (end)" } else { "" },
            );
        }
        Ok(view)
    }
}

/// Where Edit > Stroke draws, relative to the selection's outline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StrokeLocation {
    Inside,
    Center,
    Outside,
}

/// Edit > Stroke: a band `width` pixels wide along the selection's outline.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StrokeRequest {
    pub width: f64,
    pub location: StrokeLocation,
}

/// Edit > Fill and Stroke, and Delete with a selection, as paint (ADR 0027, kept apart from the
/// original like a stroke; ADR 0029): layer `layer_id` erased (`color` absent: only alpha
/// changes) or painted with `color` (sRGB-encoded RGB in `[0, 1]`) at `opacity`, within the
/// selection, or the band `stroke` draws along its outline. Without a selection, Fill paints the
/// whole layer, as in Photoshop. Painting grows the layer to the canvas if needed, as strokes
/// do. With `target` the mask, the mask is hidden or painted with the color's gray. One undo
/// entry; nothing to paint, nothing changes.
#[tauri::command]
pub async fn fill(
    app: tauri::AppHandle,
    document_id: u64,
    layer_id: u64,
    target: PaintTarget,
    color: Option<[f32; 3]>,
    opacity: f32,
    stroke: Option<StrokeRequest>,
) -> Result<DocumentView, String> {
    on_worker(move || {
        let state = app.state::<AppState>();
        let mut documents = state.documents()?;
        let document = documents.get_mut(document_id)?;
        if !(0.0..=1.0).contains(&opacity) {
            return Err("the opacity is between 0 and 1".to_owned());
        }
        let request = fill_request(layer_id, target, color, opacity);
        let label = HistoryLabel::new(match (&stroke, request.color) {
            (Some(_), _) => "stroke",
            (None, Some(_)) => "fill",
            (None, None) => "clear",
        });
        if let Some(edit) = fill_edit(document.session.document(), &request, stroke)? {
            document
                .session
                .with_label(Some(label), |s| s.perform(edit))
                .map_err(|e| e.to_string())?;
        }
        Ok(document.view())
    })
    .await
}

/// The Paint Bucket (G): the pixels of a color similar to the one at (`x`, `y`) (the Magic
/// Wand's region: `tolerance`, `contiguous`, `anti_alias`; sampled from every visible layer, or
/// `sample_layer` alone), within the selection if there is one, filled with `color` at
/// `opacity` on `target` as Edit > Fill does: one undo entry, the selection unchanged. Seconds
/// on a large document: the UI's `task` (its progress, cancellable).
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn paint_bucket(
    app: tauri::AppHandle,
    document_id: u64,
    x: u32,
    y: u32,
    tolerance: f32,
    contiguous: bool,
    anti_alias: bool,
    sample_layer: Option<u64>,
    layer_id: u64,
    target: PaintTarget,
    color: [f32; 3],
    opacity: f32,
    task: u64,
) -> Result<DocumentView, crate::ai::AiFailure> {
    use crate::selection::{failure, internal};
    use slopshop_core::selection::{self, Combine, WandOptions};
    if !(0.0..=1.0).contains(&opacity) {
        return Err(internal("the opacity is between 0 and 1"));
    }
    let options = WandOptions {
        tolerance,
        contiguous,
        anti_alias,
    };
    crate::selection::sampled_then(
        &app,
        document_id,
        sample_layer,
        task,
        "paintBucket",
        move |sampling| {
            // Within the selection, as any painting.
            let combine = if sampling.current.is_some() {
                Combine::Intersect
            } else {
                Combine::Replace
            };
            selection::magic_wand_from(
                sampling.source,
                sampling.pixels,
                sampling.current,
                (x, y),
                options,
                combine,
                sampling.progress,
                sampling.cancel,
            )
            .map_err(failure)
        },
        move |state, region| {
            let mut documents = state.documents().map_err(internal)?;
            let document = documents.get_mut(document_id).map_err(internal)?;
            let Some(region) = region else {
                return Ok(document.view());
            };
            let request = fill_request(layer_id, target, Some(color), opacity);
            let edit =
                bucket_edit(document.session.document(), region, &request).map_err(internal)?;
            if let Some(edit) = edit {
                document
                    .session
                    .with_label(Some(HistoryLabel::new("paintBucket")), |s| s.perform(edit))
                    .map_err(internal)?;
            }
            Ok(document.view())
        },
    )
    .await
}

/// The Paint Bucket's edit: `request` filled within `region` (a coverage the canvas's size)
/// of `doc`, whose own selection stays. `None`: nothing to paint.
pub(crate) fn bucket_edit(
    doc: &Document,
    region: RasterImage,
    request: &PaintRequest,
) -> Result<Option<Edit>, String> {
    // The region as the selection the fill reads, on a copy.
    let mut scratch = doc.clone();
    Edit::SetSelection {
        selection: Selection::new(Arc::new(region)),
    }
    .apply(&mut scratch)
    .map_err(|e| e.to_string())?;
    fill_edit(&scratch, request, None)
}

/// What Edit > Fill and the Paint Bucket paint: `color` (none: erase) at `opacity` everywhere
/// the coverage they read reaches.
pub(crate) fn fill_request(
    layer_id: u64,
    target: PaintTarget,
    color: Option<[f32; 3]>,
    opacity: f32,
) -> PaintRequest {
    PaintRequest {
        restore: false,
        stroke: 0,
        target,
        layer_id,
        brush: BrushRequest {
            size: 1.0,
            hardness: 1.0,
            spacing: 1.0,
            flow: 1.0,
            opacity,
            pressure_size: false,
            pressure_opacity: false,
        },
        color,
        samples: Vec::new(),
        end: true,
    }
}

/// The edit `fill` performs: `request`'s target painted within the selection of `doc` (or
/// everywhere without one), or along its outline with `stroke`. `None`: nothing to paint.
pub(crate) fn fill_edit(
    doc: &Document,
    request: &PaintRequest,
    stroke: Option<StrokeRequest>,
) -> Result<Option<Edit>, String> {
    // Erasing only removes: no need to grow.
    let grow_layer = request.color.is_some();
    let (mut painting, growth) = match stroke {
        None => start(doc, request, grow_layer)?,
        Some(stroke) => match banded(doc, stroke)? {
            Some(banded) => start(&banded, request, grow_layer)?,
            None => return Ok(None),
        },
    };
    painting.fill();
    if !painting.has_paint() {
        return Ok(None);
    }
    let painted = painted_so_far(request.target(), &mut painting)?;
    Ok(Some(paint_edit(request.target(), painted, growth.as_ref())))
}

/// `doc` with the band `stroke` draws along its selection's outline as its selection, for Fill
/// to paint (a copy: the pixels are shared); `None` when there is nothing to stroke.
fn banded(doc: &Document, stroke: StrokeRequest) -> Result<Option<Document>, String> {
    use slopshop_core::selection::{StrokeLocation as Location, stroke_band};
    let selection = doc.selection().ok_or("Stroke needs a selection")?;
    let location = match stroke.location {
        StrokeLocation::Inside => Location::Inside,
        StrokeLocation::Center => Location::Center,
        StrokeLocation::Outside => Location::Outside,
    };
    let band = stroke_band(doc.size(), selection.image(), stroke.width, location)
        .map_err(|e| format!("{e:?}"))?;
    let Some(band) = band else {
        return Ok(None);
    };
    let mut banded = doc.clone();
    Edit::SetSelection {
        selection: Selection::new(Arc::new(band)),
    }
    .apply(&mut banded)
    .map_err(|e| e.to_string())?;
    Ok(Some(banded))
}

/// The eyedropper (the tool, the color picker's): the color shown at document point (`x`, `y`),
/// every visible layer composited (with `layer_id`, that layer alone), as whole 8-bit sRGB
/// values; with `size`, the average of the `size × size` pixels around it (odd, Photoshop's
/// Sample Size). `None` outside the canvas or where nothing is shown (transparent).
#[tauri::command]
pub async fn sample_color(
    app: tauri::AppHandle,
    document_id: u64,
    x: f64,
    y: f64,
    size: Option<u32>,
    layer_id: Option<u64>,
) -> Result<Option<[u8; 3]>, String> {
    on_worker(move || {
        let state = app.state::<AppState>();
        // Cheap: the layers' pixels are shared. The lock is not held while compositing.
        let doc = state
            .documents()?
            .get_mut(document_id)?
            .session
            .document()
            .clone();
        let doc = crate::selection::sampled_document(&doc, layer_id)?;
        Ok(sample_color_at(&doc, x, y, size.unwrap_or(1)))
    })
    .await
}

/// The largest Sample Size (Photoshop's 101 × 101).
const MAX_SAMPLE_SIZE: u32 = 101;

/// [`sample_color`]'s work.
pub(crate) fn sample_color_at(doc: &Document, x: f64, y: f64, size: u32) -> Option<[u8; 3]> {
    // Negative or NaN: outside. Too large saturates, then falls outside the canvas.
    if !(x >= 0.0 && y >= 0.0) {
        return None;
    }
    // Odd, centered on the pixel under the point.
    let size = size.clamp(1, MAX_SAMPLE_SIZE) | 1;
    let half = i64::from(size / 2);
    let (cx, cy) = (x as i64, y as i64);
    average_color(&sample_region(doc, cx - half, cy - half, size, size))
}

/// The average of `colors` (straight alpha) weighted by their alpha, as a pixel showing them
/// together would be; `None` when none shows anything.
fn average_color(colors: &[[f32; 4]]) -> Option<[u8; 3]> {
    let mut sum = [0f64; 3];
    let mut weight = 0f64;
    for [r, g, b, a] in colors {
        let a = f64::from(*a);
        for (total, v) in sum.iter_mut().zip([r, g, b]) {
            *total += f64::from(*v) * a;
        }
        weight += a;
    }
    (weight > 0.0).then(|| sum.map(|v| (v / weight).round().clamp(0.0, 255.0) as u8))
}

/// The eyedropper's loupe: the colors shown around document point (`x`, `y`), the pixel under
/// it in the middle of `2 × radius + 1` pixels a side, every visible layer composited. Raw
/// 8-bit sRGB RGBA, straight alpha, row-major; transparent off the canvas.
#[tauri::command]
pub async fn sample_patch(
    app: tauri::AppHandle,
    document_id: u64,
    x: f64,
    y: f64,
    radius: u32,
    layer_id: Option<u64>,
) -> Result<Response, String> {
    on_worker(move || {
        let state = app.state::<AppState>();
        // Cheap: the layers' pixels are shared. The lock is not held while compositing.
        let doc = state
            .documents()?
            .get_mut(document_id)?
            .session
            .document()
            .clone();
        let doc = crate::selection::sampled_document(&doc, layer_id)?;
        Ok(Response::new(sample_patch_at(&doc, x, y, radius)))
    })
    .await
}

/// A loupe keeps a tile of pixels around the pointer, not a region: larger asks are cut down to
/// this (129² pixels, 66 KB).
const MAX_PATCH_RADIUS: u32 = 64;

/// [`sample_patch`]'s work.
pub(crate) fn sample_patch_at(doc: &Document, x: f64, y: f64, radius: u32) -> Vec<u8> {
    let radius = radius.min(MAX_PATCH_RADIUS);
    let side = 2 * radius + 1;
    if x.is_nan() || y.is_nan() {
        return vec![0; side as usize * side as usize * 4];
    }
    // Huge values saturate, then fall off the canvas.
    let (cx, cy) = (x.floor() as i64, y.floor() as i64);
    let left = cx.saturating_sub(i64::from(radius));
    let top = cy.saturating_sub(i64::from(radius));
    sample_region(doc, left, top, side, side)
        .iter()
        .flat_map(|c| c.map(|v| v.round().clamp(0.0, 255.0) as u8))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use slopshop_core::color::PixelFormat;
    use slopshop_core::selection::{Combine, EdgeOptions, Shape, select_shape};
    use slopshop_core::{BlendMode, Layer, Session};

    const CANVAS: Size = Size::new(64, 48);

    #[test]
    fn a_sample_averages_what_shows_weighted_by_alpha() {
        let red = [255.0, 0.0, 0.0, 1.0];
        let blue = [0.0, 0.0, 255.0, 1.0];
        let faint = [0.0, 255.0, 0.0, 0.0];
        assert_eq!(average_color(&[red, blue, faint]), Some([128, 0, 128]));
        let half_blue = [0.0, 0.0, 255.0, 0.5];
        assert_eq!(average_color(&[red, half_blue]), Some([170, 0, 85]));
        assert_eq!(average_color(&[faint]), None);
    }

    /// A document holding one transparent canvas-sized layer (id 1), `selected` as its
    /// selection if given.
    fn document(selected: Option<[f64; 4]>) -> Document {
        let image = RasterImage::from_pixels(
            CANVAS,
            PixelFormat::RGBA8_SRGB,
            &vec![0; CANVAS.pixel_count() as usize * 4],
        )
        .unwrap();
        let layer = Layer {
            style: None,
            id: LayerId::from_raw(1),
            name: "layer".into(),
            visible: true,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            content: LayerContent::raster(Arc::new(image)),
            mask: None,
            clipped: false,
            transform: Affine::IDENTITY,
        };
        let mut session = Session::new(
            Document::restore(
                CANVAS,
                slopshop_core::color::WORKING_SPACE,
                slopshop_core::BlendSpace::Perceptual,
                vec![layer],
                2,
            )
            .unwrap(),
        );
        if let Some([left, top, right, bottom]) = selected {
            let shape = Shape::Rectangle {
                left,
                top,
                right,
                bottom,
            };
            let image = select_shape(
                CANVAS,
                None,
                &shape,
                EdgeOptions::default(),
                Combine::Replace,
            )
            .unwrap()
            .unwrap();
            let selection = Selection::new(Arc::new(image));
            session.perform(Edit::SetSelection { selection }).unwrap();
        }
        session.document().clone()
    }

    fn request(opacity: f32) -> PaintRequest {
        PaintRequest {
            restore: false,
            stroke: 0,
            target: PaintTarget::Layer,
            layer_id: 1,
            brush: BrushRequest {
                size: 1.0,
                hardness: 1.0,
                spacing: 1.0,
                flow: 1.0,
                opacity,
                pressure_size: false,
                pressure_opacity: false,
            },
            color: Some([1.0, 0.0, 0.0]),
            samples: Vec::new(),
            end: true,
        }
    }

    /// The layer's painted image once `fill_edit` is applied.
    fn filled(
        doc: &Document,
        request: &PaintRequest,
        stroke: Option<StrokeRequest>,
    ) -> Arc<RasterImage> {
        let edit = fill_edit(doc, request, stroke).unwrap().unwrap();
        let mut doc = doc.clone();
        edit.apply(&mut doc).unwrap();
        let LayerContent::Raster { image, .. } = &doc.layer(LayerId::from_raw(1)).unwrap().content
        else {
            panic!("a raster layer");
        };
        image.get()
    }

    #[test]
    fn fill_paints_the_selection_or_the_whole_layer_at_its_opacity() {
        let selected = document(Some([10.0, 10.0, 20.0, 20.0]));
        let image = filled(&selected, &request(1.0), None);
        assert_eq!(image.alpha_at(10, 10), 1.0);
        assert_eq!(image.alpha_at(9, 10), 0.0);
        let everywhere = filled(&document(None), &request(0.5), None);
        assert!((everywhere.alpha_at(0, 0) - 0.5).abs() < 0.01);
        assert!((everywhere.alpha_at(63, 47) - 0.5).abs() < 0.01);
    }

    #[test]
    fn stroke_paints_a_band_along_the_outline() {
        let doc = document(Some([10.0, 10.0, 30.0, 30.0]));
        let stroke = |location| StrokeRequest {
            width: 2.0,
            location,
        };
        let inside = filled(&doc, &request(1.0), Some(stroke(StrokeLocation::Inside)));
        assert_eq!(inside.alpha_at(10, 20), 1.0);
        assert_eq!(inside.alpha_at(11, 20), 1.0);
        assert_eq!(inside.alpha_at(12, 20), 0.0);
        assert_eq!(inside.alpha_at(9, 20), 0.0);
        let outside = filled(&doc, &request(1.0), Some(stroke(StrokeLocation::Outside)));
        assert_eq!(outside.alpha_at(8, 20), 1.0);
        assert_eq!(outside.alpha_at(10, 20), 0.0);
        // Stroke needs a selection.
        let none = fill_edit(
            &document(None),
            &request(1.0),
            Some(stroke(StrokeLocation::Center)),
        );
        assert!(none.is_err());
    }
}
