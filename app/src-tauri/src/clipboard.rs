//! Edit > Cut, Copy, Copy Merged, Paste, Paste in Place and Paste Into, as in Photoshop.
//!
//! A copy is kept here whole: layers (a group with its content, masks, adjustments, paint…)
//! whose pixels are shared, the selected pixels of a layer, or the composite of the visible
//! layers in the selection; nothing is rasterized needlessly. The system clipboard gets an
//! 8-bit sRGB image of it for other applications, and its fingerprint is kept: Paste brings
//! SlopShop's own copy back while the system clipboard still holds that image, otherwise what
//! another application put there (files open as layers, an image becomes one).

use slopshop_core::HistoryLabel;
use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use slopshop_core::copy::{PasteMode, Rows, fingerprint, limit_to_selection, paste_offset};
use slopshop_core::move_pixels::{MoveMode, PixelMove};
use slopshop_core::transform::Affine;
use slopshop_core::{
    BlendMode, BlendSpace, Document, Layer, LayerContent, LayerId, RasterImage, Rect, Size,
};
use slopshop_render::Renderer;
use tauri::{AppHandle, Manager};

use crate::ipc::DocumentView;
use crate::paint::PaintTarget;
use crate::selection::on_worker;
use crate::{AppState, OpenDocument};

/// Rows composited at a time: bounds the `f32` buffer of a copy (512 rows of 16 384 pixels are
/// 128 MiB).
const BAND_ROWS: u32 = 512;

/// Largest image the system clipboard gets (8-bit RGBA: 400 MB). A larger copy is only
/// SlopShop's: other applications would choke on it anyway.
const MAX_STANDARD_PIXELS: u64 = 100_000_000;

/// What was copied (Edit > Copy, Cut or Copy Merged).
#[derive(Clone)]
pub struct Copied {
    /// Top-level layers placed where they were in the document (a copied layer's groups'
    /// transforms are folded into its own), bottom to top.
    layers: Vec<Layer>,
    /// Their warnings, depth first as `Layer::subtree` walks them.
    warnings: Vec<Vec<&'static str>>,
    /// The source document's blend space and size.
    space: BlendSpace,
    size: Size,
    /// Where the layers' pixels lie, `[x0, y0, x1, y1)`, if they have any.
    bounds: Option<[f64; 4]>,
    /// Fingerprint of the image the system clipboard got, if it got one.
    fingerprint: Option<u64>,
    /// An image another application copied: it has no place (`bounds` is `None`).
    external: bool,
}

/// What Edit > Copy takes.
#[derive(Debug, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CopyRequest {
    /// The selected layers, whole (no selection, or not a raster layer).
    Layers { ids: Vec<u64> },
    /// The selected pixels of a raster layer, or of its mask when it is the target, with their
    /// alpha and place.
    Pixels { layer_id: u64, target: PaintTarget },
    /// Edit > Copy Merged: the visible layers composited, in the selection (or the whole
    /// canvas), as one layer named `name`.
    Merged { name: String },
}

/// Edit > Copy and Copy Merged (Cut is a copy, then the UI deletes the source). Resolves with
/// whether something was copied: `false` when the selection holds nothing of the layer.
#[tauri::command]
pub async fn copy(app: AppHandle, document_id: u64, request: CopyRequest) -> Result<bool, String> {
    let (document, warnings) = {
        let state = app.state::<AppState>();
        let mut documents = state.documents()?;
        let open = documents.get_mut(document_id)?;
        (open.session.document().clone(), open.layer_warnings.clone())
    };
    let worker = app.clone();
    on_worker(move || {
        let state = worker.state::<AppState>();
        // Without a GPU, the CPU compositor gives the same pixels, only slower.
        let renderer = state.renderer().ok();
        let Some(mut copied) = take(renderer, &document, &warnings, request)? else {
            return Ok(false);
        };
        copied.fingerprint = offer(renderer, &copied);
        *state
            .layer_clipboard
            .lock()
            .map_err(|_| "clipboard state is poisoned".to_owned())? = Some(copied);
        Ok(true)
    })
    .await
}

