//! Conversion of composited pixels into a target pixel format, for export (ADR 0008).
//!
//! Input: rows of premultiplied RGBA `f32` in [`WORKING_SPACE`] (linear Rec.2020, unbounded),
//! as the compositors produce them. Output: interleaved samples of a target [`PixelFormat`]
//! (RGB or RGBA; 8/16-bit integer or 16/32-bit float; any valid color space; straight or
//! premultiplied alpha), ready for an encoder.
//!
//! Each pixel goes through, in this order:
//! 1. non-finite inputs: NaN → 0; ±inf is kept for float targets and becomes ±`f32::MAX` (then
//!    clipped) for integer targets; always counted;
//! 2. the matrix from the working space to the target primaries (in f64);
//! 3. un-premultiplying for straight-alpha targets (alpha 0 gives color 0). RGB targets keep the
//!    premultiplied color, i.e. the image over black: callers drop alpha only when the document
//!    is opaque;
//! 4. the range policy: integer targets clip each channel to the range of their codes and count
//!    it; float targets keep every finite value (half floats count what overflows their range);
//! 5. transfer encoding and quantization. For 8/16-bit, both are done at once and exactly with a
//!    threshold table: code `k` covers the linear values between the decoded midpoints
//!    `(k ± ½) / max`, adjusted so that `decode(k / max)` — what import produces — always gives
//!    `k` back. Unedited 8/16-bit sources therefore export bit-exact;
//! 6. dithering (8-bit only, optional): blue noise of ±0.49 step added in the encoded domain
//!    before rounding, indexed by absolute pixel position so that bands and tiles do not matter.
//!    A value exactly on a level never moves. The same offset is used for R, G and B (neutral
//!    noise, no color speckles).
//!
//! Premultiplied targets with a non-linear transfer follow the convention of the raster codec:
//! `encode(color / alpha) × alpha`. Alpha itself is linear in every format: it is clamped to
//! `[0, 1]` and rounded, never dithered.

use std::fmt;

use crate::blue_noise;
use crate::color::{
    AlphaMode, ChannelLayout, ColorSpace, IDENTITY, Mat3, PixelFormat, SampleType,
    TransferFunction, WORKING_SPACE, f32_to_f16, mat_vec,
};
use crate::raster::decode_levels;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConvertOptions {
    /// Blue-noise dither for 8-bit targets (ignored by the others).
    pub dither: bool,
    /// Byte order of 16/32-bit samples: big-endian (PNG) or little-endian (TIFF, EXR).
    pub big_endian: bool,
}

/// Lossy events of a conversion, counted per output sample (per input sample for
/// `non_finite`). An infinity clipped for an integer target counts in both places.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConversionReport {
    /// Values above the target's range: clipped to the top code (integers), or finite values
    /// beyond the `f32` range saturated to `f32::MAX` (floats).
    pub clipped_high: u64,
    /// Values below the target's range (negative, or out of gamut), clipped likewise.
    pub clipped_low: u64,
    /// NaN or infinite input samples.
    pub non_finite: u64,
    /// Finite values beyond the half-float range, written as ±65504 (f16 targets).
    pub half_overflow: u64,
}

impl ConversionReport {
    /// Add the counts of another report (e.g. from another band or thread).
    pub fn merge(&mut self, other: &ConversionReport) {
        self.clipped_high += other.clipped_high;
        self.clipped_low += other.clipped_low;
        self.non_finite += other.non_finite;
        self.half_overflow += other.half_overflow;
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConvertError {
    /// Only RGB and RGBA targets are supported.
    UnsupportedLayout(ChannelLayout),
    /// The target primaries do not define a usable RGB space.
    InvalidColorSpace(ColorSpace),
    /// The source is not whole RGBA pixels, or the destination does not hold exactly as many
    /// target pixels.
    BufferSizeMismatch { source_len: usize, dest_len: usize },
}

impl fmt::Display for ConvertError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConvertError::UnsupportedLayout(layout) => {
                write!(f, "cannot convert to the {layout:?} layout")
            }
            ConvertError::InvalidColorSpace(space) => write!(f, "invalid color space {space:?}"),
            ConvertError::BufferSizeMismatch {
                source_len,
                dest_len,
            } => write!(
                f,
                "cannot convert {source_len} samples into a buffer of {dest_len} bytes"
            ),
        }
    }
}

