//! Color spaces, pixel formats and transfer functions (see ADR 0007).
//!
//! Nothing here assumes "sRGB 8-bit": every buffer is described by a [`PixelFormat`], every
//! color value knows which space it lives in, and conversions are explicit named functions.
//!
//! A [`ColorSpace`] is a set of RGB primaries with a white point, plus a transfer function.
//! Conversions between spaces are one 3×3 matrix on linear values (with Bradford chromatic
//! adaptation between white points), computed in f64 and applied unbounded: nothing is clipped
//! until the display transform.

/// Row-major 3×3 matrix, applied to column vectors.
pub type Mat3 = [[f64; 3]; 3];

pub const IDENTITY: Mat3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

pub fn mat_mul(a: &Mat3, b: &Mat3) -> Mat3 {
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    out
}

pub fn mat_vec(m: &Mat3, v: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2])
}

/// Inverse of a 3×3 matrix, `None` if singular.
pub fn mat_inverse(m: &Mat3) -> Option<Mat3> {
    let [[a, b, c], [d, e, f], [g, h, i]] = *m;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    if det.abs() < 1e-12 || !det.is_finite() {
        return None;
    }
    let inv = 1.0 / det;
    Some([
        [
            (e * i - f * h) * inv,
            (c * h - b * i) * inv,
            (b * f - c * e) * inv,
        ],
        [
            (f * g - d * i) * inv,
            (a * i - c * g) * inv,
            (c * d - a * f) * inv,
        ],
        [
            (d * h - e * g) * inv,
            (b * g - a * h) * inv,
            (a * e - b * d) * inv,
        ],
    ])
}

/// CIE xy chromaticity.
pub type Xy = [f64; 2];

pub const D65: Xy = [0.3127, 0.3290];
pub const D50: Xy = [0.3457, 0.3585];

/// RGB primaries and white point, as CIE xy chromaticities.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RgbPrimaries {
    pub red: Xy,
    pub green: Xy,
    pub blue: Xy,
    pub white: Xy,
}

impl RgbPrimaries {
    /// ITU-R BT.709, also sRGB.
    pub const REC709: Self = Self {
        red: [0.640, 0.330],
        green: [0.300, 0.600],
        blue: [0.150, 0.060],
        white: D65,
    };
    pub const DISPLAY_P3: Self = Self {
        red: [0.680, 0.320],
        green: [0.265, 0.690],
        blue: [0.150, 0.060],
        white: D65,
    };
    pub const ADOBE_RGB: Self = Self {
        red: [0.640, 0.330],
        green: [0.210, 0.710],
        blue: [0.150, 0.060],
        white: D65,
    };
    /// ROMM RGB (ProPhoto), D50 white.
    pub const PROPHOTO: Self = Self {
        red: [0.7347, 0.2653],
        green: [0.1596, 0.8404],
        blue: [0.0366, 0.0001],
        white: D50,
    };
    /// ITU-R BT.2020.
    pub const REC2020: Self = Self {
        red: [0.708, 0.292],
        green: [0.170, 0.797],
        blue: [0.131, 0.046],
        white: D65,
    };

    /// Matrix from linear RGB to CIE XYZ (Y of white = 1) for this white point. Identity if the
    /// primaries are degenerate (see [`Self::is_valid`]).
    pub fn to_xyz(&self) -> Mat3 {
        self.try_to_xyz().unwrap_or(IDENTITY)
    }

    fn try_to_xyz(&self) -> Option<Mat3> {
        let column = |[x, y]: Xy| [x / y, 1.0, (1.0 - x - y) / y];
        let (r, g, b) = (column(self.red), column(self.green), column(self.blue));
        let p = [[r[0], g[0], b[0]], [r[1], g[1], b[1]], [r[2], g[2], b[2]]];
        let s = mat_vec(&mat_inverse(&p)?, column(self.white));
        let m = [0, 1, 2].map(|i| [p[i][0] * s[0], p[i][1] * s[1], p[i][2] * s[2]]);
        mat_inverse(&m).map(|_| m)
    }

    /// Whether these chromaticities define a usable (invertible) RGB space.
    pub fn is_valid(&self) -> bool {
        let points = [self.red, self.green, self.blue, self.white];
        let finite = points.iter().flatten().all(|v| v.is_finite());
        let positive_y = points.iter().all(|[_, y]| *y > 0.0);
        finite && positive_y && self.try_to_xyz().is_some()
    }
}

