//! Adjustments of adjustment layers (ADR 0020): what each one does to a straight color, with
//! Photoshop's parameters. The compositors apply them to what is below the layer, in the
//! document's blend space (`Blender::adjust`); the GPU renderer runs the same math.

use std::sync::LazyLock;

use crate::color::{ColorSpace, Mat3, TransferFunction, WORKING_SPACE, mat_vec};
use crate::curve::{Curve, lookup};

/// Number of parameters of an adjustment ([`Adjustment::params`]).
pub const PARAM_COUNT: usize = 20;

/// One set of Levels settings: input black, input white, gamma, output black, output white.
pub type LevelsChannel = [f32; 5];

/// Levels that change nothing (within `[0, 1]`).
pub const LEVELS_IDENTITY: LevelsChannel = [0.0, 1.0, 1.0, 0.0, 1.0];

/// An adjustment and its parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Adjustment {
    /// In linear light: `(v · 2^exposure + offset)^(1 / gamma)`.
    Exposure {
        exposure: f32,
        offset: f32,
        gamma: f32,
    },
    /// Hue shift in degrees (−180…180); saturation and lightness in −100…100.
    HueSaturation {
        hue: f32,
        saturation: f32,
        lightness: f32,
    },
    /// Per channel: inputs mapped from `[input_black, input_white]` (clamped) through `gamma`
    /// to `[output_black, output_white]`, values in `[0, 1]`. Red, green and blue first go
    /// through their own settings (`channels`, [`LevelsChannel`] each; a channel at
    /// [`LEVELS_IDENTITY`] is left as it is), then all three through the composite ones, as
    /// Curves (Photoshop does not publish its order; GIMP's Levels do the same).
    Levels {
        input_black: f32,
        input_white: f32,
        gamma: f32,
        output_black: f32,
        output_white: f32,
        channels: [LevelsChannel; 3],
    },
    /// Brightness (−150…150) and contrast (−50…100), keeping black and white where they are
    /// (like Photoshop's current, non-legacy mode; Adobe's exact curves are not published):
    /// brightness is a gamma, contrast an S-curve around middle gray.
    BrightnessContrast { brightness: f32, contrast: f32 },
    /// Vibrance (−100…100) raises the saturation of dull colors more than of saturated ones;
    /// saturation (−100…100) changes them all.
    Vibrance { vibrance: f32, saturation: f32 },
    /// Each channel `v ↦ 1 − v`.
    Invert,
    /// Each channel quantized to `levels` (2…255) values.
    Posterize { levels: f32 },
    /// White where the luminance reaches `level` (in `[0, 1]`, shown 1–255), black elsewhere.
    Threshold { level: f32 },
    /// Gray made from each color's hue family, with Photoshop's six weights in % (−200…300):
    /// reds, yellows, greens, cyans, blues, magentas. Optionally tinted: the gray becomes the
    /// HSL lightness of a color of hue `tint_hue` (degrees, 0…360) and saturation
    /// `tint_saturation` (0…100).
    BlackWhite {
        weights: [f32; 6],
        tint: bool,
        tint_hue: f32,
        tint_saturation: f32,
    },
    /// Cyan–red, magenta–green and yellow–blue shifts (−100…100) of the shadows, midtones and
    /// highlights, optionally keeping each color's HSL lightness (GIMP's model of Photoshop's
    /// tool, whose exact curves are not published).
    ColorBalance {
        shadows: [f32; 3],
        midtones: [f32; 3],
        highlights: [f32; 3],
        preserve_luminosity: bool,
    },
    /// A colored filter in front of the lens, in linear light with sRGB primaries (as in an
    /// sRGB document): the color is multiplied by `color` (sRGB-encoded, in `[0, 1]`) with
    /// `density` (0…100), optionally keeping the luminance.
    PhotoFilter {
        color: [f32; 3],
        density: f32,
        preserve_luminosity: bool,
    },
    /// Each output channel as a sum of the input channels weighted in % (−200…200) plus a
    /// constant in % of white (−200…200): `[red, green, blue, constant]`. Monochrome: the red
    /// row for every channel (Photoshop's gray output).
    ChannelMixer {
        red: [f32; 4],
        green: [f32; 4],
        blue: [f32; 4],
        monochrome: bool,
    },
    /// Tone curves: each channel through its own curve, then all three through the composite
    /// (`rgb`) one, as Photoshop's RGB curves. Not in [`Adjustment::params`]: the curves are
    /// their own data.
    Curves {
        rgb: Curve,
        red: Curve,
        green: Curve,
        blue: Curve,
    },
}

impl Adjustment {
    /// Every adjustment, with the parameters a new layer gets (Photoshop's defaults): neutral
    /// ones, except for Invert, Posterize, Threshold, Black & White and Photo Filter, which
    /// change the image by nature.
    pub const DEFAULTS: [Adjustment; 13] = [
        Adjustment::Exposure {
            exposure: 0.0,
            offset: 0.0,
            gamma: 1.0,
        },
        Adjustment::HueSaturation {
            hue: 0.0,
            saturation: 0.0,
            lightness: 0.0,
        },
        Adjustment::Levels {
            input_black: 0.0,
            input_white: 1.0,
            gamma: 1.0,
            output_black: 0.0,
            output_white: 1.0,
            channels: [LEVELS_IDENTITY; 3],
        },
        Adjustment::BrightnessContrast {
            brightness: 0.0,
            contrast: 0.0,
        },
        Adjustment::Vibrance {
            vibrance: 0.0,
            saturation: 0.0,
        },
        Adjustment::Invert,
        Adjustment::Posterize { levels: 4.0 },
        Adjustment::Threshold {
            level: 128.0 / 255.0,
        },
        Adjustment::BlackWhite {
            weights: [40.0, 60.0, 40.0, 60.0, 20.0, 80.0],
            tint: false,
            tint_hue: 42.0,
            tint_saturation: 20.0,
        },
        Adjustment::ColorBalance {
            shadows: [0.0; 3],
            midtones: [0.0; 3],
            highlights: [0.0; 3],
            preserve_luminosity: true,
        },
        // Photoshop's Warming Filter (85).
        Adjustment::PhotoFilter {
            color: [236.0 / 255.0, 138.0 / 255.0, 0.0],
            density: 25.0,
            preserve_luminosity: true,
        },
        Adjustment::ChannelMixer {
            red: [100.0, 0.0, 0.0, 0.0],
            green: [0.0, 100.0, 0.0, 0.0],
            blue: [0.0, 0.0, 100.0, 0.0],
            monochrome: false,
        },
        Adjustment::Curves {
            rgb: Curve::IDENTITY,
            red: Curve::IDENTITY,
            green: Curve::IDENTITY,
            blue: Curve::IDENTITY,
        },
    ];

