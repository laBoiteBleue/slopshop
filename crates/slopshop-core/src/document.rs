//! The document and its layers.
//!
//! The document is readable by anyone but only mutable through [`crate::edit::Edit`], so that
//! every change is undoable. Users see a layer stack; code refers to layers by [`LayerId`]
//! (stable, never reused) so that the model can later evolve into a DAG of nodes.

use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;

use crate::blend::{BlendMode, BlendSpace};
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
    /// How the layer combines with the layers below (ADR 0012).
    pub blend_mode: BlendMode,
    pub content: LayerContent,
    /// Hides parts of the layer (ADR 0014).
    pub mask: Option<LayerMask>,
}

/// A layer mask (ADR 0014): a gray raster at the document origin whose samples are the layer's
/// coverage, read linearly and clamped to `[0, 1]` (0 hides, 1 shows; outside the image the
/// layer is hidden).
#[derive(Debug, Clone)]
pub struct LayerMask {
    /// Gray, immutable and shared like any raster.
    pub image: Arc<RasterImage>,
    /// Applied; a disabled mask is kept but ignored.
    pub enabled: bool,
    /// Made from the layer's transparency: while the mask exists (enabled or not), the layer's
    /// own alpha is ignored, as Photoshop moves transparency into the mask.
    pub replaces_alpha: bool,
}

impl PartialEq for LayerMask {
    fn eq(&self, other: &Self) -> bool {
        // Immutable images: same allocation, same content.
        Arc::ptr_eq(&self.image, &other.image)
            && self.enabled == other.enabled
            && self.replaces_alpha == other.replaces_alpha
    }
}

impl LayerMask {
    /// A mask from the transparency of `image` (its alpha channel), enabled, replacing the
    /// layer's own alpha. `None` when the image has no alpha.
    pub fn from_transparency(image: &RasterImage) -> Option<Self> {
        Some(Self {
            image: Arc::new(image.alpha_mask()?),
            enabled: true,
            replaces_alpha: true,
        })
    }

    /// Whether `image` can be a mask: gray, without alpha.
    pub fn is_valid_image(image: &RasterImage) -> bool {
        image.format().layout == crate::color::ChannelLayout::Gray
    }
}

#[derive(Debug, Clone)]
pub struct Document {
    size: Size,
    working_space: ColorSpace,
    /// Where layers are blended (ADR 0012).
    blend_space: BlendSpace,
    /// Bottom to top.
    layers: Vec<Layer>,
    next_layer_id: u64,
    revision: u64,
}

impl Document {
    /// An empty document. Colors are stored in linear light (the working space is linear);
    /// layers blend in the default [`BlendSpace`] (perceptual).
    pub fn new(size: Size) -> Self {
        Self {
            size,
            working_space: WORKING_SPACE,
            blend_space: BlendSpace::default(),
            layers: Vec::new(),
            next_layer_id: 1,
            revision: 0,
        }
    }

    /// Rebuild a document saved earlier (e.g. read from a file), keeping its layer ids and its
    /// id counter so that ids are never reused after a reload. Everything is validated as edits
    /// would: ids are unique and below `next_layer_id` (and not 0), opacities are in `[0, 1]`,
    /// fill colors are finite. Only the engine's working space is supported. The revision
    /// starts at 0.
    pub fn restore(
        size: Size,
        working_space: ColorSpace,
        blend_space: BlendSpace,
        layers: Vec<Layer>,
        next_layer_id: u64,
    ) -> Result<Self, RestoreError> {
        if working_space != WORKING_SPACE {
            return Err(RestoreError::UnsupportedWorkingSpace(working_space));
        }
        let mut seen = HashSet::with_capacity(layers.len());
        for layer in &layers {
            let id = layer.id;
            if id.0 == 0 || id.0 >= next_layer_id {
                return Err(RestoreError::IdOutOfRange { id, next_layer_id });
            }
            if !seen.insert(id) {
                return Err(RestoreError::DuplicateId(id));
            }
            if crate::edit::validate_opacity(layer.opacity).is_err() {
                return Err(RestoreError::InvalidOpacity(id));
            }
            if let LayerContent::Fill { color } = &layer.content
                && !color.is_finite()
            {
                return Err(RestoreError::InvalidColor(id));
            }
            if let Some(mask) = &layer.mask
                && !LayerMask::is_valid_image(&mask.image)
            {
                return Err(RestoreError::InvalidMask(id));
            }
        }
        Ok(Self {
            size,
            working_space,
            blend_space,
            layers,
            next_layer_id,
            revision: 0,
        })
    }

    pub fn size(&self) -> Size {
        self.size
    }

    /// The id the next allocated layer will get: every id below it was handed out already.
    pub fn next_layer_id(&self) -> u64 {
        self.next_layer_id
    }

    pub fn working_space(&self) -> ColorSpace {
        self.working_space
    }

