//! IPC data types: the contract between the UI and the engine.
//!
//! These DTOs are deliberately separate from the core model so that the engine can evolve
//! (e.g. towards a DAG) without breaking the UI, and so that the core stays free of serde.
//! Keep `app/src/lib/engine.ts` in sync with this file.
//!
//! Layer ids travel as JSON numbers: exact up to 2^53, far beyond what a session allocates.

use serde::{Deserialize, Serialize};
use slopshop_core::color::ColorSpace;
use slopshop_core::{Document, Edit, Layer, LayerContent, LayerId, LinearRgba, Session};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentView {
    pub width: u32,
    pub height: u32,
    /// Identifier (e.g. `linear-srgb`), translated by the UI.
    pub working_space: &'static str,
    pub revision: u64,
    pub can_undo: bool,
    pub can_redo: bool,
    /// Bottom to top, like the core model.
    pub layers: Vec<LayerView>,
}

#[derive(Debug, Serialize)]
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
    pub fn new(session: &Session) -> Self {
        let doc: &Document = session.document();
        Self {
            width: doc.size().width,
            height: doc.size().height,
            working_space: color_space_id(doc.working_space()),
            revision: doc.revision(),
            can_undo: session.can_undo(),
            can_redo: session.can_redo(),
            layers: doc.layers().iter().map(LayerView::new).collect(),
        }
    }
}

impl LayerView {
    fn new(layer: &Layer) -> Self {
        let (kind, swatch) = match layer.content {
            LayerContent::Fill { color } => ("fill", color.to_srgb_encoded()),
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
    match space {
        ColorSpace::LinearSrgb => "linear-srgb",
        ColorSpace::Srgb => "srgb",
    }
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
                            color: LinearRgba::from_srgb_encoded(r, g, b, a),
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuInfo {
    pub name: String,
    pub backend: String,
    pub device_type: String,
    pub driver: String,
}
