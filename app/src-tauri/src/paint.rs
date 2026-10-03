//! Painting (ADR 0027): the Brush and Eraser strokes, computed by the engine off the UI thread.
//!
//! A stroke is sent in batches of pointer samples while it is painted. Each batch extends the
//! engine's stroke and its result is shown as a preview in place of the layer's pixels (view
//! state, like the Quick Mask overlay: no history entry, no revision). The last batch commits
//! the stroke as one edit, `Edit::SetLayerPaint`: one undo entry, the layer's original kept.
//! A layer's mask is painted the same way (`Edit::SetMaskPaint`), and the selection in Quick
//! Mask (`Edit::SetSelection`): both in gray, white showing or selecting, black hiding.
//!
//! A stroke reaches the whole canvas: a layer that does not cover it grows first, by whole
//! tiles before it (its pixels, original and mask move with it, its transform compensates), so
//! the layer looks the same until painted; the growth is part of the stroke's undo entry.

use std::sync::{Arc, Mutex};

use serde::Deserialize;
use slopshop_core::paint::{Brush, Paint, PointerSample, Stroke, canvas_growth, gray_of_srgb};
use slopshop_core::selection::{Selection, sample_colors, select_all};
use slopshop_core::{
    Affine, Document, Edit, LayerContent, LayerId, LayerMask, LinearRgba, RasterImage, Size,
};
use tauri::Manager;

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
    /// The selection in Quick Mask, in gray.
    Selection,
}

/// A stroke's target in a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Target {
    Layer(LayerId),
    Mask(LayerId),
    Selection,
}