/// What `request` copies from `document`. Blocking.
fn take(
    renderer: Option<&Renderer>,
    document: &Document,
    warnings: &std::collections::HashMap<LayerId, Vec<&'static str>>,
    request: CopyRequest,
) -> Result<Option<Copied>, String> {
    let space = document.blend_space();
    let size = document.size();
    let layers: Vec<Layer> = match request {
        CopyRequest::Layers { ids } => {
            let ids: Vec<LayerId> = ids.into_iter().map(LayerId::from_raw).collect();
            document
                .outermost(&ids)
                .into_iter()
                .filter_map(|id| {
                    let mut layer = document.layer(id)?.clone();
                    layer.transform = layer.transform.then(document.parent_transform(id));
                    Some(layer)
                })
                .collect()
        }
        CopyRequest::Pixels { layer_id, target } => {
            match selected_pixels(document, LayerId::from_raw(layer_id), target)? {
                Some(layer) => vec![layer],
                None => return Ok(None),
            }
        }
        CopyRequest::Merged { name } => match merged(renderer, document, name)? {
            Some(layer) => vec![layer],
            None => return Ok(None),
        },
    };
    if layers.is_empty() {
        return Err("nothing to copy".to_owned());
    }
    let warnings = layers
        .iter()
        .flat_map(Layer::subtree)
        .map(|layer| warnings.get(&layer.id).cloned().unwrap_or_default())
        .collect();
    let ids: Vec<LayerId> = layers.iter().map(|l| l.id).collect();
    let bounds = slopshop_core::pick::bounds_of(&holding(size, space, &layers)?, &ids)
        .map(|b| [b.left as f64, b.top as f64, b.right as f64, b.bottom as f64]);
    Ok(Some(Copied {
        layers,
        warnings,
        space,
        size,
        bounds,
        fingerprint: None,
        external: false,
    }))
}

/// A document of `size` holding only `layers` (as copied), to composite them or measure them.
fn holding(size: Size, space: BlendSpace, layers: &[Layer]) -> Result<Document, String> {
    let next = layers
        .iter()
        .flat_map(Layer::subtree)
        .map(|layer| layer.id.get())
        .max()
        .unwrap_or(0)
        + 1;
    Document::restore(
        size,
        slopshop_core::color::WORKING_SPACE,
        space,
        layers.to_vec(),
        next,
    )
    .map_err(|e| format!("{e:?}"))
}

/// A new top-level layer named `name` showing `image` placed by `transform`.
fn raster_layer(name: String, image: Arc<RasterImage>, transform: Affine) -> Layer {
    Layer {
        style: None,
        id: LayerId::from_raw(1),
        name,
        visible: true,
        opacity: 1.0,
        blend_mode: BlendMode::Normal,
        content: LayerContent::raster(image),
        mask: None,
        clipped: false,
        transform: transform.into(),
    }
}

/// The selected pixels of `layer_id`'s pixels or mask, as a layer where they were (named after
/// it); `None` when the selection holds nothing of it.
fn selected_pixels(
    document: &Document,
    layer_id: LayerId,
    target: PaintTarget,
) -> Result<Option<Layer>, String> {
    let selection = document.selection().ok_or("nothing is selected")?;
    let layer = document.layer(layer_id).ok_or("the layer is gone")?;
    let image = match (target, &layer.content) {
        (PaintTarget::Mask, _) => Arc::clone(&layer.mask.as_ref().ok_or("no mask")?.image),
        (PaintTarget::Layer, LayerContent::Raster { image, .. }) => image.get(),
        _ => return Err("only a raster layer's pixels or a mask are copied".to_owned()),
    };
    // A mask's grays are copied as a gray layer, given an alpha channel like any layer's.
    let to_document =
        crate::paint::affine_placement(layer.transform.then(document.parent_transform(layer_id)))?;
    let moving = PixelMove::new(
        image,
        to_document,
        selection,
        document.blend_space(),
        MoveMode::Copy,
        false,
    )
    .map_err(|e| e.to_string())?;
    Ok(moving
        .extract()
        .map_err(|e| e.to_string())?
        .map(|extracted| raster_layer(layer.name.clone(), extracted.image, extracted.to_document)))
}

/// Edit > Copy Merged: the visible layers composited within the selection's bounds (the whole
/// canvas without one), limited to it where it is soft, as a layer named `name`.
fn merged(
    renderer: Option<&Renderer>,
    document: &Document,
    name: String,
) -> Result<Option<Layer>, String> {
    Ok(merged_image(renderer, document)?.map(|(image, region)| {
        let at = Affine::translation(f64::from(region.x), f64::from(region.y));
        raster_layer(name, image, at)
    }))
}

