//! Blend modes and blend spaces (ADR 0012): how a layer combines with what is below it.
//!
//! Colors arrive and leave as premultiplied linear working-space values. A layer is combined
//! with its backdrop in the document's [`BlendSpace`] with the W3C "source over" formula and a
//! blend function `B`:
//! `αo = αs + αb − αs·αb`, `αo·Co = αs·(1 − αb)·Cs + αb·(1 − αs)·Cb + αs·αb·B(Cb, Cs)`.
//! This module is the reference (`f64`); the GPU shader implements the same math in `f32`.

use std::fmt;

use crate::color::{ColorSpace, IDENTITY, Mat3, TransferFunction, WORKING_SPACE, mat_vec};

/// How a layer's color combines with the color below it (Photoshop's modes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BlendMode {
    #[default]
    Normal,
    /// Each pixel is either the layer's color or what is below, at random with the layer's
    /// coverage as probability ([`dissolve`]); then like normal.
    Dissolve,
    Darken,
    Multiply,
    ColorBurn,
    LinearBurn,
    DarkerColor,
    Lighten,
    Screen,
    ColorDodge,
    LinearDodge,
    LighterColor,
    Overlay,
    SoftLight,
    HardLight,
    VividLight,
    LinearLight,
    PinLight,
    HardMix,
    Difference,
    Exclusion,
    Subtract,
    Divide,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

impl BlendMode {
    /// Every mode, in Photoshop's menu order (groups: normal, darken, lighten, contrast,
    /// inversion, cancellation, component).
    pub const ALL: [BlendMode; 27] = [
        BlendMode::Normal,
        BlendMode::Dissolve,
        BlendMode::Darken,
        BlendMode::Multiply,
        BlendMode::ColorBurn,
        BlendMode::LinearBurn,
        BlendMode::DarkerColor,
        BlendMode::Lighten,
        BlendMode::Screen,
        BlendMode::ColorDodge,
        BlendMode::LinearDodge,
        BlendMode::LighterColor,
        BlendMode::Overlay,
        BlendMode::SoftLight,
        BlendMode::HardLight,
        BlendMode::VividLight,
        BlendMode::LinearLight,
        BlendMode::PinLight,
        BlendMode::HardMix,
        BlendMode::Difference,
        BlendMode::Exclusion,
        BlendMode::Subtract,
        BlendMode::Divide,
        BlendMode::Hue,
        BlendMode::Saturation,
        BlendMode::Color,
        BlendMode::Luminosity,
    ];

    /// Stable identifier (IPC, files).
    pub fn id(self) -> &'static str {
        match self {
            BlendMode::Normal => "normal",
            BlendMode::Dissolve => "dissolve",
            BlendMode::Darken => "darken",
            BlendMode::Multiply => "multiply",
            BlendMode::ColorBurn => "colorBurn",
            BlendMode::LinearBurn => "linearBurn",
            BlendMode::DarkerColor => "darkerColor",
            BlendMode::Lighten => "lighten",
            BlendMode::Screen => "screen",
            BlendMode::ColorDodge => "colorDodge",
            BlendMode::LinearDodge => "linearDodge",
            BlendMode::LighterColor => "lighterColor",
            BlendMode::Overlay => "overlay",
            BlendMode::SoftLight => "softLight",
            BlendMode::HardLight => "hardLight",
            BlendMode::VividLight => "vividLight",
            BlendMode::LinearLight => "linearLight",
            BlendMode::PinLight => "pinLight",
            BlendMode::HardMix => "hardMix",
            BlendMode::Difference => "difference",
            BlendMode::Exclusion => "exclusion",
            BlendMode::Subtract => "subtract",
            BlendMode::Divide => "divide",
            BlendMode::Hue => "hue",
            BlendMode::Saturation => "saturation",
            BlendMode::Color => "color",
            BlendMode::Luminosity => "luminosity",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.id() == id)
    }

    /// Its index in [`Self::ALL`], as the GPU shader numbers the modes.
    pub fn index(self) -> u32 {
        // Invariant: ALL lists every variant.
        Self::ALL.iter().position(|m| *m == self).unwrap_or(0) as u32
    }
}

impl fmt::Display for BlendMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

