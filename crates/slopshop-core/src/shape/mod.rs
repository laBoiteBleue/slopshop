//! Vector shapes (ADR 0041): live shapes and paths, their fill and stroke, and the anti-aliased
//! coverage they cover, the same on every computer that reads it (the CPU compositor and the
//! GPU draw from these tiles, so they agree by construction).
//!
//! - [`model`]: what a shape is (geometry, fill, stroke), validated.
//! - [`path`]: its outline as Béziers, placed by a layer's transform.
//! - [`draw`]: the coverage of an outline, tile by tile.
//! - [`text`]: what a text layer says (its outlines are laid out by `slopshop-text`).
//! - [`layer`]: what a vector layer draws: its paints as masked fills (as a style's effects).
//! - [`mask`]: vector masks, a path masking any layer.

pub mod draw;
pub mod layer;
pub mod mask;
pub mod model;
pub mod path;
pub mod spans;
pub mod text;

pub use draw::{COVERAGE_FORMAT, coverage};
pub use layer::{Drawing, Drawn};
pub use mask::VectorMask;
pub use model::{
    FillRule, Geometry, Paint, Segment, Shape, ShapeSource, ShapeStroke, StrokeAlign, StrokeCap,
    StrokeJoin, Subpath,
};
pub use path::selection_rings;
pub use spans::{CharStyle, Resolved, TextSpan};
pub use text::{Text, TextAlign};
