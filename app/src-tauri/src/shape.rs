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
    /// Paths of lines and cubic Béziers (the Pen's); `evenOdd` where they cross themselves,
    /// else nonzero.
    #[serde(rename_all = "camelCase")]
    Path {
        subpaths: Vec<SubpathDto>,
        #[serde(default)]
        even_odd: bool,
    },
}

/// One connected run of a path: from `start`, its segments in turn, back to `start` when
/// `closed`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubpathDto {
    pub start: [f64; 2],
    pub segments: Vec<SegmentDto>,
    pub closed: bool,
}

/// A straight segment to `to`, or a cubic Bézier through its two control points.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SegmentDto {
    Line {
        to: [f64; 2],
    },
    Cubic {
        c1: [f64; 2],
        c2: [f64; 2],
        to: [f64; 2],
    },
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
        let geometry = match self.geometry {
            GeometryDto::Rectangle { rect, radii } => Geometry::Rectangle { rect, radii },
            GeometryDto::Ellipse { center, radii } => Geometry::Ellipse { center, radii },
            GeometryDto::Polygon {
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
            GeometryDto::Line { from, to } => Geometry::Line { from, to },
            GeometryDto::Path {
                ref subpaths,
                even_odd,
            } => Geometry::Path {
                subpaths: subpaths
                    .iter()
                    .map(|s| Subpath {
                        start: s.start,
                        segments: s
                            .segments
                            .iter()
                            .map(|segment| match *segment {
                                SegmentDto::Line { to } => Segment::Line(to),
                                SegmentDto::Cubic { c1, c2, to } => Segment::Cubic(c1, c2, to),
                            })
                            .collect(),
                        closed: s.closed,
                    })
                    .collect(),
                rule: if even_odd {
                    FillRule::EvenOdd
                } else {
                    FillRule::NonZero
                },
            },
        };
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
    pub fn of(shape: &Shape) -> Option<Self> {
        let geometry = match shape.geometry {
            Geometry::Rectangle { rect, radii } => GeometryDto::Rectangle { rect, radii },
            Geometry::Ellipse { center, radii } => GeometryDto::Ellipse { center, radii },
            Geometry::Polygon {
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
            Geometry::Line { from, to } => GeometryDto::Line { from, to },
            Geometry::Path { ref subpaths, rule } => GeometryDto::Path {
                subpaths: subpaths
                    .iter()
                    .map(|s| SubpathDto {
                        start: s.start,
                        segments: s
                            .segments
                            .iter()
                            .map(|segment| match *segment {
                                Segment::Line(to) => SegmentDto::Line { to },
                                Segment::Cubic(c1, c2, to) => SegmentDto::Cubic { c1, c2, to },
                            })
                            .collect(),
                        closed: s.closed,
                    })
                    .collect(),
                even_odd: rule == FillRule::EvenOdd,
            },
        };
        Some(Self {
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
        })
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
        let back = ShapeDto::of(&shape).unwrap();
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
    }

    #[test]
    fn the_pens_paths_cross_the_ipc_both_ways() {
        let json = r#"{
            "geometry": { "kind": "path", "evenOdd": true, "subpaths": [
                { "start": [10, 10], "closed": true, "segments": [
                    { "kind": "line", "to": [90, 10] },
                    { "kind": "cubic", "c1": [120, 40], "c2": [60, 90], "to": [10, 80] }
                ] }
            ] },
            "fill": [0, 0, 1, 1],
            "stroke": null
        }"#;
        let dto: ShapeDto = serde_json::from_str(json).unwrap();
        let shape = dto.shape().unwrap();
        let Geometry::Path { subpaths, rule } = &shape.geometry else {
            panic!("a path");
        };
        assert_eq!(rule, &FillRule::EvenOdd);
        assert_eq!(
            subpaths[0].segments,
            vec![
                Segment::Line([90.0, 10.0]),
                Segment::Cubic([120.0, 40.0], [60.0, 90.0], [10.0, 80.0]),
            ]
        );
        assert!(subpaths[0].closed);
        // Back to the UI, the same; nonzero by default.
        assert_eq!(ShapeDto::of(&shape).unwrap().geometry, dto.geometry);
        let open: ShapeDto = serde_json::from_str(
            r#"{ "geometry": { "kind": "path", "subpaths": [{ "start": [0, 0], "closed": false,
                 "segments": [{ "kind": "line", "to": [5, 5] }] }] }, "fill": [0, 0, 0, 1] }"#,
        )
        .unwrap();
        assert!(matches!(
            open.shape().unwrap().geometry,
            Geometry::Path {
                rule: FillRule::NonZero,
                ..
            }
        ));
        // A point out of range is refused.
        let far: ShapeDto = serde_json::from_str(
            r#"{ "geometry": { "kind": "path", "subpaths": [{ "start": [0, 0], "closed": false,
                 "segments": [{ "kind": "line", "to": [1e9, 5] }] }] }, "fill": [0, 0, 0, 1] }"#,
        )
        .unwrap();
        assert!(far.shape().is_err());
    }
}
