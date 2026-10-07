//! Layer styles in Photoshop documents (ADR 0032): the `lfx2` and `lmfx` blocks (a descriptor of
//! effects) read into a SlopShop style, and a style written back as `lfx2`.
//!
//! What SlopShop draws comes over: Drop Shadow, Inner Shadow, Outer and Inner Glow, Color
//! Overlay and Stroke, with their color, mode, opacity, angle (the global light's when they use
//! it), distance, spread or choke and size. What it does not draw (Bevel & Emboss, Satin,
//! Gradient and Pattern Overlay, gradient or precise glows, glows from the center, gradient or
//! pattern strokes, contours, noise, several effects of one kind) is reported as approximated.

use slopshop_core::blend::BlendMode;
use slopshop_core::color::LinearRgba;
use slopshop_core::gradient::{Gradient, GradientShape, GradientStop};
use slopshop_core::selection::StrokeLocation;
use slopshop_core::style::{
    BevelEmboss, BevelStyle, ColorOverlay, DropShadow, Glow, GradientOverlay, LayerStyle,
    MAX_BEVEL_DEPTH, MAX_BEVEL_SOFTEN, MAX_GRADIENT_SCALE, MAX_SATIN_DISTANCE, MAX_SIZE,
    MIN_BEVEL_DEPTH, MIN_GRADIENT_SCALE, Satin, Stroke,
};

use super::descriptor::{self, Descriptor, Value};

/// Photoshop's blend modes in descriptors (enumerated type `BlnM`).
const MODES: [(BlendMode, &[u8]); 27] = [
    (BlendMode::Normal, b"Nrml"),
    (BlendMode::Dissolve, b"Dslv"),
    (BlendMode::Darken, b"Drkn"),
    (BlendMode::Multiply, b"Mltp"),
    (BlendMode::ColorBurn, b"CBrn"),
    (BlendMode::LinearBurn, b"linearBurn"),
    (BlendMode::DarkerColor, b"darkerColor"),
    (BlendMode::Lighten, b"Lghn"),
    (BlendMode::Screen, b"Scrn"),
    (BlendMode::ColorDodge, b"CDdg"),
    (BlendMode::LinearDodge, b"linearDodge"),
    (BlendMode::LighterColor, b"lighterColor"),
    (BlendMode::Overlay, b"Ovrl"),
    (BlendMode::SoftLight, b"SftL"),
    (BlendMode::HardLight, b"HrdL"),
    (BlendMode::VividLight, b"vividLight"),
    (BlendMode::LinearLight, b"linearLight"),
    (BlendMode::PinLight, b"pinLight"),
    (BlendMode::HardMix, b"hardMix"),
    (BlendMode::Difference, b"Dfrn"),
    (BlendMode::Exclusion, b"Xclu"),
    (BlendMode::Subtract, b"blendSubtraction"),
    (BlendMode::Divide, b"blendDivide"),
    (BlendMode::Hue, b"H   "),
    (BlendMode::Saturation, b"Strt"),
    (BlendMode::Color, b"Clr "),
    (BlendMode::Luminosity, b"Lmns"),
];

/// A blend mode's identifier in descriptors.
pub(crate) fn mode_id(mode: BlendMode) -> &'static [u8] {
    MODES
        .iter()
        .find(|(m, _)| *m == mode)
        .map_or(b"Nrml".as_slice(), |(_, id)| id)
}

fn mode_of(id: &[u8]) -> Option<BlendMode> {
    MODES.iter().find(|(_, i)| *i == id).map(|(m, _)| *m)
}

/// A style read from a document: what SlopShop draws of it, and whether anything was left out.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct ImportedStyle {
    pub style: LayerStyle,
    pub approximated: bool,
}

/// What reading one effect gathers.
struct Read {
    approximated: bool,
    /// The global light's angle, for effects that use it.
    global_angle: f64,
    /// The style's scale (Layer Style > Scale Effects), applied to its sizes.
    scale: f64,
    /// The master switch: off, every effect is off.
    on: bool,
}

impl Read {
    /// `d`'s color (RGB, 0–255) as working-space color; another kind of color is approximated.
    fn color(&mut self, d: &Descriptor) -> LinearRgba {
        let rgb = d.object(b"Clr ").and_then(|c| {
            (c.class == b"RGBC").then(|| {
                [b"Rd  ", b"Grn ", b"Bl  "].map(|k| (c.number(k).unwrap_or(0.0) / 255.0) as f32)
            })
        });
        match rgb {
            Some([r, g, b]) => LinearRgba::from_srgb_encoded_to_working(
                r.clamp(0.0, 1.0),
                g.clamp(0.0, 1.0),
                b.clamp(0.0, 1.0),
                1.0,
            ),
            None => {
                self.approximated = true;
                LinearRgba::new(0.0, 0.0, 0.0, 1.0)
            }
        }
    }