/// Where blend formulas (and opacity) are computed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BlendSpace {
    /// Values encoded with the sRGB curve on sRGB primaries: the look of Photoshop's 8/16-bit
    /// documents. The default.
    #[default]
    Perceptual,
    /// The linear working space: physically based (multiply filters light, add sums it).
    Linear,
}

impl BlendSpace {
    pub fn id(self) -> &'static str {
        match self {
            BlendSpace::Perceptual => "perceptual",
            BlendSpace::Linear => "linear",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        [BlendSpace::Perceptual, BlendSpace::Linear]
            .into_iter()
            .find(|space| space.id() == id)
    }

    /// The color space the formulas work in, and whether its values are encoded.
    fn color_space(self) -> ColorSpace {
        match self {
            BlendSpace::Perceptual => ColorSpace::SRGB,
            BlendSpace::Linear => WORKING_SPACE,
        }
    }
}

/// Combines layers in one blend space. Immutable once built, so one blender can serve several
/// threads.
#[derive(Debug, Clone)]
pub struct Blender {
    space: BlendSpace,
    /// Working space → blend space primaries, and back (`None` for the identity).
    to_blend: Option<Mat3>,
    from_blend: Option<Mat3>,
    transfer: TransferFunction,
}

impl Blender {
    /// A blender for documents whose working space is [`WORKING_SPACE`].
    pub fn new(space: BlendSpace) -> Self {
        let target = space.color_space();
        let to_blend = WORKING_SPACE.matrix_to(&target);
        let from_blend = target.matrix_to(&WORKING_SPACE);
        Self {
            space,
            to_blend: (to_blend != IDENTITY).then_some(to_blend),
            from_blend: (from_blend != IDENTITY).then_some(from_blend),
            transfer: target.transfer,
        }
    }

    pub fn space(&self) -> BlendSpace {
        self.space
    }

    /// Combine `src` (a layer's premultiplied linear color, opacity included) over `dst` (what
    /// is below), both in the working space.
    pub fn blend(&self, mode: BlendMode, src: &[f64; 4], dst: &mut [f64; 4]) {
        let (alpha_s, alpha_b) = (src[3], dst[3]);
        // Exact paths: nothing to mix with, or an opaque normal layer that covers it all.
        // Dissolve blends like normal (its pixels were already chosen, see `dissolve`).
        let mode = if mode == BlendMode::Dissolve {
            BlendMode::Normal
        } else {
            mode
        };
        if mode == BlendMode::Normal
            && (self.space == BlendSpace::Linear || alpha_s >= 1.0 || alpha_b <= 0.0)
        {
            let keep = 1.0 - alpha_s;
            for (d, s) in dst.iter_mut().zip(src) {
                *d = s + *d * keep;
            }
            return;
        }
        if alpha_s <= 0.0 {
            return;
        }
        // Float sources may carry alpha above 1: the formula needs coverages.
        let (alpha_s, alpha_b) = (alpha_s.min(1.0), alpha_b.clamp(0.0, 1.0));
        let cs = self.encode(unpremultiply(src));
        let cb = self.encode(unpremultiply(dst));
        let mixed = blend_color(mode, self.space, cb, cs);
        let alpha_o = alpha_s + alpha_b - alpha_s * alpha_b;
        let mut co = [0.0; 3];
        for (i, c) in co.iter_mut().enumerate() {
            let premultiplied = alpha_s * (1.0 - alpha_b) * cs[i]
                + alpha_b * (1.0 - alpha_s) * cb[i]
                + alpha_s * alpha_b * mixed[i];
            *c = if alpha_o > 0.0 {
                premultiplied / alpha_o
            } else {
                0.0
            };
        }
        let [r, g, b] = self.decode(co);
        *dst = [r * alpha_o, g * alpha_o, b * alpha_o, alpha_o];
    }