impl std::error::Error for ConvertError {}

/// Converts rows of working-space pixels into one target format. Immutable once built, so one
/// converter can serve several threads.
#[derive(Debug)]
pub struct Converter {
    target: PixelFormat,
    options: ConvertOptions,
    /// Working space → target primaries; `None` when they are the same.
    matrix: Option<Mat3>,
    /// Integer targets only.
    quantizer: Option<Quantizer>,
}

impl Converter {
    pub fn new(target: PixelFormat, options: ConvertOptions) -> Result<Self, ConvertError> {
        if target.layout.is_gray() {
            return Err(ConvertError::UnsupportedLayout(target.layout));
        }
        let space = target.color_space;
        if !space.primaries.is_valid() {
            return Err(ConvertError::InvalidColorSpace(space));
        }
        let matrix = WORKING_SPACE.matrix_to(&space);
        let quantizer = match target.sample {
            SampleType::U8 | SampleType::U16 => Some(Quantizer::new(space.transfer, target.sample)),
            SampleType::F16 | SampleType::F32 => None,
        };
        Ok(Self {
            target,
            options,
            // Skipping the identity also keeps infinities intact (inf × 0 would be NaN).
            matrix: (matrix != IDENTITY).then_some(matrix),
            quantizer,
        })
    }

    pub fn target(&self) -> PixelFormat {
        self.target
    }

    /// Bytes of one converted pixel.
    pub fn bytes_per_pixel(&self) -> usize {
        self.target.bytes_per_pixel() as usize
    }

    /// Convert one row: `src` holds premultiplied working-space RGBA pixels whose first one
    /// sits at document position (`x0`, `y`) (which only matters for the dither), `dst`
    /// receives exactly as many target pixels. Lossy events are added to `report`.
    pub fn convert_row(
        &self,
        src: &[f32],
        x0: u32,
        y: u32,
        dst: &mut [u8],
        report: &mut ConversionReport,
    ) -> Result<(), ConvertError> {
        let bpp = self.bytes_per_pixel();
        if !src.len().is_multiple_of(4) || src.len() / 4 * bpp != dst.len() {
            return Err(ConvertError::BufferSizeMismatch {
                source_len: src.len(),
                dest_len: dst.len(),
            });
        }
        let noise = (self.options.dither && self.target.sample == SampleType::U8)
            .then(|| blue_noise::row(u64::from(y)));
        let size = blue_noise::SIZE as u64;
        for (i, (px, out)) in src
            .as_chunks::<4>()
            .0
            .iter()
            .zip(dst.chunks_exact_mut(bpp))
            .enumerate()
        {
            let offset = noise.map(|row| row[((u64::from(x0) + i as u64) % size) as usize]);
            self.convert_pixel(px, offset, out, report);
        }
        Ok(())
    }