    fn mode(&mut self, d: &Descriptor) -> BlendMode {
        match d.enumerated(b"Md  ").and_then(mode_of) {
            Some(mode) => mode,
            None => {
                self.approximated = true;
                BlendMode::Normal
            }
        }
    }

    fn enabled(&self, d: &Descriptor) -> bool {
        self.on && d.bool(b"enab").unwrap_or(true)
    }

    fn opacity(d: &Descriptor) -> f32 {
        (d.number(b"Opct").unwrap_or(100.0) / 100.0).clamp(0.0, 1.0) as f32
    }

    /// A size in pixels, scaled, within SlopShop's range.
    fn size(&self, d: &Descriptor, key: &[u8]) -> f64 {
        (d.number(key).unwrap_or(0.0) * self.scale).clamp(0.0, MAX_SIZE)
    }

    fn spread(d: &Descriptor) -> f64 {
        d.number(b"Ckmt").unwrap_or(0.0).clamp(0.0, 100.0)
    }

    fn noise(&mut self, d: &Descriptor) {
        if d.number(b"Nose").is_some_and(|n| n > 0.0) {
            self.approximated = true;
        }
    }

    fn shadow(&mut self, d: &Descriptor) -> DropShadow {
        self.noise(d);
        let angle = if d.bool(b"uglg").unwrap_or(false) {
            self.global_angle
        } else {
            d.number(b"lagl").unwrap_or(self.global_angle)
        };
        DropShadow {
            enabled: self.enabled(d),
            color: self.color(d),
            mode: self.mode(d),
            opacity: Self::opacity(d),
            angle,
            distance: (d.number(b"Dstn").unwrap_or(0.0) * self.scale)
                .clamp(0.0, slopshop_core::style::MAX_DISTANCE),
            spread: Self::spread(d),
            size: self.size(d, b"blur"),
        }
    }

    fn glow(&mut self, d: &Descriptor, inner: bool) -> Glow {
        self.noise(d);
        // Softer with a color is what SlopShop draws; Precise, a gradient or (inside) from the
        // center are approximated.
        let precise = d.enumerated(b"GlwT").is_some_and(|t| t != b"SfBL");
        let gradient = d.object(b"Clr ").is_none();
        let center = inner && d.enumerated(b"glwS").is_some_and(|s| s == b"SrcC");
        if precise || gradient || center {
            self.approximated = true;
        }
        Glow {
            enabled: self.enabled(d),
            color: if gradient {
                Glow::default().color
            } else {
                self.color(d)
            },
            mode: self.mode(d),
            opacity: Self::opacity(d),
            spread: Self::spread(d),
            size: self.size(d, b"blur"),
        }
    }

    fn stroke(&mut self, d: &Descriptor) -> Stroke {
        if d.enumerated(b"PntT").is_some_and(|t| t != b"SClr") {
            self.approximated = true;
        }
        let position = match d.enumerated(b"Styl") {
            Some(b"InsF") => StrokeLocation::Inside,
            Some(b"CtrF") => StrokeLocation::Center,
            _ => StrokeLocation::Outside,
        };
        Stroke {
            enabled: self.enabled(d),
            size: (d.number(b"Sz  ").unwrap_or(3.0) * self.scale)
                .clamp(1.0, slopshop_core::selection::MAX_MODIFY),
            position,
            color: self.color(d),
            mode: self.mode(d),
            opacity: Self::opacity(d),
        }
    }

