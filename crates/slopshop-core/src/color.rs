//! Color spaces, pixel formats and transfer functions.
//!
//! Nothing here assumes "sRGB 8-bit": every buffer is described by a [`PixelFormat`], every
//! color value knows which space it lives in, and conversions are explicit named functions.
//! The set of color spaces is intentionally tiny for now; ICC/OCIO support is an open question
//! (see `docs/architecture.md`).

/// A color space: primaries, white point and transfer function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorSpace {
    /// Rec. 709 / sRGB primaries, D65 white, linear transfer. The default working space.
    LinearSrgb,
    /// Rec. 709 / sRGB primaries, D65 white, sRGB transfer (IEC 61966-2-1). Typical for 8-bit
    /// files and displays.
    Srgb,
}

impl ColorSpace {
    /// Whether values are proportional to light (required for correct compositing).
    pub const fn is_linear(self) -> bool {
        matches!(self, ColorSpace::LinearSrgb)
    }
}

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
        color_space: ColorSpace::Srgb,
        alpha: AlphaMode::Straight,
    };

    pub const fn bytes_per_pixel(self) -> u32 {
        self.layout.channels() * self.sample.bytes()
    }
}

/// An RGBA color in linear light with straight alpha, expressed in a [`ColorSpace`] that the
/// owner defines (for document content: the document working space).
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

    /// Explicit conversion from sRGB-encoded components (e.g. a color picked in the UI).
    /// Alpha is linear in both representations and is kept as is.
    pub fn from_srgb_encoded(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self::new(srgb_decode(r), srgb_decode(g), srgb_decode(b), a)
    }

    /// Explicit conversion to sRGB-encoded components.
    pub fn to_srgb_encoded(self) -> [f32; 4] {
        [
            srgb_encode(self.r),
            srgb_encode(self.g),
            srgb_encode(self.b),
            self.a,
        ]
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn pixel_format_sizes() {
        assert_eq!(PixelFormat::RGBA8_SRGB.bytes_per_pixel(), 4);
        let rgba_f16 = PixelFormat {
            sample: SampleType::F16,
            color_space: ColorSpace::LinearSrgb,
            ..PixelFormat::RGBA8_SRGB
        };
        assert_eq!(rgba_f16.bytes_per_pixel(), 8);
    }
}
