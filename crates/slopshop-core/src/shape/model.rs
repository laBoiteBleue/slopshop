//! What a vector shape is (ADR 0041): a geometry in the layer's own space (live shapes keep
//! their parameters, as Photoshop's), an optional fill and an optional stroke.

use std::sync::Arc;

use crate::color::LinearRgba;
use crate::source::SourceId;

/// The largest coordinate or length a shape takes, in pixels (as transforms, ADR 0018).
pub const MAX_COORDINATE: f64 = 1e7;

/// The most sides a polygon takes (Photoshop's).
pub const MAX_SIDES: u32 = 100;

/// The longest dash pattern.
pub const MAX_DASHES: usize = 64;

/// The widest stroke, in pixels (Photoshop's limit for shape strokes is far below).
pub const MAX_STROKE_WIDTH: f64 = 10_000.0;

/// Where a shape's outline lies.
#[derive(Debug, Clone, PartialEq)]
pub enum Geometry {
    /// `[left, top, right, bottom]`, corners rounded by `radii` (top left, top right, bottom
    /// right, bottom left).
    Rectangle {
        rect: [f64; 4],
        radii: [f64; 4],
    },
    Ellipse {
        center: [f64; 2],
        radii: [f64; 2],
    },
    /// A regular polygon of `sides` around `center`, its first corner `radius` away at
    /// `rotation` degrees (counterclockwise from the right, up being negative y); a star when
    /// `star` gives its inner corners' distance as a fraction of `radius`.
    Polygon {
        center: [f64; 2],
        radius: f64,
        sides: u32,
        star: Option<f64>,
        rotation: f64,
    },
    /// A straight line, drawn by the stroke only.
    Line {
        from: [f64; 2],
        to: [f64; 2],
    },
    /// Paths of lines and cubic Béziers (the Pen's).
    Path {
        subpaths: Vec<Subpath>,
        rule: FillRule,
    },
}

/// One connected run of a path.
#[derive(Debug, Clone, PartialEq)]
pub struct Subpath {
    pub start: [f64; 2],
    pub segments: Vec<Segment>,
    pub closed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Segment {
    Line([f64; 2]),
    /// Two control points, then the end.
    Cubic([f64; 2], [f64; 2], [f64; 2]),
}

/// Which points a path covers where it crosses itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillRule {
    NonZero,
    EvenOdd,
}

/// What covers the shape: a color for now (gradients come later).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Paint {
    Solid(LinearRgba),
}

/// Where a stroke lies along the outline (Photoshop's alignment).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrokeAlign {
    Inside,
    Center,
    Outside,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrokeCap {
    Butt,
    Round,
    Square,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrokeJoin {
    Miter,
    Round,
    Bevel,
}

/// A shape's stroke.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapeStroke {
    pub paint: Paint,
    pub width: f64,
    pub align: StrokeAlign,
    pub cap: StrokeCap,
    pub join: StrokeJoin,
    /// The longest a miter goes, in stroke widths, before it is beveled.
    pub miter_limit: f64,
    /// Dashes and gaps in turn, in stroke widths (empty: a solid stroke).
    pub dashes: Vec<f64>,
    pub dash_offset: f64,
}

/// A vector shape: its geometry, a fill, a stroke.
#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    pub geometry: Geometry,
    pub fill: Option<Paint>,
    pub stroke: Option<ShapeStroke>,
}

fn point_ok(p: [f64; 2]) -> bool {
    p.iter().all(|v| v.is_finite() && v.abs() <= MAX_COORDINATE)
}

fn length_ok(v: f64) -> bool {
    v.is_finite() && (0.0..=MAX_COORDINATE).contains(&v)
}

fn paint_ok(paint: Paint) -> bool {
    match paint {
        Paint::Solid(c) => {
            [c.r, c.g, c.b, c.a].iter().all(|v| v.is_finite()) && (0.0..=1.0).contains(&c.a)
        }
    }
}

impl Geometry {
    pub fn is_valid(&self) -> bool {
        match self {
            Self::Rectangle { rect, radii } => {
                let [l, t, r, b] = *rect;
                point_ok([l, t])
                    && point_ok([r, b])
                    && l <= r
                    && t <= b
                    && radii.iter().all(|&v| length_ok(v))
            }
            Self::Ellipse { center, radii } => {
                point_ok(*center) && radii.iter().all(|&v| length_ok(v))
            }
            Self::Polygon {
                center,
                radius,
                sides,
                star,
                rotation,
            } => {
                point_ok(*center)
                    && length_ok(*radius)
                    && (3..=MAX_SIDES).contains(sides)
                    && star.is_none_or(|s| s.is_finite() && (0.0..=1.0).contains(&s))
                    && rotation.is_finite()
            }
            Self::Line { from, to } => point_ok(*from) && point_ok(*to),
            Self::Path { subpaths, .. } => subpaths.iter().all(|s| {
                point_ok(s.start)
                    && s.segments.iter().all(|seg| match *seg {
                        Segment::Line(p) => point_ok(p),
                        Segment::Cubic(a, b, p) => point_ok(a) && point_ok(b) && point_ok(p),
                    })
            }),
        }
    }