    /// A Gradient Overlay (`GrFl`): its gradient's color stops (`Grad` > `Clrs`, locations on
    /// 0–4096 as ours); what SlopShop does not draw (transparency stops, midpoints off the
    /// middle, the Angle, Reflected and Diamond styles, an offset, a noise gradient) is
    /// approximated.
    fn gradient_overlay(&mut self, d: &Descriptor) -> GradientOverlay {
        let mut overlay = GradientOverlay {
            enabled: self.enabled(d),
            mode: self.mode(d),
            opacity: Self::opacity(d),
            reverse: d.bool(b"Rvrs").unwrap_or(false),
            angle: d.number(b"Angl").unwrap_or(90.0),
            scale: d
                .number(b"Scl ")
                .unwrap_or(100.0)
                .clamp(MIN_GRADIENT_SCALE, MAX_GRADIENT_SCALE),
            align_with_layer: d.bool(b"Algn").unwrap_or(true),
            ..GradientOverlay::default()
        };
        overlay.shape = match d.enumerated(b"Type") {
            Some(b"Rdl ") => GradientShape::Radial,
            Some(b"Lnr ") | None => GradientShape::Linear,
            Some(_) => {
                self.approximated = true;
                GradientShape::Linear
            }
        };
        if d.object(b"Ofst").is_some_and(|o| {
            [b"Hrzn", b"Vrtc"]
                .iter()
                .any(|k| o.number(*k).unwrap_or(0.0) != 0.0)
        }) {
            self.approximated = true;
        }
        let gradient = d.object(b"Grad");
        let stops: Option<Vec<GradientStop>> = gradient.and_then(|g| g.list(b"Clrs")).map(|list| {
            list.iter()
                .filter_map(|v| match v {
                    Value::Object(stop) => Some(stop),
                    _ => None,
                })
                .map(|stop| {
                    if stop.number(b"Mdpn").is_some_and(|m| m != 50.0) {
                        self.approximated = true;
                    }
                    let [r, g, b, _] = self.color(stop).working_to_srgb_encoded();
                    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                    GradientStop {
                        location: stop.number(b"Lctn").unwrap_or(0.0).clamp(0.0, 4096.0) as u16,
                        color: [byte(r), byte(g), byte(b)],
                    }
                })
                .collect()
        });
        let transparent = gradient.and_then(|g| g.list(b"Trns")).is_some_and(|list| {
            list.iter().any(
                |v| matches!(v, Value::Object(t) if t.number(b"Opct").is_some_and(|o| o < 100.0)),
            )
        });
        if transparent {
            self.approximated = true;
        }
        match stops.as_deref().and_then(Gradient::new) {
            Some(g) => overlay.gradient = g,
            None => self.approximated = true,
        }
        overlay
    }

    /// A Bevel and Emboss (`ebbl`): Smooth; Chisel, Stroke Emboss, a texture, a gloss contour
    /// other than linear are approximated.
    fn bevel(&mut self, d: &Descriptor) -> BevelEmboss {
        let style = match d.enumerated(b"bvlS") {
            Some(b"InrB") | None => BevelStyle::InnerBevel,
            Some(b"OtrB") => BevelStyle::OuterBevel,
            Some(b"Embs") => BevelStyle::Emboss,
            Some(b"PlEb") => BevelStyle::PillowEmboss,
            Some(_) => {
                self.approximated = true;
                BevelStyle::InnerBevel
            }
        };
        if d.enumerated(b"bvlT").is_some_and(|t| t != b"SfBL")
            || d.bool(b"useTexture").unwrap_or(false)
            || d.object(b"TrnS")
                .and_then(|c| c.list(b"Crv "))
                .is_some_and(|points| points.len() > 2)
        {
            self.approximated = true;
        }
        let angle = if d.bool(b"uglg").unwrap_or(false) {
            self.global_angle
        } else {
            d.number(b"lagl").unwrap_or(120.0)
        };
        let part = |read: &mut Self, mode: &[u8], color: &[u8], opacity: &[u8], default| {
            let mode = match d.enumerated(mode).and_then(mode_of) {
                Some(mode) => mode,
                None => default,
            };
            let rgb = d.object(color).filter(|c| c.class == b"RGBC").map(|c| {
                [b"Rd  ", b"Grn ", b"Bl  "].map(|k| (c.number(k).unwrap_or(0.0) / 255.0) as f32)
            });
            let color = match rgb {
                Some([r, g, b]) => LinearRgba::from_srgb_encoded_to_working(
                    r.clamp(0.0, 1.0),
                    g.clamp(0.0, 1.0),
                    b.clamp(0.0, 1.0),
                    1.0,
                ),
                None => {
                    read.approximated |= d.get(color).is_some();
                    LinearRgba::new(0.0, 0.0, 0.0, 1.0)
                }
            };
            let opacity = (d.number(opacity).unwrap_or(75.0) / 100.0).clamp(0.0, 1.0) as f32;
            (color, mode, opacity)
        };
        let (highlight_color, highlight_mode, highlight_opacity) =
            part(self, b"hglM", b"hglC", b"hglO", BlendMode::Screen);
        let (shadow_color, shadow_mode, shadow_opacity) =
            part(self, b"sdwM", b"sdwC", b"sdwO", BlendMode::Multiply);
        BevelEmboss {
            enabled: self.enabled(d),
            style,
            depth: d
                .number(b"srgR")
                .unwrap_or(100.0)
                .clamp(MIN_BEVEL_DEPTH, MAX_BEVEL_DEPTH),
            up: d.enumerated(b"bvlD") != Some(b"Out "),
            size: self.size(d, b"blur"),
            soften: (d.number(b"Sftn").unwrap_or(0.0) * self.scale).clamp(0.0, MAX_BEVEL_SOFTEN),
            angle,
            altitude: d.number(b"Lald").unwrap_or(30.0).clamp(0.0, 90.0),
            highlight_color,
            highlight_mode,
            highlight_opacity,
            shadow_color,
            shadow_mode,
            shadow_opacity,
        }
    }