    /// Combine `src` atop `dst` (a clipped layer on its clipping group, ADR 0016): blended like
    /// [`Self::blend`], but the result keeps `dst`'s coverage (W3C source-atop with blending):
    /// `co = αs·(1 − αb)·cs + αs·αb·B(cb, cs) + (1 − αs)·cb`, alpha `αb`.
    pub fn blend_atop(&self, mode: BlendMode, src: &[f64; 4], dst: &mut [f64; 4]) {
        let (alpha_s, alpha_b) = (src[3], dst[3]);
        if alpha_b <= 0.0 {
            return;
        }
        let mode = if mode == BlendMode::Dissolve {
            BlendMode::Normal
        } else {
            mode
        };
        // Exact paths: normal mode is `src · αb + dst · (1 − αs)` in linear light, and in either
        // space for an opaque layer (perceptual mixes encoded values otherwise).
        if mode == BlendMode::Normal && (self.space == BlendSpace::Linear || alpha_s >= 1.0) {
            let keep = 1.0 - alpha_s;
            for (d, s) in dst.iter_mut().zip(src) {
                *d = s * alpha_b + *d * keep;
            }
            return;
        }
        if alpha_s <= 0.0 {
            return;
        }
        let (alpha_s, alpha_b) = (alpha_s.min(1.0), alpha_b.clamp(0.0, 1.0));
        let cs = self.encode(unpremultiply(src));
        let cb = self.encode(unpremultiply(dst));
        let mixed = blend_color(mode, self.space, cb, cs);
        let co: [f64; 3] = std::array::from_fn(|i| {
            alpha_s * (1.0 - alpha_b) * cs[i]
                + alpha_s * alpha_b * mixed[i]
                + (1.0 - alpha_s) * cb[i]
        });
        let [r, g, b] = self.decode(co);
        *dst = [r * alpha_b, g * alpha_b, b * alpha_b, alpha_b];
    }

    /// Fade from `below` to `above` by `t` (clamped to `[0, 1]`), both premultiplied
    /// working-space colors: a pass-through group's opacity and mask (ADR 0015). The mix is
    /// premultiplied in the blend space, so that a perceptual document fades encoded values as
    /// Photoshop does; it is exact at 0 and 1.
    pub fn fade(&self, below: &[f64; 4], above: &[f64; 4], t: f64) -> [f64; 4] {
        if t >= 1.0 {
            return *above;
        }
        if t.is_nan() || t <= 0.0 {
            return *below;
        }
        if self.space == BlendSpace::Linear {
            return std::array::from_fn(|k| below[k] + (above[k] - below[k]) * t);
        }
        let (alpha_b, alpha_a) = (below[3].clamp(0.0, 1.0), above[3].clamp(0.0, 1.0));
        let alpha = alpha_b + (alpha_a - alpha_b) * t;
        if alpha <= 0.0 {
            return [0.0; 4];
        }
        let (cb, ca) = (
            self.encode(unpremultiply(below)),
            self.encode(unpremultiply(above)),
        );
        let co: [f64; 3] =
            std::array::from_fn(|i| (cb[i] * alpha_b * (1.0 - t) + ca[i] * alpha_a * t) / alpha);
        let [r, g, b] = self.decode(co);
        [r * alpha, g * alpha, b * alpha, alpha]
    }

    /// `below` (a premultiplied working-space color) with `adjustment` applied, mixed with it by
    /// `coverage` (an adjustment layer's opacity × mask, ADR 0020): the adjustment runs on the
    /// straight color in the blend space (in linear light for linear adjustments), the mix is
    /// [`Self::fade`]'s; alpha is kept.
    pub fn adjust(
        &self,
        adjustment: &crate::adjust::Prepared,
        below: &[f64; 4],
        coverage: f64,
    ) -> [f64; 4] {
        let alpha = below[3];
        if coverage.is_nan() || coverage <= 0.0 || alpha <= 0.0 {
            return *below;
        }
        let straight = unpremultiply(below);
        let adjusted = if adjustment.adjustment().is_linear() {
            adjustment.apply(straight)
        } else if adjustment.adjustment().is_perceptual() && self.space != BlendSpace::Perceptual {
            // Made once: per pixel, its matrices cost twice the adjustment.
            static PERCEPTUAL: std::sync::OnceLock<Blender> = std::sync::OnceLock::new();
            let perceptual = PERCEPTUAL.get_or_init(|| Blender::new(BlendSpace::Perceptual));
            perceptual.decode(adjustment.apply(perceptual.encode(straight)))
        } else {
            self.decode(adjustment.apply(self.encode(straight)))
        };
        let above = [
            adjusted[0] * alpha,
            adjusted[1] * alpha,
            adjusted[2] * alpha,
            alpha,
        ];
        self.fade(below, &above, coverage)
    }

