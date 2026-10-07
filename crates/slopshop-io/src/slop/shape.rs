//! Vector shapes in the manifest (schema 0.30, ADR 0041): the document's `shapes` table, each
//! `{ "name": …, "shape": … }`, the vector nodes naming theirs by index (`params.shape`).
//!
//! ```json
//! { "geometry": { "rectangle": { "rect": [l, t, r, b], "radii": [tl, tr, br, bl] } },
//!   "fill": { "solid": [r, g, b, a] },
//!   "stroke": { "paint": { "solid": [r, g, b, a] }, "width": 3, "align": "outside",
//!               "cap": "butt", "join": "miter", "miter_limit": 4, "dashes": [2, 1],
//!               "dash_offset": 0 } }
//! ```
//!
//! Other geometries: `{ "ellipse": { "center": [x, y], "radii": [rx, ry] } }`,
//! `{ "polygon": { "center": [x, y], "radius": r, "sides": n, "star": 0.5, "rotation": 90 } }`
//! (`star` absent: a polygon), `{ "line": { "from": [x, y], "to": [x, y] } }`,
//! `{ "path": { "rule": "nonzero", "subpaths": [{ "start": [x, y], "segments": [[x, y],
//! [ax, ay, bx, by, x, y]], "closed": true }] } }` (a segment of two numbers is a line, of six a
//! cubic Bézier). Coordinates are in the layer's own space; colors are linear working-space RGBA,
//! as a fill node's. No fill or no stroke: absent. A shape that is not valid is a corrupt file.

use serde_json::{Value, json};
use slopshop_core::color::LinearRgba;
use slopshop_core::shape::{
    FillRule, Geometry, Paint, Segment, Shape, ShapeStroke, StrokeAlign, StrokeCap, StrokeJoin,
    Subpath,
};

fn paint(paint: Paint) -> Value {
    let Paint::Solid(c) = paint;
    json!({ "solid": [c.r, c.g, c.b, c.a] })
}

fn geometry(geometry: &Geometry) -> Value {
    match geometry {
        Geometry::Rectangle { rect, radii } => {
            json!({ "rectangle": { "rect": rect, "radii": radii } })
        }
        Geometry::Ellipse { center, radii } => {
            json!({ "ellipse": { "center": center, "radii": radii } })
        }
        Geometry::Polygon {
            center,
            radius,
            sides,
            star,
            rotation,
        } => {
            let mut value = json!({
                "center": center, "radius": radius, "sides": sides, "rotation": rotation,
            });
            if let Some(star) = star {
                value["star"] = json!(star);
            }
            json!({ "polygon": value })
        }
        Geometry::Line { from, to } => json!({ "line": { "from": from, "to": to } }),
        Geometry::Path { subpaths, rule } => {
            let subpaths: Vec<Value> = subpaths
                .iter()
                .map(|s| {
                    let segments: Vec<Value> = s
                        .segments
                        .iter()
                        .map(|segment| match *segment {
                            Segment::Line([x, y]) => json!([x, y]),
                            Segment::Cubic([ax, ay], [bx, by], [x, y]) => {
                                json!([ax, ay, bx, by, x, y])
                            }
                        })
                        .collect();
                    json!({ "start": s.start, "segments": segments, "closed": s.closed })
                })
                .collect();
            let rule = match rule {
                FillRule::NonZero => "nonzero",
                FillRule::EvenOdd => "evenodd",
            };
            json!({ "path": { "rule": rule, "subpaths": subpaths } })
        }
    }
}

/// `shape` as the manifest writes it.
pub(super) fn to_json(shape: &Shape) -> Value {
    let mut value = json!({ "geometry": geometry(&shape.geometry) });
    if let Some(fill) = shape.fill {
        value["fill"] = paint(fill);
    }
    if let Some(s) = &shape.stroke {
        value["stroke"] = json!({
            "paint": paint(s.paint),
            "width": s.width,
            "align": match s.align {
                StrokeAlign::Inside => "inside",
                StrokeAlign::Center => "center",
                StrokeAlign::Outside => "outside",
            },
            "cap": match s.cap {
                StrokeCap::Butt => "butt",
                StrokeCap::Round => "round",
                StrokeCap::Square => "square",
            },
            "join": match s.join {
                StrokeJoin::Miter => "miter",
                StrokeJoin::Round => "round",
                StrokeJoin::Bevel => "bevel",
            },
            "miter_limit": s.miter_limit,
            "dashes": s.dashes,
            "dash_offset": s.dash_offset,
        });
    }
    value
}

fn numbers<const N: usize>(value: Option<&Value>) -> Option<[f64; N]> {
    let array = value?.as_array().filter(|a| a.len() == N)?;
    let mut out = [0.0; N];
    for (o, v) in out.iter_mut().zip(array) {
        *o = v.as_f64()?;
    }
    Some(out)
}

fn number(value: &Value, key: &str) -> Option<f64> {
    value.get(key)?.as_f64()
}

fn paint_of(value: Option<&Value>) -> Option<Paint> {
    let [r, g, b, a] = numbers::<4>(value?.get("solid"))?;
    Some(Paint::Solid(LinearRgba::new(
        r as f32, g as f32, b as f32, a as f32,
    )))
}