    /// A Satin (`ChFX`); a contour other than linear is approximated.
    fn satin(&mut self, d: &Descriptor) -> Satin {
        if d.object(b"MpgS")
            .and_then(|c| c.list(b"Crv "))
            .is_some_and(|points| points.len() > 2)
        {
            self.approximated = true;
        }
        Satin {
            enabled: self.enabled(d),
            color: self.color(d),
            mode: self.mode(d),
            opacity: Self::opacity(d),
            angle: d.number(b"lagl").unwrap_or(19.0),
            distance: (d.number(b"Dstn").unwrap_or(11.0) * self.scale)
                .clamp(0.0, MAX_SATIN_DISTANCE),
            size: self.size(d, b"blur"),
            invert: d.bool(b"Invr").unwrap_or(true),
        }
    }

    fn overlay(&mut self, d: &Descriptor) -> ColorOverlay {
        ColorOverlay {
            enabled: self.enabled(d),
            color: self.color(d),
            mode: self.mode(d),
            opacity: Self::opacity(d),
        }
    }
}

/// The effect of kind `key` (or the first of `multi`, Photoshop's list when several are
/// stacked: the others are approximated away), if present.
fn effect<'a>(
    d: &'a Descriptor,
    key: &[u8],
    multi: &[u8],
    read: &mut Read,
) -> Option<&'a Descriptor> {
    let found = match d.list(multi) {
        Some(list) => {
            let objects: Vec<&Descriptor> = list
                .iter()
                .filter_map(|v| match v {
                    Value::Object(o) => Some(o),
                    _ => None,
                })
                .filter(|o| o.bool(b"present").unwrap_or(true))
                .collect();
            if objects.len() > 1 {
                read.approximated = true;
            }
            objects.first().copied()
        }
        None => d.object(key),
    };
    found.filter(|o| o.bool(b"present").unwrap_or(true))
}

/// The style of an `lfx2` or `lmfx` block (`version`, descriptor version, descriptor), with the
/// document's global light angle (degrees). `None` when the block cannot be read.
pub(crate) fn read(block: &[u8], global_angle: f64) -> Option<ImportedStyle> {
    // lfx2: object effects version (0), descriptor version (16). lmfx: the same.
    let d = descriptor::read(block.get(8..)?)?;
    let mut read = Read {
        approximated: false,
        global_angle,
        scale: d.number(b"Scl ").map_or(1.0, |s| s / 100.0),
        on: d.bool(b"masterFXSwitch").unwrap_or(true),
    };
    let mut style = LayerStyle::default();
    if let Some(e) = effect(&d, b"DrSh", b"dropShadowMulti", &mut read) {
        style.drop_shadow = Some(read.shadow(e));
    }
    if let Some(e) = effect(&d, b"IrSh", b"innerShadowMulti", &mut read) {
        style.inner_shadow = Some(read.shadow(e));
    }
    if let Some(e) = effect(&d, b"OrGl", b"outerGlowMulti", &mut read) {
        style.outer_glow = Some(read.glow(e, false));
    }
    if let Some(e) = effect(&d, b"IrGl", b"innerGlowMulti", &mut read) {
        style.inner_glow = Some(read.glow(e, true));
    }
    if let Some(e) = effect(&d, b"SoFi", b"solidFillMulti", &mut read) {
        style.color_overlay = Some(read.overlay(e));
    }
    if let Some(e) = effect(&d, b"GrFl", b"gradientFillMulti", &mut read) {
        style.gradient_overlay = Some(read.gradient_overlay(e));
    }
    if let Some(e) = d
        .object(b"ChFX")
        .filter(|o| o.bool(b"present").unwrap_or(true))
    {
        style.satin = Some(read.satin(e));
    }
    if let Some(e) = d
        .object(b"ebbl")
        .filter(|o| o.bool(b"present").unwrap_or(true))
    {
        style.bevel = Some(read.bevel(e));
    }
    if let Some(e) = effect(&d, b"FrFX", b"frameFXMulti", &mut read) {
        style.stroke = Some(read.stroke(e));
    }
    // What SlopShop does not draw yet, when enabled: Pattern Overlay.
    let pattern = match d.get(b"patternFill") {
        Some(Value::Object(o)) => o.bool(b"enab").unwrap_or(true),
        Some(Value::List(list)) => !list.is_empty(),
        _ => false,
    };
    if pattern && read.on {
        read.approximated = true;
    }
    style.is_valid().then_some(ImportedStyle {
        style,
        approximated: read.approximated,
    })
}

