//! IPC data types: the contract between the UI and the engine.
//!
//! These DTOs are deliberately separate from the core model so that the engine can evolve
//! (e.g. towards a DAG) without breaking the UI, and so that the core stays free of serde.
//! Keep `app/src/lib/engine.ts` in sync with this file.
//!
//! Layer ids travel as JSON numbers: exact up to 2^53, far beyond what a session allocates.

use serde::{Deserialize, Serialize};
use slopshop_core::color::{ColorSpace, WORKING_SPACE};
use slopshop_core::view::{Viewport, ZoomStep};
use slopshop_core::{Document, Edit, Layer, LayerContent, LayerId, LinearRgba, Session, Size};

/// Identity of an open document (one per tab). Ids are never reused, so the UI can tell
/// documents apart and ignore stale answers.
#[derive(Debug, Clone)]
pub struct DocumentMeta {
    pub id: u64,
    /// Display name (e.g. the file name). `None` for an untitled document.
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentView {
    pub id: u64,
    pub name: Option<String>,
    pub width: u32,
    pub height: u32,
    /// Identifier (e.g. `linear-srgb`), translated by the UI.
    pub working_space: &'static str,
    pub revision: u64,
    pub can_undo: bool,
    pub can_redo: bool,
    /// Bottom to top, like the core model.
    pub layers: Vec<LayerView>,
    pub warnings: Vec<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerView {
    pub id: u64,
    pub name: String,
    pub visible: bool,
    pub opacity: f32,
    pub kind: &'static str,
    /// Display swatch, sRGB-encoded RGBA in `[0, 1]` (explicitly converted, see `color`).
    pub swatch: [f32; 4],
}

impl DocumentView {
    /// `warnings`: how the sources of the current layers were interpreted (identifiers such as
    /// `iccProfileIgnored`, translated by the UI).
    pub fn new(session: &Session, meta: &DocumentMeta, warnings: Vec<&'static str>) -> Self {
        let doc: &Document = session.document();
        Self {
            id: meta.id,
            name: meta.name.clone(),
            width: doc.size().width,
            height: doc.size().height,
            working_space: color_space_id(doc.working_space()),
            revision: doc.revision(),
            can_undo: session.can_undo(),
            can_redo: session.can_redo(),
            layers: doc.layers().iter().map(LayerView::new).collect(),
            warnings,
        }
    }
}

impl LayerView {
    fn new(layer: &Layer) -> Self {
        let (kind, swatch) = match &layer.content {
            LayerContent::Fill { color } => ("fill", color.working_to_srgb_encoded()),
            LayerContent::Raster { image } => (
                "raster",
                image
                    .average_color(&WORKING_SPACE)
                    .working_to_srgb_encoded(),
            ),
        };
        Self {
            id: layer.id.get(),
            name: layer.name.clone(),
            visible: layer.visible,
            opacity: layer.opacity,
            kind,
            swatch,
        }
    }
}

/// Stable identifiers, not display text: the UI owns all user-visible (translated) strings.
fn color_space_id(space: ColorSpace) -> &'static str {
    space.id().unwrap_or("custom")
}

/// Edits the UI can request. Translated into core [`Edit`]s; the engine validates them.
#[derive(Debug, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum EditRequest {
    /// Add a fill layer on top of the stack. `color` is sRGB-encoded RGBA in `[0, 1]`, as
    /// produced by UI color pickers; it is explicitly converted to the working space.
    AddFillLayer {
        name: String,
        color: [f32; 4],
    },
    RemoveLayer {
        id: u64,
    },
    SetLayerVisible {
        id: u64,
        visible: bool,
    },
    SetLayerOpacity {
        id: u64,
        opacity: f32,
    },
    RenameLayer {
        id: u64,
        name: String,
    },
    /// Move a layer so that it ends up at `index` (0 = bottom).
    MoveLayer {
        id: u64,
        index: usize,
    },
}

impl EditRequest {
    /// Build the core edit. Needs the session to allocate ids for new layers.
    pub fn into_edit(self, session: &mut Session) -> Edit {
        match self {
            EditRequest::AddFillLayer { name, color } => {
                let [r, g, b, a] = color;
                Edit::InsertLayer {
                    index: session.document().layers().len(),
                    layer: Layer {
                        id: session.allocate_layer_id(),
                        name,
                        visible: true,
                        opacity: 1.0,
                        content: LayerContent::Fill {
                            color: LinearRgba::from_srgb_encoded_to_working(r, g, b, a),
                        },
                    },
                }
            }
            EditRequest::RemoveLayer { id } => Edit::RemoveLayer {
                id: LayerId::from_raw(id),
            },
            EditRequest::SetLayerVisible { id, visible } => Edit::SetLayerVisible {
                id: LayerId::from_raw(id),
                visible,
            },
            EditRequest::SetLayerOpacity { id, opacity } => Edit::SetLayerOpacity {
                id: LayerId::from_raw(id),
                opacity,
            },
            EditRequest::RenameLayer { id, name } => Edit::RenameLayer {
                id: LayerId::from_raw(id),
                name,
            },
            EditRequest::MoveLayer { id, index } => Edit::MoveLayer {
                id: LayerId::from_raw(id),
                index,
            },
        }
    }
}