    /// A premultiplied working-space color as premultiplied blend-space values: the encoded
    /// straight color times alpha, where a layer's paint is `P + k·B` (ADR 0029).
    pub(crate) fn encode_premultiplied(&self, color: &[f64; 4]) -> [f64; 4] {
        if self.space == BlendSpace::Linear {
            return *color;
        }
        let alpha = color[3];
        let [r, g, b] = self.encode(unpremultiply(color));
        [r * alpha, g * alpha, b * alpha, alpha]
    }

    /// The inverse of [`Self::encode_premultiplied`], alpha clamped to `[0, 1]`.
    pub(crate) fn decode_premultiplied(&self, values: &[f64; 4]) -> [f64; 4] {
        let alpha = values[3].clamp(0.0, 1.0);
        if self.space == BlendSpace::Linear {
            return [values[0], values[1], values[2], alpha];
        }
        if alpha <= 0.0 {
            return [0.0; 4];
        }
        let [r, g, b] = self.decode([values[0], values[1], values[2]].map(|v| v / values[3]));
        [r * alpha, g * alpha, b * alpha, alpha]
    }

    /// Straight working-space color → blend-space values.
    fn encode(&self, color: [f64; 3]) -> [f64; 3] {
        let color = match &self.to_blend {
            Some(m) => mat_vec(m, color),
            None => color,
        };
        if self.transfer.is_linear() {
            return color;
        }
        color.map(|c| encode_mirrored(self.transfer, c))
    }

    fn decode(&self, color: [f64; 3]) -> [f64; 3] {
        let color = if self.transfer.is_linear() {
            color
        } else {
            color.map(|c| decode_mirrored(self.transfer, c))
        };
        match &self.from_blend {
            Some(m) => mat_vec(m, color),
            None => color,
        }
    }
}

/// The transfer curve extended to negative values by symmetry (out-of-gamut colors), in `f64`.
fn encode_mirrored(transfer: TransferFunction, v: f64) -> f64 {
    match transfer {
        // The one curve blending uses: exact in f64.
        TransferFunction::Srgb => {
            let a = v.abs();
            let e = if a <= 0.003_130_8 {
                a * 12.92
            } else {
                1.055 * a.powf(1.0 / 2.4) - 0.055
            };
            e.copysign(v)
        }
        other => f64::from(other.encode(v.abs() as f32)).copysign(v),
    }
}

fn decode_mirrored(transfer: TransferFunction, e: f64) -> f64 {
    match transfer {
        TransferFunction::Srgb => {
            let a = e.abs();
            let v = if a <= 0.040_45 {
                a / 12.92
            } else {
                ((a + 0.055) / 1.055).powf(2.4)
            };
            v.copysign(e)
        }
        other => f64::from(other.decode(e.abs() as f32)).copysign(e),
    }
}

fn unpremultiply(px: &[f64; 4]) -> [f64; 3] {
    let a = px[3];
    if a > 0.0 {
        [px[0] / a, px[1] / a, px[2] / a]
    } else {
        [0.0; 3]
    }
}

/// Dissolve (Photoshop's): the layer's pixel at document position (`x`, `y`), premultiplied, is
/// kept whole (its straight color at full alpha) when [`dissolve_noise`] is below its alpha, and
/// dropped otherwise. The pattern depends only on the position, so it is the same in every view
/// and export, and on the GPU.
pub fn dissolve(src: [f64; 4], x: u32, y: u32) -> [f64; 4] {
    let alpha = src[3];
    let kept = dissolve_noise(x, y) < alpha.min(1.0);
    if !kept {
        return [0.0; 4];
    }
    let straight = |c: f64| if alpha > 0.0 { c / alpha } else { 0.0 };
    [straight(src[0]), straight(src[1]), straight(src[2]), 1.0]
}

