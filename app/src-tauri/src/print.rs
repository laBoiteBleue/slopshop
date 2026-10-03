//! File > Print: the document as displayed, rendered by the GPU at a printing resolution and
//! sent to the UI as a JPEG, which the system's print dialog prints from a hidden page. The
//! pixels cross the IPC once, compressed: printing needs them in the webview.

use slopshop_core::{Affine, BlendMode, Document, Edit, Layer, LayerContent, LinearRgba, Size};
use tauri::ipc::Response;
use tauri::{AppHandle, Manager};

use crate::AppState;

/// Longer side of the printed image, pixels: an A4 or Letter page at 300 dpi needs about 3500,
/// an A3 page about 5000. Smaller documents print at their own size.
const PRINT_SIDE: u32 = 4096;
const PRINT_QUALITY: u8 = 92;

/// The size `size` prints at (never enlarged) and the document pixels per printed pixel.
fn print_size(size: Size) -> (Size, f64) {
    let scale = (f64::from(size.width.max(size.height)) / f64::from(PRINT_SIDE)).max(1.0);
    let side = |v: u32| (f64::from(v) / scale).round().max(1.0) as u32;
    (Size::new(side(size.width), side(size.height)), scale)
}

/// `doc` over white paper: a white fill below every layer, so that transparency prints white
/// instead of the display's checkerboard.
fn on_paper(doc: &Document) -> Result<Document, String> {
    let mut doc = doc.clone();
    let paper = Layer {
        transform: Affine::IDENTITY,
        clipped: false,
        id: doc.allocate_layer_id(),
        name: "Paper".to_owned(),
        visible: true,
        opacity: 1.0,
        blend_mode: BlendMode::Normal,
        mask: None,
        content: LayerContent::Fill {
            color: LinearRgba::new(1.0, 1.0, 1.0, 1.0),
        },
    };
    Edit::InsertLayer {
        parent: None,
        index: 0,
        layer: paper,
    }
    .apply(&mut doc)
    .map_err(|e| e.to_string())?;
    Ok(doc)
}

/// File > Print's page of document `document_id`: a JPEG (sRGB) of it as displayed, at most
/// [`PRINT_SIDE`] pixels on its longer side.
#[tauri::command]
pub async fn print_page(app: AppHandle, document_id: u64) -> Result<Response, String> {
    let doc = {
        let state = app.state::<AppState>();
        let mut documents = state.documents()?;
        documents.get_mut(document_id)?.session.document().clone()
    };
    tauri::async_runtime::spawn_blocking(move || {
        let doc = on_paper(&doc)?;
        let (output, scale) = print_size(doc.size());
        let view = slopshop_core::view::ViewTransform {
            origin: [0.0, 0.0],
            scale,
        };
        let frame = app
            .state::<AppState>()
            .renderer()?
            .render_view(&doc, view, output)
            .map_err(|e| e.to_string())?;
        // Opaque over the paper: RGBA to RGB.
        let rgb: Vec<u8> = frame
            .data
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|px| [px[0], px[1], px[2]])
            .collect();
        slopshop_io::export::encode_jpeg_srgb8(&rgb, output, PRINT_QUALITY)
            .map(Response::new)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_documents_print_at_the_print_side_small_ones_as_they_are() {
        assert_eq!(
            print_size(Size::new(8192, 4096)),
            (Size::new(4096, 2048), 2.0)
        );
        assert_eq!(print_size(Size::new(800, 600)), (Size::new(800, 600), 1.0));
    }

    #[test]
    fn transparency_prints_on_white_paper() {
        let doc = Document::restore(
            Size::new(10, 10),
            slopshop_core::color::WORKING_SPACE,
            slopshop_core::blend::BlendSpace::Perceptual,
            Vec::new(),
            1,
        )
        .unwrap();
        let paper = on_paper(&doc).unwrap();
        assert_eq!(paper.layers().len(), 1);
        assert!(matches!(
            paper.layers()[0].content,
            LayerContent::Fill { color } if color == LinearRgba::new(1.0, 1.0, 1.0, 1.0)
        ));
        // The document itself is untouched.
        assert!(doc.layers().is_empty());
    }
}
