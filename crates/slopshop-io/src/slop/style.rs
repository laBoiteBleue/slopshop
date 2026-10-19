//! A layer's style in the manifest (node version 8, schema 0.17, ADR 0032): `params.style`.
//!
//! ```json
//! { "fill_opacity": 1,
//!   "drop_shadow": { "enabled": true, "color": [r, g, b], "mode": "multiply", "opacity": 0.75,
//!                    "angle": 120, "distance": 5, "spread": 0, "size": 5 },
//!   "outer_glow": { "enabled": true, "color": [r, g, b], "mode": "screen", "opacity": 0.75,
//!                   "spread": 0, "size": 5 },
//!   "inner_shadow": { …as drop_shadow… }, "inner_glow": { …as outer_glow… },
//!   "color_overlay": { "enabled": true, "color": [r, g, b], "mode": "normal", "opacity": 1 },
//!   "pattern_overlay": { "enabled": true, "source": 0, "scale": 1, "angle": 0, "link": true,
//!                        "mode": "normal", "opacity": 1 },
//!   "gradient_overlay": { "enabled": true, "gradient": [[0, 0, 0, 0], [4096, 255, 255, 255]],
//!                         "reverse": false, "shape": "linear", "angle": 90, "scale": 100,
//!                         "align": true, "mode": "normal", "opacity": 1 },
//!   "satin": { "enabled": true, "color": [r, g, b], "mode": "multiply", "opacity": 0.5,
//!              "angle": 19, "distance": 11, "size": 14, "invert": true },
//!   "bevel": { "enabled": true, "style": "innerBevel", "depth": 100, "up": true, "size": 5,
//!              "soften": 0, "angle": 120, "altitude": 30,
//!              "highlight": { "color": [r, g, b], "mode": "screen", "opacity": 0.75 },
//!              "shadow": { "color": [r, g, b], "mode": "multiply", "opacity": 0.75 } },
//!   "stroke": { "enabled": true, "size": 3, "position": "outside", "color": [r, g, b],
//!               "mode": "normal", "opacity": 1 } }
//! ```
//!
//! Colors are linear working-space RGB, as a fill node's; a gradient's stops are Gradient
//! Map's (`[location 0–4096, r, g, b]`, sRGB-encoded). Effects not added are absent; a node
//! with a Gradient Overlay is written at node version 12 (schema 0.27), one with a Satin at 13
//! (schema 0.28), one with a Bevel and Emboss at 14 (schema 0.29). From schema 0.36,
//! `"effects_visible": false` hides every effect (Photoshop's eye on the "Effects" row), each
//! keeping its own `enabled`; absent, they show (an older reader shows them, losing nothing).
//! From schema 0.37, `"use_global_light": true` in a shadow or a bevel: its angle (and a
//! bevel's altitude) follow the document's `global_light`; absent, its own.
//!
//! From schema 0.39, contours: `"contour": [[x, y, corner], …]` (points on 0–255, `corner` a
//! boolean, inputs increasing) and `"anti_aliased": true` in a shadow, a glow or a Satin, with a
//! glow's `"range"` (percent); a bevel's `"gloss_contour"` and `"gloss_anti_aliased"`, and its
//! Contour element `"contour": { "enabled": true, "curve": [[x, y, corner], …],
//! "anti_aliased": false, "range": 50 }`. Each is written when it is not what its absence reads:
//! a linear contour, not anti-aliased, a glow's range at 100, a bevel's Contour off (linear,
//! 50 %), as effects were drawn before (Photoshop's Satin, Gaussian, is written).
//!
//! From schema 0.42, a Gradient Overlay's `shape` may also be `angle`, `reflected` or
//! `diamond`; a shape this version does not know comes from a newer SlopShop.

use serde_json::{Map, Value, json};
use slopshop_core::blend::BlendMode;
use slopshop_core::color::LinearRgba;
use slopshop_core::contour::{Contour, ContourPoint};
use slopshop_core::gradient::{Gradient, GradientShape, GradientStop};
use slopshop_core::selection::StrokeLocation;
use slopshop_core::style::{
    BevelEmboss, BevelStyle, ColorOverlay, DropShadow, Glow, GradientOverlay, LayerStyle, Satin,
    Stroke,
};

