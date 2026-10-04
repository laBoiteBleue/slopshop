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
use slopshop_core::selection::StrokeLocation;
use slopshop_core::style::{ColorOverlay, DropShadow, Glow, LayerStyle, MAX_SIZE, Stroke};

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
    if let Some(e) = effect(&d, b"FrFX", b"frameFXMulti", &mut read) {
        style.stroke = Some(read.stroke(e));
    }
    // What SlopShop does not draw, when enabled.
    for key in [
        &b"ebbl"[..],
        b"ChFX",
        b"GrFl",
        b"gradientFillMulti",
        b"patternFill",
    ] {
        let on = match d.get(key) {
            Some(Value::Object(o)) => o.bool(b"enab").unwrap_or(true),
            Some(Value::List(list)) => !list.is_empty(),
            _ => false,
        };
        if on && read.on {
            read.approximated = true;
        }
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