/// Bradford chromatic adaptation matrix (XYZ → XYZ) from white `src` to white `dst`.
pub fn bradford(src: Xy, dst: Xy) -> Mat3 {
    const MB: Mat3 = [
        [0.8951, 0.2664, -0.1614],
        [-0.7502, 1.7135, 0.0367],
        [0.0389, -0.0685, 1.0296],
    ];
    if src == dst {
        return IDENTITY;
    }
    let xyz = |[x, y]: Xy| [x / y, 1.0, (1.0 - x - y) / y];
    let lms_src = mat_vec(&MB, xyz(src));
    let lms_dst = mat_vec(&MB, xyz(dst));
    let scale = [0, 1, 2].map(|i| lms_dst[i] / lms_src[i]);
    let diag = [
        [scale[0], 0.0, 0.0],
        [0.0, scale[1], 0.0],
        [0.0, 0.0, scale[2]],
    ];
    // Invariant: MB is a fixed invertible matrix.
    let mb_inv = mat_inverse(&MB).expect("Bradford matrix is invertible");
    mat_mul(&mb_inv, &mat_mul(&diag, &MB))
}

/// Reference white used to place HDR signals on the scene-linear scale: 1.0 = 203 cd/m²
/// (ITU-R BT.2408 graphics white).
pub const HDR_REFERENCE_WHITE_NITS: f32 = 203.0;

/// How encoded values relate to linear light. `decode`: encoded → linear, `encode`: the
/// inverse. Both are extended to negative values by symmetry (about the curve's value at 0)
/// so that nothing is clipped.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TransferFunction {
    Linear,
    /// IEC 61966-2-1.
    Srgb,
    /// Pure power law: linear = encoded^gamma.
    Gamma(f32),
    /// ITU-R BT.709 / BT.2020 camera curve.
    Rec709,
    /// ICC parametric curve (type 4 covers types 0–3): for x ≥ d, y = (a·x + b)^g + e;
    /// otherwise y = c·x + f.
    Parametric {
        g: f32,
        a: f32,
        b: f32,
        c: f32,
        d: f32,
        e: f32,
        f: f32,
    },
    /// SMPTE ST 2084 (HDR10). Decodes to scene-linear with 1.0 = [`HDR_REFERENCE_WHITE_NITS`].
    Pq,
    /// ARIB STD-B67 / BT.2100 HLG (scene-referred). Decodes so that HLG reference white (signal
    /// 0.75) maps to 1.0.
    Hlg,
}

// PQ constants (SMPTE ST 2084).
const PQ_M1: f32 = 2610.0 / 16384.0;
const PQ_M2: f32 = 2523.0 / 4096.0 * 128.0;
const PQ_C1: f32 = 3424.0 / 4096.0;
const PQ_C2: f32 = 2413.0 / 4096.0 * 32.0;
const PQ_C3: f32 = 2392.0 / 4096.0 * 32.0;
const PQ_PEAK_NITS: f32 = 10000.0;

// Rec.709 constants at full precision, so that decode and encode are exact inverses at the knee.
const REC709_ALPHA: f32 = 1.099_296_8;
const REC709_BETA: f32 = 0.018_053_968;

// HLG constants (BT.2100).
const HLG_A: f32 = 0.178_832_77;
const HLG_B: f32 = 1.0 - 4.0 * HLG_A;
const HLG_C: f32 = 0.559_910_7; // 0.5 - a * ln(4a)
/// Scene-linear value of HLG reference white (signal 0.75).
const HLG_REFERENCE: f32 = 0.264_962_52;

impl TransferFunction {
    pub fn is_linear(self) -> bool {
        matches!(self, TransferFunction::Linear)
    }