fn color(c: LinearRgba) -> Value {
    json!([c.r, c.g, c.b])
}

fn contour(c: &Contour) -> Value {
    Value::Array(
        c.points()
            .iter()
            .map(|p| json!([p.x, p.y, p.corner]))
            .collect(),
    )
}

/// A contour and whether it is anti-aliased into `value`, when they are not what absence reads
/// (linear, not anti-aliased).
fn insert_contour(value: &mut Value, c: &Contour, anti_aliased: bool) {
    if *c != Contour::LINEAR {
        value["contour"] = contour(c);
    }
    if anti_aliased {
        value["anti_aliased"] = json!(true);
    }
}

fn contour_of(value: &Value) -> Option<Contour> {
    let points = value
        .as_array()?
        .iter()
        .map(|p| {
            let p = p.as_array().filter(|p| p.len() == 3)?;
            let byte = |v: &Value| v.as_u64().and_then(|v| u8::try_from(v).ok());
            Some(ContourPoint {
                x: byte(&p[0])?,
                y: byte(&p[1])?,
                corner: p[2].as_bool()?,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Contour::new(&points)
}

fn position_id(position: StrokeLocation) -> &'static str {
    match position {
        StrokeLocation::Inside => "inside",
        StrokeLocation::Center => "center",
        StrokeLocation::Outside => "outside",
    }
}

fn bevel_style_id(style: BevelStyle) -> &'static str {
    match style {
        BevelStyle::InnerBevel => "innerBevel",
        BevelStyle::OuterBevel => "outerBevel",
        BevelStyle::Emboss => "emboss",
        BevelStyle::PillowEmboss => "pillowEmboss",
    }
}

/// `style` as `params.style`; `source_index` gives a Pattern Overlay's source its index in
/// the document's sources.
pub(super) fn to_json(
    style: &LayerStyle,
    source_index: &dyn Fn(&slopshop_core::Source) -> Option<usize>,
) -> Value {
    let mut value = Map::new();
    value.insert("fill_opacity".into(), json!(style.fill_opacity));
    if !style.effects_visible {
        value.insert("effects_visible".into(), json!(false));
    }
    let shadow = |s: DropShadow| {
        let mut value = json!({
            "enabled": s.enabled,
            "color": color(s.color),
            "mode": s.mode.id(),
            "opacity": s.opacity,
            "angle": s.angle,
            "distance": s.distance,
            "spread": s.spread,
            "size": s.size,
        });
        if s.use_global_light {
            value["use_global_light"] = json!(true);
        }
        insert_contour(&mut value, &s.contour, s.anti_aliased);
        value
    };
    let glow = |g: Glow| {
        let mut value = json!({
            "enabled": g.enabled,
            "color": color(g.color),
            "mode": g.mode.id(),
            "opacity": g.opacity,
            "spread": g.spread,
            "size": g.size,
        });
        insert_contour(&mut value, &g.contour, g.anti_aliased);
        if g.range != 100.0 {
            value["range"] = json!(g.range);
        }
        value
    };
    if let Some(s) = style.drop_shadow {
        value.insert("drop_shadow".into(), shadow(s));
    }
    if let Some(g) = style.outer_glow {
        value.insert("outer_glow".into(), glow(g));
    }
    if let Some(s) = style.inner_shadow {
        value.insert("inner_shadow".into(), shadow(s));
    }
    if let Some(g) = style.inner_glow {
        value.insert("inner_glow".into(), glow(g));
    }
    if let Some(o) = style.color_overlay {
        value.insert(
            "color_overlay".into(),
            json!({
                "enabled": o.enabled,
                "color": color(o.color),
                "mode": o.mode.id(),
                "opacity": o.opacity,
            }),
        );
    }
    if let Some(o) = &style.pattern_overlay {
        value.insert(
            "pattern_overlay".into(),
            json!({
                "enabled": o.enabled,
                "source": source_index(&o.pattern.source),
                "scale": o.pattern.scale,
                "angle": o.pattern.angle,
                "link": o.link_with_layer,
                "mode": o.mode.id(),
                "opacity": o.opacity,
            }),
        );
    }
    if let Some(o) = style.gradient_overlay {
        let stops: Vec<Value> = o
            .gradient
            .stops()
            .iter()
            .map(|s| json!([s.location, s.color[0], s.color[1], s.color[2]]))
            .collect();
        value.insert(
            "gradient_overlay".into(),
            json!({
                "enabled": o.enabled,
                "gradient": stops,
                "reverse": o.reverse,
                "shape": o.shape.id(),
                "angle": o.angle,
                "scale": o.scale,
                "align": o.align_with_layer,
                "mode": o.mode.id(),
                "opacity": o.opacity,
            }),
        );
    }
    if let Some(s) = style.satin {
        let mut satin = json!({
            "enabled": s.enabled,
            "color": color(s.color),
            "mode": s.mode.id(),
            "opacity": s.opacity,
            "angle": s.angle,
            "distance": s.distance,
            "size": s.size,
            "invert": s.invert,
        });
        insert_contour(&mut satin, &s.contour, s.anti_aliased);
        value.insert("satin".into(), satin);
    }
    if let Some(b) = style.bevel {
        let mut bevel = json!({
                "enabled": b.enabled,
                "style": bevel_style_id(b.style),
                "depth": b.depth,
                "up": b.up,
                "size": b.size,
                "soften": b.soften,
                "angle": b.angle,
                "altitude": b.altitude,
                "highlight": {
                    "color": color(b.highlight_color),
                    "mode": b.highlight_mode.id(),
                    "opacity": b.highlight_opacity,
                },
                "shadow": {
                    "color": color(b.shadow_color),
                    "mode": b.shadow_mode.id(),
                    "opacity": b.shadow_opacity,
                },
        });
        if b.use_global_light {
            bevel["use_global_light"] = json!(true);
        }
        if b.gloss_contour != Contour::LINEAR {
            bevel["gloss_contour"] = contour(&b.gloss_contour);
        }
        if b.gloss_anti_aliased {
            bevel["gloss_anti_aliased"] = json!(true);
        }
        let plain = BevelEmboss::default();
        if b.contour_enabled
            || b.contour != plain.contour
            || b.contour_anti_aliased
            || b.contour_range != plain.contour_range
        {
            bevel["contour"] = json!({
                "enabled": b.contour_enabled,
                "curve": contour(&b.contour),
                "anti_aliased": b.contour_anti_aliased,
                "range": b.contour_range,
            });
        }
        value.insert("bevel".into(), bevel);
    }
    if let Some(s) = style.stroke {
        value.insert(
            "stroke".into(),
            json!({
                "enabled": s.enabled,
                "size": s.size,
                "position": position_id(s.position),
                "color": color(s.color),
                "mode": s.mode.id(),
                "opacity": s.opacity,
            }),
        );
    }
    Value::Object(value)
}

/// What an effect's fields read as.
struct Fields<'a>(&'a Map<String, Value>);

impl Fields<'_> {
    fn number(&self, key: &str) -> Option<f64> {
        self.0.get(key)?.as_f64()
    }

    fn opacity(&self) -> Option<f32> {
        self.number("opacity").map(|v| v as f32)
    }

    fn enabled(&self) -> Option<bool> {
        self.0.get("enabled")?.as_bool()
    }

    /// Whether it uses the global light: absent, not (schema 0.37).
    fn global_light(&self) -> Option<bool> {
        match self.0.get("use_global_light") {
            None => Some(false),
            Some(value) => value.as_bool(),
        }
    }

    fn mode(&self) -> Option<BlendMode> {
        BlendMode::from_id(self.0.get("mode")?.as_str()?)
    }

    /// A boolean: absent, false (schema 0.39).
    fn flag(&self, key: &str) -> Option<bool> {
        match self.0.get(key) {
            None => Some(false),
            Some(value) => value.as_bool(),
        }
    }

    /// A contour: absent, linear (schema 0.39).
    fn contour(&self, key: &str) -> Option<Contour> {
        match self.0.get(key) {
            None => Some(Contour::LINEAR),
            Some(value) => contour_of(value),
        }
    }

    fn color(&self) -> Option<LinearRgba> {
        let c = self.0.get("color")?.as_array()?;
        let [r, g, b] = [c.first()?, c.get(1)?, c.get(2)?].map(|v| v.as_f64());
        (c.len() == 3).then_some(LinearRgba::new(r? as f32, g? as f32, b? as f32, 1.0))
    }
}