/// A uniform value in `[0, 1)` for each document pixel: a 32-bit integer hash of the position
/// (lowbias32), reduced to 24 bits so that it is exact in `f32` too (the GPU shader computes the
/// same one).
pub fn dissolve_noise(x: u32, y: u32) -> f64 {
    let mut h = x.wrapping_mul(0x8da6_b343) ^ y.wrapping_mul(0xd816_3841);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb_352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846c_a68b);
    h ^= h >> 16;
    f64::from(h >> 8) / 16_777_216.0
}

/// Values closer than this to 0 (or 1) count as 0 (or 1) where a formula divides by them
/// (divide, the dodges and burns): conversions leave channels that should be 0 at about ±1e-9,
/// and dividing that noise by noise would give arbitrary results. Below the resolution of
/// 16-bit samples (1.5e-5), above f32 rounding. The GPU shader uses the same value.
pub const DIVISION_EPSILON: f64 = 1e-6;

/// The blend function `B(Cb, Cs)` on straight blend-space colors.
pub fn blend_color(mode: BlendMode, space: BlendSpace, cb: [f64; 3], cs: [f64; 3]) -> [f64; 3] {
    let perceptual = space == BlendSpace::Perceptual;
    // Modes that leave [0, 1] with in-range inputs: clamped like Photoshop's 8/16-bit documents
    // in perceptual space, only at 0 in linear space.
    let clamp = |v: f64| {
        if perceptual {
            v.clamp(0.0, 1.0)
        } else {
            v.max(0.0)
        }
    };
    let separable = |f: &dyn Fn(f64, f64) -> f64| [0, 1, 2].map(|i| f(cb[i], cs[i]));
    match mode {
        BlendMode::Normal | BlendMode::Dissolve => cs,
        BlendMode::Darken => separable(&|b, s| b.min(s)),
        BlendMode::Multiply => separable(&|b, s| b * s),
        BlendMode::ColorBurn => separable(&color_burn),
        BlendMode::LinearBurn => separable(&|b, s| clamp(b + s - 1.0)),
        BlendMode::DarkerColor => {
            if cs.iter().sum::<f64>() < cb.iter().sum::<f64>() {
                cs
            } else {
                cb
            }
        }
        BlendMode::Lighten => separable(&|b, s| b.max(s)),
        BlendMode::Screen => separable(&screen),
        BlendMode::ColorDodge => separable(&color_dodge),
        BlendMode::LinearDodge => separable(&|b, s| clamp(b + s)),
        BlendMode::LighterColor => {
            if cs.iter().sum::<f64>() > cb.iter().sum::<f64>() {
                cs
            } else {
                cb
            }
        }
        BlendMode::Overlay => separable(&|b, s| hard_light(s, b)),
        BlendMode::SoftLight => separable(&soft_light),
        BlendMode::HardLight => separable(&hard_light),
        BlendMode::VividLight => separable(&|b, s| {
            if s <= 0.5 {
                color_burn(b, 2.0 * s)
            } else {
                color_dodge(b, 2.0 * s - 1.0)
            }
        }),
        BlendMode::LinearLight => separable(&|b, s| clamp(b + 2.0 * s - 1.0)),
        BlendMode::PinLight => separable(&|b, s| {
            if s <= 0.5 {
                b.min(2.0 * s)
            } else {
                b.max(2.0 * s - 1.0)
            }
        }),
        BlendMode::HardMix => separable(&|b, s| if b + s >= 1.0 { 1.0 } else { 0.0 }),
        BlendMode::Difference => separable(&|b, s| (b - s).abs()),
        BlendMode::Exclusion => separable(&|b, s| b + s - 2.0 * b * s),
        BlendMode::Subtract => separable(&|b, s| clamp(b - s)),
        BlendMode::Divide => separable(&|b, s| {
            if s > DIVISION_EPSILON {
                clamp(b / s)
            } else if b > DIVISION_EPSILON {
                // Division by zero: white, as Photoshop.
                1.0
            } else {
                0.0
            }
        }),
        BlendMode::Hue => set_lum(set_sat(cs, sat(cb)), lum(cb), perceptual),
        BlendMode::Saturation => set_lum(set_sat(cb, sat(cs)), lum(cb), perceptual),
        BlendMode::Color => set_lum(cs, lum(cb), perceptual),
        BlendMode::Luminosity => set_lum(cb, lum(cs), perceptual),
    }
}

