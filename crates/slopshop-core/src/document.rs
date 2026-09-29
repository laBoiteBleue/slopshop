//! The document and its layers.
//!
//! The document is readable by anyone but only mutable through [`crate::edit::Edit`], so that
//! every change is undoable. Users see a layer stack; code refers to layers by [`LayerId`]
//! (stable, never reused) so that the model can later evolve into a DAG of nodes.

use std::fmt;
use std::sync::Arc;

use crate::color::{ColorSpace, LinearRgba, WORKING_SPACE};
use crate::geom::Size;
use crate::raster::RasterImage;

/// Stable identifier of a layer within a document. Never reused, even after undo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LayerId(u64);

impl LayerId {
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Rebuild an id received from outside (e.g. the UI). It may not exist in the document;
    /// edits validate ids before using them.
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }
}

impl fmt::Display for LayerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "layer#{}", self.0)
    }
}

/// What a layer produces. Adjustment and AI nodes come later.
#[derive(Debug, Clone)]
pub enum LayerContent {
    /// A uniform color over the whole canvas, in the document working space.
    Fill { color: LinearRgba },
    /// Source pixels, placed at the document origin. The image is immutable and shared:
    /// cloning the layer (snapshots, undo) never copies pixels.
    Raster { image: Arc<RasterImage> },
}

impl PartialEq for LayerContent {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Fill { color: a }, Self::Fill { color: b }) => a == b,
            // Immutable images: same allocation, same content.
            (Self::Raster { image: a }, Self::Raster { image: b }) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub id: LayerId,
    pub name: String,
    pub visible: bool,
    /// In `[0, 1]`, validated by edits.
    pub opacity: f32,
    pub content: LayerContent,
}

#[derive(Debug, Clone)]
pub struct Document {
    size: Size,
    working_space: ColorSpace,
    /// Bottom to top.
    layers: Vec<Layer>,
    next_layer_id: u64,
    revision: u64,
}

impl Document {
    /// An empty document. Compositing happens in linear light, so the working space is linear.
    pub fn new(size: Size) -> Self {
        Self {
            size,
            working_space: WORKING_SPACE,
            layers: Vec::new(),
            next_layer_id: 1,
            revision: 0,
        }
    }

    pub fn size(&self) -> Size {
        self.size
    }

    pub fn working_space(&self) -> ColorSpace {
        self.working_space
    }

    /// Layers from bottom to top.
    pub fn layers(&self) -> &[Layer] {
        &self.layers
    }

    pub fn layer(&self, id: LayerId) -> Option<&Layer> {
        self.layers.iter().find(|l| l.id == id)
    }

    /// Position of a layer in the stack (0 = bottom).
    pub fn layer_index(&self, id: LayerId) -> Option<usize> {
        self.layers.iter().position(|l| l.id == id)
    }

    /// Incremented by every successful edit (including undo/redo). Lets caches and views know
    /// that something changed without diffing.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Reserve a fresh id for a layer about to be inserted with an edit. This is not a document
    /// change (ids are never reused, so undo does not give them back).
    pub fn allocate_layer_id(&mut self) -> LayerId {
        let id = LayerId(self.next_layer_id);
        self.next_layer_id += 1;
        id
    }

    // Mutation primitives, only reachable through `edit`.

    pub(crate) fn layers_mut(&mut self) -> &mut Vec<Layer> {
        &mut self.layers
    }

    pub(crate) fn layer_mut(&mut self, id: LayerId) -> Option<&mut Layer> {
        self.layers.iter_mut().find(|l| l.id == id)
    }

    pub(crate) fn bump_revision(&mut self) {
        self.revision += 1;
    }

    /// Whether `id` was ever handed out by [`Self::allocate_layer_id`].
    pub(crate) fn is_allocated(&self, id: LayerId) -> bool {
        id.0 != 0 && id.0 < self.next_layer_id
    }
}