    /// Encoded → linear.
    pub fn decode(self, v: f32) -> f32 {
        if let TransferFunction::Parametric { .. } = self {
            // Offsets (e, f) make the value at 0 non-zero: mirror about that point, not 0.
            let y0 = self.parametric_decode(0.0);
            return if v < 0.0 {
                2.0 * y0 - self.parametric_decode(-v)
            } else {
                self.parametric_decode(v)
            };
        }
        let x = v.abs();
        let y = match self {
            TransferFunction::Linear | TransferFunction::Parametric { .. } => x,
            TransferFunction::Srgb => srgb_decode(x),
            TransferFunction::Gamma(g) => x.powf(g),
            TransferFunction::Rec709 => {
                if x < 4.5 * REC709_BETA {
                    x / 4.5
                } else {
                    ((x + (REC709_ALPHA - 1.0)) / REC709_ALPHA).powf(1.0 / 0.45)
                }
            }
            TransferFunction::Pq => {
                // The PQ signal is defined on [0, 1] (10 000 cd/m²): saturate above, where the
                // formula would divide by ≤ 0.
                let p = x.min(1.0).powf(1.0 / PQ_M2);
                let nits =
                    ((p - PQ_C1).max(0.0) / (PQ_C2 - PQ_C3 * p)).powf(1.0 / PQ_M1) * PQ_PEAK_NITS;
                nits / HDR_REFERENCE_WHITE_NITS
            }
            TransferFunction::Hlg => {
                let scene = if x <= 0.5 {
                    x * x / 3.0
                } else {
                    (((x - HLG_C) / HLG_A).exp() + HLG_B) / 12.0
                };
                scene / HLG_REFERENCE
            }
        };
        if v < 0.0 { -y } else { y }
    }

    /// Linear → encoded (inverse of [`Self::decode`]).
    pub fn encode(self, v: f32) -> f32 {
        if let TransferFunction::Parametric { .. } = self {
            let y0 = self.parametric_decode(0.0);
            return if v < y0 {
                -self.parametric_encode(2.0 * y0 - v)
            } else {
                self.parametric_encode(v)
            };
        }
        let x = v.abs();
        let y = match self {
            TransferFunction::Linear | TransferFunction::Parametric { .. } => x,
            TransferFunction::Srgb => srgb_encode(x),
            TransferFunction::Gamma(g) => x.powf(1.0 / g),
            TransferFunction::Rec709 => {
                if x < REC709_BETA {
                    4.5 * x
                } else {
                    REC709_ALPHA * x.powf(0.45) - (REC709_ALPHA - 1.0)
                }
            }
            TransferFunction::Pq => {
                let l = (x * HDR_REFERENCE_WHITE_NITS / PQ_PEAK_NITS).powf(PQ_M1);
                ((PQ_C1 + PQ_C2 * l) / (1.0 + PQ_C3 * l)).powf(PQ_M2)
            }
            TransferFunction::Hlg => {
                let scene = x * HLG_REFERENCE;
                if scene <= 1.0 / 12.0 {
                    (3.0 * scene).sqrt()
                } else {
                    HLG_A * (12.0 * scene - HLG_B).ln() + HLG_C
                }
            }
        };
        if v < 0.0 { -y } else { y }
    }

    /// ICC parametric curve for `x ≥ 0`.
    fn parametric_decode(self, x: f32) -> f32 {
        let TransferFunction::Parametric {
            g,
            a,
            b,
            c,
            d,
            e,
            f,
        } = self
        else {
            return x;
        };
        if x >= d {
            (a * x + b).max(0.0).powf(g) + e
        } else {
            c * x + f
        }
    }

    /// Inverse of [`Self::parametric_decode`] for `y` at or above the curve's value at 0.
    fn parametric_encode(self, y: f32) -> f32 {
        let TransferFunction::Parametric {
            g,
            a,
            b,
            c,
            d,
            e,
            f,
        } = self
        else {
            return y;
        };
        // The linear segment covers [f, c·d + f). When it is flat (c = 0, ICC types 1–2), every
        // encoded value below d decodes to f: the power branch then returns d, its upper end.
        if d > 0.0 && c != 0.0 && y < c * d + f {
            (y - f) / c
        } else {
            ((y - e).max(0.0).powf(1.0 / g) - b) / a
        }
    }
}

/// A color space: RGB primaries with white point, and transfer function. For gray data only
/// the transfer function matters (gray is D65-neutral luminance).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorSpace {
    pub primaries: RgbPrimaries,
    pub transfer: TransferFunction,
}