    fn convert_pixel(
        &self,
        px: &[f32; 4],
        noise: Option<f32>,
        out: &mut [u8],
        report: &mut ConversionReport,
    ) {
        let integer = self.quantizer.is_some();
        let mut input = [0.0f64; 4];
        for (value, &v) in input.iter_mut().zip(px) {
            *value = if v.is_finite() {
                f64::from(v)
            } else {
                report.non_finite += 1;
                if v.is_nan() {
                    0.0
                } else if integer {
                    f64::from(f32::MAX.copysign(v))
                } else {
                    f64::from(v)
                }
            };
        }
        let alpha = input[3].clamp(0.0, 1.0);
        let premultiplied = [input[0], input[1], input[2]];
        let color = match &self.matrix {
            Some(m) => mat_vec(m, premultiplied),
            None => premultiplied,
        };

        let has_alpha = self.target.layout.has_alpha();
        let straight = has_alpha && self.target.alpha == AlphaMode::Straight;
        // `encode(color / alpha) × alpha`, see the module documentation.
        let encoded_premultiplied = has_alpha
            && self.target.alpha == AlphaMode::Premultiplied
            && !self.target.color_space.transfer.is_linear();
        let unpremultiply = |c: f64| if alpha > 0.0 { c / alpha } else { 0.0 };

        let sample_bytes = self.target.sample.bytes() as usize;
        let mut samples = out.chunks_exact_mut(sample_bytes);
        for c in color {
            let c = if c.is_nan() { 0.0 } else { c };
            let Some(sample) = samples.next() else { return };
            match &self.quantizer {
                Some(q) => {
                    let code = if encoded_premultiplied && alpha < 1.0 {
                        if alpha > 0.0 {
                            q.quantize_premultiplied(
                                unpremultiply(c) as f32,
                                alpha as f32,
                                noise,
                                report,
                            )
                        } else {
                            0
                        }
                    } else {
                        let value = if straight { unpremultiply(c) } else { c };
                        q.quantize(value as f32, noise, report)
                    };
                    self.put_int(code, sample);
                }
                None => {
                    let transfer = self.target.color_space.transfer;
                    let value = if encoded_premultiplied {
                        if alpha > 0.0 {
                            transfer.encode(saturate(unpremultiply(c), report)) * alpha as f32
                        } else {
                            0.0
                        }
                    } else {
                        let value = saturate(if straight { unpremultiply(c) } else { c }, report);
                        if transfer.is_linear() {
                            value
                        } else {
                            transfer.encode(value)
                        }
                    };
                    self.put_float(if value.is_nan() { 0.0 } else { value }, sample, report);
                }
            }
        }
        if let Some(sample) = samples.next() {
            match &self.quantizer {
                Some(q) => self.put_int((alpha * f64::from(q.max)).round() as u32, sample),
                None => self.put_float(alpha as f32, sample, report),
            }
        }
    }

    fn put_int(&self, code: u32, out: &mut [u8]) {
        match self.target.sample {
            SampleType::U8 => out[0] = code as u8,
            _ => out.copy_from_slice(&self.bytes_u16(code as u16)),
        }
    }

    fn put_float(&self, value: f32, out: &mut [u8], report: &mut ConversionReport) {
        match self.target.sample {
            SampleType::F16 => {
                let mut half = f32_to_f16(value);
                if value.is_finite() && half & 0x7fff == 0x7c00 {
                    // Rounded to infinity: keep it finite, at the largest half.
                    report.half_overflow += 1;
                    half = (half & 0x8000) | 0x7bff;
                }
                out.copy_from_slice(&self.bytes_u16(half));
            }
            _ => {
                let bits = value.to_bits();
                out.copy_from_slice(&if self.options.big_endian {
                    bits.to_be_bytes()
                } else {
                    bits.to_le_bytes()
                });
            }
        }
    }

    fn bytes_u16(&self, v: u16) -> [u8; 2] {
        if self.options.big_endian {
            v.to_be_bytes()
        } else {
            v.to_le_bytes()
        }
    }
}

/// An `f64` result as `f32` for a float target: finite values beyond the `f32` range saturate
/// (and are counted) instead of becoming infinite; infinities stay.
fn saturate(v: f64, report: &mut ConversionReport) -> f32 {
    let x = v as f32;
    if x.is_infinite() && v.is_finite() {
        if v > 0.0 {
            report.clipped_high += 1;
        } else {
            report.clipped_low += 1;
        }
        f32::MAX.copysign(x)
    } else {
        x
    }
}

/// Exact linear → integer code conversion for one transfer function and bit depth.
struct Quantizer {
    transfer: TransferFunction,
    /// Top code (255 or 65535).
    max: u32,
    /// `levels[k]`: linear value of code `k`, as import decodes it.
    levels: Vec<f32>,
    /// `thresholds[k]`: lowest linear value of code `k + 1`, sorted.
    thresholds: Vec<f32>,
    /// Values beyond these are clipped (the codes' range, plus half a step on each side).
    low_clip: f32,
    high_clip: f32,
}

impl fmt::Debug for Quantizer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The tables are large and derived: leave them out.
        f.debug_struct("Quantizer")
            .field("transfer", &self.transfer)
            .field("max", &self.max)
            .finish_non_exhaustive()
    }
}

