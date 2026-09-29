//! SlopShop core: the document model and everything that can be reasoned about without a GPU
//! or a UI.
//!
//! - [`geom`] and [`tile`]: integer geometry and tiling, the basis for regions of interest and
//!   out-of-core images.
//! - [`color`]: explicit color spaces, pixel formats and named transfer functions.
//! - [`document`]: the document and its layers (read-only from the outside).
//! - [`edit`]: the only way to mutate a document; every edit yields its inverse.
//! - [`session`]: a document plus its undo/redo history.
//! - [`view`]: mapping between output (screen) pixels and document pixels.

pub mod color;
pub mod document;
pub mod edit;
pub mod geom;
pub mod session;
pub mod tile;
pub mod view;

pub use color::{ColorSpace, LinearRgba};
pub use document::{Document, Layer, LayerContent, LayerId};
pub use edit::{Edit, EditError};
pub use geom::{Rect, Size};
pub use session::Session;
