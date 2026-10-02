//! Painting (ADR 0027): the Brush and Eraser strokes, computed by the engine off the UI thread.
//!
//! A stroke is sent in batches of pointer samples while it is painted. Each batch extends the
//! engine's stroke and its result is shown as a preview in place of the layer's pixels (view
//! state, like the Quick Mask overlay: no history entry, no revision). The last batch commits
//! the stroke as one edit, `Edit::SetLayerPaint`: one undo entry, the layer's original kept.
//!
//! A stroke reaches the whole canvas: a layer that does not cover it grows first, by whole
//! tiles before it (its pixels, original and mask move with it, its transform compensates), so
//! the layer looks the same until painted; the growth is part of the stroke's undo entry.

use std::sync::{Arc, Mutex};

use serde::Deserialize;
use slopshop_core::paint::{Brush, Paint, PointerSample, Stroke, canvas_growth};
use slopshop_core::{
    Affine, Document, Edit, LayerContent, LayerId, LayerMask, LinearRgba, RasterImage,
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
    /// The layer painted, a raster layer.
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
    layer: LayerId,
    stroke: Stroke,
    growth: Option<Growth>,
}

/// A layer grown to cover the canvas (see the module documentation): its new transform, its
/// grown original (the unpainted pixels) and its grown mask, if any.
#[derive(Debug, Clone)]
pub struct Growth {
    transform: Affine,
    original: Arc<RasterImage>,
    mask: Option<LayerMask>,
}

#[derive(Default)]
pub struct PaintState {
    stroke: Mutex<Option<ActiveStroke>>,
}

/// What a document shows of a stroke under way: the layer's pixels as painted so far.
#[derive(Debug, Clone)]
pub struct PaintPreview {
    pub layer: LayerId,
    pub image: Arc<RasterImage>,
    pub growth: Option<Growth>,
}

impl PaintPreview {
    /// `doc` (a snapshot being rendered) showing the stroke.
    pub fn apply_to(&self, doc: &mut Document) {
        // The layer may have gone meanwhile: then nothing to show.
        let _ = paint_edit(self.layer, Arc::clone(&self.image), self.growth.as_ref()).apply(doc);
    }
}

/// The edit that gives `layer` the painted `image`, grown first by `growth` if any.
fn paint_edit(layer: LayerId, image: Arc<RasterImage>, growth: Option<&Growth>) -> Edit {
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
fn grow(doc: &Document, id: LayerId) -> Result<Option<(Arc<RasterImage>, Growth)>, String> {
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
    // Around the pixels, transparency: they need an alpha channel first (lossless).
    let grown = |image: &Arc<RasterImage>| -> Result<Arc<RasterImage>, String> {
        let with_alpha = match image.with_alpha() {
            Some(converted) => Arc::new(converted.map_err(|e| e.to_string())?),
            None => Arc::clone(image),
        };
        with_alpha
            .grown((left, top), size)
            .ok_or("the layer cannot grow")?
            .map(Arc::new)
            .map_err(|e| e.to_string())
    };
    let shown = grown(image)?;
    let unpainted = match original {
        Some(original) => grown(original)?,
        None => Arc::clone(&shown),
    };
    let mask = match &layer.mask {
        Some(mask) => {
            let image = mask
                .image
                .grown((left, top), size)
                .ok_or("the mask cannot grow")?
                .map_err(|e| e.to_string())?;
            let original = match &mask.original {
                Some(original) => Some(Arc::new(
                    original
                        .grown((left, top), size)
                        .ok_or("the mask cannot grow")?
                        .map_err(|e| e.to_string())?,
                )),
                None => None,
            };
            Some(LayerMask {
                image: Arc::new(image),
                original,
                ..mask.clone()
            })
        }
        None => None,
    };
    // The grown pixels start `left` × `top` tiles before the old ones.
    let shift = slopshop_core::raster::TILE_SIZE as f64;
    let transform = Affine::translation(-(f64::from(left) * shift), -(f64::from(top) * shift))
        .then(layer.transform);
    Ok(Some((
        shown,
        Growth {
            transform,
            original: unpainted,
            mask,
        },
    )))
}

/// A stroke on layer `layer_id` of `doc`, as the request asks, on the layer grown to the canvas
/// if it needs to (`grow`: strokes; not a clear, which only removes).
fn start(
    doc: &Document,
    request: &PaintRequest,
    grow_layer: bool,
) -> Result<(Stroke, Option<Growth>), String> {
    let id = LayerId::from_raw(request.layer_id);
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
    let to_document = transform.then(doc.parent_transform(id));
    let paint = match request.color {
        Some([r, g, b]) => Paint::Color(LinearRgba::from_srgb_encoded_to_working(r, g, b, 1.0)),
        None => Paint::Erase,
    };
    let stroke = Stroke::new(
        image,
        to_document,
        doc.selection().map(|s| Arc::clone(s.image())),
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
                    layer: LayerId::from_raw(request.layer_id),
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
                    .perform(paint_edit(active.layer, image, active.growth.as_ref()))
                    .map_err(|e| e.to_string())?;
            }
            Some(document.view())
        } else {
            document.paint_preview = Some(PaintPreview {
                layer: active.layer,
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

/// Edit > Clear (Delete with a selection): the selected part of layer `layer_id` erased, kept
/// apart from its original like a stroke of the Eraser (ADR 0027). Nothing selected, or the
/// selection outside the layer: nothing changes.
#[tauri::command]
pub async fn clear_selection(
    app: tauri::AppHandle,
    document_id: u64,
    layer_id: u64,
) -> Result<DocumentView, String> {
    on_worker(move || {
        let state = app.state::<AppState>();
        let mut documents = state.documents()?;
        let document = documents.get_mut(document_id)?;
        if document.session.document().selection().is_none() {
            return Ok(document.view());
        }
        let request = PaintRequest {
            stroke: 0,
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
            color: None,
            samples: Vec::new(),
            end: true,
        };
        let (mut stroke, _) = start(document.session.document(), &request, false)?;
        stroke.fill();
        if let Some(image) = stroke.finish().map_err(|e| e.to_string())? {
            document
                .session
                .perform(Edit::SetLayerPaint {
                    id: LayerId::from_raw(layer_id),
                    painted: Some(image),
                })
                .map_err(|e| e.to_string())?;
        }
        Ok(document.view())
    })
    .await
}
