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
//!
//! A text source (schema 0.34) is a shape entry with a `text` besides its `shape`, which then
//! holds its glyph outlines (a path), so that a reader without the fonts, or an older one, still
//! draws it:
//!
//! ```json
//! { "content": "Hello\nworld", "family": "Inter Variable", "weight": 400, "italic": false,
//!   "size": 50, "color": [r, g, b, a], "tracking": 0, "leading": 60, "align": "left",
//!   "position": [x, y], "frame": [width, height],
//!   "spans": [{ "start": 0, "end": 5, "weight": 700, "color": [r, g, b, a], "leading": null }] }
//! ```
//!
//! `size` and `leading` in pixels (`leading` absent: automatic), `tracking` in thousandths of an
//! em, `align` `left`, `center` or `right`, `position` the anchor (the first baseline at the
//! alignment point) in the layer's own space; paragraph text (schema 0.38) has a `frame`, its
//! box's size, and `position` is then the box's top left corner. `spans` (schema 0.41) set
//! characters otherwise: `start` and `end` in UTF-8 bytes of `content` (on character
//! boundaries, sorted, apart), and what they set (`family`, `weight`, `italic`, `size`,
//! `color`, `tracking`, `leading`, null for automatic leading); absent: the text's. A text in
//! several colors also has `paints` beside its `shape`: its glyphs by color.
//!
//! Character styles (schema 0.44), on a text and on its spans: `faux_bold`, `faux_italic`,
//! `underline`, `strikethrough` (booleans), `ligatures` (the standard ligatures, `true` when
//! absent from a text), `case` (`normal`, `all_caps`, `small_caps`) and `baseline` (`normal`,
//! `superscript`, `subscript`). A text writes them when they are not the defaults; a span
//! when it sets them.
//!
//! A vector mask (schema 0.40, node version 19) is a node's `params.vector_mask`, its geometry
//! written as a shape's, in the layer's own space:
//!
//! ```json
//! { "geometry": { "path": { … } }, "inverted": false, "feather": 0, "enabled": true }
//! ```
//!
//! `inverted`: the path hides what it encloses (Reveal All); `feather` in pixels (0 to 250).
//!
//! The Paths panel's paths (schema 0.45) are the document's `paths`, each
//! `{ "id": 1, "name": "Path 1", "path": { "path": { … } } }` (a `path` geometry, in document
//! pixels), the Work Path with `"work": true` and no name; `next_path_id` is their counter.

use serde_json::{Value, json};
use slopshop_core::color::LinearRgba;
use slopshop_core::shape::{
    CharStyle, FillRule, Geometry, Paint, Segment, Shape, ShapeStroke, StrokeAlign, StrokeCap,
    StrokeJoin, Subpath, Text, TextAlign, TextBaseline, TextCase, TextSpan, VectorMask,
};

/// A document path's geometry (schema 0.45) as the manifest writes it: a shape's geometry.
pub(super) fn path_to_json(geometry: &Geometry) -> Value {
    self::geometry(geometry)
}

/// A document path's geometry read back; `None` when it is not a valid path.
pub(super) fn path_from_json(value: &Value) -> Option<Geometry> {
    geometry_of(value).filter(|g| matches!(g, Geometry::Path { .. }))
}

/// A vector mask as the manifest writes it.
pub(super) fn vector_mask_to_json(mask: &VectorMask) -> Value {
    json!({
        "geometry": geometry(mask.geometry()),
        "inverted": mask.inverted(),
        "feather": mask.feather(),
        "enabled": mask.enabled,
    })
}

/// A vector mask read back; `None` when it is not one, or not valid.
pub(super) fn vector_mask_from_json(value: &Value) -> Option<VectorMask> {
    let mut mask = VectorMask::new(
        geometry_of(value.get("geometry")?)?,
        value.get("inverted")?.as_bool()?,
    )?
    .with_feather(number(value, "feather")?)?;
    mask.enabled = value.get("enabled")?.as_bool()?;
    Some(mask)
}

