//! Import PDF and Import SVG dialogs: the pages of a vector file (sizes, thumbnails), then the
//! chosen ones opened at the chosen resolution, like several files (tabs, or layers of a
//! document).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use slopshop_io::vector::VectorFile;
use tauri::ipc::Response;
use tauri::{AppHandle, State};

use crate::{AppState, InsertionOrder, OpenTarget, Source, Turn, open_path};

/// Largest thumbnail side the dialog may ask for, in pixels.
const MAX_THUMBNAIL_SIDE: u32 = 512;

/// The file the dialog shows, kept parsed between its calls (one at a time).
#[derive(Default)]
pub(crate) struct VectorCache(Mutex<Option<(PathBuf, Arc<VectorFile>)>>);

impl VectorCache {
    /// The file at `path`, from the cache or read now (then cached). Blocking.
    fn get(&self, path: &Path) -> Result<Arc<VectorFile>, String> {
        if let Ok(cache) = self.0.lock()
            && let Some((cached, file)) = cache.as_ref()
            && cached == path
        {
            return Ok(Arc::clone(file));
        }
        let file = VectorFile::open(path)
            .map_err(|e| e.to_string())?
            .ok_or("not a PDF or SVG file")?;
        let file = Arc::new(file);
        if let Ok(mut cache) = self.0.lock() {
            *cache = Some((path.to_owned(), Arc::clone(&file)));
        }
        Ok(file)
    }

    fn forget(&self, path: &Path) {
        if let Ok(mut cache) = self.0.lock()
            && cache.as_ref().is_some_and(|(cached, _)| cached == path)
        {
            *cache = None;
        }
    }
}

/// One page of a vector file, opened at a chosen resolution.
#[derive(Clone)]
pub(crate) struct VectorPage {
    pub(crate) file: Arc<VectorFile>,
    /// From 0.
    pub(crate) index: usize,
    pub(crate) dpi: f32,
}

impl VectorPage {
    /// "doc.pdf 3/12" for the tab, "doc 3/12" for the layer; no number for a single page.
    pub(crate) fn name(&self, base: &str) -> String {
        match self.file.page_count() {
            1 => base.to_owned(),
            count => format!("{base} {}/{count}", self.index + 1),
        }
    }
}

/// A page's size in points (1/72 inch), rotation applied.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct PageSizeDto {
    width: f32,
    height: f32,
}

/// What the dialog shows of a vector file.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VectorInfo {
    /// "pdf" or "svg".
    kind: &'static str,
    /// The resolution used when nobody chose one (300 for PDF, 96 for SVG: its own size).
    default_dpi: f32,
    pages: Vec<PageSizeDto>,
}

/// The pages of the PDF or SVG at `path`; the file stays parsed for the dialog's next calls.
#[tauri::command]
pub(crate) async fn vector_info(
    state: State<'_, AppState>,
    path: PathBuf,
) -> Result<VectorInfo, String> {
    let file = blocking_get(&state, path).await?;
    Ok(VectorInfo {
        kind: file.kind(),
        default_dpi: file.default_dpi(),
        pages: file
            .page_sizes()
            .into_iter()
            .map(|s| PageSizeDto {
                width: s.width,
                height: s.height,
            })
            .collect(),
    })
}

/// Page `page` (from 0) on white, its longer side `max_side` pixels: width and height as u32
/// little-endian, then RGBA8 sRGB pixels.
#[tauri::command]
pub(crate) async fn vector_thumbnail(
    state: State<'_, AppState>,
    path: PathBuf,
    page: usize,
    max_side: u32,
) -> Result<Response, String> {
    let file = blocking_get(&state, path).await?;
    let max_side = max_side.clamp(1, MAX_THUMBNAIL_SIDE);
    let thumbnail = tauri::async_runtime::spawn_blocking(move || file.thumbnail(page, max_side))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let mut bytes = Vec::with_capacity(8 + thumbnail.pixels.len());
    bytes.extend(thumbnail.width.to_le_bytes());
    bytes.extend(thumbnail.height.to_le_bytes());
    bytes.extend(thumbnail.pixels);
    Ok(Response::new(bytes))
}

/// The dialog was cancelled: the parsed file can go.
#[tauri::command]
pub(crate) fn close_vector(state: State<'_, AppState>, path: PathBuf) {
    state.vector.forget(&path);
}

/// Open `pages` (from 0) of the PDF or SVG at `path` at `dpi`, rendered in parallel: each in a new
/// tab, or each as a new top layer of `document_id`, in the order given. Outcomes arrive as
/// `open-*` events, like [`open_images`](crate::open_images).
#[tauri::command]
pub(crate) async fn open_vector_pages(
    app: AppHandle,
    state: State<'_, AppState>,
    path: PathBuf,
    pages: Vec<usize>,
    dpi: f32,
    document_id: Option<u64>,
) -> Result<(), String> {
    let file = blocking_get(&state, path.clone()).await?;
    state.vector.forget(&path);
    let target = match document_id {
        Some(document_id) => OpenTarget::Layer { document_id },
        None => OpenTarget::NewTab,
    };
    let order = Arc::new(InsertionOrder::default());
    let opens: Vec<_> = pages
        .into_iter()
        .enumerate()
        .map(|(turn_index, index)| {
            let (app, order, path) = (app.clone(), order.clone(), path.clone());
            let page = VectorPage {
                file: Arc::clone(&file),
                index,
                dpi,
            };
            tauri::async_runtime::spawn_blocking(move || {
                let turn = Turn {
                    order: &order,
                    index: turn_index,
                };
                // The outcome is reported by the open's own events.
                let _ = open_path(&app, &path, Source::Page(&page), target, Some(&turn));
            })
        })
        .collect();
    for open in opens {
        open.await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

async fn blocking_get(
    state: &State<'_, AppState>,
    path: PathBuf,
) -> Result<Arc<VectorFile>, String> {
    let cache = Arc::clone(&state.vector);
    tauri::async_runtime::spawn_blocking(move || cache.get(&path))
        .await
        .map_err(|e| e.to_string())?
}
