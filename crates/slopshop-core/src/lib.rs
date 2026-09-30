//! SlopShop core: the document model and everything that can be reasoned about without a GPU
//! or a UI.
//!
//! - [`geom`] and [`tile`]: integer geometry and tiling, the basis for regions of interest and
//!   out-of-core images.
//! - [`color`]: explicit color spaces, pixel formats and named transfer functions.
//! - [`raster`]: immutable tiled pixel images with a display pyramid.
//! - [`document`]: the document and its layers (read-only from the outside).
//! - [`edit`]: the only way to mutate a document; every edit yields its inverse.
//! - [`session`]: a document plus its undo/redo history.
//! - [`view`]: mapping between output (screen) pixels and document pixels.
//! - [`composite`]: CPU reference compositor, at full resolution (export oracle and fallback).
//! - [`convert`]: working-space pixels to a target pixel format, counting every lossy event.
//! - [`job`]: cancellation and progress of background jobs.

pub mod blend;
mod blue_noise;
pub mod color;
pub mod composite;
pub mod convert;
pub mod document;
pub mod edit;
pub mod geom;
pub mod job;
pub mod raster;
pub mod session;
pub mod tile;
pub mod view;

pub use blend::{BlendMode, BlendSpace};
pub use color::{ColorSpace, LinearRgba};
pub use document::{Document, Layer, LayerContent, LayerId, RestoreError};
pub use edit::{Edit, EditError};
pub use geom::{Rect, Size};
pub use job::{CancelToken, Progress};
pub use raster::RasterImage;
pub use session::Session;