/// A text source's description.
pub(super) fn text_to_json(text: &Text) -> Value {
    let c = text.color;
    let mut value = json!({
        "content": text.content,
        "family": text.family,
        "weight": text.weight,
        "italic": text.italic,
        "size": text.size,
        "color": [c.r, c.g, c.b, c.a],
        "tracking": text.tracking,
        "align": match text.align {
            TextAlign::Left => "left",
            TextAlign::Center => "center",
            TextAlign::Right => "right",
        },
        "position": text.position,
    });
    if let Some(leading) = text.leading {
        value["leading"] = json!(leading);
    }
    if let Some(frame) = text.frame {
        value["frame"] = json!(frame);
    }
    if !text.spans.is_empty() {
        value["spans"] = text.spans.iter().map(span_to_json).collect();
    }
    let flags = [
        ("faux_bold", text.faux_bold, false),
        ("faux_italic", text.faux_italic, false),
        ("underline", text.underline, false),
        ("strikethrough", text.strikethrough, false),
        ("ligatures", text.ligatures, true),
    ];
    for (key, set, default) in flags {
        if set != default {
            value[key] = json!(set);
        }
    }
    if text.case != TextCase::Normal {
        value["case"] = json!(case_name(text.case));
    }
    if text.baseline != TextBaseline::Normal {
        value["baseline"] = json!(baseline_name(text.baseline));
    }
    value
}

fn case_name(case: TextCase) -> &'static str {
    match case {
        TextCase::Normal => "normal",
        TextCase::AllCaps => "all_caps",
        TextCase::SmallCaps => "small_caps",
    }
}

fn case_of(value: &Value) -> Option<TextCase> {
    match value.as_str()? {
        "normal" => Some(TextCase::Normal),
        "all_caps" => Some(TextCase::AllCaps),
        "small_caps" => Some(TextCase::SmallCaps),
        _ => None,
    }
}

fn baseline_name(baseline: TextBaseline) -> &'static str {
    match baseline {
        TextBaseline::Normal => "normal",
        TextBaseline::Superscript => "superscript",
        TextBaseline::Subscript => "subscript",
    }
}

fn baseline_of(value: &Value) -> Option<TextBaseline> {
    match value.as_str()? {
        "normal" => Some(TextBaseline::Normal),
        "superscript" => Some(TextBaseline::Superscript),
        "subscript" => Some(TextBaseline::Subscript),
        _ => None,
    }
}

/// An optional field: `Some(None)` when absent, `None` when it cannot be read by `read`.
fn optional<T>(value: &Value, key: &str, read: impl Fn(&Value) -> Option<T>) -> Option<Option<T>> {
    match value.get(key) {
        None => Some(None),
        Some(v) => read(v).map(Some),
    }
}

fn span_to_json(span: &TextSpan) -> Value {
    let s = &span.style;
    let mut value = json!({ "start": span.start, "end": span.end });
    if let Some(family) = &s.family {
        value["family"] = json!(family);
    }
    if let Some(weight) = s.weight {
        value["weight"] = json!(weight);
    }
    if let Some(italic) = s.italic {
        value["italic"] = json!(italic);
    }
    if let Some(size) = s.size {
        value["size"] = json!(size);
    }
    if let Some(c) = s.color {
        value["color"] = json!([c.r, c.g, c.b, c.a]);
    }
    if let Some(tracking) = s.tracking {
        value["tracking"] = json!(tracking);
    }
    if let Some(leading) = s.leading {
        value["leading"] = json!(leading);
    }
    let flags = [
        ("faux_bold", s.faux_bold),
        ("faux_italic", s.faux_italic),
        ("underline", s.underline),
        ("strikethrough", s.strikethrough),
        ("ligatures", s.ligatures),
    ];
    for (key, set) in flags {
        if let Some(set) = set {
            value[key] = json!(set);
        }
    }
    if let Some(case) = s.case {
        value["case"] = json!(case_name(case));
    }
    if let Some(baseline) = s.baseline {
        value["baseline"] = json!(baseline_name(baseline));
    }
    value
}

