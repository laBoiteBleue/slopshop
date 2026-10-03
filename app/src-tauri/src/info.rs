//! File > Document Info: what a document is made of (size, color, layers, pixel formats,
//! memory) and the files it comes from, as identifiers and numbers the UI translates.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use serde::Serialize;
use slopshop_core::color::{ChannelLayout, PixelFormat, SampleType};
use slopshop_core::{Document, LayerContent, RasterImage};
use tauri::{AppHandle, Manager};

use crate::AppState;
use crate::ipc::color_space_id;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentInfo {
    pub name: Option<String>,
    pub width: u32,
    pub height: u32,
    /// Identifiers translated by the UI, as in the document view.
    pub working_space: &'static str,
    pub blend_space: &'static str,
    pub layers: LayerCounts,
    /// The pixel formats of the raster layers, the most used first.
    pub formats: Vec<FormatCount>,
    /// RAM held by the document's pixels (layers, their originals and masks, the selection),
    /// each image counted once however many layers share it.
    pub memory_bytes: u64,
    /// The file the document was opened from (an image, or a `.slop`), if any.
    pub source: Option<FileInfo>,
    /// The `.slop` file it was saved to or opened from, if any.
    pub file: Option<FileInfo>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerCounts {
    pub raster: u32,
    pub fill: u32,
    pub adjustment: u32,
    pub group: u32,
    pub masks: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormatCount {
    /// Bits per channel.
    pub bits: u32,
    pub float: bool,
    /// `gray`, `grayAlpha`, `rgb` or `rgba`.
    pub channels: &'static str,
    pub space: &'static str,
    pub layers: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileInfo {
    pub path: String,
    /// `None` when the file cannot be read any more.
    pub bytes: Option<u64>,
}

fn file_info(path: &Path) -> FileInfo {
    FileInfo {
        path: path.to_string_lossy().into_owned(),
        bytes: std::fs::metadata(path).ok().map(|m| m.len()),
    }
}

fn channels(layout: ChannelLayout) -> &'static str {
    match layout {
        ChannelLayout::Gray => "gray",
        ChannelLayout::GrayAlpha => "grayAlpha",
        ChannelLayout::Rgb => "rgb",
        ChannelLayout::Rgba => "rgba",
    }
}

/// What `doc` is made of (everything but the files).
pub fn describe(doc: &Document, name: Option<String>) -> DocumentInfo {
    let mut layers = LayerCounts::default();
    let mut formats: Vec<(PixelFormat, u32)> = Vec::new();
    let mut seen = HashSet::new();
    let mut memory_bytes = 0;
    let mut count = |image: &Arc<RasterImage>| {
        if seen.insert(image.id()) {
            memory_bytes += image.memory_bytes();
        }
    };
    for layer in doc.all_layers() {
        match &layer.content {
            LayerContent::Raster { image, original } => {
                layers.raster += 1;
                let format = image.format();
                match formats.iter_mut().find(|(f, _)| *f == format) {
                    Some((_, n)) => *n += 1,
                    None => formats.push((format, 1)),
                }
                count(image);
                if let Some(original) = original {
                    count(original);
                }
            }
            LayerContent::Fill { .. } => layers.fill += 1,
            LayerContent::Adjustment { .. } => layers.adjustment += 1,
            LayerContent::Group { .. } => layers.group += 1,
        }
        if let Some(mask) = &layer.mask {
            layers.masks += 1;
            count(&mask.image);
            if let Some(original) = &mask.original {
                count(original);
            }
        }
    }
    if let Some(selection) = doc.selection() {
        count(selection.image());
    }
    // The most used first; ties in the order met.
    formats.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    DocumentInfo {
        name,
        width: doc.size().width,
        height: doc.size().height,
        working_space: color_space_id(doc.working_space()),
        blend_space: doc.blend_space().id(),
        layers,
        formats: formats
            .into_iter()
            .map(|(format, layers)| FormatCount {
                bits: format.sample.bytes() * 8,
                float: matches!(format.sample, SampleType::F16 | SampleType::F32),
                channels: channels(format.layout),
                space: color_space_id(format.color_space),
                layers,
            })
            .collect(),
        memory_bytes,
        source: None,
        file: None,
    }
}

/// File > Document Info of document `document_id`.
#[tauri::command]
pub async fn document_info(app: AppHandle, document_id: u64) -> Result<DocumentInfo, String> {
    let (doc, name, source, file) = {
        let state = app.state::<AppState>();
        let mut documents = state.documents()?;
        let document = documents.get_mut(document_id)?;
        (
            document.session.document().clone(),
            document.meta.name.clone(),
            document.source.clone(),
            document.path.clone(),
        )
    };
    // Walking the layers and reading the files' sizes: off the UI thread.
    tauri::async_runtime::spawn_blocking(move || {
        let mut info = describe(&doc, name);
        info.source = source.as_deref().map(file_info);
        info.file = file.as_deref().map(file_info);
        info
    })
    .await
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use slopshop_core::geom::Size;
    use slopshop_core::{Affine, BlendMode, Layer, LayerId, LayerMask, LinearRgba};

    fn layer(id: u64, content: LayerContent, mask: Option<LayerMask>) -> Layer {
        Layer {
            transform: Affine::IDENTITY,
            clipped: false,
            id: LayerId::from_raw(id),
            name: format!("{id}"),
            visible: true,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            mask,
            content,
        }
    }

    #[test]
    fn layers_formats_and_shared_memory_are_counted() {
        let size = Size::new(300, 200);
        let rgba = Arc::new(
            RasterImage::from_pixels(size, PixelFormat::RGBA8_SRGB, &vec![0; 300 * 200 * 4])
                .unwrap(),
        );
        let raster = |id| {
            let content = LayerContent::Raster {
                image: Arc::clone(&rgba),
                original: None,
            };
            layer(id, content, None)
        };
        let mask = LayerMask {
            image: Arc::new(slopshop_core::selection::select_all(size).unwrap()),
            enabled: true,
            replaces_alpha: false,
            original: None,
        };
        let fill = LayerContent::Fill {
            color: LinearRgba::new(1.0, 1.0, 1.0, 1.0),
        };
        let doc = Document::restore(
            size,
            slopshop_core::color::WORKING_SPACE,
            slopshop_core::blend::BlendSpace::Perceptual,
            vec![layer(1, fill, Some(mask.clone())), raster(2), raster(3)],
            4,
        )
        .unwrap();
        let info = describe(&doc, Some("photo.jpg".to_owned()));
        assert_eq!((info.width, info.height), (300, 200));
        assert_eq!(
            info.layers,
            LayerCounts {
                raster: 2,
                fill: 1,
                masks: 1,
                ..LayerCounts::default()
            }
        );
        assert_eq!(
            info.formats,
            [FormatCount {
                bits: 8,
                float: false,
                channels: "rgba",
                space: "srgb",
                layers: 2,
            }]
        );
        // The shared image once, and the mask.
        assert_eq!(
            info.memory_bytes,
            rgba.memory_bytes() + mask.image.memory_bytes()
        );
    }
}