/// What Copy Merged takes (Edit > Define Pattern too, ADR 0042): the visible layers composited
/// within the selection's bounds (the whole canvas without one), limited to it where it is
/// soft, and where that is in the document; `None` for an empty selection. Blocking.
pub(crate) fn merged_image(
    renderer: Option<&Renderer>,
    document: &Document,
) -> Result<Option<(Arc<RasterImage>, Rect)>, String> {
    let size = document.size();
    let selection = document.selection();
    let region = match selection {
        Some(selection) => match slopshop_core::selection::bounds(selection.image()) {
            Some(bounds) => bounds,
            None => return Ok(None),
        },
        None => Rect::new(0, 0, size.width, size.height),
    };
    let rows = Rows::merged(region.width, document.blend_space()).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    composite_bands(renderer, document, region, |band, pixels, first_row| {
        if let Some(selection) = selection {
            limit_to_selection(pixels, band, selection);
        }
        rows.convert(pixels, first_row, &mut bytes)
            .map_err(|e| e.to_string())
    })?;
    let image = RasterImage::from_pixels(region.size(), slopshop_core::copy::MERGED_FORMAT, &bytes)
        .map_err(|e| e.to_string())?;
    Ok(Some((Arc::new(image), region)))
}

/// `region` of `document` composited band by band (premultiplied working-space `f32`, on the
/// GPU when there is one), each band given to `each` with its rectangle and its first row
/// within the region. Blocking.
pub(crate) fn composite_bands(
    renderer: Option<&Renderer>,
    document: &Document,
    region: Rect,
    mut each: impl FnMut(Rect, &mut [f32], u32) -> Result<(), String>,
) -> Result<(), String> {
    let mut band = Vec::new();
    let mut y = 0;
    while y < region.height {
        let rows = BAND_ROWS.min(region.height - y);
        let rect = Rect::new(region.x, region.y + y, region.width, rows);
        band.clear();
        band.resize(rect.size().pixel_count() as usize * 4, 0.0f32);
        // Without a GPU (or when it fails), the CPU compositor gives the same pixels.
        let on_gpu = renderer.is_some_and(|r| r.render_region(document, rect, &mut band).is_ok());
        if !on_gpu {
            slopshop_core::composite::composite_region(document, rect, &mut band)
                .map_err(|e| format!("{e:?}"))?;
        }
        each(rect, &mut band, y)?;
        y += rows;
    }
    Ok(())
}

/// Put an 8-bit sRGB image of `copied` (its layers composited over the source canvas) on the
/// system clipboard for other applications, and return its fingerprint. Without one (nothing
/// on the canvas, too large, or the clipboard is busy), the system clipboard is emptied so
/// that the next Paste brings `copied` rather than an older image. Blocking.
fn offer(renderer: Option<&Renderer>, copied: &Copied) -> Option<u64> {
    let image = standard_image(renderer, copied)
        .inspect_err(|e| eprintln!("copy without an image for other applications: {e}"))
        .ok()
        .flatten();
    let mut clipboard = arboard::Clipboard::new().ok()?;
    let Some((size, bytes)) = image else {
        // Best effort: a clipboard another application holds open stays as it is.
        let _ = clipboard.clear();
        return None;
    };
    let print = fingerprint(size, &bytes);
    let data = arboard::ImageData {
        width: size.width as usize,
        height: size.height as usize,
        bytes: Cow::Owned(bytes),
    };
    match clipboard.set_image(data) {
        Ok(()) => Some(print),
        Err(e) => {
            eprintln!("cannot put the copy on the system clipboard: {e}");
            let _ = clipboard.clear();
            None
        }
    }
}

/// The image other applications get: `copied` composited where it lies on the source canvas.
fn standard_image(
    renderer: Option<&Renderer>,
    copied: &Copied,
) -> Result<Option<(Size, Vec<u8>)>, String> {
    let Some([x0, y0, x1, y1]) = copied.bounds else {
        return Ok(None);
    };
    let canvas = copied.size;
    let clamp = |v: f64, end: u32| v.clamp(0.0, f64::from(end)) as u32;
    let (left, top) = (
        clamp(x0.floor(), canvas.width),
        clamp(y0.floor(), canvas.height),
    );
    let (right, bottom) = (
        clamp(x1.ceil(), canvas.width),
        clamp(y1.ceil(), canvas.height),
    );
    if left >= right || top >= bottom {
        return Ok(None);
    }
    let region = Rect::new(left, top, right - left, bottom - top);
    if region.size().pixel_count() > MAX_STANDARD_PIXELS {
        return Ok(None);
    }
    let document = holding(canvas, copied.space, &copied.layers)?;
    let rows = Rows::standard(region.width, copied.space).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    composite_bands(renderer, &document, region, |_, pixels, first_row| {
        rows.convert(pixels, first_row, &mut bytes)
            .map_err(|e| e.to_string())
    })?;
    Ok(Some((region.size(), bytes)))
}

