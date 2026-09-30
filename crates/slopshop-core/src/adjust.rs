//! Adjustments of adjustment layers (ADR 0020): what each one does to a straight color, with
//! Photoshop's parameters. The compositors apply them to what is below the layer, in the
//! document's blend space (`Blender::adjust`); the GPU renderer runs the same math.

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
    /// to `[output_black, output_white]`, values in `[0, 1]`.
    Levels {
        input_black: f32,
        input_white: f32,
        gamma: f32,
        output_black: f32,
        output_white: f32,
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
}

impl Adjustment {
    /// Every adjustment, with the parameters a new layer gets (Photoshop's defaults): neutral
    /// ones, except for Invert, Posterize and Threshold, which change the image by nature.
    pub const DEFAULTS: [Adjustment; 8] = [
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
        }
    }

    /// The parameters in a fixed order (files, IPC, GPU): unused ones are 0.
    pub fn params(&self) -> [f32; 5] {
        match *self {
            Adjustment::Exposure {
                exposure,
                offset,
                gamma,
            } => [exposure, offset, gamma, 0.0, 0.0],
            Adjustment::HueSaturation {
                hue,
                saturation,
                lightness,
            } => [hue, saturation, lightness, 0.0, 0.0],
            Adjustment::Levels {
                input_black,
                input_white,
                gamma,
                output_black,
                output_white,
            } => [input_black, input_white, gamma, output_black, output_white],
            Adjustment::BrightnessContrast {
                brightness,
                contrast,
            } => [brightness, contrast, 0.0, 0.0, 0.0],
            Adjustment::Vibrance {
                vibrance,
                saturation,
            } => [vibrance, saturation, 0.0, 0.0, 0.0],
            Adjustment::Invert => [0.0; 5],
            Adjustment::Posterize { levels } => [levels, 0.0, 0.0, 0.0, 0.0],
            Adjustment::Threshold { level } => [level, 0.0, 0.0, 0.0, 0.0],
        }
    }

    /// The adjustment `id` with `params` (as [`Self::params`] orders them).
    pub fn from_params(id: &str, p: [f32; 5]) -> Option<Adjustment> {
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
            Adjustment::Levels { .. } => Adjustment::Levels {
                input_black: p[0],
                input_white: p[1],
                gamma: p[2],
                output_black: p[3],
                output_white: p[4],
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
            } => {
                within(input_black, 0.0, 1.0)
                    && within(input_white, 0.0, 1.0)
                    && input_black < input_white
                    && within(gamma, 0.01, 9.99)
                    && within(output_black, 0.0, 1.0)
                    && within(output_white, 0.0, 1.0)
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
        }
    }

    /// Runs in linear light whatever the document's blend space (ADR 0020).
    pub fn is_linear(&self) -> bool {
        matches!(self, Adjustment::Exposure { .. })
    }

    /// The adjusted straight color.
    pub fn apply(&self, c: [f64; 3]) -> [f64; 3] {
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
            } => {
                let (ib, iw) = (f64::from(input_black), f64::from(input_white));
                let (ob, ow) = (f64::from(output_black), f64::from(output_white));
                let inverse = 1.0 / f64::from(gamma);
                c.map(|v| {
                    let t = ((v - ib) / (iw - ib)).clamp(0.0, 1.0).powf(inverse);
                    ob + t * (ow - ob)
                })
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
        }
    }
}

/// The luminance weights of Threshold (Rec. 601, on the adjusted values, as Photoshop).
pub const LUMA: [f64; 3] = [0.299, 0.587, 0.114];

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
            assert_eq!(Adjustment::from_params(a.id(), a.params()), Some(a));
            let changes = matches!(
                a,
                Adjustment::Invert | Adjustment::Posterize { .. } | Adjustment::Threshold { .. }
            );
            assert_eq!(!close(a.apply(c), c), changes, "{a:?}");
        }
        assert_eq!(Adjustment::defaults("curves"), None);
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
        };
        assert!(close(a.apply([0.1, 0.4, 0.9]), [0.0, 0.5, 1.0]));
        let invalid = Adjustment::Levels {
            input_black: 0.6,
            input_white: 0.2,
            gamma: 1.0,
            output_black: 0.0,
            output_white: 1.0,
        };
        assert!(!invalid.is_valid());
    }
}
