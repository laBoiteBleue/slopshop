//! What a vector shape is (ADR 0041): a geometry in the layer's own space (live shapes keep
//! their parameters, as Photoshop's), an optional fill and an optional stroke.

use std::sync::Arc;

use crate::color::LinearRgba;
use crate::source::SourceId;
use crate::transform::Affine;

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

    /// The geometry `by` maps this one to, when it stays what it is, so that Free Transform
    /// reshapes a vector layer rather than scaling its stroke (Photoshop's): a line and a path
    /// always; a rectangle and an ellipse when `by` neither rotates nor skews (a rectangle's
    /// corners stay round: their radii scale as its shorter side does); a polygon when `by`
    /// keeps its proportions (a rotation, a uniform scale, no mirroring). `None` otherwise, or
    /// for a map that collapses or overflows.
    pub fn transformed(&self, by: Affine) -> Option<Geometry> {
        const EPSILON: f64 = 1e-9;
        if !by.is_finite() || by.determinant().abs() < EPSILON {
            return None;
        }
        let map = |[x, y]: [f64; 2]| {
            let (x, y) = by.apply(x, y);
            [x, y]
        };
        let aligned = by.b.abs() < EPSILON && by.c.abs() < EPSILON;
        let geometry = match self {
            &Self::Rectangle { rect, radii } if aligned => {
                let [l, t, r, b] = rect;
                let ([x0, y0], [x1, y1]) = (map([l, t]), map([r, b]));
                // Mirrored, the corners trade places.
                let [mut tl, mut tr, mut br, mut bl] = radii;
                if by.a < 0.0 {
                    (tl, tr, br, bl) = (tr, tl, bl, br);
                }
                if by.d < 0.0 {
                    (tl, tr, br, bl) = (bl, br, tr, tl);
                }
                let scale = by.a.abs().min(by.d.abs());
                Self::Rectangle {
                    rect: [x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)],
                    radii: [tl, tr, br, bl].map(|v| v * scale),
                }
            }
            &Self::Ellipse { center, radii } if aligned => Self::Ellipse {
                center: map(center),
                radii: [radii[0] * by.a.abs(), radii[1] * by.d.abs()],
            },
            &Self::Polygon {
                center,
                radius,
                sides,
                star,
                rotation,
            } => {
                let similar = (by.a - by.d).abs() < EPSILON
                    && (by.b + by.c).abs() < EPSILON
                    && by.determinant() > 0.0;
                if !similar {
                    return None;
                }
                // Turned clockwise on screen by `turn` degrees: its corners' angles
                // (counterclockwise) go down as much.
                let turn = by.b.atan2(by.a).to_degrees();
                Self::Polygon {
                    center: map(center),
                    radius: radius * by.a.hypot(by.b),
                    sides,
                    star,
                    rotation: rotation - turn,
                }
            }
            &Self::Line { from, to } => Self::Line {
                from: map(from),
                to: map(to),
            },
            Self::Path { subpaths, rule } => Self::Path {
                subpaths: subpaths
                    .iter()
                    .map(|s| Subpath {
                        start: map(s.start),
                        segments: s
                            .segments
                            .iter()
                            .map(|seg| match *seg {
                                Segment::Line(p) => Segment::Line(map(p)),
                                Segment::Cubic(a, b, p) => Segment::Cubic(map(a), map(b), map(p)),
                            })
                            .collect(),
                        closed: s.closed,
                    })
                    .collect(),
                rule: *rule,
            },
            _ => return None,
        };
        geometry.is_valid().then_some(geometry)
    }

    /// Whether a fill covers its inside: everything but a line. An open path is filled as if
    /// its ends were joined, as Photoshop fills one.
    pub fn is_fillable(&self) -> bool {
        !matches!(self, Self::Line { .. })
    }

    /// Whether its outline is closed (a stroke can lie inside or outside of it): a line's and an
    /// open path's are not.
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
    /// What a text layer says (ADR 0041): `shape` is then its glyph outlines, laid out from it.
    text: Option<super::text::Text>,
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
            text: None,
        })
    }

    /// A new source of text `text`, laid out into `outlines` (a path filled by the text's
    /// color).
    pub fn text(text: super::text::Text, outlines: Shape, name: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            id: SourceId::next(),
            name: name.into(),
            shape: outlines,
            text: Some(text),
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

    /// What it says when it is text.
    pub fn text_of(&self) -> Option<&super::text::Text> {
        self.text.as_ref()
    }

    /// Whether its shape and text are valid.
    pub fn is_valid(&self) -> bool {
        self.shape.is_valid() && self.text.as_ref().is_none_or(super::text::Text::is_valid)
    }

    /// Another source of the same shape (Make Unique).
    pub fn unique(&self) -> Arc<Self> {
        self.unique_named(self.name.clone())
    }

    /// Another source of the same shape named `name` (an independent copy).
    pub fn unique_named(&self, name: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            id: SourceId::next(),
            name: name.into(),
            shape: self.shape.clone(),
            text: self.text.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scaling(sx: f64, sy: f64) -> Affine {
        Affine {
            a: sx,
            b: 0.0,
            c: 0.0,
            d: sy,
            e: 0.0,
            f: 0.0,
        }
    }

    #[test]
    fn rectangles_and_ellipses_take_maps_that_keep_them() {
        // Scaled and moved, the corners' radii as the shorter side.
        let rounded = Geometry::Rectangle {
            rect: [10.0, 10.0, 50.0, 30.0],
            radii: [1.0, 2.0, 3.0, 4.0],
        };
        let by = Affine {
            e: 5.0,
            ..scaling(2.0, 3.0)
        };
        assert_eq!(
            rounded.transformed(by),
            Some(Geometry::Rectangle {
                rect: [25.0, 30.0, 105.0, 90.0],
                radii: [2.0, 4.0, 6.0, 8.0],
            })
        );
        // Mirrored, its corners with it.
        assert_eq!(
            rounded.transformed(scaling(-1.0, 1.0)),
            Some(Geometry::Rectangle {
                rect: [-50.0, 10.0, -10.0, 30.0],
                radii: [2.0, 1.0, 4.0, 3.0],
            })
        );
        assert_eq!(
            rounded.transformed(scaling(1.0, -1.0)),
            Some(Geometry::Rectangle {
                rect: [10.0, -30.0, 50.0, -10.0],
                radii: [4.0, 3.0, 2.0, 1.0],
            })
        );
        // Turned, it would no longer be one.
        assert_eq!(rounded.transformed(Affine::rotation(0.5)), None);
        let ellipse = Geometry::Ellipse {
            center: [10.0, 0.0],
            radii: [4.0, 2.0],
        };
        assert_eq!(
            ellipse.transformed(scaling(-2.0, 0.5)),
            Some(Geometry::Ellipse {
                center: [-20.0, 0.0],
                radii: [8.0, 1.0],
            })
        );
        assert_eq!(ellipse.transformed(Affine::rotation(0.1)), None);
    }

    #[test]
    fn polygons_take_rotations_and_uniform_scales() {
        let polygon = Geometry::Polygon {
            center: [10.0, 0.0],
            radius: 5.0,
            sides: 6,
            star: Some(0.5),
            rotation: 90.0,
        };
        // A quarter turn clockwise on screen, doubled.
        let by = Affine::rotation(std::f64::consts::FRAC_PI_2).then(scaling(2.0, 2.0));
        let Some(Geometry::Polygon {
            center,
            radius,
            sides,
            star,
            rotation,
        }) = polygon.transformed(by)
        else {
            panic!("a similarity keeps a polygon one");
        };
        let (cx, cy) = by.apply(10.0, 0.0);
        assert!((center[0] - cx).abs() < 1e-9 && (center[1] - cy).abs() < 1e-9);
        assert!((radius - 10.0).abs() < 1e-9);
        assert_eq!((sides, star), (6, Some(0.5)));
        assert!(rotation.abs() < 1e-9, "{rotation}");
        assert_eq!(polygon.transformed(scaling(2.0, 1.0)), None);
        assert_eq!(polygon.transformed(scaling(-1.0, 1.0)), None);
    }

    #[test]
    fn lines_and_paths_take_any_map() {
        let skew = Affine {
            c: 0.5,
            ..scaling(1.0, 2.0)
        };
        let line = Geometry::Line {
            from: [0.0, 2.0],
            to: [4.0, 0.0],
        };
        assert_eq!(
            line.transformed(skew),
            Some(Geometry::Line {
                from: [1.0, 4.0],
                to: [4.0, 0.0],
            })
        );
        let path = Geometry::Path {
            subpaths: vec![Subpath {
                start: [0.0, 0.0],
                segments: vec![
                    Segment::Line([2.0, 0.0]),
                    Segment::Cubic([2.0, 2.0], [0.0, 2.0], [0.0, 0.0]),
                ],
                closed: true,
            }],
            rule: FillRule::EvenOdd,
        };
        assert_eq!(
            path.transformed(skew),
            Some(Geometry::Path {
                subpaths: vec![Subpath {
                    start: [0.0, 0.0],
                    segments: vec![
                        Segment::Line([2.0, 0.0]),
                        Segment::Cubic([3.0, 4.0], [1.0, 4.0], [0.0, 0.0]),
                    ],
                    closed: true,
                }],
                rule: FillRule::EvenOdd,
            })
        );
        // A map that collapses everything onto a line keeps nothing.
        assert_eq!(path.transformed(scaling(1.0, 0.0)), None);
    }

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
        let open = Geometry::Path {
            subpaths: vec![Subpath {
                start: [0.0, 0.0],
                segments: vec![Segment::Line([10.0, 0.0]), Segment::Line([10.0, 10.0])],
                closed: false,
            }],
            rule: FillRule::NonZero,
        };
        assert!(open.is_fillable() && !open.is_closed());
        assert!(
            !Geometry::Line {
                from: [0.0, 0.0],
                to: [1.0, 1.0]
            }
            .is_fillable()
        );
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