/// What the system clipboard holds.
enum SystemContent {
    /// Files copied in the file manager.
    Files(Vec<PathBuf>),
    /// An image (copied from a browser, a screenshot, SlopShop…): 8-bit sRGB, straight alpha.
    Image(Size, Vec<u8>),
    Nothing,
}

/// Read the system clipboard: copied files first (the file manager may also put an icon image),
/// then an image. Blocking.
fn read_system() -> Result<SystemContent, String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    if let Ok(files) = clipboard.get().file_list()
        && !files.is_empty()
    {
        return Ok(SystemContent::Files(files));
    }
    match clipboard.get_image() {
        Ok(image) => {
            let size = Size::new(
                u32::try_from(image.width).map_err(|e| e.to_string())?,
                u32::try_from(image.height).map_err(|e| e.to_string())?,
            );
            Ok(SystemContent::Image(size, image.bytes.into_owned()))
        }
        Err(arboard::Error::ContentNotAvailable) => Ok(SystemContent::Nothing),
        Err(e) => Err(e.to_string()),
    }
}

/// Which paste (Edit > Paste, Paste in Place, Paste Into).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PasteKind {
    Paste,
    InPlace,
    Into,
    /// Paste Here (the image's right-click menu): centered on the point clicked.
    At,
}

/// The outcome of `paste`.
#[derive(Debug, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Pasted {
    /// Files copied in the file manager: the UI opens them (as layers, or tabs).
    Files { paths: Vec<String> },
    /// The pasted layers, in `document` (a new tab when `new_tab`); `ids` are the new
    /// top-level layers (the group of a Paste Into).
    Layers {
        /// Boxed: a document view is much larger than the other variants.
        document: Box<DocumentView>,
        new_tab: bool,
        ids: Vec<u64>,
    },
    /// Neither files, nor an image, nor a copy.
    Nothing,
    /// Paste Into without a selection.
    NoSelection,
}

/// Paste into `document_id` (or a new document without one): SlopShop's copy while the system
/// clipboard still holds its image, else what another application put there. `name` names an
/// image from elsewhere and the group of a Paste Into; `view` is the part of the document
/// shown, `[x0, y0, x1, y1)`, where a paste out of sight lands (see `paste_offset`); `at` the
/// point of a Paste Here.
#[tauri::command]
pub async fn paste(
    app: AppHandle,
    document_id: Option<u64>,
    name: String,
    kind: PasteKind,
    view: Option<[f64; 4]>,
    at: Option<[f64; 2]>,
) -> Result<Pasted, String> {
    let system = on_worker(read_system).await?;
    let state = app.state::<AppState>();
    let ours = state
        .layer_clipboard
        .lock()
        .map_err(|_| "clipboard state is poisoned".to_owned())?
        .clone();
    let copied = match system {
        SystemContent::Files(paths) => {
            let paths = paths.iter().map(|p| p.display().to_string()).collect();
            return Ok(Pasted::Files { paths });
        }
        SystemContent::Image(size, bytes) => match ours {
            Some(copied) if copied.fingerprint == Some(fingerprint(size, &bytes)) => copied,
            _ => external(size, &bytes, &name)?,
        },
        SystemContent::Nothing => match ours {
            Some(copied) => copied,
            None => return Ok(Pasted::Nothing),
        },
    };
    let mut documents = state.documents()?;
    let Some(document_id) = document_id else {
        drop(documents);
        let (session, warnings) = new_document(copied)?;
        let document = state.add_document_with(session, Some(name), warnings, None)?;
        let ids = document.layers.iter().map(|l| l.id).collect();
        return Ok(Pasted::Layers {
            document: Box::new(document),
            new_tab: true,
            ids,
        });
    };
    let target = documents.get_mut(document_id)?;
    match place(target, copied, &name, kind, view, at)? {
        Some(ids) => Ok(Pasted::Layers {
            document: Box::new(target.view()),
            new_tab: false,
            ids,
        }),
        None => Ok(Pasted::NoSelection),
    }
}

