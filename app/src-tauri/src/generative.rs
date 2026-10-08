//! Delete's generative fill (ADR 0045): the selection replaced by its background by the Erase
//! tool (FLUX.2 [klein] with `erase_v1`, in the `slopshop-ai` helper), then baked into the
//! layer's paint like the Patch. The model works on the region around the selection (its box
//! plus context, at most 1 Mpx at the model); the selection, grown by 2 % of that region and
//! kept within it, decides what changes, its soft edge included. Nothing else changes.

use std::sync::Arc;

use serde::Serialize;
use slopshop_ai::erase::{self, Region, pil};
use slopshop_ai::{ProtocolError, Stage};
use slopshop_core::clone::CloneSource;
use slopshop_core::color::PixelFormat;
use slopshop_core::selection::{
    self as core_selection, Combine, EdgeOptions, Modify, Selection, Shape,
};
use slopshop_core::view::ViewTransform;
use slopshop_core::{
    BlendMode, Document, Edit, HistoryLabel, Layer, LayerContent, LayerId, Projective, RasterImage,
    Rect, Size,
};
use tauri::{AppHandle, Manager};

use crate::AppState;
use crate::ai::{AiFailure, Feature};
use crate::ipc::DocumentView;
use crate::segment::{self, Task};

/// The selection grows by this fraction of the work region's larger side (ADR 0045).
const GROWTH: f64 = 0.02;

fn internal(e: impl ToString) -> AiFailure {
    AiFailure::new("internal", e)
}

/// What the model is given and what its result paints.
struct Plan {
    /// The document pixels the model sees (the work region, as the working size covers it).
    covered: Rect,
    /// The model's working size.
    size: (u32, u32),
    /// From the working size to the document: the view the input is rendered through.
    view: ViewTransform,
    /// The selection grown and kept within `covered`: what the result paints.
    selection: Arc<RasterImage>,
    /// That selection at the working size, one byte per pixel (255: to erase).
    mask: Vec<u8>,
}

/// The plan for the document's selection; `None` without one.
fn plan(doc: &Document) -> Result<Option<Plan>, String> {
    let Some(current) = doc.selection() else {
        return Ok(None);
    };
    let Some(bounds) = core_selection::bounds(current.image()) else {
        return Ok(None);
    };
    let canvas = doc.size();
    let region = erase::work_region(
        canvas.width,
        canvas.height,
        Region {
            x: bounds.x,
            y: bounds.y,
            width: bounds.width,
            height: bounds.height,
        },
    );
    let (w, h) = erase::working_size(region.width, region.height);
    let scale =
        (f64::from(region.width) / f64::from(w)).max(f64::from(region.height) / f64::from(h));
    // The working size covers the region up to a few pixels (its sides are rounded down to
    // multiples of 16): what it covers, within the canvas.
    let side = |start: u32, extent: u32, limit: u32| {
        ((f64::from(extent) * scale).round() as u32).min(limit - start)
    };
    let covered = Rect::new(
        region.x,
        region.y,
        side(region.x, w, canvas.width),
        side(region.y, h, canvas.height),
    );
    let radius = (GROWTH * f64::from(region.width.max(region.height)))
        .round()
        .max(1.0);
    let grown = core_selection::modify(canvas, current.image(), Modify::Expand(radius))
        .map_err(|e| format!("{e:?}"))?
        .map_or_else(|| Arc::clone(current.image()), Arc::new);
    let within = Shape::Rectangle {
        left: f64::from(covered.x),
        top: f64::from(covered.y),
        right: f64::from(covered.x + covered.width),
        bottom: f64::from(covered.y + covered.height),
    };
    let edges = EdgeOptions {
        anti_alias: false,
        feather: 0.0,
    };
    let Some(selection) = core_selection::select_shape(
        canvas,
        Some(grown.as_ref()),
        &within,
        edges,
        Combine::Intersect,
    )
    .map_err(|e| format!("{e:?}"))?
    else {
        return Ok(None);
    };
    let mask = core_selection::sample_grid(&selection, covered, w as usize, h as usize)
        .into_iter()
        .map(|c| if c >= 0.5 { 255 } else { 0 })
        .collect();
    Ok(Some(Plan {
        covered,
        size: (w, h),
        view: ViewTransform {
            origin: [f64::from(covered.x), f64::from(covered.y)],
            scale,
        },
        selection: Arc::new(selection),
        mask,
    }))
}