/// An effect's fields, `None` when absent, an error when malformed.
fn effect<T>(
    style: &Map<String, Value>,
    key: &str,
    read: impl Fn(&Fields) -> Option<T>,
) -> Result<Option<T>, ()> {
    match style.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(fields)) => read(&Fields(fields)).map(Some).ok_or(()),
        Some(_) => Err(()),
    }
}

/// `Err` when `params.style` names what a newer SlopShop wrote: a Gradient Overlay's shape this
/// version does not know. What is malformed is left to [`from_json`].
pub(super) fn check_known(value: &Value) -> Result<(), super::FileError> {
    match value
        .get("gradient_overlay")
        .and_then(|o| o.get("shape"))
        .and_then(Value::as_str)
    {
        Some(id) => super::read::gradient_shape(id).map(|_| ()),
        None => Ok(()),
    }
}

/// `params.style` as a style; `None` when it is malformed or out of range. `source(i)` is the
/// document's source `i` (a Pattern Overlay's).
pub(super) fn from_json(
    value: &Value,
    source: &dyn Fn(usize) -> Option<std::sync::Arc<slopshop_core::Source>>,
) -> Option<LayerStyle> {
    let style = value.as_object()?;
    let fill_opacity = style.get("fill_opacity")?.as_f64()? as f32;
    let effects_visible = match style.get("effects_visible") {
        None => true,
        Some(value) => value.as_bool()?,
    };
    let shadow = |f: &Fields| {
        Some(DropShadow {
            enabled: f.enabled()?,
            color: f.color()?,
            mode: f.mode()?,
            opacity: f.opacity()?,
            angle: f.number("angle")?,
            use_global_light: f.global_light()?,
            distance: f.number("distance")?,
            spread: f.number("spread")?,
            size: f.number("size")?,
            contour: f.contour("contour")?,
            anti_aliased: f.flag("anti_aliased")?,
        })
    };
    let glow = |f: &Fields| {
        Some(Glow {
            enabled: f.enabled()?,
            color: f.color()?,
            mode: f.mode()?,
            opacity: f.opacity()?,
            spread: f.number("spread")?,
            size: f.number("size")?,
            contour: f.contour("contour")?,
            anti_aliased: f.flag("anti_aliased")?,
            // Absent: the whole fade, as glows were drawn before.
            range: match f.0.get("range") {
                None => 100.0,
                Some(range) => range.as_f64()?,
            },
        })
    };
    let drop_shadow = effect(style, "drop_shadow", shadow).ok()?;
    let outer_glow = effect(style, "outer_glow", glow).ok()?;
    let inner_shadow = effect(style, "inner_shadow", shadow).ok()?;
    let inner_glow = effect(style, "inner_glow", glow).ok()?;
    let color_overlay = effect(style, "color_overlay", |f| {
        Some(ColorOverlay {
            enabled: f.enabled()?,
            color: f.color()?,
            mode: f.mode()?,
            opacity: f.opacity()?,
        })
    })
    .ok()?;
    let pattern_overlay = effect(style, "pattern_overlay", |f| {
        let index = usize::try_from(f.0.get("source")?.as_u64()?).ok()?;
        Some(slopshop_core::style::PatternOverlay {
            enabled: f.enabled()?,
            pattern: slopshop_core::pattern::PatternFill {
                source: source(index)?,
                scale: f.number("scale")?,
                angle: f.number("angle")?,
            },
            link_with_layer: f.0.get("link")?.as_bool()?,
            mode: f.mode()?,
            opacity: f.opacity()?,
        })
    })
    .ok()?;
    let gradient_overlay = effect(style, "gradient_overlay", |f| {
        let stops =
            f.0.get("gradient")?
                .as_array()?
                .iter()
                .map(|s| {
                    let s = s.as_array().filter(|s| s.len() == 4)?;
                    let byte = |v: &Value| v.as_u64().and_then(|v| u8::try_from(v).ok());
                    Some(GradientStop {
                        location: s[0].as_u64().and_then(|v| u16::try_from(v).ok())?,
                        color: [byte(&s[1])?, byte(&s[2])?, byte(&s[3])?],
                    })
                })
                .collect::<Option<Vec<_>>>()?;
        Some(GradientOverlay {
            enabled: f.enabled()?,
            gradient: Gradient::new(&stops)?,
            reverse: f.0.get("reverse")?.as_bool()?,
            shape: GradientShape::from_id(f.0.get("shape")?.as_str()?)?,
            angle: f.number("angle")?,
            scale: f.number("scale")?,
            align_with_layer: f.0.get("align")?.as_bool()?,
            mode: f.mode()?,
            opacity: f.opacity()?,
        })
    })
    .ok()?;
    let satin = effect(style, "satin", |f| {
        Some(Satin {
            enabled: f.enabled()?,
            color: f.color()?,
            mode: f.mode()?,
            opacity: f.opacity()?,
            angle: f.number("angle")?,
            distance: f.number("distance")?,
            size: f.number("size")?,
            invert: f.0.get("invert")?.as_bool()?,
            contour: f.contour("contour")?,
            anti_aliased: f.flag("anti_aliased")?,
        })
    })
    .ok()?;
    let bevel = effect(style, "bevel", |f| {
        let light = |key: &str| {
            let fields = Fields(f.0.get(key)?.as_object()?);
            Some((fields.color()?, fields.mode()?, fields.opacity()?))
        };
        let (highlight_color, highlight_mode, highlight_opacity) = light("highlight")?;
        let (shadow_color, shadow_mode, shadow_opacity) = light("shadow")?;
        let style = [
            BevelStyle::InnerBevel,
            BevelStyle::OuterBevel,
            BevelStyle::Emboss,
            BevelStyle::PillowEmboss,
        ]
        .into_iter()
        .find(|s| Some(bevel_style_id(*s)) == f.0.get("style").and_then(Value::as_str))?;
        let plain = BevelEmboss::default();
        let (contour_enabled, contour, contour_anti_aliased, contour_range) =
            match f.0.get("contour") {
                None => (false, plain.contour, false, plain.contour_range),
                Some(value) => {
                    let element = Fields(value.as_object()?);
                    (
                        element.0.get("enabled")?.as_bool()?,
                        contour_of(element.0.get("curve")?)?,
                        element.0.get("anti_aliased")?.as_bool()?,
                        element.number("range")?,
                    )
                }
            };
        Some(BevelEmboss {
            enabled: f.enabled()?,
            style,
            depth: f.number("depth")?,
            up: f.0.get("up")?.as_bool()?,
            size: f.number("size")?,
            soften: f.number("soften")?,
            angle: f.number("angle")?,
            altitude: f.number("altitude")?,
            use_global_light: f.global_light()?,
            highlight_color,
            highlight_mode,
            highlight_opacity,
            shadow_color,
            shadow_mode,
            shadow_opacity,
            gloss_contour: f.contour("gloss_contour")?,
            gloss_anti_aliased: f.flag("gloss_anti_aliased")?,
            contour_enabled,
            contour,
            contour_anti_aliased,
            contour_range,
        })
    })
    .ok()?;
    let stroke = effect(style, "stroke", |f| {
        let position = match f.0.get("position")?.as_str()? {
            "inside" => StrokeLocation::Inside,
            "center" => StrokeLocation::Center,
            "outside" => StrokeLocation::Outside,
            _ => return None,
        };
        Some(Stroke {
            enabled: f.enabled()?,
            size: f.number("size")?,
            position,
            color: f.color()?,
            mode: f.mode()?,
            opacity: f.opacity()?,
        })
    })
    .ok()?;
    let style = LayerStyle {
        fill_opacity,
        drop_shadow,
        outer_glow,
        inner_shadow,
        inner_glow,
        color_overlay,
        gradient_overlay,
        pattern_overlay,
        satin,
        stroke,
        bevel,
        effects_visible,
    };
    style.is_valid().then_some(style)
}