    /// Stable identifier (files, IPC).
    pub fn id(&self) -> &'static str {
        match self {
            Adjustment::Exposure { .. } => "exposure",
            Adjustment::HueSaturation { .. } => "hueSaturation",
            Adjustment::Levels { .. } => "levels",
            Adjustment::BrightnessContrast { .. } => "brightnessContrast",
            Adjustment::Vibrance { .. } => "vibrance",
            Adjustment::Invert => "invert",
            Adjustment::Posterize { .. } => "posterize",
            Adjustment::Threshold { .. } => "threshold",
            Adjustment::BlackWhite { .. } => "blackWhite",
            Adjustment::ColorBalance { .. } => "colorBalance",
            Adjustment::PhotoFilter { .. } => "photoFilter",
            Adjustment::ChannelMixer { .. } => "channelMixer",
            Adjustment::Curves { .. } => "curves",
        }
    }

    /// The adjustment with this identifier, with its default parameters ([`Self::DEFAULTS`]).
    pub fn defaults(id: &str) -> Option<Adjustment> {
        Self::DEFAULTS.into_iter().find(|a| a.id() == id)
    }

    /// Number shared with the GPU renderer.
    pub fn index(&self) -> u32 {
        match self {
            Adjustment::Exposure { .. } => 0,
            Adjustment::HueSaturation { .. } => 1,
            Adjustment::Levels { .. } => 2,
            Adjustment::BrightnessContrast { .. } => 3,
            Adjustment::Vibrance { .. } => 4,
            Adjustment::Invert => 5,
            Adjustment::Posterize { .. } => 6,
            Adjustment::Threshold { .. } => 7,
            Adjustment::BlackWhite { .. } => 8,
            Adjustment::ColorBalance { .. } => 9,
            Adjustment::PhotoFilter { .. } => 10,
            Adjustment::ChannelMixer { .. } => 11,
            Adjustment::Curves { .. } => 12,
        }
    }

    /// How many of [`Self::params`] the adjustment uses.
    pub fn param_count(&self) -> usize {
        match self {
            Adjustment::Exposure { .. } | Adjustment::HueSaturation { .. } => 3,
            Adjustment::Levels { .. } => 20,
            Adjustment::BrightnessContrast { .. } | Adjustment::Vibrance { .. } => 2,
            Adjustment::Invert | Adjustment::Curves { .. } => 0,
            Adjustment::Posterize { .. } | Adjustment::Threshold { .. } => 1,
            Adjustment::BlackWhite { .. } => 9,
            Adjustment::ColorBalance { .. } => 10,
            Adjustment::PhotoFilter { .. } => 5,
            Adjustment::ChannelMixer { .. } => 13,
        }
    }

    /// The parameters in a fixed order (files, IPC, GPU): unused ones are 0, flags 0 or 1.
    pub fn params(&self) -> [f32; PARAM_COUNT] {
        let mut p = [0.0; PARAM_COUNT];
        let used = self.used_params();
        p[..used.len()].copy_from_slice(&used);
        p
    }

    /// The parameters the adjustment uses ([`Self::param_count`] of them).
    fn used_params(&self) -> Vec<f32> {
        let flag = |b: bool| if b { 1.0 } else { 0.0 };
        match *self {
            Adjustment::Exposure {
                exposure,
                offset,
                gamma,
            } => vec![exposure, offset, gamma],
            Adjustment::HueSaturation {
                hue,
                saturation,
                lightness,
            } => vec![hue, saturation, lightness],
            Adjustment::Levels {
                input_black,
                input_white,
                gamma,
                output_black,
                output_white,
                channels,
            } => [input_black, input_white, gamma, output_black, output_white]
                .into_iter()
                .chain(channels.into_iter().flatten())
                .collect(),
            Adjustment::BrightnessContrast {
                brightness,
                contrast,
            } => vec![brightness, contrast],
            Adjustment::Vibrance {
                vibrance,
                saturation,
            } => vec![vibrance, saturation],
            Adjustment::Invert | Adjustment::Curves { .. } => vec![],
            Adjustment::Posterize { levels } => vec![levels],
            Adjustment::Threshold { level } => vec![level],
            Adjustment::BlackWhite {
                weights,
                tint,
                tint_hue,
                tint_saturation,
            } => [&weights[..], &[flag(tint), tint_hue, tint_saturation]].concat(),
            Adjustment::ColorBalance {
                shadows,
                midtones,
                highlights,
                preserve_luminosity,
            } => [
                &shadows[..],
                &midtones,
                &highlights,
                &[flag(preserve_luminosity)],
            ]
            .concat(),
            Adjustment::PhotoFilter {
                color,
                density,
                preserve_luminosity,
            } => [&color[..], &[density, flag(preserve_luminosity)]].concat(),
            Adjustment::ChannelMixer {
                red,
                green,
                blue,
                monochrome,
            } => [&red[..], &green, &blue, &[flag(monochrome)]].concat(),
        }
    }

    /// The adjustment `id` with `params` (as [`Self::params`] orders them; missing ones are 0,
    /// flags are set when not 0; Levels given five values leave the channels unchanged). `None` for an unknown id or more than [`PARAM_COUNT`] values.
    pub fn from_params(id: &str, params: &[f32]) -> Option<Adjustment> {
        if params.len() > PARAM_COUNT {
            return None;
        }
        let mut p = [0.0; PARAM_COUNT];
        p[..params.len()].copy_from_slice(params);
        let flag = |v: f32| v != 0.0;
        let array = |at: usize| -> [f32; 3] { [p[at], p[at + 1], p[at + 2]] };
        let row = |at: usize| -> [f32; 4] { [p[at], p[at + 1], p[at + 2], p[at + 3]] };
        Some(match Self::defaults(id)? {
            Adjustment::Exposure { .. } => Adjustment::Exposure {
                exposure: p[0],
                offset: p[1],
                gamma: p[2],
            },
            Adjustment::HueSaturation { .. } => Adjustment::HueSaturation {
                hue: p[0],
                saturation: p[1],
                lightness: p[2],
            },
            // Files and requests from before the channels have the composite's five only.
            Adjustment::Levels { .. } => Adjustment::Levels {
                input_black: p[0],
                input_white: p[1],
                gamma: p[2],
                output_black: p[3],
                output_white: p[4],
                channels: if params.len() > 5 {
                    [0, 1, 2].map(|i| {
                        let at = 5 + 5 * i;
                        [p[at], p[at + 1], p[at + 2], p[at + 3], p[at + 4]]
                    })
                } else {
                    [LEVELS_IDENTITY; 3]
                },
            },
            Adjustment::BrightnessContrast { .. } => Adjustment::BrightnessContrast {
                brightness: p[0],
                contrast: p[1],
            },
            Adjustment::Vibrance { .. } => Adjustment::Vibrance {
                vibrance: p[0],
                saturation: p[1],
            },
            Adjustment::Invert => Adjustment::Invert,
            Adjustment::Posterize { .. } => Adjustment::Posterize { levels: p[0] },
            Adjustment::Threshold { .. } => Adjustment::Threshold { level: p[0] },
            Adjustment::BlackWhite { .. } => Adjustment::BlackWhite {
                weights: [p[0], p[1], p[2], p[3], p[4], p[5]],
                tint: flag(p[6]),
                tint_hue: p[7],
                tint_saturation: p[8],
            },
            Adjustment::ColorBalance { .. } => Adjustment::ColorBalance {
                shadows: array(0),
                midtones: array(3),
                highlights: array(6),
                preserve_luminosity: flag(p[9]),
            },
            Adjustment::PhotoFilter { .. } => Adjustment::PhotoFilter {
                color: array(0),
                density: p[3],
                preserve_luminosity: flag(p[4]),
            },
            Adjustment::ChannelMixer { .. } => Adjustment::ChannelMixer {
                red: row(0),
                green: row(4),
                blue: row(8),
                monochrome: flag(p[12]),
            },
            // Curves are set with Adjustment::Curves itself.
            curves @ Adjustment::Curves { .. } => curves,
        })
    }

    /// Whether the parameters are in Photoshop's ranges (edits and files refuse others).
    pub fn is_valid(&self) -> bool {
        let within = |v: f32, lo: f32, hi: f32| v.is_finite() && (lo..=hi).contains(&v);
        match *self {
            Adjustment::Exposure {
                exposure,
                offset,
                gamma,
            } => {
                within(exposure, -20.0, 20.0)
                    && within(offset, -0.5, 0.5)
                    && within(gamma, 0.01, 9.99)
            }
            Adjustment::HueSaturation {
                hue,
                saturation,
                lightness,
            } => {
                within(hue, -180.0, 180.0)
                    && within(saturation, -100.0, 100.0)
                    && within(lightness, -100.0, 100.0)
            }
            Adjustment::Levels {
                input_black,
                input_white,
                gamma,
                output_black,
                output_white,
                channels,
            } => {
                let valid = |[ib, iw, g, ob, ow]: LevelsChannel| {
                    within(ib, 0.0, 1.0)
                        && within(iw, 0.0, 1.0)
                        && ib < iw
                        && within(g, 0.01, 9.99)
                        && within(ob, 0.0, 1.0)
                        && within(ow, 0.0, 1.0)
                };
                valid([input_black, input_white, gamma, output_black, output_white])
                    && channels.into_iter().all(valid)
            }
            Adjustment::BrightnessContrast {
                brightness,
                contrast,
            } => within(brightness, -150.0, 150.0) && within(contrast, -50.0, 100.0),
            Adjustment::Vibrance {
                vibrance,
                saturation,
            } => within(vibrance, -100.0, 100.0) && within(saturation, -100.0, 100.0),
            Adjustment::Invert => true,
            Adjustment::Posterize { levels } => within(levels, 2.0, 255.0),
            Adjustment::Threshold { level } => within(level, 0.0, 1.0),
            Adjustment::BlackWhite {
                weights,
                tint_hue,
                tint_saturation,
                ..
            } => {
                weights.iter().all(|&w| within(w, -200.0, 300.0))
                    && within(tint_hue, 0.0, 360.0)
                    && within(tint_saturation, 0.0, 100.0)
            }
            Adjustment::ColorBalance {
                shadows,
                midtones,
                highlights,
                ..
            } => [shadows, midtones, highlights]
                .iter()
                .flatten()
                .all(|&v| within(v, -100.0, 100.0)),
            Adjustment::PhotoFilter { color, density, .. } => {
                color.iter().all(|&c| within(c, 0.0, 1.0)) && within(density, 0.0, 100.0)
            }
            Adjustment::ChannelMixer {
                red, green, blue, ..
            } => [red, green, blue]
                .iter()
                .flatten()
                .all(|&v| within(v, -200.0, 200.0)),
            // Valid by construction (Curve::new).
            Adjustment::Curves { .. } => true,
        }
    }

    /// Curves' curves: composite, red, green, blue.
    pub fn curves(&self) -> Option<[Curve; 4]> {
        match *self {
            Adjustment::Curves {
                rgb,
                red,
                green,
                blue,
            } => Some([rgb, red, green, blue]),
            _ => None,
        }
    }

    /// Ready to apply to many colors: Curves' lookup tables computed once.
    pub fn prepare(&self) -> Prepared {
        let luts = match self.curves() {
            Some(curves) => curves.iter().map(Curve::lut).collect(),
            None => Vec::new(),
        };
        Prepared {
            adjustment: *self,
            luts,
        }
    }

    /// Runs in linear light whatever the document's blend space (ADR 0020).
    pub fn is_linear(&self) -> bool {
        matches!(
            self,
            Adjustment::Exposure { .. } | Adjustment::PhotoFilter { .. }
        )
    }

    /// The adjusted straight color (for one color; [`Self::prepare`] for many).
    pub fn apply(&self, c: [f64; 3]) -> [f64; 3] {
        self.prepare().apply(c)
    }

    /// The adjusted straight color, with the lookup tables of [`Self::prepare`].
    fn apply_with(&self, c: [f64; 3], luts: &[Vec<f32>]) -> [f64; 3] {
        match *self {
            Adjustment::Exposure {
                exposure,
                offset,
                gamma,
            } => {
                let (scale, offset) = (f64::from(exposure).exp2(), f64::from(offset));
                let inverse = 1.0 / f64::from(gamma);
                c.map(|v| {
                    let v = v * scale + offset;
                    v.abs().powf(inverse).copysign(v)
                })
            }
            Adjustment::HueSaturation {
                hue,
                saturation,
                lightness,
            } => {
                let shifted = shift_hue(c, f64::from(hue));
                let saturated = saturate(shifted, f64::from(saturation) / 100.0);
                let l = f64::from(lightness) / 100.0;
                saturated.map(|v| {
                    if l < 0.0 {
                        v * (1.0 + l)
                    } else {
                        v + (1.0 - v) * l
                    }
                })
            }
            Adjustment::Levels {
                input_black,
                input_white,
                gamma,
                output_black,
                output_white,
                channels,
            } => {
                let mut c = c;
                for (v, channel) in c.iter_mut().zip(channels) {
                    if channel != LEVELS_IDENTITY {
                        *v = levels(*v, channel);
                    }
                }
                let composite = [input_black, input_white, gamma, output_black, output_white];
                c.map(|v| levels(v, composite))
            }
            Adjustment::BrightnessContrast {
                brightness,
                contrast,
            } => {
                // Brightness: a gamma (100 → v^0.5); contrast: an S-curve of exponent
                // 1 + contrast / 100 on each half, 0, ½ and 1 fixed.
                let gamma = (-f64::from(brightness) / 100.0).exp2();
                let k = 1.0 + f64::from(contrast) / 100.0;
                c.map(|v| {
                    let v = v.abs().powf(gamma).copysign(v);
                    if !(0.0..=1.0).contains(&v) {
                        v
                    } else if v < 0.5 {
                        0.5 * (2.0 * v).powf(k)
                    } else {
                        1.0 - 0.5 * (2.0 * (1.0 - v)).powf(k)
                    }
                })
            }
            Adjustment::Vibrance {
                vibrance,
                saturation,
            } => {
                let (max, min) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
                // Dull colors (little chroma) get the most of it.
                let dullness = 1.0 - (max - min).clamp(0.0, 1.0);
                let vibrant = saturate(c, f64::from(vibrance) / 100.0 * dullness);
                saturate(vibrant, f64::from(saturation) / 100.0)
            }
            Adjustment::Invert => c.map(|v| 1.0 - v),
            Adjustment::Posterize { levels } => {
                let n = f64::from(levels).round();
                c.map(|v| ((v.clamp(0.0, 1.0) * n).floor().min(n - 1.0)) / (n - 1.0))
            }
            Adjustment::Threshold { level } => {
                let luminance = LUMA[0] * c[0] + LUMA[1] * c[1] + LUMA[2] * c[2];
                let v = if luminance >= f64::from(level) {
                    1.0
                } else {
                    0.0
                };
                [v; 3]
            }
            Adjustment::BlackWhite {
                weights,
                tint,
                tint_hue,
                tint_saturation,
            } => {
                let gray = black_and_white(c, weights.map(|w| f64::from(w) / 100.0));
                if tint {
                    from_hsl(
                        f64::from(tint_hue),
                        f64::from(tint_saturation) / 100.0,
                        gray,
                    )
                } else {
                    [gray; 3]
                }
            }
            Adjustment::ColorBalance {
                shadows,
                midtones,
                highlights,
                preserve_luminosity,
            } => {
                let lightness = hsl_lightness(c);
                // Each range's share at this lightness (GIMP's masks): shadows fade out above
                // ⅓, highlights in above ⅔, midtones in between.
                let (a, b, scale) = (0.25, 0.333, 0.7);
                let weights = [
                    ((lightness - b) / -a + 0.5).clamp(0.0, 1.0) * scale,
                    ((lightness - b) / a + 0.5).clamp(0.0, 1.0)
                        * ((lightness + b - 1.0) / -a + 0.5).clamp(0.0, 1.0)
                        * scale,
                    ((lightness + b - 1.0) / a + 0.5).clamp(0.0, 1.0) * scale,
                ];
                let mut out = c;
                for (channel, v) in out.iter_mut().enumerate() {
                    for (range, weight) in [shadows, midtones, highlights].iter().zip(weights) {
                        *v += f64::from(range[channel]) / 100.0 * weight;
                    }
                }
                if preserve_luminosity {
                    set_hsl_lightness(out, lightness)
                } else {
                    out
                }
            }
            Adjustment::PhotoFilter {
                color,
                density,
                preserve_luminosity,
            } => {
                let (to_srgb, from_srgb) = &*SRGB_MATRICES;
                let c = mat_vec(to_srgb, c);
                let filter = filter_color(color);
                let d = f64::from(density) / 100.0;
                let mut out = [0, 1, 2].map(|i| c[i] + (c[i] * filter[i] - c[i]) * d);
                let y = |c: [f64; 3]| (0..3).map(|i| SRGB_LUMA[i] * c[i]).sum::<f64>();
                let (before, after) = (y(c), y(out));
                if preserve_luminosity && after > 0.0 {
                    out = out.map(|v| v * before / after);
                    // Above white, toward gray at the same luminance rather than clipped later
                    // (as the W3C Color mode does); HDR luminances are left as they are.
                    let max = out[0].max(out[1]).max(out[2]);
                    // (White's luminance may round a little above 1.)
                    if max > 1.0 && before <= 1.0 + 1e-6 {
                        let top = before.max(1.0);
                        out = out.map(|v| before + (v - before) * (top - before) / (max - before));
                    }
                }
                mat_vec(from_srgb, out)
            }
            Adjustment::ChannelMixer {
                red,
                green,
                blue,
                monochrome,
            } => {
                let mix = |row: [f32; 4]| {
                    let w = row.map(|w| f64::from(w) / 100.0);
                    w[0] * c[0] + w[1] * c[1] + w[2] * c[2] + w[3]
                };
                if monochrome {
                    [mix(red); 3]
                } else {
                    [mix(red), mix(green), mix(blue)]
                }
            }
            Adjustment::Curves { .. } => {
                // Composite, red, green, blue.
                let rgb = &luts[0];
                [0, 1, 2].map(|i| lookup(rgb, lookup(&luts[i + 1], c[i])))
            }
        }
    }
}