/// View changes the UI can request. Positions and deltas are in viewport device pixels.
#[derive(Debug, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ViewRequest {
    /// Show the whole document, and keep doing so on resize.
    Fit,
    /// Set the zoom (1.0 = 100%) around the viewport center.
    SetZoom { zoom: f64 },
    /// Multiply the zoom around a point (wheel, pinch).
    ZoomBy { factor: f64, x: f64, y: f64 },
    /// Next preset zoom in or out, around a point or the viewport center.
    Step {
        zoom_in: bool,
        x: Option<f64>,
        y: Option<f64>,
    },
    /// Move the content by a delta (it follows the pointer).
    Pan { dx: f64, dy: f64 },
}

impl ViewRequest {
    pub fn apply(self, viewport: &mut Viewport, document: Size) {
        match self {
            ViewRequest::Fit => viewport.fit(document),
            ViewRequest::SetZoom { zoom } => viewport.set_zoom(document, zoom),
            ViewRequest::ZoomBy { factor, x, y } => viewport.zoom_by(document, [x, y], factor),
            ViewRequest::Step { zoom_in, x, y } => {
                let direction = if zoom_in { ZoomStep::In } else { ZoomStep::Out };
                viewport.step(document, x.zip(y).map(|(x, y)| [x, y]), direction);
            }
            ViewRequest::Pan { dx, dy } => viewport.pan(document, dx, dy),
        }
    }
}

/// The view after a change: enough for the UI to reproject the frame it is showing
/// (`document = origin + output / zoom`, output in device pixels) until the new one arrives.
/// Outcome of presenting a view directly to the window (native presentation).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresentInfo {
    /// False when nothing was shown (window occluded, swapchain busy): present again later.
    pub presented: bool,
    pub revision: u64,
    /// 1.0 = 100%.
    pub zoom: f64,
    pub fit: bool,
    /// Engine-side time (composite + copy + present call), in ms.
    pub render_ms: f32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewInfo {
    /// 1.0 = 100%.
    pub zoom: f64,
    pub origin: [f64; 2],
    pub fit: bool,
}

impl ViewInfo {
    pub fn new(viewport: &Viewport) -> Self {
        Self {
            zoom: viewport.zoom(),
            origin: viewport.transform().origin,
            fit: viewport.is_fit(),
        }
    }
}

/// Binary frame sent to the UI: a fixed little-endian header followed by the RGBA8 sRGB
/// pixels (tightly packed rows, top to bottom). Keep `parseFrame` in `engine.ts` in sync.
///
/// | offset | type | field                                         |
/// | ------ | ---- | --------------------------------------------- |
/// | 0      | u32  | format version (2)                            |
/// | 4      | u32  | width                                         |
/// | 8      | u32  | height                                        |
/// | 12     | u32  | flags (bit 0: view is in fit mode)            |
/// | 16     | u64  | document revision rendered                    |
/// | 24     | f64  | zoom (1.0 = 100%)                             |
/// | 32     | f32  | engine render time in ms (GPU + readback)     |
/// | 36     | u32  | document id (low 32 bits)                     |
/// | 40     | f64  | view origin x (document px at output 0,0)     |
/// | 48     | f64  | view origin y                                 |
pub const FRAME_HEADER_LEN: usize = 56;
pub const FRAME_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy)]
pub struct FrameHeader {
    pub size: Size,
    pub fit: bool,
    pub revision: u64,
    pub zoom: f64,
    pub render_ms: f32,
    pub document_id: u64,
    pub origin: [f64; 2],
}

impl FrameHeader {
    pub fn to_bytes(self) -> [u8; FRAME_HEADER_LEN] {
        let mut bytes = [0u8; FRAME_HEADER_LEN];
        bytes[0..4].copy_from_slice(&FRAME_VERSION.to_le_bytes());
        bytes[4..8].copy_from_slice(&self.size.width.to_le_bytes());
        bytes[8..12].copy_from_slice(&self.size.height.to_le_bytes());
        bytes[12..16].copy_from_slice(&u32::from(self.fit).to_le_bytes());
        bytes[16..24].copy_from_slice(&self.revision.to_le_bytes());
        bytes[24..32].copy_from_slice(&self.zoom.to_le_bytes());
        bytes[32..36].copy_from_slice(&self.render_ms.to_le_bytes());
        // Truncation is intended: the UI only compares ids, and 2^32 opened documents is plenty.
        bytes[36..40].copy_from_slice(&(self.document_id as u32).to_le_bytes());
        bytes[40..48].copy_from_slice(&self.origin[0].to_le_bytes());
        bytes[48..56].copy_from_slice(&self.origin[1].to_le_bytes());
        bytes
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuInfo {
    pub name: String,
    pub backend: String,
    pub device_type: String,
    pub driver: String,
}