/// An image another application copied, as a copy without a place.
fn external(size: Size, bytes: &[u8], name: &str) -> Result<Copied, String> {
    // System clipboard images are 8-bit sRGB RGBA with straight alpha on every platform.
    let image =
        RasterImage::from_pixels(size, slopshop_core::color::PixelFormat::RGBA8_SRGB, bytes)
            .map_err(|e| e.to_string())?;
    Ok(Copied {
        layers: vec![raster_layer(
            name.to_owned(),
            Arc::new(image),
            Affine::IDENTITY,
        )],
        warnings: vec![Vec::new()],
        space: BlendSpace::Perceptual,
        size,
        bounds: None,
        fingerprint: None,
        external: true,
    })
}

/// Paste `copied` on top of `target` (one undo entry), placed as `kind` says; the new top-level
/// layers, or `None` for a Paste Into without a selection.
fn place(
    target: &mut OpenDocument,
    copied: Copied,
    name: &str,
    kind: PasteKind,
    view: Option<[f64; 4]>,
    at: Option<[f64; 2]>,
) -> Result<Option<Vec<u64>>, String> {
    let document = target.session.document();
    let canvas = document.size();
    let mode = match kind {
        PasteKind::Paste => PasteMode::Paste,
        PasteKind::At => match at {
            Some(point) => PasteMode::At { point },
            None => PasteMode::Paste,
        },
        PasteKind::InPlace => PasteMode::InPlace,
        PasteKind::Into => {
            let Some(selection) = document.selection() else {
                return Ok(None);
            };
            let Some(b) = slopshop_core::selection::bounds(selection.image()) else {
                return Ok(None);
            };
            PasteMode::Into {
                selection: [
                    f64::from(b.x),
                    f64::from(b.y),
                    b.right() as f64,
                    b.bottom() as f64,
                ],
            }
        }
    };
    let (dx, dy) = match copied.bounds {
        Some(bounds) => paste_offset(bounds, true, mode, view, canvas),
        // Layers without pixels (adjustments): nothing to place.
        None if !copied.external => (0, 0),
        // An image from elsewhere: its size, at the origin.
        None => {
            let size = [
                0.0,
                0.0,
                f64::from(copied.size.width),
                f64::from(copied.size.height),
            ];
            paste_offset(size, false, mode, view, canvas)
        }
    };
    let shift = Affine::translation(dx as f64, dy as f64);
    let layers: Vec<Layer> = copied
        .layers
        .into_iter()
        .map(|mut layer| {
            layer.transform = layer.transform.then(shift.into());
            layer
        })
        .collect();
    let other_space = copied.space != document.blend_space();
    let copies = if kind == PasteKind::Into {
        match target
            .session
            .with_label(Some(HistoryLabel::new("pasteInto")), |s| {
                s.insert_into_selection(&layers, name.to_owned())
            })
            .map_err(|e| e.to_string())?
        {
            Some(copies) => copies,
            None => return Ok(None),
        }
    } else {
        target
            .session
            .with_label(Some(HistoryLabel::new("paste")), |s| {
                s.insert_layer_copies(&layers, None)
            })
            .map_err(|e| e.to_string())?
    };
    for (id, mut warnings) in copies.ids.iter().copied().zip(copied.warnings) {
        if other_space {
            warnings.push(crate::WARNING_BLEND_SPACE);
        }
        if !warnings.is_empty() {
            target.layer_warnings.insert(id, warnings);
        }
    }
    let document = target.session.document();
    let top: Vec<u64> = match copies.group {
        Some(group) => vec![group.get()],
        None => copies
            .ids
            .iter()
            .filter(|id| document.layers().iter().any(|l| l.id == **id))
            .map(|id| id.get())
            .collect(),
    };
    Ok(Some(top))
}

/// The size of a document made from `copied`, as Photoshop makes one from the clipboard: the
/// size of what was copied (the source's canvas for layers without pixels), and the move that
/// brings its content to the origin.
fn document_size(copied: &Copied) -> (Size, Affine) {
    match copied.bounds {
        Some([x0, y0, x1, y1]) if x1 > x0 && y1 > y0 => {
            let (x0, y0) = (x0.floor(), y0.floor());
            (
                Size::new((x1 - x0).ceil() as u32, (y1 - y0).ceil() as u32),
                Affine::translation(-x0, -y0),
            )
        }
        _ => (copied.size, Affine::IDENTITY),
    }
}