/// The edit painting the model's `result` (8-bit sRGB RGB at the working size) on `layer_id` of
/// `doc` through the plan's selection; `None`: nothing to paint.
fn fill_edit(
    doc: &Document,
    layer_id: u64,
    plan: &Plan,
    result: &[u8],
) -> Result<Option<Edit>, String> {
    let (w, h) = (plan.size.0 as usize, plan.size.1 as usize);
    let (cw, ch) = (plan.covered.width as usize, plan.covered.height as usize);
    let rgb = if (w, h) == (cw, ch) {
        result.to_vec()
    } else {
        pil::resize_lanczos(result, 3, (w, h), (cw, ch))
    };
    let rgba: Vec<u8> = rgb
        .as_chunks::<3>()
        .0
        .iter()
        .flat_map(|&[r, g, b]| [r, g, b, 255])
        .collect();
    // The result as a layer of the canvas's size (transparent beyond the covered region, which
    // costs almost nothing): the source the fill copies from, unshifted.
    let image = RasterImage::from_placed(
        doc.size(),
        PixelFormat::RGBA8_SRGB,
        plan.covered,
        &rgba,
        &[0; 4],
    )
    .map_err(|e| e.to_string())?;
    let layer = Layer {
        id: LayerId::from_raw(1),
        name: String::new(),
        visible: true,
        opacity: 1.0,
        blend_mode: BlendMode::Normal,
        content: LayerContent::raster(Arc::new(image)),
        mask: None,
        clipped: false,
        transform: Projective::IDENTITY,
        style: None,
    };
    let source = Document::restore(
        doc.size(),
        doc.working_space(),
        doc.blend_space(),
        vec![layer],
        2,
    )
    .map_err(|e| e.to_string())?;
    // The plan's selection as the one the fill reads, on a copy.
    let mut scratch = doc.clone();
    Edit::SetSelection {
        selection: Selection::new(Arc::clone(&plan.selection)),
    }
    .apply(&mut scratch)
    .map_err(|e| e.to_string())?;
    crate::paint::source_fill_edit(&scratch, layer_id, CloneSource::new(source))
}

/// The UI's stages: `load` while the model loads (first use), then `erase`.
#[derive(Debug, Clone, Copy, Serialize)]
enum Step {
    Load,
    Erase,
}

/// The helper's progress as the UI's: loading on its own, then the encoding (2), the steps (4)
/// and the decoding (1) as 7 parts of one bar.
fn step(stage: Stage, done: u32, total: u32) -> (Step, u32, u32) {
    match stage {
        Stage::Loading => (Step::Load, done, total),
        Stage::Encoding => (Step::Erase, done.min(2), 7),
        Stage::Denoising => (Step::Erase, 2 + done.min(4), 7),
        Stage::Decoding => (Step::Erase, 6 + done.min(1), 7),
    }
}

