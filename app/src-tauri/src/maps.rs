//! Filter > Distort > Displace's maps (ADR 0034, 0040): an image file opened as a source of
//! the document, which the filter entry then keeps, as a library pattern is loaded for a
//! Pattern Overlay.

use std::path::PathBuf;
use std::sync::Arc;

use slopshop_core::Source;
use tauri::{AppHandle, Manager};

use crate::AppState;
use crate::ipc::MapView;
use crate::selection::on_worker;

/// The file at `path` opened as a displacement map for document `document_id`: its pixels (a
/// layered file's merged, as Photoshop reads a PSD's composite), a source named after the file
/// that the document keeps while it is open, so that Displace may name it. Its id and name.
#[tauri::command]
pub async fn load_displacement_map(
    app: AppHandle,
    document_id: u64,
    path: PathBuf,
) -> Result<MapView, String> {
    let worker = app.clone();
    let source = on_worker(move || {
        let name = crate::file_name(&path);
        // Decoders parse untrusted files: a panic is a failure, not a crash.
        let opened = std::panic::catch_unwind(|| slopshop_io::open_file(&path))
            .map_err(|_| "the decoder failed".to_owned())?
            .map_err(|e| e.to_string())?;
        let image = match opened {
            slopshop_io::Opened::Image(imported) => Arc::new(imported.image),
            slopshop_io::Opened::Layers(layers) => {
                let state = worker.state::<AppState>();
                crate::clipboard::merged_image(state.renderer().ok(), &layers.document)?
                    .map(|(image, _)| image)
                    .ok_or("the file shows nothing")?
            }
        };
        Ok(Source::new(image, name))
    })
    .await?;
    let state = app.state::<AppState>();
    let mut documents = state.documents()?;
    let document = documents.get_mut(document_id)?;
    let view = MapView {
        id: source.id().get(),
        name: source.name().to_owned(),
    };
    document.loaded_patterns.insert(view.id, source);
    Ok(view)
}