// --- Writing ------------------------------------------------------------------------------------

/// A descriptor item: key, type, value bytes.
type Item = (&'static [u8], &'static [u8; 4], Vec<u8>);

fn id(id: &[u8]) -> Vec<u8> {
    let mut out = (if id.len() == 4 { 0 } else { id.len() as u32 })
        .to_be_bytes()
        .to_vec();
    out.extend(id);
    out
}

/// An object's body: an empty name, its class, its items.
fn object(class: &[u8], items: &[Item]) -> Vec<u8> {
    let mut out = 1u32.to_be_bytes().to_vec();
    out.extend([0, 0]);
    out.extend(id(class));
    out.extend((items.len() as u32).to_be_bytes());
    for (key, ty, value) in items {
        out.extend(id(key));
        out.extend(*ty);
        out.extend(value);
    }
    out
}

fn unit(unit: &[u8; 4], v: f64) -> Vec<u8> {
    let mut out = unit.to_vec();
    out.extend(v.to_be_bytes());
    out
}

fn enumerated(ty: &[u8], value: &[u8]) -> Vec<u8> {
    let mut out = id(ty);
    out.extend(id(value));
    out
}

fn color(c: LinearRgba) -> Vec<u8> {
    let [r, g, b, _] = c.working_to_srgb_encoded();
    let channel = |v: f32| {
        (f64::from(v.clamp(0.0, 1.0)) * 255.0)
            .to_be_bytes()
            .to_vec()
    };
    object(
        b"RGBC",
        &[
            (b"Rd  ", b"doub", channel(r)),
            (b"Grn ", b"doub", channel(g)),
            (b"Bl  ", b"doub", channel(b)),
        ],
    )
}

fn common(enabled: bool, mode: BlendMode, c: LinearRgba, opacity: f32) -> Vec<Item> {
    vec![
        (b"enab", b"bool", vec![u8::from(enabled)]),
        (b"present", b"bool", vec![1]),
        (b"showInDialog", b"bool", vec![1]),
        (b"Md  ", b"enum", enumerated(b"BlnM", mode_id(mode))),
        (b"Clr ", b"Objc", color(c)),
        (b"Opct", b"UntF", unit(b"#Prc", f64::from(opacity) * 100.0)),
    ]
}

fn shadow(s: DropShadow) -> Vec<Item> {
    let mut items = common(s.enabled, s.mode, s.color, s.opacity);
    items.extend([
        (&b"uglg"[..], b"bool", vec![0]),
        (b"lagl", b"UntF", unit(b"#Ang", s.angle)),
        (b"Dstn", b"UntF", unit(b"#Pxl", s.distance)),
        (b"Ckmt", b"UntF", unit(b"#Pxl", s.spread)),
        (b"blur", b"UntF", unit(b"#Pxl", s.size)),
        (b"Nose", b"UntF", unit(b"#Prc", 0.0)),
        (b"AntA", b"bool", vec![0]),
    ]);
    items
}

fn glow(g: Glow, inner: bool) -> Vec<Item> {
    let mut items = common(g.enabled, g.mode, g.color, g.opacity);
    items.extend([
        (&b"GlwT"[..], b"enum", enumerated(b"BETE", b"SfBL")),
        (b"Ckmt", b"UntF", unit(b"#Pxl", g.spread)),
        (b"blur", b"UntF", unit(b"#Pxl", g.size)),
        (b"Nose", b"UntF", unit(b"#Prc", 0.0)),
        (b"ShdN", b"UntF", unit(b"#Prc", 0.0)),
        (b"AntA", b"bool", vec![0]),
        (b"Inpr", b"UntF", unit(b"#Prc", 50.0)),
    ]);
    if inner {
        items.push((b"glwS", b"enum", enumerated(b"IGSr", b"SrcE")));
    }
    items
}

