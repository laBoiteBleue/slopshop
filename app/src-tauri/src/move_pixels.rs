//! The Move tool dragged from inside a selection moves the selected pixels, as in Photoshop:
//! those of the active layer, or of its mask when the mask is the target. They leave a hole
//! (Alt copies them instead) and the selection follows them. Each request of a drag replaces
//! the drag's live edit (one undo entry per drag); the result is the layer's painted image
//! (ADR 0027), so the original stays intact. A layer grows with the pixels that land beyond
//! it (off the canvas too), its original, mask and transform with it, so nothing is lost.
//!
//! As in Photoshop, the pixels float until something else happens: the next drag moves them
//! from where they started, so what they covered at an intermediate place comes back. A drag
//! continues the float only while the document still shows what the last one left.

use std::sync::Arc;

use serde::Deserialize;
use slopshop_core::move_pixels::{MoveMode, Moved, PixelMove};
use slopshop_core::selection::{Selection, translated};
use slopshop_core::{Document, Edit, LayerContent, LayerId, RasterImage, Size};
use tauri::Manager;

use crate::AppState;
use crate::ipc::DocumentView;
use crate::paint::{
    Growth, PaintTarget, Target, grow, grown_mask, grown_pixels, grown_transform, paint_edit,
};
use crate::selection::on_worker;

/// A request of a drag moving selected pixels.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MovePixelsRequest {
    /// Requests of one drag share its id; a new id starts a new drag.
    pub drag: u64,
    /// The layer's pixels or its mask (not the selection).
    pub target: PaintTarget,
    pub layer_id: u64,
    /// The move since the drag began, whole document pixels.
    pub dx: i64,
    pub dy: i64,
    /// Alt: the pixels are copied, not cut.
    pub copy: bool,
    /// The drag is over: its move becomes one undo entry.
    pub end: bool,
}

/// Selected pixels floating (see the module documentation).
pub struct Floating {
    target: Target,
    moving: PixelMove,
    /// The selection the pixels started from.
    selection: Arc<RasterImage>,
    /// A layer as the float found it (grown to the canvas if it needed to, as for painting):
    /// its original, mask and transform, which every move sets again, grown if it grew.
    lifted: Option<Growth>,
    /// `lifted` grown with the pixels of the last move that grew (tiles before, size), kept
    /// while the next ones grow the same.
    grown: Option<((u32, u32), Size, Growth)>,
    /// The move the ended drags add up to, document pixels.
    offset: (i64, i64),
    /// What the last ended drag left: the target's pixels and the selection.
    shown: Option<(Arc<RasterImage>, Option<Selection>)>,
    /// The drag under way, if any.
    drag: Option<u64>,
}

impl Floating {
    /// Whether `doc` still shows what the last drag left, for a drag on `target`.
    fn continues(&self, doc: &Document, target: Target) -> bool {
        let Some((image, selection)) = &self.shown else {
            return false;
        };
        self.target == target
            && target_image(doc, target).is_ok_and(|shown| Arc::ptr_eq(&shown, image))
            && doc.selection() == selection.as_ref()
    }
}

/// The pixels `target` shows in `doc`.
fn target_image(doc: &Document, target: Target) -> Result<Arc<RasterImage>, String> {
    match target {
        Target::Layer(id) => {
            let layer = doc.layer(id).ok_or("the moved layer is gone")?;
            match &layer.content {
                LayerContent::Raster { image, .. } => Ok(Arc::clone(image)),
                _ => Err("only raster layers have pixels to move".to_owned()),
            }
        }
        Target::Mask(id) => {
            let layer = doc.layer(id).ok_or("the moved layer is gone")?;
            let mask = layer.mask.as_ref().ok_or("the layer has no mask")?;
            Ok(Arc::clone(&mask.image))
        }
        Target::Selection => Err("the selection cannot move its own pixels".to_owned()),
    }
}

/// Selected pixels of `target` in `doc`, ready to float: a layer grows to the canvas first,
/// as for painting.
fn lift(doc: &Document, target: Target, copy: bool) -> Result<Floating, String> {
    let selection = doc.selection().ok_or("nothing is selected")?;
    let (image, lifted, id) = match target {
        Target::Layer(id) => match grow(doc, id)? {
            Some((grown, growth)) => (grown, Some(growth), id),
            None => {
                let layer = doc.layer(id).ok_or("the moved layer is gone")?;
                let LayerContent::Raster { image, original } = &layer.content else {
                    return Err("only raster layers have pixels to move".to_owned());
                };
                let lifted = Growth {
                    transform: layer.transform,
                    original: Arc::clone(original.as_ref().unwrap_or(image)),
                    mask: layer.mask.clone(),
                };
                (Arc::clone(image), Some(lifted), id)
            }
        },
        Target::Mask(id) => (target_image(doc, target)?, None, id),
        Target::Selection => return Err("the selection cannot move its own pixels".to_owned()),
    };
    let layer = doc.layer(id).ok_or("the moved layer is gone")?;
    let transform = lifted.as_ref().map_or(layer.transform, |g| g.transform);
    let mode = if copy { MoveMode::Copy } else { MoveMode::Cut };
    let moving = PixelMove::new(
        image,
        transform.then(doc.parent_transform(id)),
        selection,
        doc.blend_space(),
        mode,
        matches!(target, Target::Mask(_)),
    )
    .map_err(|e| e.to_string())?;
    Ok(Floating {
        target,
        moving,
        selection: Arc::clone(selection.image()),
        lifted,
        grown: None,
        offset: (0, 0),
        shown: None,
        drag: None,
    })
}