impl ColorSpace {
    pub const SRGB: Self = Self {
        primaries: RgbPrimaries::REC709,
        transfer: TransferFunction::Srgb,
    };
    pub const LINEAR_SRGB: Self = Self {
        primaries: RgbPrimaries::REC709,
        transfer: TransferFunction::Linear,
    };
    pub const DISPLAY_P3: Self = Self {
        primaries: RgbPrimaries::DISPLAY_P3,
        transfer: TransferFunction::Srgb,
    };
    pub const ADOBE_RGB: Self = Self {
        primaries: RgbPrimaries::ADOBE_RGB,
        transfer: TransferFunction::Gamma(563.0 / 256.0),
    };
    pub const PROPHOTO: Self = Self {
        primaries: RgbPrimaries::PROPHOTO,
        transfer: TransferFunction::Gamma(1.8),
    };
    pub const REC2020: Self = Self {
        primaries: RgbPrimaries::REC2020,
        transfer: TransferFunction::Rec709,
    };
    pub const LINEAR_REC2020: Self = Self {
        primaries: RgbPrimaries::REC2020,
        transfer: TransferFunction::Linear,
    };
    pub const REC2100_PQ: Self = Self {
        primaries: RgbPrimaries::REC2020,
        transfer: TransferFunction::Pq,
    };
    pub const REC2100_HLG: Self = Self {
        primaries: RgbPrimaries::REC2020,
        transfer: TransferFunction::Hlg,
    };

    /// Whether values are proportional to light (required for correct compositing).
    pub fn is_linear(&self) -> bool {
        self.transfer.is_linear()
    }

    /// Matrix from this space's linear RGB to `dst`'s linear RGB, adapting the white point.
    pub fn matrix_to(&self, dst: &ColorSpace) -> Mat3 {
        if self.primaries == dst.primaries {
            return IDENTITY;
        }
        let to_xyz = self.primaries.to_xyz();
        let adapt = bradford(self.primaries.white, dst.primaries.white);
        // Invariant: named and validated primaries give invertible matrices.
        let from_xyz = mat_inverse(&dst.primaries.to_xyz()).unwrap_or(IDENTITY);
        mat_mul(&from_xyz, &mat_mul(&adapt, &to_xyz))
    }

    /// Stable identifier of well-known spaces (for UIs to translate), `None` for others.
    pub fn id(&self) -> Option<&'static str> {
        const KNOWN: [(ColorSpace, &str); 9] = [
            (ColorSpace::SRGB, "srgb"),
            (ColorSpace::LINEAR_SRGB, "linear-srgb"),
            (ColorSpace::DISPLAY_P3, "display-p3"),
            (ColorSpace::ADOBE_RGB, "adobe-rgb"),
            (ColorSpace::PROPHOTO, "prophoto"),
            (ColorSpace::REC2020, "rec2020"),
            (ColorSpace::LINEAR_REC2020, "linear-rec2020"),
            (ColorSpace::REC2100_PQ, "rec2100-pq"),
            (ColorSpace::REC2100_HLG, "rec2100-hlg"),
        ];
        KNOWN
            .iter()
            .find(|(space, _)| space == self)
            .map(|(_, id)| *id)
    }
}

/// The document working space (ADR 0007): linear Rec.2020, unbounded.
pub const WORKING_SPACE: ColorSpace = ColorSpace::LINEAR_REC2020;

/// Storage type of one channel sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SampleType {
    U8,
    U16,
    F16,
    F32,
}

impl SampleType {
    pub const fn bytes(self) -> u32 {
        match self {
            SampleType::U8 => 1,
            SampleType::U16 | SampleType::F16 => 2,
            SampleType::F32 => 4,
        }
    }

    pub const fn is_float(self) -> bool {
        matches!(self, SampleType::F16 | SampleType::F32)
    }
}

/// Channels present in a pixel, in memory order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChannelLayout {
    Gray,
    GrayAlpha,
    Rgb,
    Rgba,
}

impl ChannelLayout {
    pub const fn channels(self) -> u32 {
        match self {
            ChannelLayout::Gray => 1,
            ChannelLayout::GrayAlpha => 2,
            ChannelLayout::Rgb => 3,
            ChannelLayout::Rgba => 4,
        }
    }

    pub const fn has_alpha(self) -> bool {
        matches!(self, ChannelLayout::GrayAlpha | ChannelLayout::Rgba)
    }

    pub const fn is_gray(self) -> bool {
        matches!(self, ChannelLayout::Gray | ChannelLayout::GrayAlpha)
    }
}