/// An adjustment with what applying it needs computed once ([`Adjustment::prepare`]).
#[derive(Debug, Clone)]
pub struct Prepared {
    adjustment: Adjustment,
    /// Curves' lookup tables (composite, red, green, blue); empty for other adjustments.
    luts: Vec<Vec<f32>>,
}

impl Prepared {
    pub fn adjustment(&self) -> &Adjustment {
        &self.adjustment
    }

    /// The adjusted straight color.
    pub fn apply(&self, c: [f64; 3]) -> [f64; 3] {
        self.adjustment.apply_with(c, &self.luts)
    }
}

/// The luminance weights of Threshold (Rec. 601, on the adjusted values, as Photoshop).
pub const LUMA: [f64; 3] = [0.299, 0.587, 0.114];

/// The luminance (`Y`) of each sRGB primary.
pub const SRGB_LUMA: [f64; 3] = [0.2126, 0.7152, 0.0722];

/// Working space → linear sRGB, and back (Photo Filter's primaries).
static SRGB_MATRICES: LazyLock<(Mat3, Mat3)> = LazyLock::new(|| {
    (
        WORKING_SPACE.matrix_to(&ColorSpace::LINEAR_SRGB),
        ColorSpace::LINEAR_SRGB.matrix_to(&WORKING_SPACE),
    )
});