fn geometry_of(value: &Value) -> Option<Geometry> {
    let (kind, v) = value.as_object()?.iter().next()?;
    Some(match kind.as_str() {
        "rectangle" => Geometry::Rectangle {
            rect: numbers(v.get("rect"))?,
            radii: numbers(v.get("radii"))?,
        },
        "ellipse" => Geometry::Ellipse {
            center: numbers(v.get("center"))?,
            radii: numbers(v.get("radii"))?,
        },
        "polygon" => Geometry::Polygon {
            center: numbers(v.get("center"))?,
            radius: number(v, "radius")?,
            sides: u32::try_from(v.get("sides")?.as_u64()?).ok()?,
            star: match v.get("star") {
                None => None,
                Some(star) => Some(star.as_f64()?),
            },
            rotation: number(v, "rotation")?,
        },
        "line" => Geometry::Line {
            from: numbers(v.get("from"))?,
            to: numbers(v.get("to"))?,
        },
        "path" => Geometry::Path {
            rule: match v.get("rule")?.as_str()? {
                "nonzero" => FillRule::NonZero,
                "evenodd" => FillRule::EvenOdd,
                _ => return None,
            },
            subpaths: v
                .get("subpaths")?
                .as_array()?
                .iter()
                .map(|s| {
                    Some(Subpath {
                        start: numbers(s.get("start"))?,
                        segments: s
                            .get("segments")?
                            .as_array()?
                            .iter()
                            .map(|segment| match segment.as_array()?.len() {
                                2 => numbers(Some(segment)).map(Segment::Line),
                                6 => numbers::<6>(Some(segment)).map(|[ax, ay, bx, by, x, y]| {
                                    Segment::Cubic([ax, ay], [bx, by], [x, y])
                                }),
                                _ => None,
                            })
                            .collect::<Option<_>>()?,
                        closed: s.get("closed")?.as_bool()?,
                    })
                })
                .collect::<Option<_>>()?,
        },
        _ => return None,
    })
}

fn stroke_of(v: &Value) -> Option<ShapeStroke> {
    Some(ShapeStroke {
        paint: paint_of(v.get("paint"))?,
        width: number(v, "width")?,
        align: match v.get("align")?.as_str()? {
            "inside" => StrokeAlign::Inside,
            "center" => StrokeAlign::Center,
            "outside" => StrokeAlign::Outside,
            _ => return None,
        },
        cap: match v.get("cap")?.as_str()? {
            "butt" => StrokeCap::Butt,
            "round" => StrokeCap::Round,
            "square" => StrokeCap::Square,
            _ => return None,
        },
        join: match v.get("join")?.as_str()? {
            "miter" => StrokeJoin::Miter,
            "round" => StrokeJoin::Round,
            "bevel" => StrokeJoin::Bevel,
            _ => return None,
        },
        miter_limit: number(v, "miter_limit")?,
        dashes: v
            .get("dashes")?
            .as_array()?
            .iter()
            .map(Value::as_f64)
            .collect::<Option<_>>()?,
        dash_offset: number(v, "dash_offset")?,
    })
}

/// The shape `value` describes, `None` when it is malformed or not valid.
pub(super) fn from_json(value: &Value) -> Option<Shape> {
    let shape = Shape {
        geometry: geometry_of(value.get("geometry")?)?,
        fill: match value.get("fill") {
            None => None,
            Some(fill) => Some(paint_of(Some(fill))?),
        },
        stroke: match value.get("stroke") {
            None => None,
            Some(stroke) => Some(stroke_of(stroke)?),
        },
    };
    shape.is_valid().then_some(shape)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_geometry_round_trips_and_invalid_shapes_are_refused() {
        let red = Paint::Solid(LinearRgba::new(1.0, 0.0, 0.25, 0.5));
        let stroke = ShapeStroke {
            paint: red,
            width: 3.0,
            align: StrokeAlign::Inside,
            cap: StrokeCap::Round,
            join: StrokeJoin::Bevel,
            miter_limit: 4.0,
            dashes: vec![2.0, 1.0],
            dash_offset: 0.5,
        };
        let geometries = [
            Geometry::Rectangle {
                rect: [1.0, 2.0, 30.0, 40.0],
                radii: [1.0, 2.0, 3.0, 4.0],
            },
            Geometry::Ellipse {
                center: [5.0, 6.0],
                radii: [7.0, 8.0],
            },
            Geometry::Polygon {
                center: [0.0, 0.0],
                radius: 10.0,
                sides: 5,
                star: Some(0.5),
                rotation: 90.0,
            },
            Geometry::Polygon {
                center: [0.0, 0.0],
                radius: 10.0,
                sides: 6,
                star: None,
                rotation: 0.0,
            },
            Geometry::Line {
                from: [0.0, 0.0],
                to: [3.0, 4.0],
            },
            Geometry::Path {
                subpaths: vec![Subpath {
                    start: [0.0, 0.0],
                    segments: vec![
                        Segment::Line([10.0, 0.0]),
                        Segment::Cubic([10.0, 10.0], [0.0, 10.0], [0.0, 0.0]),
                    ],
                    closed: true,
                }],
                rule: FillRule::EvenOdd,
            },
        ];
        for geometry in geometries {
            for (fill, stroke) in [(Some(red), None), (None, Some(stroke.clone()))] {
                let shape = Shape {
                    geometry: geometry.clone(),
                    fill,
                    stroke,
                };
                assert_eq!(from_json(&to_json(&shape)).as_ref(), Some(&shape));
            }
        }
        let mut bad = to_json(&Shape {
            geometry: Geometry::Line {
                from: [0.0, 0.0],
                to: [1.0, 1.0],
            },
            fill: None,
            stroke: Some(stroke),
        });
        bad["stroke"]["width"] = json!(-1.0);
        assert_eq!(from_json(&bad), None);
        assert_eq!(from_json(&json!({ "geometry": { "blob": {} } })), None);
        assert_eq!(from_json(&json!({})), None);
    }
}