/// How color channels relate to alpha.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AlphaMode {
    /// Color is independent of alpha ("unassociated").
    Straight,
    /// Color has been multiplied by alpha ("associated").
    Premultiplied,
}

/// Full description of an interleaved pixel buffer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PixelFormat {
    pub layout: ChannelLayout,
    pub sample: SampleType,
    pub color_space: ColorSpace,
    pub alpha: AlphaMode,
}

impl PixelFormat {
    /// 8-bit sRGB RGBA with straight alpha: what a web canvas `ImageData` or a typical PNG holds.
    pub const RGBA8_SRGB: PixelFormat = PixelFormat {
        layout: ChannelLayout::Rgba,
        sample: SampleType::U8,
        color_space: ColorSpace::SRGB,
        alpha: AlphaMode::Straight,
    };

    pub const fn bytes_per_pixel(self) -> u32 {
        self.layout.channels() * self.sample.bytes()
    }
}

/// An RGBA color in linear light with straight alpha, expressed in a [`ColorSpace`] that the
/// owner defines (for document content: [`WORKING_SPACE`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinearRgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl LinearRgba {
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    /// Explicit conversion from sRGB-encoded components to linear sRGB (same primaries).
    /// Alpha is linear in both representations and is kept as is.
    pub fn from_srgb_encoded(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self::new(srgb_decode(r), srgb_decode(g), srgb_decode(b), a)
    }

    /// Explicit conversion to sRGB-encoded components (same primaries).
    pub fn to_srgb_encoded(self) -> [f32; 4] {
        [
            srgb_encode(self.r),
            srgb_encode(self.g),
            srgb_encode(self.b),
            self.a,
        ]
    }

    /// Apply a linear-RGB conversion matrix (see [`ColorSpace::matrix_to`]).
    pub fn transform(self, m: &Mat3) -> Self {
        let [r, g, b] = mat_vec(m, [self.r, self.g, self.b].map(f64::from));
        Self::new(r as f32, g as f32, b as f32, self.a)
    }

    /// A UI color (sRGB-encoded, e.g. from a color picker) in the working space.
    pub fn from_srgb_encoded_to_working(r: f32, g: f32, b: f32, a: f32) -> Self {
        let to_working = ColorSpace::LINEAR_SRGB.matrix_to(&WORKING_SPACE);
        Self::from_srgb_encoded(r, g, b, a).transform(&to_working)
    }

    /// A working-space color as sRGB-encoded components for UI display. Out-of-gamut values
    /// are clipped here, and only here.
    pub fn working_to_srgb_encoded(self) -> [f32; 4] {
        // Before the matrix: inf - inf would already be NaN after it.
        let finite = |v: f32| {
            if v.is_nan() {
                0.0
            } else {
                v.clamp(-f32::MAX, f32::MAX)
            }
        };
        let color = Self::new(
            finite(self.r),
            finite(self.g),
            finite(self.b),
            finite(self.a),
        );
        let srgb = color.transform(&WORKING_SPACE.matrix_to(&ColorSpace::LINEAR_SRGB));
        srgb.to_srgb_encoded()
            .map(|v| if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) })
    }

    pub fn is_finite(self) -> bool {
        self.r.is_finite() && self.g.is_finite() && self.b.is_finite() && self.a.is_finite()
    }
}

/// sRGB transfer function (OETF): linear light → encoded value. Defined for `[0, 1]`; values
/// outside are extended symmetrically so that no information is clipped here.
pub fn srgb_encode(linear: f32) -> f32 {
    let x = linear.abs();
    let encoded = if x <= 0.003_130_8 {
        x * 12.92
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    };
    encoded.copysign(linear)
}

/// Inverse sRGB transfer function (EOTF): encoded value → linear light.
pub fn srgb_decode(encoded: f32) -> f32 {
    let x = encoded.abs();
    let linear = if x <= 0.040_45 {
        x / 12.92
    } else {
        ((x + 0.055) / 1.055).powf(2.4)
    };
    linear.copysign(encoded)
}

