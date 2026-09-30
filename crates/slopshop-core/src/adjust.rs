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
}

impl Adjustment {
    /// Every adjustment, at its neutral parameters (changing nothing).
    pub const NEUTRAL: [Adjustment; 3] = [
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
    ];

    /// Stable identifier (files, IPC).
    pub fn id(&self) -> &'static str {
        match self {
            Adjustment::Exposure { .. } => "exposure",
            Adjustment::HueSaturation { .. } => "hueSaturation",
            Adjustment::Levels { .. } => "levels",
        }
    }

    /// The adjustment with this identifier, at its neutral parameters.
    pub fn neutral(id: &str) -> Option<Adjustment> {
        Self::NEUTRAL.into_iter().find(|a| a.id() == id)
    }

    /// Number shared with the GPU renderer.
    pub fn index(&self) -> u32 {
        match self {
            Adjustment::Exposure { .. } => 0,
            Adjustment::HueSaturation { .. } => 1,
            Adjustment::Levels { .. } => 2,
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
        }
    }

    /// The adjustment `id` with `params` (as [`Self::params`] orders them).
    pub fn from_params(id: &str, p: [f32; 5]) -> Option<Adjustment> {
        Some(match Self::neutral(id)? {
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
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f64; 3], b: [f64; 3]) -> bool {
        // Parameters are f32: about 1e-7 of rounding.
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6)
    }

    #[test]
    fn neutral_adjustments_change_nothing() {
        let c = [0.2, 0.55, 0.9];
        for a in Adjustment::NEUTRAL {
            assert!(a.is_valid(), "{a:?}");
            assert!(close(a.apply(c), c), "{a:?}");
            assert_eq!(Adjustment::from_params(a.id(), a.params()), Some(a));
        }
        assert_eq!(Adjustment::neutral("curves"), None);
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
