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
//! - [`view`]: mapping between output (screen) pixels and document pixels.
//! - [`transform`] and [`resample`]: layer transforms and how transformed layers are sampled.
//! - [`composite`]: CPU reference compositor, at full resolution (export oracle and fallback).
//! - [`convert`]: working-space pixels to a target pixel format, counting every lossy event.
//! - [`job`]: cancellation and progress of background jobs.

pub mod adjust;
pub mod blend;
mod blue_noise;
pub mod color;
pub mod composite;
pub mod convert;
pub mod curve;
pub mod document;
pub mod edit;
pub mod geom;
pub mod job;
mod maxflow;
pub mod paint;
pub mod pick;
pub mod quick_select;
pub mod raster;
pub mod resample;
pub mod selection;
pub mod session;
pub mod thumbnail;
pub mod tile;
pub mod transform;
pub mod view;

pub use blend::{BlendMode, BlendSpace};
pub use color::{ColorSpace, LinearRgba};
pub use document::{Document, Layer, LayerContent, LayerId, LayerMask, RestoreError};
pub use edit::{Edit, EditError, ImageTurn};
pub use geom::{Rect, Size};
pub use job::{CancelToken, Progress};
pub use raster::RasterImage;
pub use session::{Copies, Session};
pub use transform::Affine;