/// A Photo Filter color (sRGB-encoded) as the linear sRGB values it multiplies.
pub fn filter_color(color: [f32; 3]) -> [f64; 3] {
    color.map(|v| f64::from(TransferFunction::Srgb.decode(v)))
}

/// Black & White's gray: the smallest component, plus what the two others add, weighted by
/// the hue family they make (`weights`: reds, yellows, greens, cyans, blues, magentas): the
/// largest component's primary for the part above the middle one, the secondary of the two
/// largest for the part between the smallest and the middle one.
fn black_and_white(c: [f64; 3], weights: [f64; 6]) -> f64 {
    let mut order = [0, 1, 2];
    order.sort_by(|&i, &j| c[j].total_cmp(&c[i]));
    let [max, mid, min] = order;
    // Primaries at even indices (red 0, green 2, blue 4), secondaries between them.
    let primary = weights[2 * max];
    let secondary = weights[match (max.min(mid), max.max(mid)) {
        (0, 1) => 1,
        (1, 2) => 3,
        _ => 5,
    }];
    c[min] + (c[mid] - c[min]) * secondary + (c[max] - c[mid]) * primary
}

/// HSL lightness: the mean of the smallest and largest components.
fn hsl_lightness(c: [f64; 3]) -> f64 {
    (c[0].max(c[1]).max(c[2]) + c[0].min(c[1]).min(c[2])) / 2.0
}