/// Delete's generative fill on `layer_id`: the selection replaced by its background, one undo
/// entry, the selection unchanged. Seconds (about 20 more the first time, to load the model):
/// the UI's `task`, its progress and cancellation.
#[tauri::command]
pub(crate) async fn ai_generative_fill(
    app: AppHandle,
    document_id: u64,
    layer_id: u64,
    task: u64,
) -> Result<DocumentView, AiFailure> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let (root, doc) = segment::prepare(&app, document_id)?;
        segment::require(&root, Feature::Erase)?;
        let task = Task::start(&app, task);
        let plan = plan(&doc)
            .map_err(internal)?
            .ok_or_else(|| internal("nothing is selected"))?;
        // The layer's own pixels, as the model reads them: 8-bit sRGB.
        let source = crate::selection::sampled_document(&doc, Some(layer_id)).map_err(internal)?;
        let (w, h) = plan.size;
        let rgb = segment::model_rgb(&state, &source, plan.view, Size::new(w, h))?;
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64);
        let (load, erase) = (task.shared("load"), task.shared("erase"));
        let cancel = task.cancel_token().clone();
        let result = {
            let mut session = segment::lock(&state)?;
            let client = session.client(&root)?;
            let mut progress = |stage, done, total| {
                match step(stage, done, total) {
                    (Step::Load, d, t) => load.report(d as usize, t as usize),
                    (Step::Erase, d, t) => erase.report(d as usize, t as usize),
                }
                !cancel.is_cancelled()
            };
            match client.erase(w, h, rgb, plan.mask.clone(), seed, &mut progress) {
                Ok(result) => result,
                // The helper is still at work: it is stopped (started again next time).
                Err(ProtocolError::Cancelled) => {
                    session.failed("cancelled");
                    return Err(AiFailure::new("cancelled", ""));
                }
                Err(e) => return Err(session.failed(e)),
            }
        };
        if cancel.is_cancelled() {
            return Err(AiFailure::new("cancelled", ""));
        }
        let mut documents = state.documents().map_err(internal)?;
        let document = documents.get_mut(document_id).map_err(internal)?;
        // On the document as it is now (the result applies to its current pixels).
        let edit =
            fill_edit(document.session.document(), layer_id, &plan, &result).map_err(internal)?;
        if let Some(edit) = edit {
            document
                .session
                .with_label(Some(HistoryLabel::new("generativeFill")), |s| {
                    s.perform(edit)
                })
                .map_err(internal)?;
        }
        Ok(document.view())
    })
    .await
    .map_err(internal)?
}

#[cfg(test)]
mod tests {
    use super::*;
    use slopshop_core::composite::composite_region;

    /// A gray document of `width` × `height` with one raster layer, and a rectangle selected.
    fn document(width: u32, height: u32, selected: [f64; 4]) -> Document {
        let size = Size::new(width, height);
        let pixels: Vec<u8> = (0..width * height)
            .flat_map(|_| [128, 128, 128, 255])
            .collect();
        let image = RasterImage::from_pixels(size, PixelFormat::RGBA8_SRGB, &pixels).unwrap();
        let layer = Layer {
            id: LayerId::from_raw(1),
            name: "photo".into(),
            visible: true,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            content: LayerContent::raster(Arc::new(image)),
            mask: None,
            clipped: false,
            transform: Projective::IDENTITY,
            style: None,
        };
        let mut doc = Document::restore(
            size,
            slopshop_core::color::WORKING_SPACE,
            slopshop_core::BlendSpace::default(),
            vec![layer],
            2,
        )
        .unwrap();
        let [left, top, right, bottom] = selected;
        let shape = Shape::Rectangle {
            left,
            top,
            right,
            bottom,
        };
        let selection = core_selection::select_shape(
            size,
            None,
            &shape,
            EdgeOptions::default(),
            Combine::Replace,
        )
        .unwrap()
        .unwrap();
        Edit::SetSelection {
            selection: Selection::new(Arc::new(selection)),
        }
        .apply(&mut doc)
        .unwrap();
        doc
    }