/// The edit that gives `floating`'s target the `moved` pixels: a layer gets them with its
/// original, mask and transform as the float found it, grown with the pixels if they grew.
fn moved_edit(floating: &mut Floating, moved: &Moved) -> Result<Edit, String> {
    let image = Arc::clone(&moved.image);
    let Some(lifted) = &floating.lifted else {
        return Ok(paint_edit(floating.target, image, None));
    };
    let size = image.size();
    let offset = moved.grown.unwrap_or((0, 0));
    let growth = match &floating.grown {
        _ if offset == (0, 0) && size == lifted.original.size() => lifted.clone(),
        Some((o, s, growth)) if (*o, *s) == (offset, size) => growth.clone(),
        _ => {
            let growth = Growth {
                transform: grown_transform(lifted.transform, offset),
                original: grown_pixels(&lifted.original, offset, size)?,
                mask: lifted
                    .mask
                    .as_ref()
                    .map(|mask| grown_mask(mask, offset, size))
                    .transpose()?,
            };
            floating.grown = Some((offset, size, growth.clone()));
            growth
        }
    };
    Ok(paint_edit(floating.target, image, Some(&growth)))
}

/// Move the selected pixels (see the module documentation).
#[tauri::command]
pub async fn move_selected_pixels(
    app: tauri::AppHandle,
    document_id: u64,
    request: MovePixelsRequest,
) -> Result<DocumentView, String> {
    on_worker(move || move_pixels(&app.state::<AppState>(), document_id, &request)).await
}

/// [`move_selected_pixels`]'s work, on the calling thread.
pub(crate) fn move_pixels(
    state: &AppState,
    document_id: u64,
    request: &MovePixelsRequest,
) -> Result<DocumentView, String> {
    let id = LayerId::from_raw(request.layer_id);
    let target = match request.target {
        PaintTarget::Layer => Target::Layer(id),
        PaintTarget::Mask => Target::Mask(id),
        PaintTarget::Selection => return Err("the selection cannot move its own pixels".into()),
    };
    // The float is taken out while the pixels are computed: frames keep rendering meanwhile.
    let (mut floating, canvas) = {
        let mut documents = state.documents()?;
        let document = documents.get_mut(document_id)?;
        let previous = document.floating.take();
        if previous.as_ref().and_then(|f| f.drag) != Some(request.drag) {
            // A new drag: whatever gesture was under way is its own.
            document.session.end_gesture();
        }
        let doc = document.session.document();
        let floating = match previous {
            Some(f) if f.drag == Some(request.drag) => f,
            Some(f) if !request.copy && f.drag.is_none() && f.continues(doc, target) => f,
            _ => lift(doc, target, request.copy)?,
        };
        (floating, doc.size())
    };
    floating.drag = Some(request.drag);
    let (dx, dy) = (
        floating.offset.0 + request.dx,
        floating.offset.1 + request.dy,
    );
    let moved = floating.moving.image(dx, dy).map_err(|e| e.to_string())?;
    let selection = translated(canvas, &floating.selection, dx, dy)
        .map_err(|e| e.to_string())?
        .and_then(|image| Selection::new(Arc::new(image)));
    let edit = Edit::Batch(vec![
        moved_edit(&mut floating, &moved)?,
        Edit::SetSelection {
            selection: selection.clone(),
        },
    ]);

    let mut documents = state.documents()?;
    let document = documents.get_mut(document_id)?;
    let session = &mut document.session;
    session.cancel_gesture().map_err(|e| e.to_string())?;
    let shifted = (request.dx, request.dy) != (0, 0);
    if shifted {
        session
            .perform_in_gesture(edit)
            .map_err(|e| e.to_string())?;
    }
    if request.end {
        floating.drag = None;
        if shifted {
            session.end_gesture();
            floating.offset = (dx, dy);
            floating.shown = Some((moved.image, selection));
        }
    }
    document.floating = Some(floating);
    Ok(document.view())
}

/// The selection's bounds when document point (`x`, `y`) is inside it (selected at least
/// half, as its outline shows), else `None`: a Move tool drag from there moves the selected
/// pixels.
#[tauri::command]
pub async fn selection_bounds_at(
    app: tauri::AppHandle,
    document_id: u64,
    x: f64,
    y: f64,
) -> Result<Option<crate::BoundsDto>, String> {
    on_worker(move || {
        let state = app.state::<AppState>();
        let selection = {
            let mut documents = state.documents()?;
            let document = documents.get_mut(document_id)?;
            match document.session.document().selection() {
                Some(selection) => Arc::clone(selection.image()),
                None => return Ok(None),
            }
        };
        let size = selection.size();
        let inside = x >= 0.0
            && y >= 0.0
            && x < f64::from(size.width)
            && y < f64::from(size.height)
            && selection.gray_at(x as u32, y as u32) >= 0.5;
        Ok(inside
            .then(|| slopshop_core::selection::bounds(&selection))
            .flatten()
            .map(|b| crate::BoundsDto {
                left: i64::from(b.x),
                top: i64::from(b.y),
                right: b.right() as i64,
                bottom: b.bottom() as i64,
            }))
    })
    .await
}
