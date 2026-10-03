//! SlopShop core: the document model and everything that can be reasoned about without a GPU
//! or a UI.
//!
//! - [`geom`] and [`tile`]: integer geometry and tiling, the basis for regions of interest and
//!   out-of-core images.
//! - [`color`]: explicit color spaces, pixel formats and named transfer functions.
//! - [`raster`]: immutable tiled pixel images with a display pyramid.
//! - [`document`]: the document and its layers (read-only from the outside).
//! - [`edit`]: the only way to mutate a document; every edit yields its inverse.
//! - [`selection`]: selections as coverage masks: shapes, combining, feather, outlines;
//!   [`quick_select`]: Quick Selection by color (a minimum cut).
//! - [`session`]: a document plus its undo/redo history.
//! - [`stack`]: a raster layer's own stack of paint and applied effects (ADR 0029).
//! - [`view`]: mapping between output (screen) pixels and document pixels.
//! - [`transform`] and [`resample`]: layer transforms and how transformed layers are sampled.
//! - [`composite`]: CPU reference compositor, at full resolution (export oracle and fallback).
//! - [`convert`]: working-space pixels to a target pixel format, counting every lossy event.
//! - [`copy`]: the clipboard's pixels (Copy Merged, the image other applications get) and where
//!   Paste places what it pastes.
//! - [`job`]: cancellation and progress of background jobs.
//! - [`align`]: Layer > Align and Distribute.
//! - [`bake`]: Layer > Bake to Pixels: Rasterize, Merge, Merge Visible, Flatten (ADR 0030).
//! - [`trim`]: Image > Trim, the canvas reduced to the image without its uniform margins.
//! - [`auto`]: Image > Auto Tone, Auto Contrast and Auto Color, Levels computed from the image.

pub mod adjust;
pub mod align;
pub mod auto;
pub mod bake;
pub mod blend;
mod blue_noise;
pub mod color;
pub mod composite;
pub mod convert;
pub mod copy;
pub mod curve;
pub mod document;
pub mod edit;
pub mod geom;
pub mod gradient;
pub mod job;
mod maxflow;
pub mod move_pixels;
pub mod paint;
pub mod pick;
pub mod quick_select;
pub mod raster;
pub mod resample;
pub mod selection;
pub mod session;
pub mod stack;
pub mod thumbnail;
pub mod tile;
pub mod transform;
pub mod trim;
pub mod view;

pub use blend::{BlendMode, BlendSpace};
pub use color::{ColorSpace, LinearRgba};
pub use document::{
    Document, Layer, LayerContent, LayerId, LayerMask, RestoreError, SavedSelection,
    SavedSelectionId,
};
pub use edit::{Arrange, Edit, EditError, ImageTurn};
pub use geom::{Rect, Size};
pub use job::{CancelToken, Progress};
pub use raster::RasterImage;
pub use session::{Copies, Session};
pub use transform::Affine;