impl Quantizer {
    fn new(transfer: TransferFunction, sample: SampleType) -> Self {
        let levels = decode_levels(transfer, sample);
        let max = levels.len().saturating_sub(1) as u32;
        let steps = max as f32;
        let mut thresholds: Vec<f32> = Vec::with_capacity(levels.len());
        for (k, pair) in levels.windows(2).enumerate() {
            // Midpoint between two codes in the encoded domain, i.e. rounding `encode(v)`...
            let mut t = transfer.decode((k as f32 + 0.5) / steps);
            // ...adjusted so that each code's own level falls in its interval despite f32
            // rounding in `decode`: pair[0] < t <= pair[1].
            if pair[0] < pair[1] {
                t = t.clamp(pair[0].next_up(), pair[1]);
            }
            // Keep the table sorted even where levels collide (flat curves cannot round-trip).
            if let Some(&previous) = thresholds.last() {
                t = t.max(previous);
            }
            thresholds.push(t);
        }
        let first = levels.first().copied().unwrap_or(0.0);
        let last = levels.last().copied().unwrap_or(1.0);
        Self {
            transfer,
            max,
            low_clip: transfer.decode(-0.5 / steps).min(first),
            high_clip: transfer.decode((steps + 0.5) / steps).max(last),
            levels,
            thresholds,
        }
    }

    /// Code of a linear value, clipped to the codes' range (counted) and optionally dithered by
    /// `noise` (in steps).
    fn quantize(&self, v: f32, noise: Option<f32>, report: &mut ConversionReport) -> u32 {
        if v > self.high_clip {
            report.clipped_high += 1;
            return self.max;
        }
        if v < self.low_clip {
            report.clipped_low += 1;
            return 0;
        }
        let k = self.thresholds.partition_point(|&t| t <= v);
        let Some(noise) = noise else {
            return k as u32;
        };
        let dithered = (self.fraction(k, v) + noise).round();
        (k as f32 + dithered).clamp(0.0, self.max as f32) as u32
    }

    /// Position of `v` relative to code `k`'s level, in steps, within `[-½, ½]`: interpolated
    /// linearly between the level (0) and the neighboring threshold (±½). Exactly 0 on the
    /// level, so dither (under ½ step) never moves a value that sits exactly on a level.
    fn fraction(&self, k: usize, v: f32) -> f32 {
        let level = self.levels[k];
        if v >= level {
            match self.thresholds.get(k) {
                Some(&upper) if upper > level => (0.5 * (v - level) / (upper - level)).min(0.5),
                _ => 0.0,
            }
        } else {
            match k.checked_sub(1).and_then(|i| self.thresholds.get(i)) {
                Some(&lower) if level > lower => (-0.5 * (level - v) / (level - lower)).max(-0.5),
                _ => 0.0,
            }
        }
    }