/// A Gradient Overlay's items: Photoshop's own gradient object (`Grdn`, its color stops at
/// their locations, opaque, midpoints in the middle).
fn gradient_overlay(o: GradientOverlay) -> Vec<Item> {
    let long = |v: i32| v.to_be_bytes().to_vec();
    let list = |objects: Vec<Vec<u8>>| {
        let mut out = (objects.len() as u32).to_be_bytes().to_vec();
        for object in objects {
            out.extend(b"Objc");
            out.extend(object);
        }
        out
    };
    let colors = o
        .gradient
        .stops()
        .iter()
        .map(|s| {
            let [r, g, b] = s.color.map(|c| f32::from(c) / 255.0);
            let c = LinearRgba::from_srgb_encoded_to_working(r, g, b, 1.0);
            object(
                b"Clrt",
                &[
                    (b"Clr ", b"Objc", color(c)),
                    (b"Type", b"enum", enumerated(b"Clry", b"UsrS")),
                    (b"Lctn", b"long", long(i32::from(s.location))),
                    (b"Mdpn", b"long", long(50)),
                ],
            )
        })
        .collect();
    let opaque = |location: i32| {
        object(
            b"TrnS",
            &[
                (b"Opct", b"UntF", unit(b"#Prc", 100.0)),
                (b"Lctn", b"long", long(location)),
                (b"Mdpn", b"long", long(50)),
            ],
        )
    };
    let mut name = 1u32.to_be_bytes().to_vec();
    name.extend([0, 0]);
    let gradient = object(
        b"Grdn",
        &[
            (b"Nm  ", b"TEXT", name),
            (b"GrdF", b"enum", enumerated(b"GrdF", b"CstS")),
            (b"Intr", b"doub", 4096.0f64.to_be_bytes().to_vec()),
            (b"Clrs", b"VlLs", list(colors)),
            (b"Trns", b"VlLs", list(vec![opaque(0), opaque(4096)])),
        ],
    );
    let shape: &[u8] = match o.shape {
        GradientShape::Linear => b"Lnr ",
        GradientShape::Radial => b"Rdl ",
    };
    vec![
        (b"enab", b"bool", vec![u8::from(o.enabled)]),
        (b"present", b"bool", vec![1]),
        (b"showInDialog", b"bool", vec![1]),
        (b"Md  ", b"enum", enumerated(b"BlnM", mode_id(o.mode))),
        (
            b"Opct",
            b"UntF",
            unit(b"#Prc", f64::from(o.opacity) * 100.0),
        ),
        (b"Grad", b"Objc", gradient),
        (b"Angl", b"UntF", unit(b"#Ang", o.angle)),
        (b"Type", b"enum", enumerated(b"GrdT", shape)),
        (b"Rvrs", b"bool", vec![u8::from(o.reverse)]),
        (b"Dthr", b"bool", vec![0]),
        (b"Algn", b"bool", vec![u8::from(o.align_with_layer)]),
        (b"Scl ", b"UntF", unit(b"#Prc", o.scale)),
    ]
}