/// IEEE 754 binary16 → f32 (exact).
pub fn f16_to_f32(half: u16) -> f32 {
    let sign = u32::from(half >> 15) << 31;
    let exp = u32::from((half >> 10) & 0x1f);
    let mant = u32::from(half & 0x3ff);
    let bits = match (exp, mant) {
        (0, 0) => sign,
        (0, _) => {
            // Subnormal: mant × 2^-24.
            let value = mant as f32 * f32::from_bits(0x3380_0000); // 2^-24
            return if sign != 0 { -value } else { value };
        }
        (0x1f, _) => sign | 0x7f80_0000 | (mant << 13),
        _ => sign | ((exp + 112) << 23) | (mant << 13),
    };
    f32::from_bits(bits)
}

/// f32 → IEEE 754 binary16, rounding to nearest even (overflow gives infinity).
pub fn f32_to_f16(value: f32) -> u16 {
    let x = value.to_bits();
    let sign = ((x >> 16) & 0x8000) as u16;
    let exp = ((x >> 23) & 0xff) as i32;
    let mant = x & 0x7f_ffff;
    if exp == 0xff {
        return sign | 0x7c00 | if mant != 0 { 0x200 } else { 0 };
    }
    let e = exp - 127 + 15;
    if e >= 0x1f {
        return sign | 0x7c00;
    }
    if e <= 0 {
        if e < -10 {
            return sign;
        }
        let m = mant | 0x80_0000;
        let shift = (14 - e) as u32;
        let mut half = m >> shift;
        let rem = m & ((1 << shift) - 1);
        let halfway = 1 << (shift - 1);
        if rem > halfway || (rem == halfway && half & 1 == 1) {
            half += 1;
        }
        return sign | half as u16;
    }
    let mut half = u32::from(sign) | ((e as u32) << 10) | (mant >> 13);
    let rem = mant & 0x1fff;
    if rem > 0x1000 || (rem == 0x1000 && half & 1 == 1) {
        half += 1; // may carry into the exponent: correct rounding up to the next binade/inf
    }
    half as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, eps: f64) -> bool {
        (a - b).abs() < eps
    }

    #[test]
    fn srgb_known_values() {
        assert_eq!(srgb_encode(0.0), 0.0);
        assert!((srgb_encode(1.0) - 1.0).abs() < 1e-6);
        // Mid grey: linear 0.2140 ≈ encoded 0.5.
        assert!((srgb_encode(0.214_041_14) - 0.5).abs() < 1e-5);
        assert!((srgb_decode(0.5) - 0.214_041_14).abs() < 1e-6);
    }

    #[test]
    fn srgb_round_trip() {
        for i in 0..=1000 {
            let v = i as f32 / 1000.0;
            assert!((srgb_decode(srgb_encode(v)) - v).abs() < 1e-5, "v = {v}");
        }
    }

    #[test]
    fn srgb_does_not_clip_out_of_range_values() {
        assert!(srgb_encode(2.0) > 1.0);
        assert!(srgb_encode(-0.5) < 0.0);
        assert!((srgb_decode(srgb_encode(-0.5)) + 0.5).abs() < 1e-5);
    }

    #[test]
    fn every_transfer_function_round_trips() {
        let functions = [
            TransferFunction::Linear,
            TransferFunction::Srgb,
            TransferFunction::Gamma(2.2),
            TransferFunction::Gamma(1.8),
            TransferFunction::Rec709,
            // sRGB expressed as an ICC parametric curve.
            TransferFunction::Parametric {
                g: 2.4,
                a: 1.0 / 1.055,
                b: 0.055 / 1.055,
                c: 1.0 / 12.92,
                d: 0.04045,
                e: 0.0,
                f: 0.0,
            },
            TransferFunction::Pq,
            TransferFunction::Hlg,
        ];
        for tf in functions {
            for i in 0..=100 {
                let encoded = i as f32 / 100.0;
                let back = tf.encode(tf.decode(encoded));
                assert!((back - encoded).abs() < 2e-4, "{tf:?} at {encoded}: {back}");
            }
            // Negative values are mirrored, not clipped.
            assert!(tf.decode(-0.5) <= 0.0, "{tf:?}");
        }
    }

    #[test]
    fn every_16_bit_code_round_trips() {
        let functions = [
            TransferFunction::Linear,
            TransferFunction::Srgb,
            TransferFunction::Gamma(563.0 / 256.0),
            TransferFunction::Gamma(1.8),
            TransferFunction::Rec709,
        ];
        for tf in functions {
            for code in 0..=u16::MAX {
                let encoded = f32::from(code) / 65535.0;
                let back = (tf.encode(tf.decode(encoded)) * 65535.0).round();
                assert_eq!(back, f32::from(code), "{tf:?} at code {code}");
            }
        }
    }

    #[test]
    fn rec709_is_continuous_and_monotonic_at_the_knee() {
        let tf = TransferFunction::Rec709;
        let mut previous = tf.decode(0.0805);
        for i in 1..=150 {
            let v = 0.0805 + i as f32 * 1e-5;
            let y = tf.decode(v);
            assert!(y > previous, "not increasing at {v}");
            previous = y;
        }
        for i in 0..=200 {
            let linear = 0.017 + i as f32 * 1e-5;
            assert!((tf.decode(tf.encode(linear)) - linear).abs() < 1e-6);
        }
    }

    #[test]
    fn parametric_offsets_are_mirrored_about_the_value_at_zero() {
        // ICC type 2 shape: flat f below d, power curve above, with an offset.
        let tf = TransferFunction::Parametric {
            g: 2.2,
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 0.0,
            e: 0.05,
            f: 0.05,
        };
        // Continuous across zero.
        assert!((tf.decode(1e-4) - tf.decode(-1e-4)).abs() < 1e-3);
        for i in -100..=100 {
            let encoded = i as f32 / 100.0;
            let back = tf.encode(tf.decode(encoded));
            assert!((back - encoded).abs() < 2e-4, "at {encoded}: {back}");
        }
        // Flat linear segment (c = 0, d > 0): the value of the whole segment encodes to d.
        let flat = TransferFunction::Parametric {
            g: 2.0,
            a: 1.0,
            b: -0.1,
            c: 0.0,
            d: 0.1,
            e: 0.2,
            f: 0.2,
        };
        assert!((flat.encode(0.2) - 0.1).abs() < 1e-6);
        assert!(flat.encode(0.2).is_finite());
    }

    #[test]
    fn pq_saturates_out_of_its_domain() {
        for v in [1.5_f32, 2.0, 10.0, 1e6] {
            let y = TransferFunction::Pq.decode(v);
            assert_eq!(y, TransferFunction::Pq.decode(1.0), "PQ {v}");
        }
    }

    #[test]
    fn swatches_of_non_finite_colors_are_finite() {
        let colors = [
            LinearRgba::new(f32::INFINITY, 0.0, 0.0, 1.0),
            LinearRgba::new(f32::INFINITY, f32::INFINITY, f32::INFINITY, 1.0),
            LinearRgba::new(f32::NAN, 0.5, f32::NEG_INFINITY, f32::NAN),
        ];
        for color in colors {
            let swatch = color.working_to_srgb_encoded();
            assert!(swatch.iter().all(|v| (0.0..=1.0).contains(v)), "{swatch:?}");
        }
    }

    #[test]
    fn parametric_srgb_matches_srgb() {
        let tf = TransferFunction::Parametric {
            g: 2.4,
            a: 1.0 / 1.055,
            b: 0.055 / 1.055,
            c: 1.0 / 12.92,
            d: 0.04045,
            e: 0.0,
            f: 0.0,
        };
        for i in 0..=255 {
            let v = i as f32 / 255.0;
            assert!((tf.decode(v) - srgb_decode(v)).abs() < 1e-6);
        }
    }

    #[test]
    fn hdr_reference_whites_map_to_one() {
        // PQ: 203 cd/m² → 1.0.
        let pq_203 = TransferFunction::Pq.encode(1.0);
        assert!((TransferFunction::Pq.decode(pq_203) - 1.0).abs() < 1e-4);
        assert!(
            (pq_203 - 0.58).abs() < 0.01,
            "203 nits is ~58% PQ, got {pq_203}"
        );
        // HLG: signal 0.75 → 1.0.
        assert!((TransferFunction::Hlg.decode(0.75) - 1.0).abs() < 1e-3);
    }

    #[test]
    fn srgb_to_xyz_matches_the_standard() {
        let m = RgbPrimaries::REC709.to_xyz();
        let expected = [
            [0.4124, 0.3576, 0.1805],
            [0.2126, 0.7152, 0.0722],
            [0.0193, 0.1192, 0.9505],
        ];
        for i in 0..3 {
            for j in 0..3 {
                assert!(close(m[i][j], expected[i][j], 1e-3), "{m:?}");
            }
        }
    }

    #[test]
    fn white_maps_to_white_across_spaces() {
        let spaces = [
            ColorSpace::SRGB,
            ColorSpace::DISPLAY_P3,
            ColorSpace::ADOBE_RGB,
            ColorSpace::PROPHOTO,
            ColorSpace::REC2020,
        ];
        for src in spaces {
            let m = src.matrix_to(&WORKING_SPACE);
            let white = mat_vec(&m, [1.0, 1.0, 1.0]);
            for c in white {
                assert!(close(c, 1.0, 1e-4), "{src:?}: {white:?}");
            }
        }
    }

    #[test]
    fn conversions_round_trip_and_keep_out_of_gamut_values() {
        let to = ColorSpace::LINEAR_SRGB.matrix_to(&WORKING_SPACE);
        let back = WORKING_SPACE.matrix_to(&ColorSpace::LINEAR_SRGB);
        // Pure sRGB red is inside Rec.2020: all components positive there.
        let red = mat_vec(&to, [1.0, 0.0, 0.0]);
        assert!(red.iter().all(|&c| c >= 0.0), "{red:?}");
        let again = mat_vec(&back, red);
        assert!(close(again[0], 1.0, 1e-9) && close(again[1], 0.0, 1e-9));
        // Pure Rec.2020 green is outside sRGB: negative components survive, unclipped.
        let green = mat_vec(&back, [0.0, 1.0, 0.0]);
        assert!(green.iter().any(|&c| c < 0.0));
    }

    #[test]
    fn prophoto_d50_white_is_adapted() {
        // Without Bradford adaptation ProPhoto white would come out tinted in a D65 space.
        let white = mat_vec(
            &ColorSpace::PROPHOTO.matrix_to(&ColorSpace::LINEAR_SRGB),
            [1.0, 1.0, 1.0],
        );
        for c in white {
            assert!(close(c, 1.0, 1e-3), "{white:?}");
        }
    }

    #[test]
    fn ui_colors_round_trip_through_the_working_space() {
        let c = LinearRgba::from_srgb_encoded_to_working(0.9, 0.3, 0.6, 1.0);
        let [r, g, b, a] = c.working_to_srgb_encoded();
        assert!((r - 0.9).abs() < 1e-5 && (g - 0.3).abs() < 1e-5 && (b - 0.6).abs() < 1e-5);
        assert_eq!(a, 1.0);
    }

    #[test]
    fn known_spaces_have_ids() {
        assert_eq!(WORKING_SPACE.id(), Some("linear-rec2020"));
        assert_eq!(ColorSpace::SRGB.id(), Some("srgb"));
        let custom = ColorSpace {
            transfer: TransferFunction::Gamma(2.0),
            ..ColorSpace::SRGB
        };
        assert_eq!(custom.id(), None);
    }

    #[test]
    fn f16_conversion_is_exact_for_every_half() {
        for bits in 0..=u16::MAX {
            let exp = (bits >> 10) & 0x1f;
            let is_nan = exp == 0x1f && bits & 0x3ff != 0;
            if is_nan {
                assert!(f16_to_f32(bits).is_nan());
                continue;
            }
            assert_eq!(f32_to_f16(f16_to_f32(bits)), bits, "bits {bits:#06x}");
        }
    }

    #[test]
    fn f16_rounding() {
        assert_eq!(f16_to_f32(f32_to_f16(1.0)), 1.0);
        assert_eq!(f16_to_f32(f32_to_f16(65504.0)), 65504.0);
        assert!(f16_to_f32(f32_to_f16(1e6)).is_infinite());
        assert_eq!(f32_to_f16(1e-9), 0);
        // 1 + 2^-11 is halfway between two halves: rounds to even (1.0).
        assert_eq!(f16_to_f32(f32_to_f16(1.0 + 2f32.powi(-11))), 1.0);
    }

    #[test]
    fn pixel_format_sizes() {
        assert_eq!(PixelFormat::RGBA8_SRGB.bytes_per_pixel(), 4);
        let rgba_f16 = PixelFormat {
            sample: SampleType::F16,
            color_space: ColorSpace::LINEAR_SRGB,
            ..PixelFormat::RGBA8_SRGB
        };
        assert_eq!(rgba_f16.bytes_per_pixel(), 8);
    }
}