/// The color of hue `degrees`, HSL saturation `s` and lightness `l`.
fn from_hsl(degrees: f64, s: f64, l: f64) -> [f64; 3] {
    let chroma = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let gray = [l - chroma / 2.0; 3];
    if chroma <= 0.0 {
        return [l; 3];
    }
    // The fully saturated hue, shifted and scaled to the chroma.
    let hue = shift_hue([1.0, 0.0, 0.0], degrees);
    [0, 1, 2].map(|i| gray[i] + hue[i] * chroma)
}

/// `c` with HSL lightness `l`, keeping its hue and HSL saturation.
fn set_hsl_lightness(c: [f64; 3], l: f64) -> [f64; 3] {
    let (max, min) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
    let chroma = max - min;
    let room = 1.0 - (2.0 * hsl_lightness(c) - 1.0).abs();
    if chroma <= 0.0 || room <= 0.0 {
        return [l; 3];
    }
    let s = (chroma / room).min(1.0);
    let target = (1.0 - (2.0 * l - 1.0).abs()) * s;
    // The hue as components in [0, 1] (smallest 0, largest 1), scaled to the new chroma.
    let lo = l - target / 2.0;
    c.map(|v| lo + (v - min) / chroma * target)
}

/// Rotate the hue by `degrees`, keeping each color's smallest and largest component (the hue of
/// HSL and HSV, around the RGB hexagon); works for values outside `[0, 1]` too.
fn shift_hue([r, g, b]: [f64; 3], degrees: f64) -> [f64; 3] {
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let chroma = max - min;
    if chroma <= 0.0 || degrees == 0.0 {
        return [r, g, b];
    }
    let h = if max == r {
        ((g - b) / chroma).rem_euclid(6.0)
    } else if max == g {
        (b - r) / chroma + 2.0
    } else {
        (r - g) / chroma + 4.0
    };
    let h = (h + degrees / 60.0).rem_euclid(6.0);
    let x = chroma * (1.0 - ((h % 2.0) - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    [r + min, g + min, b + min]
}

/// Scale the HSL saturation by `1 + amount` (`amount` in −1…1; −1 is gray), at most to full
/// saturation, keeping the HSL lightness. Colors outside the HSL range keep their saturation.
fn saturate(c: [f64; 3], amount: f64) -> [f64; 3] {
    let (max, min) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
    let chroma = max - min;
    let l = (max + min) / 2.0;
    let room = 1.0 - (2.0 * l - 1.0).abs();
    if chroma <= 0.0 || room <= 0.0 || amount == 0.0 {
        return c;
    }
    let s = chroma / room;
    let target = (s * (1.0 + amount)).clamp(0.0, 1.0f64.max(s));
    let k = target / s;
    c.map(|v| l + (v - l) * k)
}

/// `v` through one set of Levels settings.
fn levels(v: f64, [ib, iw, gamma, ob, ow]: LevelsChannel) -> f64 {
    let (ib, iw) = (f64::from(ib), f64::from(iw));
    let (ob, ow) = (f64::from(ob), f64::from(ow));
    let t = ((v - ib) / (iw - ib))
        .clamp(0.0, 1.0)
        .powf(1.0 / f64::from(gamma));
    ob + t * (ow - ob)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f64; 3], b: [f64; 3]) -> bool {
        // Parameters are f32: about 1e-7 of rounding.
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6)
    }

    #[test]
    fn default_adjustments_are_valid_and_the_neutral_ones_change_nothing() {
        let c = [0.2, 0.55, 0.9];
        for a in Adjustment::DEFAULTS {
            assert!(a.is_valid(), "{a:?}");
            assert_eq!(Adjustment::from_params(a.id(), &a.params()), Some(a));
            assert!(a.params()[a.param_count()..].iter().all(|&p| p == 0.0));
            let changes = matches!(
                a,
                Adjustment::Invert
                    | Adjustment::Posterize { .. }
                    | Adjustment::Threshold { .. }
                    | Adjustment::BlackWhite { .. }
                    | Adjustment::PhotoFilter { .. }
            );
            assert_eq!(!close(a.apply(c), c), changes, "{a:?}");
        }
        assert_eq!(Adjustment::defaults("selectiveColor"), None);
    }

    #[test]
    fn brightness_and_contrast_keep_black_and_white() {
        let a = Adjustment::BrightnessContrast {
            brightness: 100.0,
            contrast: 50.0,
        };
        assert!(close(
            a.apply([0.0, 1.0, 0.5]).map(|v| v.clamp(0.0, 1.0))[..2]
                .try_into()
                .map(|x: [f64; 2]| [x[0], x[1], 0.0])
                .unwrap(),
            [0.0, 1.0, 0.0]
        ));
        // Brighter: middle gray rises; more contrast: dark gets darker around a fixed middle.
        let bright = Adjustment::BrightnessContrast {
            brightness: 100.0,
            contrast: 0.0,
        };
        assert!(bright.apply([0.25; 3])[0] > 0.25);
        let contrast = Adjustment::BrightnessContrast {
            brightness: 0.0,
            contrast: 100.0,
        };
        assert!(close(
            contrast.apply([0.5, 0.25, 0.75]),
            [0.5, 0.125, 0.875]
        ));
    }

    #[test]
    fn vibrance_favors_dull_colors_and_invert_posterize_threshold_quantize() {
        let v = Adjustment::Vibrance {
            vibrance: 100.0,
            saturation: 0.0,
        };
        let dull = [0.55, 0.45, 0.45];
        let vivid = [0.9, 0.1, 0.1];
        let gain = |c: [f64; 3], out: [f64; 3]| (out[0] - out[1]) / (c[0] - c[1]);
        assert!(gain(dull, v.apply(dull)) > gain(vivid, v.apply(vivid)));
        assert!(close(
            Adjustment::Invert.apply([0.2, 0.5, 1.0]),
            [0.8, 0.5, 0.0]
        ));
        let p = Adjustment::Posterize { levels: 2.0 };
        assert!(close(p.apply([0.2, 0.6, 1.0]), [0.0, 1.0, 1.0]));
        let t = Adjustment::Threshold { level: 0.5 };
        assert!(close(t.apply([0.9, 0.9, 0.9]), [1.0; 3]));
        assert!(close(t.apply([0.1, 0.9, 0.1]), [1.0; 3]));
        assert!(close(t.apply([0.9, 0.1, 0.1]), [0.0; 3]));
    }

    #[test]
    fn exposure_doubles_per_stop_in_linear_light() {
        let a = Adjustment::Exposure {
            exposure: 1.0,
            offset: 0.0,
            gamma: 1.0,
        };
        assert!(close(a.apply([0.1, 0.25, 2.0]), [0.2, 0.5, 4.0]));
        assert!(a.is_linear());
        let g = Adjustment::Exposure {
            exposure: 0.0,
            offset: 0.0,
            gamma: 2.0,
        };
        assert!(close(g.apply([0.25, 0.0, 1.0]), [0.5, 0.0, 1.0]));
    }

    #[test]
    fn hue_turns_around_the_color_wheel_and_saturation_toward_gray() {
        let red = [1.0, 0.0, 0.0];
        let turn = |degrees: f32| Adjustment::HueSaturation {
            hue: degrees,
            saturation: 0.0,
            lightness: 0.0,
        };
        assert!(close(turn(120.0).apply(red), [0.0, 1.0, 0.0]));
        assert!(close(turn(-120.0).apply(red), [0.0, 0.0, 1.0]));
        assert!(close(turn(60.0).apply(red), [1.0, 1.0, 0.0]));
        let gray = Adjustment::HueSaturation {
            hue: 0.0,
            saturation: -100.0,
            lightness: 0.0,
        };
        assert!(close(gray.apply([0.8, 0.4, 0.2]), [0.5, 0.5, 0.5]));
        let full = Adjustment::HueSaturation {
            hue: 0.0,
            saturation: 100.0,
            lightness: 0.0,
        };
        // Twice as saturated, the same lightness; never beyond full saturation.
        assert!(close(full.apply([0.6, 0.4, 0.4]), [0.7, 0.3, 0.3]));
        assert!(close(full.apply([0.9, 0.1, 0.1]), [1.0, 0.0, 0.0]));
        let light = Adjustment::HueSaturation {
            hue: 0.0,
            saturation: 0.0,
            lightness: 50.0,
        };
        assert!(close(light.apply([0.0, 0.5, 1.0]), [0.5, 0.75, 1.0]));
    }

    #[test]
    fn levels_stretch_the_inputs_to_the_outputs() {
        let a = Adjustment::Levels {
            input_black: 0.2,
            input_white: 0.6,
            gamma: 1.0,
            output_black: 0.0,
            output_white: 1.0,
            channels: [crate::adjust::LEVELS_IDENTITY; 3],
        };
        assert!(close(a.apply([0.1, 0.4, 0.9]), [0.0, 0.5, 1.0]));
        let invalid = Adjustment::Levels {
            input_black: 0.6,
            input_white: 0.2,
            gamma: 1.0,
            output_black: 0.0,
            output_white: 1.0,
            channels: [crate::adjust::LEVELS_IDENTITY; 3],
        };
        assert!(!invalid.is_valid());
    }

    #[test]
    fn levels_channels_come_before_the_composite() {
        let channels = [
            [0.2, 0.6, 1.0, 0.0, 1.0],
            LEVELS_IDENTITY,
            [0.0, 1.0, 1.0, 0.5, 1.0],
        ];
        let a = Adjustment::Levels {
            input_black: 0.0,
            input_white: 1.0,
            gamma: 1.0,
            output_black: 0.0,
            output_white: 0.5,
            channels,
        };
        // Red stretched then halved; green only halved; blue raised then halved.
        assert!(close(a.apply([0.4, 0.4, 0.0]), [0.25, 0.2, 0.25]));
        // The parameters: the composite's five, then red, green and blue.
        let params = a.params();
        assert_eq!(&params[..5], &[0.0, 1.0, 1.0, 0.0, 0.5]);
        assert_eq!(&params[5..10], &channels[0]);
        assert_eq!(&params[15..20], &channels[2]);
        assert_eq!(Adjustment::from_params("levels", &params), Some(a));
        // Five values (files and requests from before the channels): channels unchanged.
        let five = Adjustment::from_params("levels", &params[..5]).unwrap();
        assert!(
            matches!(five, Adjustment::Levels { channels, .. } if channels == [LEVELS_IDENTITY; 3])
        );
        // A channel out of range makes the whole adjustment invalid.
        let mut wrong = params;
        wrong[10..15].copy_from_slice(&[0.7, 0.3, 1.0, 0.0, 1.0]);
        let invalid = Adjustment::from_params("levels", &wrong).unwrap();
        assert!(!invalid.is_valid());
    }

    #[test]
    fn parameters_round_trip_and_missing_ones_are_zero() {
        let mixer = Adjustment::ChannelMixer {
            red: [80.0, 20.0, 0.0, -10.0],
            green: [0.0, 100.0, 0.0, 0.0],
            blue: [10.0, 10.0, 80.0, 5.0],
            monochrome: true,
        };
        assert_eq!(
            Adjustment::from_params("channelMixer", &mixer.params()),
            Some(mixer)
        );
        // Files of schema 0.7 hold five values.
        assert_eq!(
            Adjustment::from_params("posterize", &[6.0, 0.0, 0.0, 0.0, 0.0]),
            Some(Adjustment::Posterize { levels: 6.0 })
        );
        assert_eq!(Adjustment::from_params("invert", &[0.0; 21]), None);
    }

    #[test]
    fn black_and_white_weighs_each_hue_family() {
        let a = Adjustment::defaults("blackWhite").unwrap();
        // Primaries and secondaries get their weight; grays stay; mixtures blend.
        assert!(close(a.apply([1.0, 0.0, 0.0]), [0.4; 3]));
        assert!(close(a.apply([1.0, 1.0, 0.0]), [0.6; 3]));
        assert!(close(a.apply([0.0, 0.0, 1.0]), [0.2; 3]));
        assert!(close(a.apply([1.0, 0.0, 1.0]), [0.8; 3]));
        assert!(close(a.apply([0.3; 3]), [0.3; 3]));
        // Orange: 0.5 of yellow above black, 0.5 of red above that.
        assert!(close(a.apply([1.0, 0.5, 0.0]), [0.5; 3]));
        let Adjustment::BlackWhite { weights, .. } = a else {
            unreachable!()
        };
        let tinted = Adjustment::BlackWhite {
            weights,
            tint: true,
            tint_hue: 120.0,
            tint_saturation: 100.0,
        };
        // Green at the gray's lightness.
        assert!(close(tinted.apply([0.25; 3]), [0.0, 0.5, 0.0]));
    }

    #[test]
    fn color_balance_moves_each_range_and_can_keep_the_lightness() {
        let reds = |range: usize, preserve: bool| {
            let mut ranges = [[0.0; 3]; 3];
            ranges[range][0] = 100.0;
            Adjustment::ColorBalance {
                shadows: ranges[0],
                midtones: ranges[1],
                highlights: ranges[2],
                preserve_luminosity: preserve,
            }
        };
        let (dark, mid, light) = ([0.1; 3], [0.5; 3], [0.9; 3]);
        // Shadows move dark colors, not light ones; highlights the reverse.
        assert!(reds(0, false).apply(dark)[0] > 0.5);
        assert!(close(reds(0, false).apply(light), light));
        assert!(reds(2, false).apply(light)[0] > 0.9);
        assert!(close(reds(2, false).apply(dark), dark));
        assert!(reds(1, false).apply(mid)[0] > 0.5);
        // Keeping the lightness: redder, as light as before.
        let kept = reds(1, true).apply(mid);
        assert!(kept[0] > kept[1]);
        assert!((hsl_lightness(kept) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn photo_filter_multiplies_in_linear_light_and_can_keep_the_luminance() {
        let full = |preserve: bool| Adjustment::PhotoFilter {
            color: [1.0, 0.0, 0.0],
            density: 100.0,
            preserve_luminosity: preserve,
        };
        assert!(full(false).is_linear());
        let white = [1.0; 3];
        // A red filter over white leaves sRGB red.
        let red = mat_vec(&SRGB_MATRICES.1, [1.0, 0.0, 0.0]);
        assert!(close(full(false).apply(white), red));
        let (to_srgb, _) = &*SRGB_MATRICES;
        let y = |c: [f64; 3]| {
            let s = mat_vec(to_srgb, c);
            (0..3).map(|i| SRGB_LUMA[i] * s[i]).sum::<f64>()
        };
        // Keeping the luminance never goes above white: white stays white, a light color keeps
        // its luminance within range.
        assert!(close(full(true).apply(white), white));
        let light = [0.9, 0.8, 0.7];
        let kept = full(true).apply(light);
        assert!((y(kept) - y(light)).abs() < 1e-9);
        // Within sRGB, as a filter over an sRGB color should be.
        let s = mat_vec(to_srgb, kept);
        assert!(
            s.iter().all(|&v| (-1e-9..=1.0 + 1e-9).contains(&v)),
            "{s:?}"
        );
        assert!(s[0] > s[2]);
        let none = Adjustment::PhotoFilter {
            color: [1.0, 0.0, 0.0],
            density: 0.0,
            preserve_luminosity: false,
        };
        assert!(close(none.apply([0.2, 0.4, 0.6]), [0.2, 0.4, 0.6]));
    }

    #[test]
    fn srgb_luma_is_the_y_of_the_srgb_primaries() {
        let y = ColorSpace::SRGB.primaries.to_xyz()[1];
        for (a, b) in y.iter().zip(SRGB_LUMA) {
            assert!((a - b).abs() < 1e-4, "{y:?}");
        }
    }

    #[test]
    fn curves_map_each_channel_then_all_through_the_composite() {
        let curve = |points: &[[u8; 2]]| Curve::new(points).unwrap();
        let a = Adjustment::Curves {
            rgb: curve(&[[0, 0], [255, 128]]),
            red: curve(&[[0, 255], [255, 0]]),
            green: Curve::IDENTITY,
            blue: Curve::IDENTITY,
        };
        // Red inverted, then everything halved.
        let half = 128.0 / 255.0;
        assert!(close(a.apply([0.0, 1.0, 0.5]), [half, half, half * 0.5]));
        assert_eq!(a.param_count(), 0);
        assert!(a.is_valid());
        // Close to the exact curves (the lookup tables are interpolated).
        let s = curve(&[[0, 0], [90, 150], [255, 255]]);
        let b = Adjustment::Curves {
            rgb: s,
            red: Curve::IDENTITY,
            green: Curve::IDENTITY,
            blue: Curve::IDENTITY,
        };
        let out = b.prepare().apply([0.2, 0.4, 0.8]);
        for (o, x) in out.iter().zip([0.2, 0.4, 0.8]) {
            assert!((o - s.value(x)).abs() < 1e-4);
        }
    }

    #[test]
    fn channel_mixer_sums_weighted_channels() {
        let swap = Adjustment::ChannelMixer {
            red: [0.0, 0.0, 100.0, 0.0],
            green: [0.0, 100.0, 0.0, 0.0],
            blue: [100.0, 0.0, 0.0, 10.0],
            monochrome: false,
        };
        assert!(close(swap.apply([0.2, 0.4, 0.6]), [0.6, 0.4, 0.3]));
        let mono = Adjustment::ChannelMixer {
            red: [40.0, 40.0, 20.0, 0.0],
            green: [0.0, 100.0, 0.0, 0.0],
            blue: [0.0, 0.0, 100.0, 0.0],
            monochrome: true,
        };
        assert!(close(mono.apply([1.0, 0.5, 0.0]), [0.6; 3]));
    }
}