fn screen(b: f64, s: f64) -> f64 {
    b + s - b * s
}

fn hard_light(b: f64, s: f64) -> f64 {
    if s <= 0.5 {
        b * 2.0 * s
    } else {
        screen(b, 2.0 * s - 1.0)
    }
}

/// Photoshop's soft light.
fn soft_light(b: f64, s: f64) -> f64 {
    if s <= 0.5 {
        2.0 * b * s + b * b * (1.0 - 2.0 * s)
    } else {
        2.0 * b * (1.0 - s) + b.max(0.0).sqrt() * (2.0 * s - 1.0)
    }
}

fn color_dodge(b: f64, s: f64) -> f64 {
    if b <= DIVISION_EPSILON {
        0.0
    } else if s >= 1.0 - DIVISION_EPSILON {
        1.0
    } else {
        (b / (1.0 - s)).min(1.0)
    }
}

fn color_burn(b: f64, s: f64) -> f64 {
    if b >= 1.0 - DIVISION_EPSILON {
        1.0
    } else if s <= DIVISION_EPSILON {
        0.0
    } else {
        1.0 - ((1.0 - b) / s).min(1.0)
    }
}

/// W3C luminance of the non-separable modes.
fn lum(c: [f64; 3]) -> f64 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}

fn sat(c: [f64; 3]) -> f64 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}

/// W3C `ClipColor`: back into range keeping the luminance. The top only in perceptual space
/// (linear values may exceed 1).
fn clip_color(c: [f64; 3], perceptual: bool) -> [f64; 3] {
    let l = lum(c);
    let n = c[0].min(c[1]).min(c[2]);
    let x = c[0].max(c[1]).max(c[2]);
    let mut c = c;
    if n < 0.0 && l - n > 0.0 {
        c = c.map(|v| l + (v - l) * l / (l - n));
    }
    if perceptual && x > 1.0 && x - l > 0.0 {
        c = c.map(|v| l + (v - l) * (1.0 - l) / (x - l));
    }
    c
}

fn set_lum(c: [f64; 3], l: f64, perceptual: bool) -> [f64; 3] {
    let d = l - lum(c);
    clip_color(c.map(|v| v + d), perceptual)
}