/// `style` as the data of an `lfx2` block (`None` when it has no effect).
pub(crate) fn write(style: &LayerStyle) -> Option<Vec<u8>> {
    let mut items: Vec<Item> = vec![
        (b"Scl ", b"UntF", unit(b"#Prc", 100.0)),
        (b"masterFXSwitch", b"bool", vec![1]),
    ];
    if let Some(s) = style.drop_shadow {
        items.push((b"DrSh", b"Objc", object(b"DrSh", &shadow(s))));
    }
    if let Some(s) = style.inner_shadow {
        items.push((b"IrSh", b"Objc", object(b"IrSh", &shadow(s))));
    }
    if let Some(g) = style.outer_glow {
        items.push((b"OrGl", b"Objc", object(b"OrGl", &glow(g, false))));
    }
    if let Some(g) = style.inner_glow {
        items.push((b"IrGl", b"Objc", object(b"IrGl", &glow(g, true))));
    }
    if let Some(o) = style.color_overlay {
        items.push((
            b"SoFi",
            b"Objc",
            object(b"SoFi", &common(o.enabled, o.mode, o.color, o.opacity)),
        ));
    }
    if let Some(o) = style.gradient_overlay {
        items.push((b"GrFl", b"Objc", object(b"GrFl", &gradient_overlay(o))));
    }
    if let Some(s) = style.satin {
        let mut satin = common(s.enabled, s.mode, s.color, s.opacity);
        satin.extend([
            (&b"AntA"[..], b"bool", vec![1]),
            (b"Invr", b"bool", vec![u8::from(s.invert)]),
            (b"lagl", b"UntF", unit(b"#Ang", s.angle)),
            (b"Dstn", b"UntF", unit(b"#Pxl", s.distance)),
            (b"blur", b"UntF", unit(b"#Pxl", s.size)),
        ]);
        items.push((b"ChFX", b"Objc", object(b"ChFX", &satin)));
    }
    if let Some(b) = style.bevel {
        let style: &[u8] = match b.style {
            BevelStyle::InnerBevel => b"InrB",
            BevelStyle::OuterBevel => b"OtrB",
            BevelStyle::Emboss => b"Embs",
            BevelStyle::PillowEmboss => b"PlEb",
        };
        let bevel = vec![
            (&b"enab"[..], b"bool", vec![u8::from(b.enabled)]),
            (b"present", b"bool", vec![1]),
            (b"showInDialog", b"bool", vec![1]),
            (
                b"hglM",
                b"enum",
                enumerated(b"BlnM", mode_id(b.highlight_mode)),
            ),
            (b"hglC", b"Objc", color(b.highlight_color)),
            (
                b"hglO",
                b"UntF",
                unit(b"#Prc", f64::from(b.highlight_opacity) * 100.0),
            ),
            (
                b"sdwM",
                b"enum",
                enumerated(b"BlnM", mode_id(b.shadow_mode)),
            ),
            (b"sdwC", b"Objc", color(b.shadow_color)),
            (
                b"sdwO",
                b"UntF",
                unit(b"#Prc", f64::from(b.shadow_opacity) * 100.0),
            ),
            (b"bvlT", b"enum", enumerated(b"bvlT", b"SfBL")),
            (b"bvlS", b"enum", enumerated(b"BESl", style)),
            (b"uglg", b"bool", vec![0]),
            (b"lagl", b"UntF", unit(b"#Ang", b.angle)),
            (b"Lald", b"UntF", unit(b"#Ang", b.altitude)),
            (b"srgR", b"UntF", unit(b"#Prc", b.depth)),
            (b"blur", b"UntF", unit(b"#Pxl", b.size)),
            (
                b"bvlD",
                b"enum",
                enumerated(b"BESs", if b.up { b"In  " } else { b"Out " }),
            ),
            (b"Sftn", b"UntF", unit(b"#Pxl", b.soften)),
            (b"useTexture", b"bool", vec![0]),
        ];
        items.push((b"ebbl", b"Objc", object(b"ebbl", &bevel)));
    }
    if let Some(s) = style.stroke {
        let mut stroke = common(s.enabled, s.mode, s.color, s.opacity);
        let position: &[u8] = match s.position {
            StrokeLocation::Inside => b"InsF",
            StrokeLocation::Center => b"CtrF",
            StrokeLocation::Outside => b"OutF",
        };
        stroke.extend([
            (&b"Styl"[..], b"enum", enumerated(b"FStl", position)),
            (b"PntT", b"enum", enumerated(b"FrFl", b"SClr")),
            (b"Sz  ", b"UntF", unit(b"#Pxl", s.size)),
        ]);
        items.push((b"FrFX", b"Objc", object(b"FrFX", &stroke)));
    }
    if items.len() == 2 {
        return None;
    }
    // Object effects version 0, descriptor version 16, the descriptor (class `null`).
    let mut out = 0u32.to_be_bytes().to_vec();
    out.extend(16u32.to_be_bytes());
    out.extend(object(b"null", &items));
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_effect_round_trips_through_lfx2() {
        let style = LayerStyle {
            fill_opacity: 1.0,
            drop_shadow: Some(DropShadow {
                angle: 75.0,
                distance: 12.0,
                spread: 20.0,
                size: 9.0,
                ..DropShadow::default()
            }),
            inner_shadow: Some(DropShadow {
                enabled: false,
                mode: BlendMode::Overlay,
                ..DropShadow::default()
            }),
            outer_glow: Some(Glow {
                opacity: 0.5,
                ..Glow::default()
            }),
            inner_glow: Some(Glow {
                spread: 30.0,
                ..Glow::default()
            }),
            color_overlay: Some(ColorOverlay::default()),
            gradient_overlay: Some(GradientOverlay {
                gradient: Gradient::new(&[
                    GradientStop {
                        location: 0,
                        color: [200, 30, 10],
                    },
                    GradientStop {
                        location: 1500,
                        color: [255, 255, 0],
                    },
                    GradientStop {
                        location: 4096,
                        color: [0, 40, 255],
                    },
                ])
                .unwrap(),
                reverse: true,
                shape: GradientShape::Radial,
                angle: -30.0,
                scale: 80.0,
                align_with_layer: false,
                mode: BlendMode::Multiply,
                opacity: 0.6,
                enabled: true,
            }),
            satin: Some(Satin {
                angle: -40.0,
                distance: 20.0,
                size: 7.0,
                invert: false,
                ..Satin::default()
            }),
            bevel: Some(BevelEmboss {
                style: BevelStyle::PillowEmboss,
                depth: 250.0,
                up: false,
                size: 12.0,
                soften: 3.0,
                angle: 45.0,
                altitude: 60.0,
                highlight_mode: BlendMode::Overlay,
                shadow_opacity: 0.4,
                ..BevelEmboss::default()
            }),
            stroke: Some(Stroke {
                position: StrokeLocation::Center,
                size: 7.0,
                mode: BlendMode::LinearDodge,
                ..Stroke::default()
            }),
        };
        let block = write(&style).unwrap();
        let read = read(&block, 120.0).unwrap();
        assert!(!read.approximated);
        let close = |a: LinearRgba, b: LinearRgba| {
            [a.r - b.r, a.g - b.g, a.b - b.b]
                .iter()
                .all(|d| d.abs() < 1e-4)
        };
        let back = read.style;
        let (s, t) = (style.drop_shadow.unwrap(), back.drop_shadow.unwrap());
        assert_eq!(
            (s.angle, s.distance, s.spread, s.size, s.mode, s.enabled),
            (t.angle, t.distance, t.spread, t.size, t.mode, t.enabled)
        );
        assert!((s.opacity - t.opacity).abs() < 1e-6);
        assert!(close(s.color, t.color));
        assert_eq!(back.inner_shadow.unwrap().mode, BlendMode::Overlay);
        assert!(!back.inner_shadow.unwrap().enabled);
        assert_eq!(back.inner_glow.unwrap().spread, 30.0);
        assert!(close(back.outer_glow.unwrap().color, Glow::default().color));
        assert_eq!(back.stroke.unwrap().position, StrokeLocation::Center);
        assert_eq!(back.stroke.unwrap().mode, BlendMode::LinearDodge);
        assert!(back.color_overlay.is_some());
        // The gradient's stops come back exactly (sRGB-encoded bytes through Photoshop's 0–255).
        let (g, h) = (
            style.gradient_overlay.unwrap(),
            back.gradient_overlay.unwrap(),
        );
        assert_eq!(g.gradient, h.gradient);
        assert_eq!(
            (
                g.reverse,
                g.shape,
                g.angle,
                g.scale,
                g.align_with_layer,
                g.mode
            ),
            (
                h.reverse,
                h.shape,
                h.angle,
                h.scale,
                h.align_with_layer,
                h.mode
            )
        );
        assert!((g.opacity - h.opacity).abs() < 1e-6);
        let (s, t) = (style.satin.unwrap(), back.satin.unwrap());
        assert_eq!(
            (s.angle, s.distance, s.size, s.invert, s.mode, s.enabled),
            (t.angle, t.distance, t.size, t.invert, t.mode, t.enabled)
        );
        assert!((s.opacity - t.opacity).abs() < 1e-6);
        let (b, c) = (style.bevel.unwrap(), back.bevel.unwrap());
        assert_eq!(
            (
                b.style, b.depth, b.up, b.size, b.soften, b.angle, b.altitude
            ),
            (
                c.style, c.depth, c.up, c.size, c.soften, c.angle, c.altitude
            )
        );
        assert_eq!(
            (b.highlight_mode, b.shadow_mode),
            (c.highlight_mode, c.shadow_mode)
        );
        assert!((b.shadow_opacity - c.shadow_opacity).abs() < 1e-6);
        assert!(close(b.highlight_color, c.highlight_color));
        assert_eq!(write(&LayerStyle::default()), None);
    }

    #[test]
    fn what_slopshop_does_not_draw_is_reported() {
        // A bevel, and the master switch off: every effect off.
        let mut block = 0u32.to_be_bytes().to_vec();
        block.extend(16u32.to_be_bytes());
        block.extend(object(
            b"null",
            &[
                (b"masterFXSwitch", b"bool", vec![0]),
                (
                    b"DrSh",
                    b"Objc",
                    object(b"DrSh", &shadow(DropShadow::default())),
                ),
                (
                    b"ebbl",
                    b"Objc",
                    object(b"ebbl", &[(b"enab", b"bool", vec![1])]),
                ),
            ],
        ));
        let read = read(&block, 30.0).unwrap();
        assert!(!read.style.drop_shadow.unwrap().enabled);
        // The bevel is off with the switch: nothing is left out.
        assert!(!read.approximated);
        assert_eq!(super::read(&block[..10], 30.0), None);
    }

    #[test]
    fn global_light_and_scale_apply() {
        let mut uses_global = shadow(DropShadow::default());
        uses_global.retain(|(k, _, _)| *k != b"uglg");
        uses_global.push((b"uglg", b"bool", vec![1]));
        let mut block = 0u32.to_be_bytes().to_vec();
        block.extend(16u32.to_be_bytes());
        block.extend(object(
            b"null",
            &[
                (b"Scl ", b"UntF", unit(b"#Prc", 200.0)),
                (b"DrSh", b"Objc", object(b"DrSh", &uses_global)),
            ],
        ));
        let s = read(&block, 30.0).unwrap().style.drop_shadow.unwrap();
        assert_eq!(s.angle, 30.0);
        assert_eq!((s.distance, s.size), (10.0, 10.0));
    }
}