    /// Code of `encode(straight) × alpha` (premultiplied target, non-linear transfer,
    /// `0 < alpha < 1`), clipping `straight` to the codes' range first.
    fn quantize_premultiplied(
        &self,
        straight: f32,
        alpha: f32,
        noise: Option<f32>,
        report: &mut ConversionReport,
    ) -> u32 {
        let first = self.levels.first().copied().unwrap_or(0.0);
        let last = self.levels.last().copied().unwrap_or(1.0);
        if straight > self.high_clip {
            report.clipped_high += 1;
        } else if straight < self.low_clip {
            report.clipped_low += 1;
        }
        let encoded = self
            .transfer
            .encode(straight.clamp(first, last))
            .clamp(0.0, 1.0);
        let steps = self.max as f32;
        (encoded * alpha * steps + noise.unwrap_or(0.0))
            .round()
            .clamp(0.0, steps) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::RgbPrimaries;

    const NAMED: [TransferFunction; 8] = [
        TransferFunction::Linear,
        TransferFunction::Srgb,
        TransferFunction::Gamma(563.0 / 256.0),
        TransferFunction::Gamma(1.8),
        TransferFunction::Rec709,
        // Non-zero value at 0 and a jump at d: the general ICC type-4 shape.
        TransferFunction::Parametric {
            g: 2.2,
            a: 0.9,
            b: 0.1,
            c: 0.1,
            d: 0.1,
            e: 0.01,
            f: 0.01,
        },
        TransferFunction::Pq,
        TransferFunction::Hlg,
    ];

    /// Rec.2020 primaries (no matrix from the working space) with any transfer.
    fn rec2020(transfer: TransferFunction) -> ColorSpace {
        ColorSpace {
            primaries: RgbPrimaries::REC2020,
            transfer,
        }
    }

    fn target(
        layout: ChannelLayout,
        sample: SampleType,
        color_space: ColorSpace,
        alpha: AlphaMode,
    ) -> PixelFormat {
        PixelFormat {
            layout,
            sample,
            color_space,
            alpha,
        }
    }

    fn options(dither: bool) -> ConvertOptions {
        ConvertOptions {
            dither,
            big_endian: false,
        }
    }

    fn convert(
        format: PixelFormat,
        opts: ConvertOptions,
        src: &[f32],
        x0: u32,
        y: u32,
    ) -> (Vec<u8>, ConversionReport) {
        let converter = Converter::new(format, opts).unwrap();
        let mut dst = vec![0u8; src.len() / 4 * converter.bytes_per_pixel()];
        let mut report = ConversionReport::default();
        converter
            .convert_row(src, x0, y, &mut dst, &mut report)
            .unwrap();
        (dst, report)
    }

    fn u16s(bytes: &[u8]) -> Vec<u16> {
        bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u16::from_le_bytes(*b))
            .collect()
    }

    fn f32s(bytes: &[u8]) -> Vec<f32> {
        bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect()
    }

    /// Opaque gray pixels at the given linear values.
    fn grays(values: &[f32]) -> Vec<f32> {
        values.iter().flat_map(|&v| [v, v, v, 1.0]).collect()
    }

    #[test]
    fn rejects_unsupported_targets() {
        let gray = PixelFormat {
            layout: ChannelLayout::Gray,
            ..PixelFormat::RGBA8_SRGB
        };
        assert_eq!(
            Converter::new(gray, options(false)).unwrap_err(),
            ConvertError::UnsupportedLayout(ChannelLayout::Gray)
        );
        let mut broken = PixelFormat::RGBA8_SRGB;
        broken.color_space.primaries.green = broken.color_space.primaries.red;
        assert!(matches!(
            Converter::new(broken, options(false)),
            Err(ConvertError::InvalidColorSpace(_))
        ));
        let converter = Converter::new(PixelFormat::RGBA8_SRGB, options(false)).unwrap();
        let mut report = ConversionReport::default();
        for (src, dst) in [(5, 4), (8, 4), (4, 3)] {
            assert!(matches!(
                converter.convert_row(&vec![0.0; src], 0, 0, &mut vec![0; dst], &mut report),
                Err(ConvertError::BufferSizeMismatch { .. })
            ));
        }
    }

    #[test]
    fn every_integer_code_round_trips_for_every_transfer() {
        // What import decodes code k to must export back to k, for all codes (U8 and U16) and
        // every named transfer, with a straight RGBA and an RGB target.
        for tf in NAMED {
            let space = rec2020(tf);
            for sample in [SampleType::U8, SampleType::U16] {
                let levels = decode_levels(tf, sample);
                for layout in [ChannelLayout::Rgba, ChannelLayout::Rgb] {
                    let format = target(layout, sample, space, AlphaMode::Straight);
                    let (bytes, report) = convert(format, options(false), &grays(&levels), 0, 0);
                    let channels = layout.channels() as usize;
                    let codes: Vec<u32> = match sample {
                        SampleType::U8 => bytes.iter().map(|&b| u32::from(b)).collect(),
                        _ => u16s(&bytes).into_iter().map(u32::from).collect(),
                    };
                    for (k, px) in codes.chunks_exact(channels).enumerate() {
                        assert_eq!(px[0], k as u32, "{tf:?} {sample:?} {layout:?} code {k}");
                        assert!(px[..3].iter().all(|&c| c == px[0]));
                    }
                    assert_eq!(report, ConversionReport::default(), "{tf:?} {sample:?}");
                }
            }
        }
    }

