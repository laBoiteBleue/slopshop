//! Vector shapes over the IPC (ADR 0041): what the shape tools draw and the Properties panel
//! shows. Colors are sRGB-encoded RGBA in `[0, 1]`, as fill layers' (`AddFillLayer`);
//! coordinates are in the layer's own space.

use serde::{Deserialize, Serialize};
use slopshop_core::LinearRgba;
use slopshop_core::shape::{
    FillRule, Geometry, Paint, Segment, Shape, ShapeStroke, StrokeAlign, StrokeCap, StrokeJoin,
    Subpath,
};

/// Where a shape's outline lies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum GeometryDto {
    /// `[left, top, right, bottom]`, corners rounded by `radii` (top left first, clockwise).
    Rectangle {
        rect: [f64; 4],
        radii: [f64; 4],
    },
    Ellipse {
        center: [f64; 2],
        radii: [f64; 2],
    },
    /// A star when `star` gives its inner corners' distance as a fraction of `radius`.
    Polygon {
        center: [f64; 2],
        radius: f64,
        sides: u32,
        star: Option<f64>,
        rotation: f64,
    },
    Line {
        from: [f64; 2],
        to: [f64; 2],
    },
    /// Paths of lines and cubic Béziers (the Pen's), as `.slop` writes them.
    Path {
        subpaths: Vec<SubpathDto>,
        rule: RuleDto,
    },
}

/// One connected run of a path: each segment `[x, y]` (a line to there) or
/// `[ax, ay, bx, by, x, y]` (a cubic Bézier: two control points, then the end).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubpathDto {
    pub start: [f64; 2],
    pub segments: Vec<Vec<f64>>,
    pub closed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuleDto {
    Nonzero,
    Evenodd,
}

impl SubpathDto {
    fn subpath(&self) -> Result<Subpath, String> {
        let segments = self
            .segments
            .iter()
            .map(|s| match *s.as_slice() {
                [x, y] => Ok(Segment::Line([x, y])),
                [ax, ay, bx, by, x, y] => Ok(Segment::Cubic([ax, ay], [bx, by], [x, y])),
                _ => Err("a path segment has 2 or 6 coordinates".to_owned()),
            })
            .collect::<Result<_, _>>()?;
        Ok(Subpath {
            start: self.start,
            segments,
            closed: self.closed,
        })
    }

    fn of(subpath: &Subpath) -> Self {
        Self {
            start: subpath.start,
            segments: subpath
                .segments
                .iter()
                .map(|s| match *s {
                    Segment::Line(p) => p.to_vec(),
                    Segment::Cubic(a, b, p) => vec![a[0], a[1], b[0], b[1], p[0], p[1]],
                })
                .collect(),
            closed: subpath.closed,
        }
    }
}

impl GeometryDto {
    /// `geometry` as the UI sees it.
    pub fn of(geometry: &Geometry) -> Self {
        match geometry {
            &Geometry::Rectangle { rect, radii } => GeometryDto::Rectangle { rect, radii },
            &Geometry::Ellipse { center, radii } => GeometryDto::Ellipse { center, radii },
            &Geometry::Polygon {
                center,
                radius,
                sides,
                star,
                rotation,
            } => GeometryDto::Polygon {
                center,
                radius,
                sides,
                star,
                rotation,
            },
            &Geometry::Line { from, to } => GeometryDto::Line { from, to },
            Geometry::Path { subpaths, rule } => GeometryDto::Path {
                subpaths: subpaths.iter().map(SubpathDto::of).collect(),
                rule: match rule {
                    FillRule::NonZero => RuleDto::Nonzero,
                    FillRule::EvenOdd => RuleDto::Evenodd,
                },
            },
        }
    }

