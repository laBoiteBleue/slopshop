//! Image > Adjustments > Color Lookup's tables: loading a `.cube` or `.3dl` file, and finding a
//! table again by the id the UI sends back (`ColorLut::id`). A table just loaded is kept here
//! until an edit puts it in the document; afterwards the document, its history and the renderer
//! hold it.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use slopshop_core::Document;
use slopshop_core::document::LayerContent;
use slopshop_core::lut::{ColorLut, LutDimension};
use slopshop_core::stack::Entry;

/// A table as the UI shows it (the Properties panel and the adjustment dialog).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LutView {
    /// What edits send back to use it (`lut` of `setAdjustment` and `applyEffect`).
    pub id: u64,
    /// The file's name.
    pub name: String,
    /// 1 or 3.
    pub dimension: u8,
    /// Points a side.
    pub size: u32,
}

impl LutView {
    pub fn new(lut: &ColorLut) -> Self {
        Self {
            id: lut.id().get(),
            name: lut.name().to_owned(),
            dimension: match lut.dimension() {
                LutDimension::One => 1,
                LutDimension::Three => 3,
            },
            size: lut.size(),
        }
    }
}

/// The tables loaded last, newest first: one is used by an edit soon after it is loaded.
static LOADED: Mutex<Vec<Arc<ColorLut>>> = Mutex::new(Vec::new());
const KEPT: usize = 8;

fn remember(lut: &Arc<ColorLut>) {
    remember_in(&mut LOADED.lock().unwrap_or_else(|e| e.into_inner()), lut);
}

/// `lut` first in `loaded`, which keeps the last [`KEPT`].
fn remember_in(loaded: &mut Vec<Arc<ColorLut>>, lut: &Arc<ColorLut>) {
    loaded.insert(0, Arc::clone(lut));
    loaded.truncate(KEPT);
}

/// The table `id`: one loaded lately, or one the document uses (adjustment layers and stack
/// effects).
pub fn find(document: &Document, id: u64) -> Option<Arc<ColorLut>> {
    let loaded = LOADED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(lut) = loaded.iter().find(|l| l.id().get() == id) {
        return Some(Arc::clone(lut));
    }
    drop(loaded);
    document
        .all_layers()
        .find_map(|layer| match &layer.content {
            LayerContent::Adjustment { adjustment } => {
                adjustment.lut().filter(|l| l.id().get() == id).cloned()
            }
            content => content
                .stack()?
                .entries()
                .iter()
                .find_map(|entry| match entry {
                    Entry::Effect(effect) => effect
                        .steps()
                        .iter()
                        .find_map(|s| s.adjustment.lut().filter(|l| l.id().get() == id).cloned()),
                    _ => None,
                }),
        })
}

/// The table of the file at `path`, kept for the edit that uses it.
pub fn load(path: &std::path::Path) -> Result<LutView, String> {
    let lut = slopshop_io::lut::read_lut(path).map_err(|e| e.to_string())?;
    remember(&lut);
    Ok(LutView::new(&lut))
}

/// Color Lookup > Load 3D LUT: the file at `path` read off the UI thread.
#[tauri::command]
pub async fn load_color_lookup(path: PathBuf) -> Result<LutView, String> {
    tauri::async_runtime::spawn_blocking(move || load(&path))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use slopshop_core::adjust::Adjustment;
    use slopshop_core::geom::Size;

    #[test]
    fn a_loaded_table_is_found_by_its_id_then_in_the_document() {
        let path = std::env::temp_dir().join(format!("slopshop-lut-{}.cube", std::process::id()));
        std::fs::write(&path, "LUT_1D_SIZE 2\n0 0 0\n1 1 1\n").unwrap();
        let view = load(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(
            (view.name.as_str(), view.dimension, view.size),
            (path.file_name().unwrap().to_str().unwrap(), 1, 2)
        );
        let doc = Document::new(Size::new(2, 2));
        assert!(find(&doc, view.id).is_some(), "loaded");
        // The last ones loaded are kept (a list of the test's own: others use the app's).
        let table = || {
            ColorLut::new(
                "other",
                LutDimension::One,
                2,
                ColorLut::UNIT_DOMAIN,
                vec![0.0; 6],
            )
            .unwrap()
        };
        let first = table();
        let mut kept = Vec::new();
        remember_in(&mut kept, &first);
        for _ in 0..KEPT {
            remember_in(&mut kept, &table());
        }
        assert_eq!(kept.len(), KEPT);
        assert!(!kept.iter().any(|l| Arc::ptr_eq(l, &first)));
        // A table never loaded here is found in a document using it.
        let lut = table();
        assert!(find(&doc, lut.id().get()).is_none());
        let mut doc = Document::new(Size::new(2, 2));
        let id = doc.allocate_layer_id();
        slopshop_core::Edit::InsertLayer {
            parent: None,
            index: 0,
            layer: slopshop_core::Layer {
                locks: Default::default(),
                style: None,
                transform: slopshop_core::Projective::IDENTITY,
                clipped: false,
                id,
                name: "lookup".into(),
                visible: true,
                opacity: 1.0,
                blend_mode: slopshop_core::BlendMode::Normal,
                mask: None,
                content: LayerContent::Adjustment {
                    adjustment: Adjustment::ColorLookup {
                        lut: Some(Arc::clone(&lut)),
                    },
                },
                warp: None,
                vector_mask: None,
            },
        }
        .apply(&mut doc)
        .unwrap();
        assert!(Arc::ptr_eq(&find(&doc, lut.id().get()).unwrap(), &lut));
        assert!(load(std::path::Path::new("missing.cube")).is_err());
    }
}
