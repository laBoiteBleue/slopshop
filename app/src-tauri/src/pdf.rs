//! PDF Import dialog: the pages of a PDF (sizes, thumbnails), then the chosen ones opened at the
//! chosen resolution, like several files (tabs, or layers of a document).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use slopshop_io::pdf::PdfFile;
use tauri::ipc::Response;
use tauri::{AppHandle, State};

use crate::{AppState, InsertionOrder, OpenTarget, Turn, open_path};

/// Largest thumbnail side the dialog may ask for, in pixels.
const MAX_THUMBNAIL_SIDE: u32 = 512;

/// The PDF the dialog shows, kept parsed between its calls (one at a time).
#[derive(Default)]
pub(crate) struct PdfCache(Mutex<Option<(PathBuf, Arc<PdfFile>)>>);

impl PdfCache {
    /// The file at `path`, from the cache or read now (then cached). Blocking.
    fn get(&self, path: &Path) -> Result<Arc<PdfFile>, String> {
        if let Ok(cache) = self.0.lock()
            && let Some((cached, file)) = cache.as_ref()
            && cached == path
        {
            return Ok(Arc::clone(file));
        }
        let file = Arc::new(PdfFile::open(path).map_err(|e| e.to_string())?);
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

/// One page of a PDF, opened at a chosen resolution.
#[derive(Clone)]
pub(crate) struct PdfPage {
    pub(crate) file: Arc<PdfFile>,
    /// From 0.
    pub(crate) index: usize,
    pub(crate) dpi: f32,
}

impl PdfPage {
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

/// The pages of the PDF at `path`; the file stays parsed for the dialog's next calls.
#[tauri::command]
pub(crate) async fn pdf_pages(
    state: State<'_, AppState>,
    path: PathBuf,
) -> Result<Vec<PageSizeDto>, String> {
    let file = blocking_get(&state, path).await?;
    Ok(file
        .page_sizes()
        .into_iter()
        .map(|s| PageSizeDto {
            width: s.width,
            height: s.height,
        })
        .collect())
}

/// Page `page` (from 0) on white, its longer side `max_side` pixels: width and height as u32
/// little-endian, then RGBA8 sRGB pixels.
#[tauri::command]
pub(crate) async fn pdf_thumbnail(
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
pub(crate) fn close_pdf(state: State<'_, AppState>, path: PathBuf) {
    state.pdf.forget(&path);
}

/// Open `pages` (from 0) of the PDF at `path` at `dpi`, rendered in parallel: each in a new
/// tab, or each as a new top layer of `document_id`, in the order given. Outcomes arrive as
/// `open-*` events, like [`open_images`](crate::open_images).
#[tauri::command]
pub(crate) async fn open_pdf_pages(
    app: AppHandle,
    state: State<'_, AppState>,
    path: PathBuf,
    pages: Vec<usize>,
    dpi: f32,
    document_id: Option<u64>,
) -> Result<(), String> {
    let file = blocking_get(&state, path.clone()).await?;
    state.pdf.forget(&path);
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
            let page = PdfPage {
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
                let _ = open_path(&app, &path, Some(&page), target, Some(&turn));
            })
        })
        .collect();
    for open in opens {
        open.await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

async fn blocking_get(state: &State<'_, AppState>, path: PathBuf) -> Result<Arc<PdfFile>, String> {
    let cache = Arc::clone(&state.pdf);
    tauri::async_runtime::spawn_blocking(move || cache.get(&path))
        .await
        .map_err(|e| e.to_string())?
}