/// File > New's Clipboard preset: the size of the document Paste would make (see
/// `document_size`), from SlopShop's copy or an image another application copied; `None`
/// without either.
#[tauri::command]
pub async fn clipboard_size(app: AppHandle) -> Result<Option<(u32, u32)>, String> {
    let system = on_worker(read_system).await?;
    let state = app.state::<AppState>();
    let ours = state
        .layer_clipboard
        .lock()
        .map_err(|_| "clipboard state is poisoned".to_owned())?
        .clone();
    let size = match (system, ours) {
        (SystemContent::Image(size, bytes), Some(copied))
            if copied.fingerprint == Some(fingerprint(size, &bytes)) =>
        {
            Some(document_size(&copied).0)
        }
        (SystemContent::Image(size, _), _) => Some(size),
        (SystemContent::Nothing, Some(copied)) => Some(document_size(&copied).0),
        _ => None,
    };
    Ok(size.map(|s| (s.width, s.height)))
}

/// What the clipboard holds, for the menus to gray the pastes that do not apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ClipboardContents {
    Nothing,
    /// Files copied in the file manager.
    Files,
    /// An image another application copied: it has no place.
    Image,
    /// SlopShop's own copy, with a place (pixels).
    Placed,
    /// SlopShop's own copy without pixels (adjustment layers).
    Layers,
}

/// What a paste would bring (see `paste`).
#[tauri::command]
pub async fn clipboard_contents(app: AppHandle) -> Result<ClipboardContents, String> {
    let system = on_worker(read_system).await?;
    let state = app.state::<AppState>();
    let ours = state
        .layer_clipboard
        .lock()
        .map_err(|_| "clipboard state is poisoned".to_owned())?
        .clone();
    let own = |copied: &Copied| {
        if copied.bounds.is_some() {
            ClipboardContents::Placed
        } else {
            ClipboardContents::Layers
        }
    };
    Ok(match (system, ours) {
        (SystemContent::Files(_), _) => ClipboardContents::Files,
        (SystemContent::Image(size, bytes), Some(copied))
            if copied.fingerprint == Some(fingerprint(size, &bytes)) =>
        {
            own(&copied)
        }
        (SystemContent::Image(..), _) => ClipboardContents::Image,
        (SystemContent::Nothing, Some(copied)) => own(&copied),
        (SystemContent::Nothing, None) => ClipboardContents::Nothing,
    })
}

/// A new document holding `copied`, of its size (see `document_size`), its content at the
/// origin.
fn new_document(
    copied: Copied,
) -> Result<(slopshop_core::Session, Vec<Vec<&'static str>>), String> {
    let (size, shift) = document_size(&copied);
    let layers: Vec<Layer> = copied
        .layers
        .into_iter()
        .map(|mut layer| {
            layer.transform = layer.transform.then(shift.into());
            layer
        })
        .collect();
    let document = holding(size, copied.space, &layers)?;
    Ok((slopshop_core::Session::new(document), copied.warnings))
}

#[cfg(test)]
mod tests {
    use super::*;
    use slopshop_core::color::PixelFormat;
    use slopshop_core::selection::{Combine, EdgeOptions, Selection, Shape, select_shape};
    use slopshop_core::{Edit, LinearRgba, Session};

    const CANVAS: Size = Size::new(100, 80);

    /// An opaque gray raster layer of `size` placed by `transform`.
    fn raster(id: u64, size: Size, transform: Affine) -> Layer {
        let pixels = [200u8, 200, 200, 255].repeat(size.pixel_count() as usize);
        let image = RasterImage::from_pixels(size, PixelFormat::RGBA8_SRGB, &pixels).unwrap();
        Layer {
            id: LayerId::from_raw(id),
            ..raster_layer("layer".into(), Arc::new(image), transform)
        }
    }

    fn session(layers: Vec<Layer>) -> Session {
        let next = layers
            .iter()
            .flat_map(Layer::subtree)
            .map(|l| l.id.get())
            .max()
            .unwrap_or(0)
            + 1;
        let document = Document::restore(
            CANVAS,
            slopshop_core::color::WORKING_SPACE,
            BlendSpace::Perceptual,
            layers,
            next,
        )
        .unwrap();
        Session::new(document)
    }