fn set_sat(c: [f64; 3], s: f64) -> [f64; 3] {
    let max = c[0].max(c[1]).max(c[2]);
    let min = c[0].min(c[1]).min(c[2]);
    if max <= min {
        return [0.0; 3];
    }
    c.map(|v| (v - min) * s / (max - min))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GRAY: [f64; 3] = [0.5; 3];

    fn close(a: [f64; 3], b: [f64; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9)
    }

    #[test]
    fn ids_are_unique_and_round_trip() {
        for mode in BlendMode::ALL {
            assert_eq!(BlendMode::from_id(mode.id()), Some(mode));
            assert_eq!(BlendMode::ALL[mode.index() as usize], mode);
        }
        let mut ids: Vec<_> = BlendMode::ALL.iter().map(|m| m.id()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), BlendMode::ALL.len());
        for space in [BlendSpace::Perceptual, BlendSpace::Linear] {
            assert_eq!(BlendSpace::from_id(space.id()), Some(space));
        }
        assert_eq!(BlendMode::from_id("pinkify"), None);
    }

    #[test]
    fn known_values_of_the_blend_functions() {
        let p = BlendSpace::Perceptual;
        let b = [0.2, 0.5, 0.8];
        let s = [0.6, 0.5, 0.1];
        let f = |mode| blend_color(mode, p, b, s);
        assert!(close(f(BlendMode::Multiply), [0.12, 0.25, 0.08]));
        assert!(close(f(BlendMode::Screen), [0.68, 0.75, 0.82]));
        assert!(close(f(BlendMode::Darken), [0.2, 0.5, 0.1]));
        assert!(close(f(BlendMode::Lighten), [0.6, 0.5, 0.8]));
        assert!(close(f(BlendMode::Difference), [0.4, 0.0, 0.7]));
        assert!(close(f(BlendMode::Exclusion), [0.56, 0.5, 0.74]));
        assert!(close(f(BlendMode::LinearDodge), [0.8, 1.0, 0.9]));
        assert!(close(f(BlendMode::LinearBurn), [0.0, 0.0, 0.0]));
        assert!(close(f(BlendMode::Subtract), [0.0, 0.0, 0.7]));
        // Overlay is hard light with the layers swapped.
        assert!(close(
            f(BlendMode::Overlay),
            blend_color(BlendMode::HardLight, p, s, b)
        ));
        // Photoshop's soft light at s = ½ leaves the backdrop unchanged.
        assert!(close(blend_color(BlendMode::SoftLight, p, b, GRAY), b));
        // Hard mix is 0 or 1.
        assert!(close(f(BlendMode::HardMix), [0.0, 1.0, 0.0]));
        // Darker color picks a whole color.
        assert_eq!(f(BlendMode::DarkerColor), s);
        assert_eq!(f(BlendMode::LighterColor), b);
    }

    #[test]
    fn neutral_layers_leave_the_backdrop_unchanged() {
        let p = BlendSpace::Perceptual;
        let b = [0.2, 0.5, 0.8];
        // White multiplies to identity, black screens to identity, gray is neutral for the
        // contrast modes.
        assert!(close(blend_color(BlendMode::Multiply, p, b, [1.0; 3]), b));
        assert!(close(blend_color(BlendMode::Screen, p, b, [0.0; 3]), b));
        for mode in [
            BlendMode::HardLight,
            BlendMode::SoftLight,
            BlendMode::LinearLight,
            BlendMode::VividLight,
        ] {
            assert!(close(blend_color(mode, p, b, GRAY), b), "{mode}");
        }
        // Luminosity of a gray with the backdrop's own luminance keeps the backdrop.
        let l = lum(b);
        assert!(close(blend_color(BlendMode::Luminosity, p, b, [l; 3]), b));
        // Hue and saturation of the backdrop itself keep it.
        for mode in [BlendMode::Hue, BlendMode::Saturation, BlendMode::Color] {
            assert!(close(blend_color(mode, p, b, b), b), "{mode}");
        }
    }

    #[test]
    fn results_stay_finite_for_extreme_values() {
        let values = [
            -3.0,
            -1e-12,
            0.0,
            1e-12,
            0.5,
            1.0,
            1.0 + 1e-12,
            7.0,
            65504.0,
        ];
        for space in [BlendSpace::Perceptual, BlendSpace::Linear] {
            for mode in BlendMode::ALL {
                for &b in &values {
                    for &s in &values {
                        let r = blend_color(mode, space, [b, s, 0.3], [s, b, 0.7]);
                        assert!(r.iter().all(|v| v.is_finite()), "{mode} {space:?} {b} {s}");
                    }
                }
            }
        }
    }

    #[test]
    fn perceptual_clamps_additive_modes_and_linear_does_not() {
        let b = [0.8; 3];
        let s = [0.7; 3];
        let dodge = |space| blend_color(BlendMode::LinearDodge, space, b, s)[0];
        assert_eq!(dodge(BlendSpace::Perceptual), 1.0);
        assert!((dodge(BlendSpace::Linear) - 1.5).abs() < 1e-12);
        let sub = |space| blend_color(BlendMode::Subtract, space, s, b)[0];
        assert_eq!(sub(BlendSpace::Perceptual), 0.0);
        assert_eq!(sub(BlendSpace::Linear), 0.0);
    }

    fn px(r: f64, g: f64, b: f64, a: f64) -> [f64; 4] {
        [r * a, g * a, b * a, a]
    }

    #[test]
    fn normal_opaque_and_over_nothing_are_exact_in_both_spaces() {
        let src = px(0.123_456_789, 0.9, 1e-7, 1.0);
        for space in [BlendSpace::Perceptual, BlendSpace::Linear] {
            let blender = Blender::new(space);
            let mut dst = px(0.4, 0.2, 0.7, 0.6);
            blender.blend(BlendMode::Normal, &src, &mut dst);
            assert_eq!(dst, src, "{space:?} opaque");
            let translucent = px(0.3, 0.6, 0.1, 0.4);
            let mut empty = [0.0; 4];
            blender.blend(BlendMode::Normal, &translucent, &mut empty);
            assert_eq!(empty, translucent, "{space:?} over nothing");
        }
    }

    #[test]
    fn linear_normal_is_premultiplied_over() {
        let blender = Blender::new(BlendSpace::Linear);
        let src = px(1.0, 0.0, 0.0, 0.5);
        let mut dst = px(0.0, 0.0, 1.0, 1.0);
        blender.blend(BlendMode::Normal, &src, &mut dst);
        assert_eq!(dst, [0.5, 0.0, 0.5, 1.0]);
    }

    #[test]
    fn perceptual_opacity_mixes_encoded_values() {
        // Half-transparent white over black: mid-gray in sRGB-encoded values (as Photoshop's
        // 8-bit documents), about 0.214 linear; linear mixing would give 0.5.
        let blender = Blender::new(BlendSpace::Perceptual);
        let white = [0.5, 0.5, 0.5, 0.5];
        let mut black = [0.0, 0.0, 0.0, 1.0];
        blender.blend(BlendMode::Normal, &white, &mut black);
        let expected = decode_mirrored(TransferFunction::Srgb, 0.5);
        for c in &black[..3] {
            assert!((c - expected).abs() < 1e-9, "{c} vs {expected}");
        }
        assert!((black[3] - 1.0).abs() < 1e-12);
    }

    #[test]
    fn multiply_over_opaque_backdrop() {
        // Perceptual multiply of two mid-grays: 0.25 encoded.
        let blender = Blender::new(BlendSpace::Perceptual);
        let gray = decode_mirrored(TransferFunction::Srgb, 0.5);
        let src = [gray, gray, gray, 1.0];
        let mut dst = src;
        blender.blend(BlendMode::Multiply, &src, &mut dst);
        let expected = decode_mirrored(TransferFunction::Srgb, 0.25);
        assert!((dst[0] - expected).abs() < 1e-9);
        // Linear multiply: product of the linear values.
        let linear = Blender::new(BlendSpace::Linear);
        let mut dst = src;
        linear.blend(BlendMode::Multiply, &src, &mut dst);
        assert!((dst[0] - gray * gray).abs() < 1e-12);
    }

    #[test]
    fn transparent_layers_change_nothing() {
        for space in [BlendSpace::Perceptual, BlendSpace::Linear] {
            let blender = Blender::new(space);
            for mode in BlendMode::ALL {
                let before = px(0.3, 0.4, 0.5, 0.8);
                let mut dst = before;
                blender.blend(mode, &[0.0; 4], &mut dst);
                assert_eq!(dst, before, "{mode} {space:?}");
            }
        }
    }

    #[test]
    fn encoding_round_trips() {
        for v in [-2.0, -0.01, 0.0, 0.001, 0.2, 1.0, 40.0] {
            let e = encode_mirrored(TransferFunction::Srgb, v);
            assert!(
                (decode_mirrored(TransferFunction::Srgb, e) - v).abs() < 1e-12 * v.abs().max(1.0)
            );
        }
    }

    #[test]
    fn dissolve_keeps_whole_pixels_with_the_layer_coverage_as_probability() {
        let src = px(0.2, 0.4, 0.6, 0.3);
        let mut kept = 0;
        for y in 0..100 {
            for x in 0..100 {
                let out = dissolve(src, x, y);
                if out[3] > 0.0 {
                    kept += 1;
                    assert!(close([out[0], out[1], out[2]], [0.2, 0.4, 0.6]));
                    assert_eq!(out[3], 1.0);
                } else {
                    assert_eq!(out, [0.0; 4]);
                }
                // The same pixel always gives the same answer.
                assert_eq!(dissolve(src, x, y), out);
            }
        }
        assert!((2700..3300).contains(&kept), "{kept} of 10000 at 30%");
        // Opaque layers are kept whole everywhere, transparent ones nowhere.
        assert!((0..50).all(|i| dissolve(px(1.0, 0.0, 0.0, 1.0), i, 7 * i)[3] == 1.0));
        assert!((0..50).all(|i| dissolve([0.0; 4], i, i)[3] == 0.0));
        let noise = dissolve_noise(123, 456);
        assert!((0.0..1.0).contains(&noise));
    }
}