    /// The geometry it describes (not validated: see [`ShapeDto::shape`]).
    pub fn geometry(&self) -> Result<Geometry, String> {
        Ok(match self {
            &GeometryDto::Rectangle { rect, radii } => Geometry::Rectangle { rect, radii },
            &GeometryDto::Ellipse { center, radii } => Geometry::Ellipse { center, radii },
            &GeometryDto::Polygon {
                center,
                radius,
                sides,
                star,
                rotation,
            } => Geometry::Polygon {
                center,
                radius,
                sides,
                star,
                rotation,
            },
            &GeometryDto::Line { from, to } => Geometry::Line { from, to },
            GeometryDto::Path { subpaths, rule } => Geometry::Path {
                subpaths: subpaths
                    .iter()
                    .map(SubpathDto::subpath)
                    .collect::<Result<_, _>>()?,
                rule: match rule {
                    RuleDto::Nonzero => FillRule::NonZero,
                    RuleDto::Evenodd => FillRule::EvenOdd,
                },
            },
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AlignDto {
    Inside,
    Center,
    Outside,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CapDto {
    Butt,
    Round,
    Square,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JoinDto {
    Miter,
    Round,
    Bevel,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StrokeDto {
    pub color: [f32; 4],
    /// Pixels.
    pub width: f64,
    pub align: AlignDto,
    pub cap: CapDto,
    pub join: JoinDto,
    /// Dashes and gaps in turn, in stroke widths; empty: a solid stroke.
    #[serde(default)]
    pub dashes: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShapeDto {
    pub geometry: GeometryDto,
    /// None: no fill.
    pub fill: Option<[f32; 4]>,
    /// None: no stroke.
    pub stroke: Option<StrokeDto>,
}

/// Miters longer than this many stroke widths are beveled (Photoshop's default).
const MITER_LIMIT: f64 = 4.0;

fn paint([r, g, b, a]: [f32; 4]) -> Paint {
    Paint::Solid(LinearRgba::from_srgb_encoded_to_working(r, g, b, a))
}

fn color(paint: Paint) -> [f32; 4] {
    let Paint::Solid(c) = paint;
    c.working_to_srgb_encoded()
}

impl ShapeDto {
    /// The shape it describes, validated.
    pub fn shape(&self) -> Result<Shape, String> {
        let geometry = self.geometry.geometry()?;
        let shape = Shape {
            geometry,
            fill: self.fill.map(paint),
            stroke: self.stroke.as_ref().map(|s| ShapeStroke {
                paint: paint(s.color),
                width: s.width,
                align: match s.align {
                    AlignDto::Inside => StrokeAlign::Inside,
                    AlignDto::Center => StrokeAlign::Center,
                    AlignDto::Outside => StrokeAlign::Outside,
                },
                cap: match s.cap {
                    CapDto::Butt => StrokeCap::Butt,
                    CapDto::Round => StrokeCap::Round,
                    CapDto::Square => StrokeCap::Square,
                },
                join: match s.join {
                    JoinDto::Miter => StrokeJoin::Miter,
                    JoinDto::Round => StrokeJoin::Round,
                    JoinDto::Bevel => StrokeJoin::Bevel,
                },
                miter_limit: MITER_LIMIT,
                dashes: s.dashes.clone(),
                dash_offset: 0.0,
            }),
        };
        if shape.is_valid() {
            Ok(shape)
        } else {
            Err("invalid shape".to_owned())
        }
    }

    /// `shape` as the UI sees it.
    pub fn of(shape: &Shape) -> Self {
        let geometry = GeometryDto::of(&shape.geometry);
        Self {
            geometry,
            fill: shape.fill.map(color),
            stroke: shape.stroke.as_ref().map(|s| StrokeDto {
                color: color(s.paint),
                width: s.width,
                align: match s.align {
                    StrokeAlign::Inside => AlignDto::Inside,
                    StrokeAlign::Center => AlignDto::Center,
                    StrokeAlign::Outside => AlignDto::Outside,
                },
                cap: match s.cap {
                    StrokeCap::Butt => CapDto::Butt,
                    StrokeCap::Round => CapDto::Round,
                    StrokeCap::Square => CapDto::Square,
                },
                join: match s.join {
                    StrokeJoin::Miter => JoinDto::Miter,
                    StrokeJoin::Round => JoinDto::Round,
                    StrokeJoin::Bevel => JoinDto::Bevel,
                },
                dashes: s.dashes.clone(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_cross_the_ipc_both_ways_and_invalid_ones_are_refused() {
        let json = r#"{
            "geometry": { "kind": "polygon", "center": [50, 40], "radius": 30, "sides": 5,
                          "star": 0.5, "rotation": 90 },
            "fill": [1, 0.5, 0, 1],
            "stroke": { "color": [0, 0, 0, 1], "width": 3, "align": "outside",
                        "cap": "round", "join": "bevel" }
        }"#;
        let dto: ShapeDto = serde_json::from_str(json).unwrap();
        let shape = dto.shape().unwrap();
        assert!(matches!(
            shape.geometry,
            Geometry::Polygon {
                sides: 5,
                star: Some(_),
                ..
            }
        ));
        let stroke = shape.stroke.as_ref().unwrap();
        assert_eq!(
            (stroke.align, stroke.cap, stroke.join),
            (StrokeAlign::Outside, StrokeCap::Round, StrokeJoin::Bevel)
        );
        // Back to the UI: the same, colors within rounding.
        let back = ShapeDto::of(&shape);
        assert_eq!(back.geometry, dto.geometry);
        for (a, b) in back.fill.unwrap().iter().zip(dto.fill.unwrap()) {
            assert!((a - b).abs() < 1e-5, "{a} vs {b}");
        }
        let line = ShapeDto {
            geometry: GeometryDto::Line {
                from: [0.0, 0.0],
                to: [10.0, 0.0],
            },
            fill: None,
            stroke: Some(StrokeDto {
                width: 0.0,
                ..dto.stroke.clone().unwrap()
            }),
        };
        assert!(line.shape().is_err());
        // The Pen's paths, both ways; a segment of 4 coordinates is refused.
        let json = r#"{
            "geometry": { "kind": "path", "rule": "nonzero", "subpaths": [
                { "start": [0, 0], "segments": [[10, 0], [10, 5, 5, 10, 0, 10]], "closed": false }
            ] },
            "fill": [1, 0, 0, 1],
            "stroke": null
        }"#;
        let dto: ShapeDto = serde_json::from_str(json).unwrap();
        let shape = dto.shape().unwrap();
        let Geometry::Path { subpaths, rule } = &shape.geometry else {
            panic!("a path: {:?}", shape.geometry);
        };
        assert_eq!(*rule, FillRule::NonZero);
        assert_eq!(
            subpaths[0].segments,
            vec![
                Segment::Line([10.0, 0.0]),
                Segment::Cubic([10.0, 5.0], [5.0, 10.0], [0.0, 10.0])
            ]
        );
        assert!(!subpaths[0].closed);
        assert_eq!(ShapeDto::of(&shape).geometry, dto.geometry);
        let wrong = json.replace("[10, 0],", "[10, 0, 1, 1],");
        let dto: ShapeDto = serde_json::from_str(&wrong).unwrap();
        assert!(dto.shape().is_err());
    }
}