/// A span, `None` when it is not one (its validity is the text's to check).
fn span_from_json(value: &Value) -> Option<TextSpan> {
    let index = |key| usize::try_from(value.get(key)?.as_u64()?).ok();
    let style = CharStyle {
        family: match value.get("family") {
            None => None,
            Some(v) => Some(v.as_str()?.to_owned()),
        },
        weight: match value.get("weight") {
            None => None,
            Some(v) => Some(v.as_f64()? as f32),
        },
        italic: match value.get("italic") {
            None => None,
            Some(v) => Some(v.as_bool()?),
        },
        size: match value.get("size") {
            None => None,
            Some(v) => Some(v.as_f64()?),
        },
        color: match value.get("color") {
            None => None,
            Some(v) => {
                let [r, g, b, a] = numbers::<4>(Some(v))?;
                Some(LinearRgba::new(r as f32, g as f32, b as f32, a as f32))
            }
        },
        tracking: match value.get("tracking") {
            None => None,
            Some(v) => Some(v.as_f64()?),
        },
        leading: match value.get("leading") {
            None => None,
            Some(Value::Null) => Some(None),
            Some(v) => Some(Some(v.as_f64()?)),
        },
        faux_bold: optional(value, "faux_bold", Value::as_bool)?,
        faux_italic: optional(value, "faux_italic", Value::as_bool)?,
        case: optional(value, "case", case_of)?,
        baseline: optional(value, "baseline", baseline_of)?,
        underline: optional(value, "underline", Value::as_bool)?,
        strikethrough: optional(value, "strikethrough", Value::as_bool)?,
        ligatures: optional(value, "ligatures", Value::as_bool)?,
    };
    Some(TextSpan {
        start: index("start")?,
        end: index("end")?,
        style,
    })
}

/// A text source's description, `None` when it is not one or not valid.
pub(super) fn text_from_json(value: &Value) -> Option<Text> {
    let [r, g, b, a] = numbers::<4>(value.get("color"))?;
    let text = Text {
        content: value.get("content")?.as_str()?.to_owned(),
        family: value.get("family")?.as_str()?.to_owned(),
        weight: number(value, "weight")? as f32,
        italic: value.get("italic")?.as_bool()?,
        size: number(value, "size")?,
        color: LinearRgba::new(r as f32, g as f32, b as f32, a as f32),
        tracking: number(value, "tracking")?,
        leading: match value.get("leading") {
            None => None,
            Some(v) => Some(v.as_f64()?),
        },
        faux_bold: optional(value, "faux_bold", Value::as_bool)?.unwrap_or(false),
        faux_italic: optional(value, "faux_italic", Value::as_bool)?.unwrap_or(false),
        case: optional(value, "case", case_of)?.unwrap_or_default(),
        baseline: optional(value, "baseline", baseline_of)?.unwrap_or_default(),
        underline: optional(value, "underline", Value::as_bool)?.unwrap_or(false),
        strikethrough: optional(value, "strikethrough", Value::as_bool)?.unwrap_or(false),
        ligatures: optional(value, "ligatures", Value::as_bool)?.unwrap_or(true),
        align: match value.get("align")?.as_str()? {
            "left" => TextAlign::Left,
            "center" => TextAlign::Center,
            "right" => TextAlign::Right,
            _ => return None,
        },
        position: numbers::<2>(value.get("position"))?,
        frame: match value.get("frame") {
            None => None,
            Some(v) => Some(numbers::<2>(Some(v))?),
        },
        spans: match value.get("spans") {
            None => Vec::new(),
            Some(v) => v
                .as_array()?
                .iter()
                .map(span_from_json)
                .collect::<Option<_>>()?,
        },
    };
    text.is_valid().then_some(text)
}

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