impl PaintRequest {
    fn target(&self) -> Target {
        let id = LayerId::from_raw(self.layer_id);
        match self.target {
            PaintTarget::Layer => Target::Layer(id),
            PaintTarget::Mask => Target::Mask(id),
            PaintTarget::Selection => Target::Selection,
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

/// A layer grown to cover the canvas (see the module documentation): its new transform, its
/// grown original (the unpainted pixels) and its grown mask, if any.
#[derive(Debug, Clone)]
pub struct Growth {
    pub(crate) transform: Affine,
    pub(crate) original: Arc<RasterImage>,
    pub(crate) mask: Option<LayerMask>,
}

#[derive(Default)]
pub struct PaintState {
    stroke: Mutex<Option<ActiveStroke>>,
}

/// What a document shows of a stroke under way: its target as painted so far.
#[derive(Debug, Clone)]
pub struct PaintPreview {
    target: Target,
    pub image: Arc<RasterImage>,
    pub growth: Option<Growth>,
}

impl PaintPreview {
    /// `doc` (a snapshot being rendered) showing the stroke.
    pub fn apply_to(&self, doc: &mut Document) {
        // The layer may have gone meanwhile: then nothing to show.
        let _ = paint_edit(self.target, Arc::clone(&self.image), self.growth.as_ref()).apply(doc);
    }
}

/// The edit that gives `target` the painted `image`, a layer grown first by `growth` if any.
pub(crate) fn paint_edit(target: Target, image: Arc<RasterImage>, growth: Option<&Growth>) -> Edit {
    let layer = match target {
        Target::Layer(id) => id,
        Target::Mask(id) => {
            return Edit::SetMaskPaint {
                id,
                painted: Some(image),
            };
        }
        // A stroke paints a gray image of the selection's: always a selection.
        Target::Selection => {
            return Edit::SetSelection {
                selection: Selection::new(image),
            };
        }
    };
    let Some(growth) = growth else {
        return Edit::SetLayerPaint {
            id: layer,
            painted: Some(image),
        };
    };
    let mut edits = vec![
        Edit::SetLayerPixels {
            id: layer,
            image,
            original: Some(Arc::clone(&growth.original)),
        },
        Edit::SetLayerTransform {
            id: layer,
            transform: growth.transform,
        },
    ];
    if let Some(mask) = &growth.mask {
        edits.push(Edit::SetLayerMask {
            id: layer,
            mask: Some(mask.clone()),
        });
    }
    Edit::Batch(edits)
}

/// The growth of layer `id` that lets a stroke reach the whole canvas of `doc`, if it needs one
/// (see the module documentation): its pixels to paint, and the growth.
pub(crate) fn grow(
    doc: &Document,
    id: LayerId,
) -> Result<Option<(Arc<RasterImage>, Growth)>, String> {
    let layer = doc.layer(id).ok_or("the painted layer is gone")?;
    let LayerContent::Raster { image, original } = &layer.content else {
        return Ok(None);
    };
    let parent = doc.parent_transform(id);
    let Some(((left, top), size)) =
        canvas_growth(image.size(), layer.transform.then(parent), doc.size())
    else {
        return Ok(None);
    };
    let shown = grown_pixels(image, (left, top), size)?;
    let unpainted = match original {
        Some(original) => grown_pixels(original, (left, top), size)?,
        None => Arc::clone(&shown),
    };
    let mask = match &layer.mask {
        Some(mask) => Some(grown_mask(mask, (left, top), size)?),
        None => None,
    };
    let transform = grown_transform(layer.transform, (left, top));
    Ok(Some((
        shown,
        Growth {
            transform,
            original: unpainted,
            mask,
        },
    )))
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
            let layer = doc.layer(id).ok_or("the painted layer is gone")?;
            let LayerContent::Raster { image, .. } = &layer.content else {
                return Err("only raster layers can be painted".to_owned());
            };
            let (image, growth) = match grow_layer.then(|| grow(doc, id)).transpose()?.flatten() {
                Some((grown, growth)) => (grown, Some(growth)),
                None => (Arc::clone(image), None),
            };
            // Into the parent, then into the document.
            let transform = growth.as_ref().map_or(layer.transform, |g| g.transform);
            let paint = match request.color {
                Some([r, g, b]) => {
                    Paint::Color(LinearRgba::from_srgb_encoded_to_working(r, g, b, 1.0))
                }
                None => Paint::Erase,
            };
            let to_document = transform.then(doc.parent_transform(id));
            (image, to_document, growth, paint, selection)
        }
        // A mask lies in its layer's pixel grid and covers what the layer can show: it does not
        // grow.
        Target::Mask(id) => {
            let layer = doc.layer(id).ok_or("the painted layer is gone")?;
            let mask = layer.mask.as_ref().ok_or("the layer has no mask")?;
            let to_document = layer.transform.then(doc.parent_transform(id));
            (Arc::clone(&mask.image), to_document, None, gray, selection)
        }
        // Without a selection everything is selected (Quick Mask shows no tint): black then
        // unselects from the whole canvas. The selection does not limit its own paint.
        Target::Selection => {
            let image = match selection {
                Some(image) => image,
                None => Arc::new(select_all(doc.size()).map_err(|e| e.to_string())?),
            };
            (image, Affine::IDENTITY, None, gray, None)
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
                let mut documents = state.documents()?;
                let document = documents.get_mut(document_id)?;
                let (stroke, growth) = start(document.session.document(), &request, true)?;
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
        let painted = active.stroke.has_paint();
        let image = active.stroke.image().map_err(|e| e.to_string())?;
        let computed = started.elapsed();

        let mut documents = state.documents()?;
        let document = documents.get_mut(document_id)?;
        let view = if request.end {
            document.paint_preview = None;
            if painted {
                document
                    .session
                    .perform(paint_edit(active.target, image, active.growth.as_ref()))
                    .map_err(|e| e.to_string())?;
            }
            Some(document.view())
        } else {
            document.paint_preview = Some(PaintPreview {
                target: active.target,
                image,
                growth: active.growth.clone(),
            });
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

/// Delete with a selection (ADR 0027): the selected part of layer `layer_id` erased (`color`
/// absent: only alpha changes) or filled with `color` (sRGB-encoded RGB in `[0, 1]`), kept
/// apart from its original like a stroke. A fill grows the layer to the canvas if needed, as
/// strokes do. With `target` the mask, the mask is hidden there or filled with the color's
/// gray. Nothing selected: nothing changes.
#[tauri::command]
pub async fn fill_selection(
    app: tauri::AppHandle,
    document_id: u64,
    layer_id: u64,
    target: PaintTarget,
    color: Option<[f32; 3]>,
) -> Result<DocumentView, String> {
    on_worker(move || {
        let state = app.state::<AppState>();
        let mut documents = state.documents()?;
        let document = documents.get_mut(document_id)?;
        if document.session.document().selection().is_none() {
            return Ok(document.view());
        }
        if target == PaintTarget::Selection {
            return Err("the selection cannot fill itself".to_owned());
        }
        let request = PaintRequest {
            stroke: 0,
            target,
            layer_id,
            brush: BrushRequest {
                size: 1.0,
                hardness: 1.0,
                spacing: 1.0,
                flow: 1.0,
                opacity: 1.0,
                pressure_size: false,
                pressure_opacity: false,
            },
            color,
            samples: Vec::new(),
            end: true,
        };
        // Erasing only removes: no need to grow.
        let (mut stroke, growth) = start(document.session.document(), &request, color.is_some())?;
        stroke.fill();
        if let Some(image) = stroke.finish().map_err(|e| e.to_string())? {
            document
                .session
                .perform(paint_edit(request.target(), image, growth.as_ref()))
                .map_err(|e| e.to_string())?;
        }
        Ok(document.view())
    })
    .await
}

/// The color picker's eyedropper: the color shown at document point (`x`, `y`), every visible
/// layer composited, as whole 8-bit sRGB values. `None` outside the canvas or where nothing is
/// shown (transparent).
#[tauri::command]
pub async fn sample_color(
    app: tauri::AppHandle,
    document_id: u64,
    x: f64,
    y: f64,
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
        Ok(sample_color_at(&doc, x, y))
    })
    .await
}

/// [`sample_color`]'s work.
pub(crate) fn sample_color_at(doc: &Document, x: f64, y: f64) -> Option<[u8; 3]> {
    // Negative or NaN: outside. Too large saturates, then falls outside the canvas.
    if !(x >= 0.0 && y >= 0.0) {
        return None;
    }
    let [r, g, b, alpha] = *sample_colors(doc, &[(x as u32, y as u32)]).first()?;
    (alpha > 0.0).then(|| [r, g, b].map(|v| v.round().clamp(0.0, 255.0) as u8))
}