    #[test]
    fn quantization_rounds_in_the_encoded_domain() {
        // sRGB 8-bit: just below and above the midpoint between codes 100 and 101.
        let tf = TransferFunction::Srgb;
        let format = target(
            ChannelLayout::Rgb,
            SampleType::U8,
            rec2020(tf),
            AlphaMode::Straight,
        );
        let below = tf.decode(100.49 / 255.0);
        let above = tf.decode(100.51 / 255.0);
        let (bytes, _) = convert(format, options(false), &grays(&[below, above]), 0, 0);
        assert_eq!(bytes, [100, 100, 100, 101, 101, 101]);
    }

    #[test]
    fn straight_targets_unpremultiply_and_zero_alpha_gives_zero() {
        let format = target(
            ChannelLayout::Rgba,
            SampleType::U8,
            ColorSpace::LINEAR_REC2020,
            AlphaMode::Straight,
        );
        let src = [0.25, 0.5, 0.1, 0.5, 0.7, 0.2, 0.9, 0.0];
        let (bytes, report) = convert(format, options(false), &src, 0, 0);
        assert_eq!(bytes, [128, 255, 51, 128, 0, 0, 0, 0]);
        assert_eq!(report, ConversionReport::default());
        // Float straight target: same rule.
        let (bytes, _) = convert(
            PixelFormat {
                sample: SampleType::F32,
                ..format
            },
            options(false),
            &src,
            0,
            0,
        );
        assert_eq!(f32s(&bytes), [0.5, 1.0, 0.2, 0.5, 0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn premultiplied_targets_follow_the_raster_convention() {
        // Linear: stored as is. Non-linear: encode(color / alpha) × alpha.
        let src = [0.25, 0.1, 0.0, 0.5];
        let linear = target(
            ChannelLayout::Rgba,
            SampleType::F32,
            ColorSpace::LINEAR_REC2020,
            AlphaMode::Premultiplied,
        );
        let (bytes, _) = convert(linear, options(false), &src, 0, 0);
        assert_eq!(f32s(&bytes), src);

        let space = rec2020(TransferFunction::Srgb);
        let encoded = PixelFormat {
            color_space: space,
            ..linear
        };
        let (bytes, _) = convert(encoded, options(false), &src, 0, 0);
        let expected = [0.5f32, 0.2, 0.0].map(|v| TransferFunction::Srgb.encode(v) * 0.5);
        assert_eq!(f32s(&bytes)[..3], expected);
        let (bytes, _) = convert(
            PixelFormat {
                sample: SampleType::U8,
                ..encoded
            },
            options(false),
            &src,
            0,
            0,
        );
        let expected = expected.map(|v| (v * 255.0).round() as u8);
        assert_eq!(bytes, [expected[0], expected[1], expected[2], 128]);
    }

    #[test]
    fn integer_targets_clip_and_count() {
        let format = target(
            ChannelLayout::Rgb,
            SampleType::U16,
            ColorSpace::LINEAR_REC2020,
            AlphaMode::Straight,
        );
        // Within half a step of the range: rounding, not clipping.
        let half_step = 0.4 / 65535.0;
        let src = [2.0, -0.5, 0.5, 1.0, 1.0 + half_step, -half_step, 3.0, 1.0];
        let (bytes, report) = convert(format, options(false), &src, 0, 0);
        assert_eq!(u16s(&bytes), [65535, 0, 32768, 65535, 0, 65535]);
        assert_eq!(report.clipped_high, 2);
        assert_eq!(report.clipped_low, 1);

        // Out of gamut: pure Rec.2020 green is negative in sRGB red and blue.
        let (bytes, report) = convert(
            PixelFormat::RGBA8_SRGB,
            options(false),
            &[0.0, 1.0, 0.0, 1.0],
            0,
            0,
        );
        assert_eq!(bytes[0], 0);
        assert_eq!(bytes[3], 255);
        assert_eq!((report.clipped_low, report.clipped_high), (2, 1));
    }

    #[test]
    fn float_targets_keep_finite_values() {
        let format = target(
            ChannelLayout::Rgba,
            SampleType::F32,
            ColorSpace::LINEAR_REC2020,
            AlphaMode::Premultiplied,
        );
        let src = [1e5, -3.0, 1e30, 1.0];
        let (bytes, report) = convert(format, options(false), &src, 0, 0);
        assert_eq!(f32s(&bytes), src);
        assert_eq!(report, ConversionReport::default());
        // Un-premultiplying beyond the f32 range saturates, counted.
        let straight = PixelFormat {
            alpha: AlphaMode::Straight,
            ..format
        };
        let (bytes, report) = convert(straight, options(false), &[3e38, 1.0, 0.0, 0.5], 0, 0);
        assert_eq!(f32s(&bytes), [f32::MAX, 2.0, 0.0, 0.5]);
        assert_eq!(report.clipped_high, 1);
    }

    #[test]
    fn non_finite_inputs_are_mapped_and_counted() {
        let src = [
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            1.0,
            0.5,
            0.5,
            0.5,
            f32::NAN,
        ];
        // Integer: NaN → 0, ±inf clipped.
        let (bytes, report) = convert(PixelFormat::RGBA8_SRGB, options(false), &src, 0, 0);
        assert_eq!(bytes[..4], [0, 255, 0, 255]);
        // NaN alpha is 0: color 0 in a straight target.
        assert_eq!(bytes[4..], [0, 0, 0, 0]);
        assert_eq!(report.non_finite, 4);
        // Float, same primaries: infinities are kept, NaN → 0.
        let format = target(
            ChannelLayout::Rgba,
            SampleType::F32,
            ColorSpace::LINEAR_REC2020,
            AlphaMode::Premultiplied,
        );
        let (bytes, report) = convert(format, options(false), &src, 0, 0);
        assert_eq!(
            f32s(&bytes),
            [
                0.0,
                f32::INFINITY,
                f32::NEG_INFINITY,
                1.0,
                0.5,
                0.5,
                0.5,
                0.0
            ]
        );
        assert_eq!(report.non_finite, 4);
        // Through a matrix, the output stays free of NaN.
        let (bytes, _) = convert(
            PixelFormat {
                color_space: ColorSpace::LINEAR_SRGB,
                ..format
            },
            options(false),
            &src,
            0,
            0,
        );
        assert!(f32s(&bytes).iter().all(|v| !v.is_nan()));
    }

    #[test]
    fn half_float_overflow_is_counted_and_kept_finite() {
        let format = target(
            ChannelLayout::Rgba,
            SampleType::F16,
            ColorSpace::LINEAR_REC2020,
            AlphaMode::Premultiplied,
        );
        let src = [1e5, -70000.0, 65504.0, 1.0, f32::INFINITY, 0.5, 0.0, 1.0];
        let (bytes, report) = convert(format, options(false), &src, 0, 0);
        let halves: Vec<f32> = u16s(&bytes)
            .into_iter()
            .map(crate::color::f16_to_f32)
            .collect();
        assert_eq!(
            halves,
            [
                65504.0,
                -65504.0,
                65504.0,
                1.0,
                f32::INFINITY,
                0.5,
                0.0,
                1.0
            ]
        );
        assert_eq!(report.half_overflow, 2);
        assert_eq!(report.non_finite, 1);
    }

    #[test]
    fn byte_order_is_explicit() {
        let format = target(
            ChannelLayout::Rgb,
            SampleType::U16,
            ColorSpace::LINEAR_REC2020,
            AlphaMode::Straight,
        );
        let src = grays(&[258.0 / 65535.0]);
        let big = ConvertOptions {
            dither: false,
            big_endian: true,
        };
        assert_eq!(convert(format, big, &src, 0, 0).0, [1, 2, 1, 2, 1, 2]);
        assert_eq!(
            convert(format, options(false), &src, 0, 0).0,
            [2, 1, 2, 1, 2, 1]
        );
    }

    #[test]
    fn dither_never_moves_values_on_a_level() {
        for tf in NAMED {
            let format = target(
                ChannelLayout::Rgb,
                SampleType::U8,
                rec2020(tf),
                AlphaMode::Straight,
            );
            let src = grays(&decode_levels(tf, SampleType::U8));
            // 64 rows × 256 pixels cover every offset of the noise table.
            for y in 0..64 {
                let (bytes, _) = convert(format, options(true), &src, 3, y);
                for (k, px) in bytes.as_chunks::<3>().0.iter().enumerate() {
                    assert_eq!(*px, [k as u8; 3], "{tf:?} code {k} row {y}");
                }
            }
        }
    }

    #[test]
    fn dither_spreads_values_between_levels() {
        // A value a quarter of the way from code 100 to 101 (in the encoded domain): about a
        // quarter of the pixels round up, the mean is preserved.
        let tf = TransferFunction::Srgb;
        let format = target(
            ChannelLayout::Rgb,
            SampleType::U8,
            rec2020(tf),
            AlphaMode::Straight,
        );
        let src = grays(&vec![tf.decode(100.25 / 255.0); 64]);
        let mut sum = 0u32;
        for y in 0..64 {
            let (bytes, _) = convert(format, options(true), &src, 0, y);
            assert!(bytes.iter().all(|&b| b == 100 || b == 101));
            sum += bytes.iter().step_by(3).map(|&b| u32::from(b)).sum::<u32>();
        }
        let mean = f64::from(sum) / 4096.0;
        assert!((mean - 100.25).abs() < 0.02, "{mean}");
    }

    #[test]
    fn dither_depends_only_on_absolute_position() {
        let tf = TransferFunction::Srgb;
        let format = target(
            ChannelLayout::Rgba,
            SampleType::U8,
            rec2020(tf),
            AlphaMode::Straight,
        );
        let src: Vec<f32> = (0..300)
            .flat_map(|i| {
                let v = i as f32 / 299.0;
                [v * 0.8, v * 0.5, v * 0.2, 0.9]
            })
            .collect();
        let (whole, _) = convert(format, options(true), &src, 1000, 77);
        let (left, _) = convert(format, options(true), &src[..123 * 4], 1000, 77);
        let (right, _) = convert(format, options(true), &src[123 * 4..], 1123, 77);
        assert_eq!(whole, [left, right].concat());
        // And it really does something.
        let (plain, _) = convert(format, options(false), &src, 1000, 77);
        assert_ne!(whole, plain);
    }

    #[test]
    fn eight_bit_srgb_survives_the_working_space() {
        // Decoded as import does, taken to the working space and back: same bytes, for all
        // 256 values, at several alphas, with and without dither.
        let levels = decode_levels(TransferFunction::Srgb, SampleType::U8);
        let to_working = ColorSpace::LINEAR_SRGB.matrix_to(&WORKING_SPACE);
        for alpha_code in [255u8, 254, 128, 3, 1] {
            let alpha = f32::from(alpha_code) / 255.0;
            let expected: Vec<u8> = (0..=255u8)
                .flat_map(|k| [k, 255 - k, k.wrapping_mul(7), alpha_code])
                .collect();
            let src: Vec<f32> = expected
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|px| {
                    let linear = [0, 1, 2].map(|c| f64::from(levels[px[c] as usize] * alpha));
                    let [r, g, b] = mat_vec(&to_working, linear);
                    [r as f32, g as f32, b as f32, alpha]
                })
                .collect();
            for dither in [false, true] {
                for y in [0, 1, 63] {
                    let (bytes, report) =
                        convert(PixelFormat::RGBA8_SRGB, options(dither), &src, 5, y);
                    assert_eq!(bytes, expected, "alpha {alpha_code} dither {dither}");
                    assert_eq!(report, ConversionReport::default());
                }
            }
        }
    }

    #[test]
    fn reports_merge() {
        let mut a = ConversionReport {
            clipped_high: 1,
            clipped_low: 2,
            non_finite: 3,
            half_overflow: 4,
        };
        a.merge(&a.clone());
        assert_eq!(
            a,
            ConversionReport {
                clipped_high: 2,
                clipped_low: 4,
                non_finite: 6,
                half_overflow: 8,
            }
        );
    }
}