    /// Where layers are blended.
    pub fn blend_space(&self) -> BlendSpace {
        self.blend_space
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

    pub(crate) fn set_blend_space(&mut self, space: BlendSpace) -> BlendSpace {
        std::mem::replace(&mut self.blend_space, space)
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

/// Why [`Document::restore`] refused its input.
#[derive(Debug, Clone, PartialEq)]
pub enum RestoreError {
    /// The engine composites in [`WORKING_SPACE`] only.
    UnsupportedWorkingSpace(ColorSpace),
    /// 0, or not below the id counter.
    IdOutOfRange {
        id: LayerId,
        next_layer_id: u64,
    },
    DuplicateId(LayerId),
    /// Opacity outside `[0, 1]` (or NaN).
    InvalidOpacity(LayerId),
    /// A fill color with a non-finite component.
    InvalidColor(LayerId),
    /// A mask that is not a gray image.
    InvalidMask(LayerId),
}

impl fmt::Display for RestoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RestoreError::UnsupportedWorkingSpace(space) => {
                write!(f, "unsupported working space {space:?}")
            }
            RestoreError::IdOutOfRange { id, next_layer_id } => {
                write!(f, "{id} is not below the id counter {next_layer_id}")
            }
            RestoreError::DuplicateId(id) => write!(f, "{id} appears twice"),
            RestoreError::InvalidOpacity(id) => write!(f, "{id} has an invalid opacity"),
            RestoreError::InvalidColor(id) => write!(f, "{id} has a non-finite color"),
            RestoreError::InvalidMask(id) => write!(f, "{id} has a mask that is not gray"),
        }
    }
}

impl std::error::Error for RestoreError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::Edit;

    fn fill(id: u64, opacity: f32) -> Layer {
        Layer {
            id: LayerId(id),
            name: format!("fill {id}"),
            visible: true,
            opacity,
            blend_mode: BlendMode::Normal,
            mask: None,
            content: LayerContent::Fill {
                color: LinearRgba::new(0.1, 0.2, 0.3, 1.0),
            },
        }
    }

    #[test]
    fn restore_keeps_ids_and_the_counter() {
        let mut doc = Document::new(Size::new(4, 3));
        for _ in 0..3 {
            let id = doc.allocate_layer_id();
            let index = doc.layers().len();
            let layer = Layer {
                id,
                ..fill(id.get(), 0.5)
            };
            Edit::InsertLayer { index, layer }.apply(&mut doc).unwrap();
        }
        // A removed layer: its id is never given out again.
        let removed = doc.layers()[1].id;
        Edit::RemoveLayer { id: removed }.apply(&mut doc).unwrap();

        let mut restored = Document::restore(
            doc.size(),
            doc.working_space(),
            BlendSpace::Linear,
            doc.layers().to_vec(),
            doc.next_layer_id(),
        )
        .unwrap();
        assert_eq!(restored.layers(), doc.layers());
        assert_eq!(restored.blend_space(), BlendSpace::Linear);
        assert_eq!(restored.size(), doc.size());
        assert_eq!(restored.revision(), 0);
        assert_eq!(restored.allocate_layer_id(), LayerId(4));
    }

    #[test]
    fn restore_validates_like_edits() {
        let size = Size::new(2, 2);
        let restore = |layers: Vec<Layer>, next: u64| {
            Document::restore(size, WORKING_SPACE, BlendSpace::default(), layers, next).map(|_| ())
        };
        assert_eq!(restore(vec![fill(1, 1.0), fill(2, 0.0)], 3), Ok(()));
        assert_eq!(restore(vec![], 1), Ok(()));
        assert_eq!(
            restore(vec![fill(0, 1.0)], 3),
            Err(RestoreError::IdOutOfRange {
                id: LayerId(0),
                next_layer_id: 3
            })
        );
        assert!(matches!(
            restore(vec![fill(3, 1.0)], 3),
            Err(RestoreError::IdOutOfRange { .. })
        ));
        assert_eq!(
            restore(vec![fill(1, 1.0), fill(1, 1.0)], 3),
            Err(RestoreError::DuplicateId(LayerId(1)))
        );
        for opacity in [-0.1, 1.5, f32::NAN] {
            assert_eq!(
                restore(vec![fill(1, opacity)], 2),
                Err(RestoreError::InvalidOpacity(LayerId(1)))
            );
        }
        let mut infinite = fill(1, 1.0);
        infinite.content = LayerContent::Fill {
            color: LinearRgba::new(f32::INFINITY, 0.0, 0.0, 1.0),
        };
        assert_eq!(
            restore(vec![infinite], 2),
            Err(RestoreError::InvalidColor(LayerId(1)))
        );
        assert!(matches!(
            Document::restore(size, ColorSpace::SRGB, BlendSpace::default(), vec![], 1),
            Err(RestoreError::UnsupportedWorkingSpace(_))
        ));
    }
}