    fn select(session: &mut Session, [left, top, right, bottom]: [f64; 4]) {
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

    /// `session` opened in `state`: its id.
    fn open(state: &AppState, session: Session) -> u64 {
        state.add_document(session, None, Vec::new()).unwrap().id
    }

    fn copy(document: &Document, request: CopyRequest) -> Copied {
        take(None, document, &Default::default(), request)
            .unwrap()
            .unwrap()
    }

    fn top_transform(state: &AppState, id: u64) -> Affine {
        let mut documents = state.documents().unwrap();
        let document = documents.get_mut(id).unwrap().session.document();
        document
            .layers()
            .last()
            .unwrap()
            .transform
            .as_affine()
            .unwrap()
    }

    #[test]
    fn layers_paste_where_they_were_when_in_sight_else_in_the_view() {
        let state = AppState::new();
        let layer = raster(1, Size::new(10, 10), Affine::translation(70.0, 60.0));
        let copied = copy(
            session(vec![layer]).document(),
            CopyRequest::Layers { ids: vec![1] },
        );
        assert_eq!(copied.bounds, Some([70.0, 60.0, 80.0, 70.0]));
        let id = open(&state, session(Vec::new()));
        let paste = |kind, view| {
            let mut documents = state.documents().unwrap();
            let target = documents.get_mut(id).unwrap();
            place(target, copied.clone(), "Pasted", kind, view, None).unwrap()
        };
        // In sight: where it was.
        paste(PasteKind::Paste, Some([50.0, 50.0, 100.0, 80.0])).unwrap();
        assert_eq!(top_transform(&state, id), Affine::translation(70.0, 60.0));
        // Out of sight: in the middle of the view (center 75, 65 → 25, 20).
        paste(PasteKind::Paste, Some([0.0, 0.0, 50.0, 40.0])).unwrap();
        assert_eq!(top_transform(&state, id), Affine::translation(20.0, 15.0));
        // In place: where it was, always; each paste is one undo entry.
        paste(PasteKind::InPlace, Some([0.0, 0.0, 50.0, 40.0])).unwrap();
        assert_eq!(top_transform(&state, id), Affine::translation(70.0, 60.0));
        let mut documents = state.documents().unwrap();
        let session = &mut documents.get_mut(id).unwrap().session;
        for _ in 0..3 {
            session.undo().unwrap();
        }
        assert!(session.document().layers().is_empty());
    }

    #[test]
    fn copied_layers_keep_their_place_out_of_their_groups() {
        let child = raster(2, Size::new(10, 10), Affine::translation(1.0, 2.0));
        let group = Layer {
            content: LayerContent::Group {
                children: vec![child],
                pass_through: false,
            },
            ..raster(1, Size::new(1, 1), Affine::translation(30.0, 40.0))
        };
        let copied = copy(
            session(vec![group]).document(),
            CopyRequest::Layers { ids: vec![2] },
        );
        assert_eq!(
            copied.layers[0].transform,
            Affine::translation(31.0, 42.0).into()
        );
        assert_eq!(copied.bounds, Some([31.0, 42.0, 41.0, 52.0]));
    }

    #[test]
    fn selected_pixels_are_copied_with_their_place() {
        let layer = raster(1, Size::new(50, 50), Affine::translation(20.0, 10.0));
        let mut source = session(vec![layer]);
        select(&mut source, [30.0, 20.0, 40.0, 25.0]);
        let pixels = || CopyRequest::Pixels {
            layer_id: 1,
            target: PaintTarget::Layer,
        };
        let copied = copy(source.document(), pixels());
        // The layer's tile, placed where the layer is; only the selected pixels are opaque.
        let layer = &copied.layers[0];
        assert_eq!(layer.transform, Affine::translation(20.0, 10.0).into());
        let LayerContent::Raster { image, .. } = &layer.content else {
            panic!("a raster layer");
        };
        let image = image.get();
        assert_eq!(image.alpha_at(10, 10), 1.0);
        assert_eq!(image.alpha_at(9, 10), 0.0);
        assert_eq!(image.alpha_at(19, 14), 1.0);
        assert_eq!(image.alpha_at(19, 15), 0.0);
        assert_eq!(copied.bounds, Some([30.0, 20.0, 40.0, 25.0]));
        // Nothing of the layer selected: nothing copied.
        select(&mut source, [0.0, 0.0, 10.0, 10.0]);
        let taken = take(None, source.document(), &Default::default(), pixels()).unwrap();
        assert!(taken.is_none());
    }

    #[test]
    fn copy_merged_is_the_composite_within_the_selection() {
        let fill = Layer {
            content: LayerContent::Fill {
                color: LinearRgba::new(0.25, 0.5, 1.0, 1.0),
            },
            ..raster(1, Size::new(1, 1), Affine::IDENTITY)
        };
        let mut source = session(vec![fill]);
        select(&mut source, [10.0, 20.0, 30.0, 25.0]);
        let merged = || CopyRequest::Merged {
            name: "Merged".into(),
        };
        let copied = copy(source.document(), merged());
        let layer = &copied.layers[0];
        assert_eq!(layer.name, "Merged");
        assert_eq!(layer.transform, Affine::translation(10.0, 20.0).into());
        let LayerContent::Raster { image, .. } = &layer.content else {
            panic!("a raster layer");
        };
        assert_eq!(image.size(), Size::new(20, 5));
        assert_eq!(image.format(), slopshop_core::copy::MERGED_FORMAT);
        let color = image
            .get()
            .average_color(&slopshop_core::color::WORKING_SPACE);
        assert!((color.b - 1.0).abs() < 1e-3 && (color.r - 0.25).abs() < 1e-3);
        // Without a selection, the whole canvas.
        source
            .perform(Edit::SetSelection { selection: None })
            .unwrap();
        let copied = copy(source.document(), merged());
        assert_eq!(copied.bounds, Some([0.0, 0.0, 100.0, 80.0]));
    }

    #[test]
    fn paste_into_masks_a_group_with_the_selection() {
        let state = AppState::new();
        let layer = raster(1, Size::new(10, 10), Affine::IDENTITY);
        let copied = copy(
            session(vec![layer]).document(),
            CopyRequest::Layers { ids: vec![1] },
        );
        let id = open(&state, session(Vec::new()));
        let mut documents = state.documents().unwrap();
        let target = documents.get_mut(id).unwrap();
        // Without a selection: nothing.
        let none = place(
            target,
            copied.clone(),
            "Pasted",
            PasteKind::Into,
            None,
            None,
        )
        .unwrap();
        assert!(none.is_none());
        select(&mut target.session, [50.0, 40.0, 60.0, 50.0]);
        let ids = place(target, copied, "Pasted", PasteKind::Into, None, None)
            .unwrap()
            .unwrap();
        let document = target.session.document();
        assert!(document.selection().is_none());
        let group = document.layer(LayerId::from_raw(ids[0])).unwrap();
        assert!(group.mask.is_some());
        // Centered on the selection, since it did not meet it.
        let child = &group.children().unwrap()[0];
        assert_eq!(child.transform, Affine::translation(50.0, 40.0).into());
        // One undo entry, which brings the selection back.
        target.session.undo().unwrap();
        assert!(target.session.document().selection().is_some());
        assert!(target.session.document().layers().is_empty());
    }

    #[test]
    fn an_image_from_elsewhere_lands_in_the_view_or_makes_a_document_of_its_size() {
        let state = AppState::new();
        let id = open(&state, session(Vec::new()));
        let image = external(Size::new(20, 10), &[255; 20 * 10 * 4], "Pasted").unwrap();
        {
            let mut documents = state.documents().unwrap();
            let target = documents.get_mut(id).unwrap();
            let view = Some([0.0, 0.0, 100.0, 80.0]);
            place(
                target,
                image.clone(),
                "Pasted",
                PasteKind::Paste,
                view,
                None,
            )
            .unwrap();
        }
        assert_eq!(top_transform(&state, id), Affine::translation(40.0, 35.0));
        let (session, _) = new_document(image).unwrap();
        assert_eq!(session.document().size(), Size::new(20, 10));
    }

    #[test]
    fn a_document_from_a_copy_has_the_size_of_the_copy() {
        let layer = raster(1, Size::new(50, 50), Affine::translation(20.0, 10.0));
        let mut source = session(vec![layer]);
        select(&mut source, [30.0, 20.0, 40.0, 25.0]);
        let copied = copy(
            source.document(),
            CopyRequest::Pixels {
                layer_id: 1,
                target: PaintTarget::Layer,
            },
        );
        let (session, _) = new_document(copied).unwrap();
        let document = session.document();
        assert_eq!(document.size(), Size::new(10, 5));
        let ids = [document.layers()[0].id];
        let bounds = slopshop_core::pick::bounds_of(document, &ids).unwrap();
        assert_eq!((bounds.left, bounds.top), (0, 0));
    }
}