    /// Whether it has an inside to fill (a line has none).
    pub fn is_closed(&self) -> bool {
        match self {
            Self::Line { .. } => false,
            Self::Path { subpaths, .. } => subpaths.iter().any(|s| s.closed),
            _ => true,
        }
    }
}

impl ShapeStroke {
    pub fn is_valid(&self) -> bool {
        paint_ok(self.paint)
            && self.width.is_finite()
            && self.width > 0.0
            && self.width <= MAX_STROKE_WIDTH
            && self.miter_limit.is_finite()
            && self.miter_limit >= 1.0
            && self.dashes.len() <= MAX_DASHES
            && self.dashes.iter().all(|&d| d.is_finite() && d >= 0.0)
            && (self.dashes.is_empty() || self.dashes.iter().sum::<f64>() > 0.0)
            && self.dash_offset.is_finite()
    }
}

impl Shape {
    pub fn is_valid(&self) -> bool {
        self.geometry.is_valid()
            && self.fill.is_none_or(paint_ok)
            && self.stroke.as_ref().is_none_or(ShapeStroke::is_valid)
    }
}

/// A shape kept once and referenced by the layers showing it (ADR 0040, ADR 0041): immutable;
/// changing it makes a new one, the layers repointed.
#[derive(Debug)]
pub struct ShapeSource {
    id: SourceId,
    /// What the user knows it by; empty when nothing names it.
    name: String,
    shape: Shape,
}

/// Sources are immutable: the same source is the same allocation, told by its id.
impl PartialEq for ShapeSource {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl ShapeSource {
    /// A new source of `shape`.
    pub fn new(shape: Shape, name: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            id: SourceId::next(),
            name: name.into(),
            shape,
        })
    }

    pub fn id(&self) -> SourceId {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn shape(&self) -> &Shape {
        &self.shape
    }

    /// Another source of the same shape (Make Unique).
    pub fn unique(&self) -> Arc<Self> {
        Self::new(self.shape.clone(), self.name.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> Geometry {
        Geometry::Rectangle {
            rect: [10.0, 10.0, 50.0, 30.0],
            radii: [0.0; 4],
        }
    }

    #[test]
    fn shapes_are_validated() {
        let red = Paint::Solid(LinearRgba::new(1.0, 0.0, 0.0, 1.0));
        let stroke = ShapeStroke {
            paint: red,
            width: 3.0,
            align: StrokeAlign::Center,
            cap: StrokeCap::Butt,
            join: StrokeJoin::Miter,
            miter_limit: 4.0,
            dashes: vec![2.0, 1.0],
            dash_offset: 0.0,
        };
        let shape = Shape {
            geometry: rect(),
            fill: Some(red),
            stroke: Some(stroke.clone()),
        };
        assert!(shape.is_valid());
        let bad = |geometry| {
            !Shape {
                geometry,
                fill: None,
                stroke: None,
            }
            .is_valid()
        };
        assert!(bad(Geometry::Rectangle {
            rect: [50.0, 10.0, 10.0, 30.0],
            radii: [0.0; 4],
        }));
        assert!(bad(Geometry::Ellipse {
            center: [f64::NAN, 0.0],
            radii: [1.0, 1.0],
        }));
        assert!(bad(Geometry::Polygon {
            center: [0.0, 0.0],
            radius: 5.0,
            sides: 2,
            star: None,
            rotation: 0.0,
        }));
        assert!(bad(Geometry::Polygon {
            center: [0.0, 0.0],
            radius: 5.0,
            sides: 5,
            star: Some(1.5),
            rotation: 0.0,
        }));
        for wrong in [
            ShapeStroke {
                width: 0.0,
                ..stroke.clone()
            },
            ShapeStroke {
                miter_limit: 0.5,
                ..stroke.clone()
            },
            ShapeStroke {
                dashes: vec![0.0, 0.0],
                ..stroke.clone()
            },
            ShapeStroke {
                dashes: vec![-1.0],
                ..stroke.clone()
            },
        ] {
            let shape = Shape {
                geometry: rect(),
                fill: None,
                stroke: Some(wrong),
            };
            assert!(!shape.is_valid(), "{shape:?}");
        }
        assert!(
            !Geometry::Line {
                from: [0.0, 0.0],
                to: [1.0, 1.0]
            }
            .is_closed()
        );
        assert!(rect().is_closed());
    }

    #[test]
    fn shape_sources_are_told_apart_by_identity() {
        let shape = Shape {
            geometry: rect(),
            fill: None,
            stroke: None,
        };
        let a = ShapeSource::new(shape.clone(), "Rectangle 1");
        let b = a.unique();
        assert_ne!(a.id(), b.id());
        assert_ne!(a, b);
        assert_eq!(b.shape(), &shape);
        assert_eq!(b.name(), "Rectangle 1");
    }
}