    #[test]
    fn the_plan_frames_a_small_selection_at_full_resolution() {
        // A 100 × 60 object in a 4000 × 3000 photo: the work region is 512 × 512 around it,
        // worked on unscaled; the selection grows by 2 % of 512 ≈ 10 px.
        let doc = document(4000, 3000, [2000.0, 1500.0, 2100.0, 1560.0]);
        let plan = plan(&doc).unwrap().unwrap();
        assert_eq!(plan.size, (512, 512));
        assert_eq!((plan.covered.width, plan.covered.height), (512, 512));
        assert_eq!(plan.view.scale, 1.0);
        let bounds = core_selection::bounds(&plan.selection).unwrap();
        assert_eq!((bounds.x, bounds.y), (1990, 1490));
        assert_eq!((bounds.width, bounds.height), (120, 80));
        // The mask is the grown selection at the working size.
        let selected = plan.mask.iter().filter(|&&m| m == 255).count();
        assert!((9000..=9800).contains(&selected), "{selected}");
    }

    #[test]
    fn the_plan_stays_within_the_covered_region() {
        // A selection over most of a large image: worked at 1 Mpx, the region the whole image.
        let doc = document(3000, 2000, [10.0, 10.0, 2990.0, 1990.0]);
        let plan = plan(&doc).unwrap().unwrap();
        let (w, h) = plan.size;
        assert!(w * h <= 1_048_576 && w % 16 == 0 && h % 16 == 0);
        assert!(plan.view.scale > 2.0);
        let bounds = core_selection::bounds(&plan.selection).unwrap();
        assert!(bounds.x + bounds.width <= plan.covered.x + plan.covered.width);
        assert!(bounds.y + bounds.height <= plan.covered.y + plan.covered.height);
        assert_eq!(plan.mask.len(), (w * h) as usize);
    }

    #[test]
    fn no_selection_no_plan() {
        let mut doc = document(64, 64, [0.0, 0.0, 10.0, 10.0]);
        assert!(plan(&doc).unwrap().is_some());
        Edit::SetSelection { selection: None }
            .apply(&mut doc)
            .unwrap();
        assert!(plan(&doc).unwrap().is_none());
    }

    #[test]
    fn the_result_paints_the_grown_selection_only() {
        let doc = document(700, 600, [300.0, 250.0, 340.0, 290.0]);
        let plan = plan(&doc).unwrap().unwrap();
        // The model's result: red everywhere; only the grown selection takes it.
        let (w, h) = plan.size;
        let red: Vec<u8> = (0..w * h).flat_map(|_| [255, 0, 0]).collect();
        let edit = fill_edit(&doc, 1, &plan, &red).unwrap().unwrap();
        let mut after = doc.clone();
        edit.apply(&mut after).unwrap();
        let pixel = |d: &Document, x: u32, y: u32| {
            let mut out = vec![0.0f32; 4];
            composite_region(d, Rect::new(x, y, 1, 1), &mut out).unwrap();
            out
        };
        // sRGB red, in the working space (premultiplied, opaque).
        let red = slopshop_core::LinearRgba::from_srgb_encoded_to_working(1.0, 0.0, 0.0, 1.0);
        let is_red = |p: Vec<f32>| {
            let close = |a: f32, b: f32| (a - b).abs() < 0.01;
            close(p[0], red.r) && close(p[1], red.g) && close(p[2], red.b) && close(p[3], 1.0)
        };
        assert!(is_red(pixel(&after, 320, 270)));
        // Grown by 2 % of the 512 px work region (10 px): 6 px outside the selection is
        // filled too…
        assert!(is_red(pixel(&after, 294, 270)));
        // …20 px outside is not: the photo's pixels, unchanged.
        assert_eq!(pixel(&after, 280, 270), pixel(&doc, 280, 270));
        assert_eq!(pixel(&after, 10, 10), pixel(&doc, 10, 10));
    }

    #[test]
    fn progress_maps_to_the_ui_stages() {
        assert!(matches!(step(Stage::Loading, 0, 1), (Step::Load, 0, 1)));
        assert!(matches!(step(Stage::Encoding, 2, 2), (Step::Erase, 2, 7)));
        assert!(matches!(step(Stage::Denoising, 3, 4), (Step::Erase, 5, 7)));
        assert!(matches!(step(Stage::Decoding, 1, 1), (Step::Erase, 7, 7)));
    }
}
