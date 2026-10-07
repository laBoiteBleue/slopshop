//! Patterns (ADR 0042): the library (`slopshop_io::patterns`, in the app's data folder), Edit >
//! Define Pattern, and pattern fill layers made from a library pattern. A pattern chosen for a
//! document becomes one of its sources.

use std::sync::Arc;

use slopshop_core::edit::Edit;
use slopshop_core::pattern::PatternFill;
use slopshop_core::session::HistoryLabel;
use slopshop_core::{BlendMode, Layer, LayerContent, LayerId, Projective, Source};
use slopshop_io::patterns::{Library, PatternEntry};
use tauri::ipc::Response;
use tauri::{AppHandle, Manager};

use crate::AppState;
use crate::ipc::DocumentView;
use crate::selection::on_worker;

/// The library's folder.
const FOLDER: &str = "patterns";

fn library(app: &AppHandle) -> Result<Library, String> {
    let folder = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join(FOLDER);
    Ok(Library::new(folder))
}

/// The library's patterns: the generated ones first, then the user's.
#[tauri::command]
pub async fn list_patterns(app: AppHandle) -> Result<Vec<PatternEntry>, String> {
    let library = library(&app)?;
    on_worker(move || Ok(library.list())).await
}

/// Thumbnail of library pattern `pattern`, as layer thumbnails are sent: a header of width and
/// height (`u32` little-endian), then RGBA8 pixels with straight alpha (sRGB).
#[tauri::command]
pub async fn pattern_thumbnail(
    app: AppHandle,
    pattern: String,
    max_side: u32,
) -> Result<Response, String> {
    let library = library(&app)?;
    let max_side = max_side.clamp(1, 256);
    let thumbnail = on_worker(move || {
        let image = library.load(&pattern)?;
        Ok(slopshop_core::thumbnail::raster_thumbnail(&image, max_side))
    })
    .await?;
    Ok(Response::new(crate::thumbnail_bytes(
        thumbnail.size,
        thumbnail.pixels,
    )))
}

/// Edit > Define Pattern: what the image shows (every visible layer) within the selection's
/// bounds (the whole canvas without one), added to the library as `name`. `None` when the
/// selection is empty.
#[tauri::command]
pub async fn define_pattern(
    app: AppHandle,
    document_id: u64,
    name: String,
) -> Result<Option<PatternEntry>, String> {
    let document = {
        let state = app.state::<AppState>();
        let mut documents = state.documents()?;
        documents.get_mut(document_id)?.session.document().clone()
    };
    let library = library(&app)?;
    let worker = app.clone();
    on_worker(move || {
        let state = worker.state::<AppState>();
        let Some((image, _)) = crate::clipboard::merged_image(state.renderer().ok(), &document)?
        else {
            return Ok(None);
        };
        library.add(&name, image).map(Some)
    })
    .await
}

/// A source of library pattern `pattern`, named after it (`name` for a generated one).
async fn source_of(app: &AppHandle, pattern: String, name: String) -> Result<Arc<Source>, String> {
    let library = library(app)?;
    on_worker(move || {
        let image = library.load(&pattern)?;
        Ok(Source::new(image, name))
    })
    .await
}

/// Layer > New Fill Layer > Pattern: a pattern fill layer of library pattern `pattern` at 100 %,
/// named `name`, at `index` among the layers of `parent` (absent: the top level), one undo
/// entry. Its pattern becomes one of the document's sources, named `source_name`.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn add_pattern_fill(
    app: AppHandle,
    document_id: u64,
    pattern: String,
    source_name: String,
    name: String,
    parent: Option<u64>,
    index: usize,
) -> Result<DocumentView, String> {
    let source = source_of(&app, pattern, source_name).await?;
    let state = app.state::<AppState>();
    let mut documents = state.documents()?;
    let document = documents.get_mut(document_id)?;
    let id = document.session.allocate_layer_id();
    let edit = Edit::InsertLayer {
        parent: parent.map(LayerId::from_raw),
        index,
        layer: Layer {
            style: None,
            transform: Projective::IDENTITY,
            clipped: false,
            id,
            name,
            visible: true,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            mask: None,
            content: LayerContent::PatternFill {
                pattern: PatternFill::new(source),
            },
        },
    };
    document
        .session
        .with_label(Some(HistoryLabel::new("newFillLayer")), |s| s.perform(edit))
        .map_err(|e| e.to_string())?;
    Ok(document.view())
}

/// Library pattern `pattern` loaded for document `document_id` (a Pattern Overlay chosen in the
/// Layer Style dialog): its source's id, which a style may then name. Kept while the document
/// is open.
#[tauri::command]
pub async fn load_pattern(
    app: AppHandle,
    document_id: u64,
    pattern: String,
    source_name: String,
) -> Result<u64, String> {
    let source = source_of(&app, pattern, source_name).await?;
    let state = app.state::<AppState>();
    let mut documents = state.documents()?;
    let document = documents.get_mut(document_id)?;
    let id = source.id().get();
    document.loaded_patterns.insert(id, source);
    Ok(id)
}

/// Another library pattern for pattern fill layer `layer_id`, its scale and angle kept (the
/// Properties panel), one undo entry.
#[tauri::command]
pub async fn replace_pattern(
    app: AppHandle,
    document_id: u64,
    layer_id: u64,
    pattern: String,
    source_name: String,
) -> Result<DocumentView, String> {
    let source = source_of(&app, pattern, source_name).await?;
    let state = app.state::<AppState>();
    let mut documents = state.documents()?;
    let document = documents.get_mut(document_id)?;
    let id = LayerId::from_raw(layer_id);
    let LayerContent::PatternFill { pattern: current } = &document
        .session
        .document()
        .layer(id)
        .ok_or("unknown layer")?
        .content
    else {
        return Err("not a pattern fill layer".to_owned());
    };
    let pattern = PatternFill {
        source,
        ..current.clone()
    };
    document
        .session
        .with_label(Some(HistoryLabel::new("patternFill")), |s| {
            s.perform(Edit::SetPatternFill { id, pattern })
        })
        .map_err(|e| e.to_string())?;
    Ok(document.view())
}
